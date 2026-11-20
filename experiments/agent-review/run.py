"""Falsification run: does a tool-less agent reviewer catch the cart scenario's bugs without tests?

See README.md for the cases and the rule fixed before the run.
"""

import pathlib
import subprocess
import sys
import tempfile
from collections import Counter
from concurrent.futures import ThreadPoolExecutor

HERE = pathlib.Path(__file__).resolve().parent
CART = HERE.parent / "cart-scenario"
WORKDIR = pathlib.Path("/Volumes/Data/concord-review-runs")
PYTHON = "python3.12"

FINAL = (CART / "evidence/v2-final-cart.py").read_text()
TESTS = (CART / "evidence/v2-final-test_cart.py").read_text()
OK_RECEIPT = '''    amount = subtotal(items)
    shown = int(amount) if amount == int(amount) else amount
    return f"{item_count(items)} items, total {shown}"'''
assert OK_RECEIPT in FINAL


BASE = (CART / "base/cart.py").read_text()
HEAD_A = BASE.replace("def total(items):", "def subtotal(items):")
HEAD_B = FINAL[: FINAL.index("\n\ndef receipt")] + "\n"


def receipt(body: str) -> str:
    return FINAL.replace(OK_RECEIPT, body)


# Recorded in the LLM run (evidence/v2-board-log.txt).
NOTES_RECORDED = [
    "total#1 by agent-a: cart.subtotal(items) -> int|float: sum of price * qty over items "
    "(formerly total; body unchanged)",
    "total#2 by agent-b: subtotal(items, discount_pct=0) -> int|float: sum of price * qty over "
    "items, times (1 - discount_pct / 100); discount_pct is a percent 0-100; returns a number "
    "(e.g. subtotal(ITEMS, discount_pct=20) == 20)",
]
NOTES_PRECISE = [
    "total#1 by agent-a: subtotal(items) -> int: sum of price * qty; CHANGED: renamed from total",
    "total#2 by agent-b: subtotal(items, discount_pct=0) -> float; CHANGED: now always float, "
    "format before printing",
]

CRITERIA = {
    "agent-a": "total is renamed to subtotal with unchanged behavior; nothing in the project "
    "calls total any more.",
    "agent-b": "subtotal(items, discount_pct=0) returns gross * (1 - discount_pct / 100), gross "
    "being the sum of price * qty; subtotal(ITEMS, discount_pct=20) == 20.",
    "agent-c": 'receipt(items) returns "<count> items, total <total>" reusing the existing '
    'functions; receipt(ITEMS) == "3 items, total 25".',
}

CASES = {
    "bug-float-vague": (receipt('    return f"{item_count(items)} items, total {subtotal(items)}"'),
                        NOTES_RECORDED, "reject"),
    "bug-float-precise": (receipt('    return f"{item_count(items)} items, total {subtotal(items)}"'),
                          NOTES_PRECISE, "reject"),
    "bug-rename": (receipt('    return f"{item_count(items)} items, total {total(items)}"'),
                   NOTES_RECORDED, "reject"),
    "ok-v2": (FINAL, NOTES_RECORDED, "land"),
    "bug-latent-g": (receipt('    return f"{item_count(items)} items, total {subtotal(items):g}"'),
                     NOTES_RECORDED, "reject"),
    "ok-alt": (receipt('    t = subtotal(items)\n    return f"{item_count(items)} items, total {int(t) if t == int(t) else t}"'),
               NOTES_RECORDED, "land"),
}

PROMPT = """You are the reviewer at concord's landing gate. Several coding agents worked in parallel \
on one repository; their work has been merged into the combined result below. Nothing lands unless \
you approve it. You cannot run code: judge by reading.

Two checks:
1. Each agent's done-criteria describe that agent's own work: check them against that agent's own \
result below. A criterion is met only if the code, as written, satisfies it exactly.
2. Check the combined result against every contract change of the round: is every caller of a \
changed function adapted to the new contract?

## Done-criteria (stated by each agent before it started)
{criteria}

## Each agent's own result (cart.py at the agent's last change)

### agent-a
```python
{head_a}```

### agent-b
```python
{head_b}```

### agent-c
Its own result is the combined result below.

## Contract changes this round
{notes}

## Combined result

### cart.py
```python
{cart}```

### test_cart.py
```python
{tests}```

Answer with your reasoning, then a final line that is exactly one of:
LAND
REJECT <agent>: <reason>
"""


