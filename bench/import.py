"""Convert downloaded public benchmarks into concord's single case format.

usage: python3 -I bench/import.py <benchmarks-dir> <cases-dir> [source...]

Every case is one directory <cases-dir>/<source>/<id>/:
  case.json   {"source", "id", "lang", "category", "agents": [names],
               "base": {"tree": "base"} | {"git": url, "commit": sha},
               "check": {"expected": "expected"} | {"cooperbench": {...}}}
  base/                 the starting files (tree bases)
  agents/<name>/        files that agent's change leaves (overlay), or
  agents/<name>.patch   that agent's change as a patch
  expected/             what the merged files must look like (merge checks)

Sources (read-only input, never executed):
  ConGra        congra-data/raw_datasets/<Lang>/<project>/conflict_files_<n>/{a,b,base,resolved}
  ConflictBench ConflictBench/Resource/merge_scenarios/<project>/<sha>/{base,left,right,child}
  CooperBench   cooperbench-dataset/<repo>_task/task<id>/feature<n>/{feature,tests}.patch and the
                task's run_tests.sh (run on the host by check_cooperbench.sh)
"""

import itertools
import json
import re
import shutil
import sys
from pathlib import Path


def write_case(out: Path, case: dict) -> None:
    out.mkdir(parents=True, exist_ok=True)
    (out / "case.json").write_text(json.dumps(case, indent=1))


def copy_tree(src: Path, dst: Path) -> None:
    """Copy regular files only: symlinks in untrusted data may point anywhere."""
    if src.is_dir():
        shutil.copytree(src, dst, dirs_exist_ok=True, ignore=skip_symlinks)


def skip_symlinks(directory: str, names: list[str]) -> list[str]:
    return [n for n in names if (Path(directory) / n).is_symlink()]


def congra(src: Path, cases: Path) -> int:
    root = src / "congra-data"
    categories = {}
    for meta in (root / "congra_full_datasets").glob("*/*/meta_list.txt"):
        category = meta.parent.name
        for line in meta.read_text(errors="replace").splitlines():
            path = line.split(":", 1)[0]
            parts = path.split("/")
            if len(parts) >= 3:
                categories["/".join(parts[:3])] = category
    count = 0
    for pair in sorted((root / "raw_datasets").glob("*/*/conflict_files_*")):
        lang, project = pair.parent.parent.name, pair.parent.name
        names = {p.name for p in (pair / "base").iterdir()} if (pair / "base").is_dir() else set()
        if not names or not all((pair / side).is_dir() for side in ("a", "b", "resolved")):
            continue
        key = f"{lang}/{project}/{pair.name}"
        out = cases / "congra" / f"{lang}-{project}-{pair.name}"
        copy_tree(pair / "base", out / "base")
        copy_tree(pair / "a", out / "agents" / "a")
        copy_tree(pair / "b", out / "agents" / "b")
        copy_tree(pair / "resolved", out / "expected")
        write_case(out, {
            "source": "congra",
            "id": key,
            "lang": lang.lower(),
            "category": categories.get(key, "unclassified"),
            "agents": ["a", "b"],
            "base": {"tree": "base"},
            "check": {"expected": "expected"},
        })
        count += 1
    return count


def conflictbench(src: Path, cases: Path) -> int:
    root = src / "ConflictBench" / "Resource" / "merge_scenarios"
    count = 0
    for scenario in sorted(root.glob("*/*")):
        if not all((scenario / side).is_dir() for side in ("base", "left", "right", "child")):
            continue
        project, sha = scenario.parent.name, scenario.name
        out = cases / "conflictbench" / f"{project}-{sha[:12]}"
        copy_tree(scenario / "base", out / "base")
        copy_tree(scenario / "left", out / "agents" / "left")
        copy_tree(scenario / "right", out / "agents" / "right")
        copy_tree(scenario / "child", out / "expected")
        write_case(out, {
            "source": "conflictbench",
            "id": f"{project}/{sha}",
            "lang": "java",
            "category": "java",
            "agents": ["left", "right"],
            "base": {"tree": "base"},
            "check": {"expected": "expected"},
        })
        count += 1
    return count


CLONE = re.compile(r"git clone\s+(\S+)\s+repo.*?git checkout\s+([0-9a-f]{7,40})", re.S)


def cooperbench(src: Path, cases: Path) -> int:
    root = src / "cooperbench-dataset"
    count = 0
    for task in sorted(root.glob("*_task/task*")):
        dockerfile = task / "Dockerfile"
        match = CLONE.search(dockerfile.read_text()) if dockerfile.is_file() else None
        if not match:
            continue
        url, commit = match.groups()
        repo = task.parent.name.removesuffix("_task")
        features = sorted(
            (f for f in task.glob("feature*") if (f / "feature.patch").is_file()),
            key=lambda f: int(f.name.removeprefix("feature")),
        )
        for f1, f2 in itertools.combinations(features, 2):
            out = cases / "cooperbench" / f"{repo}-{task.name}-{f1.name}-{f2.name}"
            agents = [f1.name, f2.name]
            (out / "agents").mkdir(parents=True, exist_ok=True)
            (out / "tests").mkdir(parents=True, exist_ok=True)
            for f in (f1, f2):
                shutil.copy(f / "feature.patch", out / "agents" / f"{f.name}.patch")
                shutil.copy(f / "tests.patch", out / "tests" / f"{f.name}.patch")
            shutil.copy(task / "run_tests.sh", out / "tests" / "run_tests.sh")
            shutil.copy(task / "combined.patch", out / "tests" / "oracle.diff")
            write_case(out, {
                "source": "cooperbench",
                "id": f"{repo}/{task.name}/{f1.name}+{f2.name}",
                "lang": repo,
                "category": repo,
                "agents": agents,
                "base": {"git": url, "commit": commit},
                "check": {"cooperbench": {
                    "runner": "tests/run_tests.sh",
                    "oracle": "tests/oracle.diff",
                    "task": f"{repo}/{task.name}",
                    "tests": [f"tests/{a}.patch" for a in agents],
                }},
            })
            count += 1
    return count


def main() -> None:
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    src, cases, only = Path(sys.argv[1]), Path(sys.argv[2]), set(sys.argv[3:])
    for name, convert in (("congra", congra), ("conflictbench", conflictbench),
                          ("cooperbench", cooperbench)):
        if only and name not in only:
            continue
        shutil.rmtree(cases / name, ignore_errors=True)
        print(f"{name}: {convert(src, cases)} cases", flush=True)


if __name__ == "__main__":
    main()
