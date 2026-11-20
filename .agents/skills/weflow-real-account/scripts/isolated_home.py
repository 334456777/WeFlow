"""Copies a WeFlow config into an isolated WEFLOW_HOME with every profile's cache path cleared.

Usage: isolated_home.py <new home> [<config.json>]

Without <config.json> it reads the default config: weflow/config.json in the platform's config folder (%APPDATA% on
Windows, ~/Library/Application Support on macOS, $XDG_CONFIG_HOME or ~/.config elsewhere). Commands run with
WEFLOW_HOME=<new home> then read the same accounts, while the real config and caches stay untouched: the caches go
beside the copied config.
"""
import json
import os
import sys
from pathlib import Path


def default_config() -> Path:
    """The config file weflow reads when WEFLOW_HOME is not set."""
    if sys.platform == "win32":
        base = Path(os.environ["APPDATA"])
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return base / "weflow" / "config.json"


def clear_caches(node):
    """Sets every `cache_path` (or `cachePath`) to null, so each profile keeps its cache beside the new config."""
    if isinstance(node, dict):
        for key in list(node):
            if key in ("cache_path", "cachePath"):
                node[key] = None
            else:
                clear_caches(node[key])
    elif isinstance(node, list):
        for value in node:
            clear_caches(value)


def main():
    if len(sys.argv) not in (2, 3):
        sys.exit(__doc__)
    home = Path(sys.argv[1])
    source = Path(sys.argv[2]) if len(sys.argv) == 3 else default_config()
    if home.resolve() == source.parent.resolve():
        sys.exit("the new home must be another folder than the config's own")
    config = json.loads(source.read_text(encoding="utf-8"))
    clear_caches(config)
    home.mkdir(parents=True, exist_ok=True)
    (home / "config.json").write_text(json.dumps(config, ensure_ascii=False, indent=2), encoding="utf-8")
    print("isolated home ready")


if __name__ == "__main__":
    main()
