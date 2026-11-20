"""The one big test: every converted benchmark case through concord, plus a plain-git baseline.

usage: python3 -I bench/run.py <cases-dir> <out-dir> [--source S] [--sample N] [--jobs J]
                              [--concord PATH]

Per case: a git repo at the case's base on `main`, `concord init <agents>`, each agent applies its
change in its own workspace and runs `plan`/`finish`; the last finisher is asked to review the
combined result (exit 12), runs the check in its checkout and answers `land` or `reject`:
  merge cases       the combined files must match the developer's resolution (check_tree.py)
  cooperbench cases the combined result must pass every feature's tests (check_cooperbench.sh,
                    the task's own run_tests.sh on the host; caches under <out-dir>/cache)
Verdicts: pass | conflict (combining left conflicts) | wrong (check failed) | error (harness/concord)
| env (the dataset's own resolution fails the check here: the environment, not concord).
Merge cases also get a baseline verdict from `git merge-file` on the same three versions.

Results append to <out-dir>/results.jsonl (one line per case; finished cases are skipped on a
re-run, so an interrupted run resumes). <out-dir>/summary.md is rewritten at the end.
"""

import argparse
import collections
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

BENCH = Path(__file__).resolve().parent
ROOT = BENCH.parent
CASE_TIMEOUT = 1800


# Every git and concord call sees only this config: the machine's own git hooks, aliases and
# identity must not change results. Set by main() to a file in the out dir.
ENV = dict(os.environ)
# Scripted agents have no coding-agent session: without dropping these, concord would record the
# session running the bench for every agent, and a rejection would continue that session.
SESSION_VARS = ("CLAUDE_CODE_SESSION_ID", "CODEX_THREAD_ID", "CODEX_SESSION_ID")


def isolated_env(out: Path) -> dict:
    config = out / "gitconfig"
    config.write_text("[user]\n\tname = bench\n\temail = bench@example.invalid\n"
                      "[init]\n\tdefaultBranch = main\n")
    env = dict(os.environ, GIT_CONFIG_GLOBAL=str(config), GIT_CONFIG_NOSYSTEM="1",
               BENCH_CACHE=str(out / "cache"))
    for var in ("GIT_DIR", *SESSION_VARS):
        env.pop(var, None)
    return env


def sh(args, cwd, timeout=CASE_TIMEOUT):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                          env=ENV, errors="replace")


def git(cwd, *args):
    out = sh(["git", *args], cwd)
    if out.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {out.stderr.strip()[:300]}")
    return out.stdout.strip()


def overlay(src: Path, dst: Path) -> None:
    for f in src.rglob("*"):
        if f.is_file() and not f.is_symlink():
            target = dst / f.relative_to(src)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(f, target)


mirror_locks = collections.defaultdict(threading.Lock)


def mirror(url: str, repos: Path) -> Path:
    """One bare clone per upstream repo, shared by every case that starts from it."""
    path = repos / (url.rstrip("/").split("/")[-1].removesuffix(".git") + ".git")
    with mirror_locks[str(path)]:
        if not path.exists():
            repos.mkdir(parents=True, exist_ok=True)
            out = sh(["git", "clone", "--bare", "--quiet", url, str(path)], repos, timeout=3600)
            if out.returncode != 0:
                raise RuntimeError(f"clone {url}: {out.stderr.strip()[:300]}")
    return path


def make_repo(case_dir: Path, case: dict, repo: Path, repos: Path) -> str:
    """The case's base as commit on `main` in `repo`; returns the commit id."""
    base = case["base"]
    if "tree" in base:
        repo.mkdir(parents=True)
        overlay(case_dir / base["tree"], repo)
        git(repo, "init", "--quiet", "-b", "main")
        git(repo, "add", "-A")
        git(repo, "commit", "--quiet", "--allow-empty", "-m", "base")
    else:
        source = mirror(base["git"], repos)
        git(repo.parent, "clone", "--quiet", "--shared", "--no-checkout", str(source), str(repo))
        git(repo, "checkout", "--quiet", "-B", "main", base["commit"])
    return git(repo, "rev-parse", "HEAD")


def check_command(case_dir: Path, case: dict, base_commit: str) -> list[str]:
    check = case["check"]
    if "expected" in check:
        return [sys.executable, "-I", str(BENCH / "check_tree.py"), str(case_dir / check["expected"])]
    return ["bash", str(BENCH / "check_cooperbench.sh"), str(case_dir), base_commit]


