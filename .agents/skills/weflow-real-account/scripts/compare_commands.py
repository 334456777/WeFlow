"""Runs the same weflow commands with two binaries and compares their JSON answers. Prints counts only.

Usage: compare_commands.py <old binary> <new binary> <WEFLOW_HOME> <commands file>

The commands file holds one command per line, its arguments split like a shell would (`analytics rankings --limit 10`);
blank lines and lines starting with `#` are skipped. Each command runs with `--json` and English output. Keys that differ
between any two runs (when something was made, how long it took) are left out, and numbers keep their type, so `1` and
`1.0` differ. Only line numbers, exit codes, counts and times are printed, never an answer or a command's arguments,
so the output can be shared even when the commands name a conversation.
"""
import json
import os
import shlex
import subprocess
import sys
import time

VOLATILE = {"updatedAt", "createdAt", "generatedAt", "elapsed_ms", "elapsedMs", "diagnostics"}


def strip(value):
    """The answer without the keys that change on every run."""
    if isinstance(value, dict):
        return {k: strip(v) for k, v in value.items() if k not in VOLATILE}
    if isinstance(value, list):
        return [strip(v) for v in value]
    return value


def run(binary, args, env):
    """Exit code, comparable answer and seconds taken."""
    start = time.monotonic()
    done = subprocess.run([binary, "--json", *args], env=env, capture_output=True)
    took = time.monotonic() - start
    try:
        answer = json.dumps(strip(json.loads(done.stdout)), sort_keys=True)
    except ValueError:
        answer = done.stdout
    return done.returncode, answer, took


def main():
    if len(sys.argv) != 5:
        sys.exit(__doc__)
    old, new, home, commands_file = sys.argv[1:]
    env = dict(os.environ, WEFLOW_HOME=home, WEFLOW_LANG="en")
    with open(commands_file, encoding="utf-8") as f:
        commands = [(n, shlex.split(line)) for n, line in enumerate(f, 1)
                    if line.strip() and not line.lstrip().startswith("#")]
    same = differ = 0
    spent = [0.0, 0.0]
    for n, args in commands:
        a, b = run(old, args, env), run(new, args, env)
        spent[0] += a[2]
        spent[1] += b[2]
        if a[:2] == b[:2] and a[0] == 0:
            same += 1
        else:
            differ += 1
            print(f"  line {n} differs (exit {a[0]} / {b[0]})")
    print(f"{len(commands)} commands: {same} identical, {differ} differ; "
          f"time old {spent[0]:.1f}s, new {spent[1]:.1f}s")
    sys.exit(1 if differ else 0)


if __name__ == "__main__":
    main()