HIDDEN = """

def test_hidden_large_total():
    assert cart.receipt([{"price": 1234567, "qty": 1}]) == "1 items, total 1234567"


def test_hidden_fractional_total():
    assert cart.receipt([{"price": 12.5, "qty": 1}]) == "1 items, total 12.5"
"""


def truth(cart: str) -> str:
    """Ground truth: the four tests plus hidden checks the reviewer never sees."""
    with tempfile.TemporaryDirectory(dir=WORKDIR) as d:
        (pathlib.Path(d) / "cart.py").write_text(cart)
        (pathlib.Path(d) / "test_cart.py").write_text(TESTS + HIDDEN)
        r = subprocess.run([PYTHON, "-m", "pytest", "-q", "-p", "no:cacheprovider"], cwd=d,
                           capture_output=True, text=True)
        return "land" if r.returncode == 0 else "reject"


def review(model: str, case: str) -> tuple[str, str, str, str]:
    cart, notes, _ = CASES[case]
    prompt = PROMPT.format(
        criteria="\n".join(f"- {a}: {c}" for a, c in CRITERIA.items()),
        notes="\n".join(f"- {n}" for n in notes),
        cart=cart, tests=TESTS, head_a=HEAD_A, head_b=HEAD_B)
    r = subprocess.run(
        ["claude", "-p", "--setting-sources", "project", "--strict-mcp-config",
         "--no-session-persistence", "--model", model, "--tools", ""],
        input=prompt, cwd=WORKDIR, capture_output=True, text=True, timeout=600)
    lines = [l.strip() for l in r.stdout.splitlines() if l.strip()]
    last = lines[-1] if lines else f"(no output) {r.stderr.strip()[:200]}"
    verdict = "land" if last == "LAND" else "reject" if last.startswith("REJECT") else "unparsed"
    return model, case, verdict, last


def main() -> int:
    runs = int(sys.argv[1]) if len(sys.argv) > 1 else 5
    models = sys.argv[2:] or ["haiku", "sonnet"]
    WORKDIR.mkdir(exist_ok=True)
    for case, (cart, _, expected) in CASES.items():
        actual = truth(cart)
        if actual != expected:
            print(f"case {case} is labeled {expected} but pytest says {actual}")
            return 2
    print("ground truth matches the labels for all cases")

    jobs = [(m, c) for m in models for c in CASES for _ in range(runs)]
    with ThreadPoolExecutor(max_workers=6) as pool:
        results = list(pool.map(lambda j: review(*j), jobs))

    out = [f"# agent-review results\n\n{runs} runs per case; rule in README.md.\n"]
    survived = {}
    for m in models:
        out.append(f"\n## {m}\n\n| Case | Expected | land | reject | unparsed | Pass |\n|---|---|---|---|---|---|")
        ok = True
        for case, (_, _, expected) in CASES.items():
            c = Counter(v for mm, cc, v, _ in results if mm == m and cc == case)
            passed = c[expected] >= max(1, runs - 1)
            if case == "bug-float-vague" and c["land"] >= 2:
                passed = False
            if case == "bug-latent-g":
                passed = True  # run 3 rule: reported, not gated (the four tests land it)
            ok &= passed
            out.append(f"| `{case}` | {expected} | {c['land']} | {c['reject']} | {c['unparsed']} | "
                       f"{'yes' if passed else 'NO'} |")
        survived[m] = ok
        out.append(f"\nDesign survives for {m}: **{'yes' if ok else 'no'}**")
    out.append("\n## Final lines\n")
    for m, case, verdict, last in sorted(results):
        out.append(f"- {m} `{case}` {verdict}: {last[:220]}")
    (HERE / "results.md").write_text("\n".join(out) + "\n")
    print("\n".join(out[: 3 + len(models) * (len(CASES) + 4)]))
    return 0 if all(survived.values()) else 1


if __name__ == "__main__":
    sys.exit(main())