def apply_change(case_dir: Path, agent: str, workspace: Path) -> None:
    tree = case_dir / "agents" / agent
    patch = case_dir / "agents" / f"{agent}.patch"
    if tree.is_dir():
        overlay(tree, workspace)
    elif patch.is_file():
        out = sh(["git", "apply", "--whitespace=nowarn", str(patch)], workspace)
        if out.returncode != 0:
            out = sh(["patch", "-p1", "--forward", "--silent", "-i", str(patch)], workspace)
            if out.returncode != 0:
                raise RuntimeError(f"apply {agent}: {(out.stdout + out.stderr).strip()[:300]}")


def through_concord(case_dir: Path, case: dict, work: Path, repos: Path, concord: str) -> dict:
    repo = work / "repo"
    base_commit = make_repo(case_dir, case, repo, repos)
    agents = case["agents"]
    run = lambda args, cwd: sh([concord, *args], cwd)  # noqa: E731
    out = run(["init", *agents], repo)
    if out.returncode != 0:
        return {"verdict": "error", "detail": f"init: {out.stderr.strip()[:300]}"}
    # Every agent plans before any finishes: `finish` waits for the whole team's plans.
    for agent in agents:
        out = run(["plan", agent, "-", "-", "--", f"{agent}'s change of the case is applied"],
                  work / f"repo-{agent}")
        if out.returncode != 0:
            return {"verdict": "error", "detail": f"plan {agent}: {out.stderr.strip()[:300]}"}
    for agent in agents:
        workspace = work / f"repo-{agent}"
        apply_change(case_dir, agent, workspace)
        out = run(["finish"], workspace)
        text = out.stdout + out.stderr
        if "unresolved conflicts" in text:
            return {"verdict": "conflict", "detail": last_line(text, "FAIL")}
        if out.returncode not in (0, REVIEW):
            return {"verdict": "error", "detail": f"finish {agent}: {text.strip()[-300:]}"}
    if out.returncode != REVIEW:
        return {"verdict": "error", "detail": f"no review after the last finish: {text.strip()[-300:]}"}
    # The last finisher reviews: the check stands in for reading the work against the criteria.
    checkout = next((l.split(" checked out at ", 1)[1] for l in text.splitlines()
                     if l.startswith("COMBINED ") and " checked out at " in l), None)
    if checkout is None:
        return {"verdict": "error", "detail": f"no combined checkout: {text.strip()[-300:]}"}
    check = sh(check_command(case_dir, case, base_commit), Path(checkout))
    report = check.stdout + check.stderr
    if check.returncode == 0:
        out = run(["land", "bench check passed"], workspace)
        if out.returncode != 0 or "LANDED" not in out.stdout:
            return {"verdict": "error", "detail": f"land: {(out.stdout + out.stderr).strip()[-300:]}"}
        return {"verdict": "pass"}
    run(["reject", agents[0], "bench check failed"], workspace)
    if "ENV BROKEN" in report:
        return {"verdict": "env", "detail": last_line(report, "ENV BROKEN")}
    return {"verdict": "wrong", "detail": last_line(report, "MISMATCH", "TESTS FAILED")}


REVIEW = 12  # concord's exit code: this agent reviews the round before it lands


def last_line(text: str, *prefixes: str) -> str:
    hits = [l for l in text.splitlines() if l.startswith(prefixes)]
    return hits[-1][:300] if hits else ""


def git_baseline(case_dir: Path, case: dict, work: Path) -> str | None:
    """`git merge-file` on the same three versions: pass | conflict | wrong (merge cases only)."""
    if "expected" not in case["check"] or len(case["agents"]) != 2:
        return None
    base, (a, b) = case_dir / case["base"]["tree"], case["agents"]
    expected = case_dir / case["check"]["expected"]
    verdict = "pass"
    for want in (p for p in expected.rglob("*") if p.is_file()):
        rel = want.relative_to(expected)
        versions = []
        for side in (case_dir / "agents" / a, base, case_dir / "agents" / b):
            f = side / rel
            versions.append(f if f.is_file() else None)
        if any(v is None for v in versions):
            return "error"
        out = sh(["git", "merge-file", "-p", *map(str, versions)], work)
        if out.returncode != 0:
            return "conflict"
        merged = work / "merged"
        merged.write_text(out.stdout)
        got = [l.rstrip() for l in out.stdout.splitlines() if l.strip()]
        exp = [l.rstrip() for l in want.read_text(errors="replace").splitlines() if l.strip()]
        if got != exp:
            verdict = "wrong"
    return verdict


