"""Gate check for merge cases: the combined files must match the developer's resolution.

usage (run by bench/run.py in the combined checkout): python3 -I check_tree.py <expected-dir>
Match = same lines after trimming trailing whitespace and dropping blank lines, so formatting
noise does not count, content does. Exit 0 = match, 1 = mismatch (first difference printed).
"""

import sys
from pathlib import Path


def lines(path: Path) -> list[str]:
    text = path.read_bytes().decode("utf-8", errors="replace")
    return [line.rstrip() for line in text.splitlines() if line.strip()]


def main() -> int:
    expected = Path(sys.argv[1])
    for want in sorted(p for p in expected.rglob("*") if p.is_file() and not p.is_symlink()):
        rel = want.relative_to(expected)
        got = Path(rel)
        if not got.is_file():
            print(f"MISMATCH {rel}: missing")
            return 1
        a, b = lines(got), lines(want)
        if a != b:
            at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
            print(f"MISMATCH {rel}: line {at + 1}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
