"""Lists the comments of the repository's tracked source files in numbered chunk files, to be read one after another.

Usage: list_comments.py <out dir> [<chunk KiB>]

Each chunk (64 KiB by default) holds a `== <path>` line for each file, then one line for each comment: the line it
starts on and its text, the lines of a comment joined into one. Line, doc, block and HTML comments, Python docstrings
and comments after code are listed. Markdown is not, since those files are read whole, and vendored and generated
files are left out. The chunk files already in <out dir> are removed first.

The parsing goes line by line and is not a tokenizer: a comment marker inside a string can slip in, and a comment
after code is missed when the code before it holds an odd number of quotes.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

PATTERNS = ["*.rs", "*.ts", "*.tsx", "*.js", "*.mjs", "*.css", "*.html", "*.py", "*.sh", "*.ps1", "*.yml", "*.yaml",
            "*.toml", "Makefile"]
SKIPPED = ("/vendor/", "contract.gen.ts", ".zh.html", ".en.html", "web/contract/")
HASH = {".py", ".sh", ".ps1", ".yml", ".yaml", ".toml", ""}
QUOTES = {".rs": '"', ".css": "\"'", ".html": '"', ".ts": "\"'`", ".tsx": "\"'`", ".js": "\"'`", ".mjs": "\"'`"}
BLOCKS = {".py": [('"""', '"""')], ".ps1": [("<#", "#>")], ".html": [("<!--", "-->"), ("/*", "*/")]}


def after_code(line: str, marker: str, quotes: str) -> str | None:
    """The text of a comment that follows code on this line, if there is one."""
    start = 0
    while (i := line.find(marker, start)) > 0:
        before = line[:i]
        if before[-1].isspace() and before.strip() and all(before.count(q) % 2 == 0 for q in quotes):
            return line[i + len(marker):].strip()
        start = i + 1
    return None


def comments(path: str, text: str) -> list[tuple[int, str]]:
    """Every comment of one file: the line it starts on and its text."""
    ext = Path(path).suffix
    hashed = ext in HASH
    marker = "#" if hashed else "//"
    quotes = QUOTES.get(ext, "\"'")
    blocks = BLOCKS.get(ext, [] if hashed else [("/*", "*/")])
    found: list[tuple[int, str]] = []
    parts: list[str] = []
    first = 0
    closer = None

    def flush() -> None:
        joined = " ".join(p for p in parts if p).strip()
        if joined:
            found.append((first, joined))
        parts.clear()

    for number, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if closer:
            end = stripped.find(closer)
            body = stripped if end < 0 else stripped[:end]
            parts.append(body.lstrip("*").strip())
            if end >= 0:
                closer = None
                flush()
            continue
        opened = next(((o, c) for o, c in blocks if stripped.startswith(o) or stripped.startswith("r" + o)), None)
        if opened:
            flush()
            opener, end_marker = opened
            first = number
            body = stripped[stripped.find(opener) + len(opener):].lstrip("*!")
            end = body.find(end_marker)
            parts.append(body if end < 0 else body[:end])
            if end < 0:
                closer = end_marker
            else:
                flush()
            continue
        is_line_comment = stripped.startswith(marker) and not (hashed and stripped[1:2] in ("!", "["))
        if is_line_comment:
            if not parts:
                first = number
            parts.append(stripped[len(marker):].lstrip("/!").strip())
            continue
        flush()
        trailing = after_code(line, marker, quotes)
        if trailing:
            found.append((number, trailing))
    flush()
    return found


def main() -> None:
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    out = Path(sys.argv[1])
    limit = int(sys.argv[2] if len(sys.argv) == 3 else 64) * 1024
    root = Path(subprocess.run(["git", "rev-parse", "--show-toplevel"], capture_output=True, text=True,
                               check=True).stdout.strip())
    listed = subprocess.run(["git", "ls-files", "-z", *PATTERNS], cwd=root, capture_output=True, text=True,
                            check=True).stdout.split("\0")
    files = [f for f in listed if f and not any(s in f for s in SKIPPED)]

    chunks: list[str] = []
    rows: list[str] = []
    size = 0
    total = 0
    for path in files:
        try:
            text = (root / path).read_text(encoding="utf-8")
        except (UnicodeDecodeError, FileNotFoundError):
            continue
        header = f"== {path}\n"
        pending = header
        for number, comment in comments(path, text):
            row = f"{number}: {comment}\n"
            if rows and size + len(((pending or "") + row).encode()) > limit:
                chunks.append("".join(rows))
                rows, size, pending = [], 0, header
            if pending:
                rows.append(pending)
                size += len(pending.encode())
                pending = None
            rows.append(row)
            size += len(row.encode())
            total += 1
    if rows:
        chunks.append("".join(rows))

    out.mkdir(parents=True, exist_ok=True)
    for old in out.glob("chunk-*.txt"):
        old.unlink()
    for index, chunk in enumerate(chunks):
        (out / f"chunk-{index:02}.txt").write_text(chunk, encoding="utf-8", newline="\n")
    print(f"{total} comments from {len(files)} files in {len(chunks)} chunks under {out}")


if __name__ == "__main__":
    main()