def run_case(case_dir: Path, scratch: Path, repos: Path, concord: str) -> dict:
    case = json.loads((case_dir / "case.json").read_text())
    started = time.time()
    work = Path(tempfile.mkdtemp(dir=scratch))
    result = {k: case[k] for k in ("source", "id", "lang", "category")}
    try:
        result.update(through_concord(case_dir, case, work, repos, concord))
    except subprocess.TimeoutExpired:
        result.update(verdict="error", detail="timeout")
    except Exception as e:  # noqa: BLE001 - every failure is a recorded verdict, not a crash
        result.update(verdict="error", detail=str(e)[:300])
    try:
        result["git"] = git_baseline(case_dir, case, work)
    except Exception as e:  # noqa: BLE001
        result["git"] = "error"
        result.setdefault("detail", str(e)[:300])
    result["seconds"] = round(time.time() - started, 1)
    shutil.rmtree(work, ignore_errors=True)
    return result


def select(cases: Path, source: str | None, sample: int | None) -> list[Path]:
    picked = []
    for src in sorted(p for p in cases.iterdir() if p.is_dir()):
        if source and src.name != source:
            continue
        dirs = sorted(p for p in src.iterdir() if (p / "case.json").is_file())
        if sample and len(dirs) > sample:
            step = len(dirs) / sample
            dirs = [dirs[int(i * step)] for i in range(sample)]
        picked += dirs
    return picked


def summarize(results: Path, summary: Path) -> str:
    rows = [json.loads(l) for l in results.read_text().splitlines() if l.strip()]
    groups = collections.defaultdict(list)
    for r in rows:
        groups[(r["source"], r["category"])].append(r)
        groups[(r["source"], "ALL")].append(r)
    groups[("ALL", "ALL")] = rows
    head = ("| source | category | cases | concord pass | conflict | wrong | error | env "
            "| git pass | git conflict | git wrong |")
    lines = [head, "|" + "---|" * 11]
    for (source, category), rs in sorted(groups.items()):
        n = len(rs)
        c = collections.Counter(r["verdict"] for r in rs)
        g = collections.Counter(r.get("git") for r in rs)
        pct = lambda k, cnt: f"{cnt[k]} ({100 * cnt[k] / n:.0f}%)"  # noqa: E731
        git_cols = (" | ".join(pct(k, g) for k in ("pass", "conflict", "wrong"))
                    if any(r.get("git") for r in rs) else "- | - | -")
        lines.append(f"| {source} | {category} | {n} | {pct('pass', c)} | {pct('conflict', c)} "
                     f"| {pct('wrong', c)} | {pct('error', c)} | {pct('env', c)} | {git_cols} |")
    text = "\n".join(lines) + "\n"
    summary.write_text(text)
    return text


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawTextHelpFormatter)
    p.add_argument("cases", type=Path)
    p.add_argument("out", type=Path)
    p.add_argument("--source")
    p.add_argument("--sample", type=int)
    p.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    p.add_argument("--concord", type=Path, default=ROOT / "target" / "release" / "concord")
    a = p.parse_args()
    a.cases, a.out, a.concord = a.cases.resolve(), a.out.resolve(), a.concord.resolve()
    if a.out.is_relative_to(ROOT):
        sys.exit(f"out dir {a.out} is inside {ROOT}: case builds would see concord's Cargo.toml")
    a.out.mkdir(parents=True, exist_ok=True)
    ENV.clear()
    ENV.update(isolated_env(a.out))
    results = a.out / "results.jsonl"
    done = set()
    if results.exists():
        done = {json.loads(l)["id"] for l in results.read_text().splitlines() if l.strip()}
    todo = [d for d in select(a.cases, a.source, a.sample)
            if json.loads((d / "case.json").read_text())["id"] not in done]
    scratch, repos = a.out / "work", a.out / "repos"
    scratch.mkdir(exist_ok=True)
    print(f"{len(todo)} cases to run ({len(done)} already done), {a.jobs} at a time", flush=True)
    lock = threading.Lock()
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool, results.open("a") as sink:
        futures = [pool.submit(run_case, d, scratch, repos, a.concord) for d in todo]
        for i, f in enumerate(concurrent.futures.as_completed(futures), 1):
            r = f.result()
            with lock:
                sink.write(json.dumps(r) + "\n")
                sink.flush()
            if i % 50 == 0 or i == len(todo):
                print(f"{i}/{len(todo)}", flush=True)
    print(summarize(results, a.out / "summary.md"))


if __name__ == "__main__":
    main()
