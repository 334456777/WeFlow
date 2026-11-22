"""Copies a WeFlow config into an isolated WEFLOW_HOME: every profile's cache path is cleared and what reaches beyond
the account's own files is switched off.

Usage: isolated_home.py [--weflow <weflow binary>] <new home> [<config file>]

Without <config file> it reads the config that weflow reads (`config.rs` in weflow-core): in the home folder, the file
that the `config_path` pointer names when it holds a path, else `config.json`. The home folder is WEFLOW_HOME when
that is not blank, else `weflow` in the platform's config folder (%APPDATA% on Windows, ~/Library/Application Support
on macOS, $XDG_CONFIG_HOME or ~/.config elsewhere). With --weflow the binary is asked (`weflow --json config path`)
and these rules apply only when it cannot answer. The config may be JSON or TOML, like weflow's.

Commands run with WEFLOW_HOME=<new home> then read the same accounts, while the real config and caches stay
untouched: the caches go beside the copied config. The copy is readable by its owner only on Linux and macOS (it
holds the keys), and has the settings that reach outside the account's files switched off: `serve.image_auto_download`
(hooks the running WeChat), `serve.insight` and every profile's `ai_insight_enabled` (call a model API). A check
that needs one of them turns it on in the copy.
"""
import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

POSIX = os.name == "posix"


def platform_folder():
    """`weflow` in the platform's config folder, where weflow keeps its files unless WEFLOW_HOME says otherwise."""
    if sys.platform == "win32":
        base = Path(os.environ["APPDATA"])
    elif sys.platform == "darwin":
        base = Path.home() / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    return base / "weflow"


def read_pointer(pointer):
    """The path a `config_path` pointer file holds once trimmed; None when there is no file or nothing in it."""
    try:
        text = pointer.read_text(encoding="utf-8").strip()
    except (OSError, ValueError):
        return None
    return Path(text) if text else None


def ask_weflow(binary):
    """The config file `weflow --json config path` names; None when the binary cannot say."""
    try:
        done = subprocess.run([binary, "--json", "config", "path"], capture_output=True, timeout=60)
        answer = json.loads(done.stdout)
    except (OSError, subprocess.SubprocessError, ValueError):
        return None
    path = answer.get("data") if isinstance(answer, dict) else None
    return Path(path) if done.returncode == 0 and isinstance(path, str) and path else None


def default_config(weflow=None):
    """The config file weflow reads, and a few words on how it was found (no path in them)."""
    if weflow:
        asked = ask_weflow(weflow)
        if asked:
            return asked, "named by weflow"
        print("weflow did not name its config; using the rules of this script", file=sys.stderr)
    custom = os.environ.get("WEFLOW_HOME", "")
    if custom.strip():
        folder, where = Path(custom), "WEFLOW_HOME"
    else:
        folder, where = platform_folder(), "the platform's config folder"
    chosen = read_pointer(folder / "config_path")
    if chosen:
        return chosen, f"named by config_path in {where}"
    return folder / "config.json", f"config.json in {where}"


def read_config(path):
    """The config in `path`, JSON when it starts with `{` and else TOML, as weflow reads it."""
    text = path.read_text(encoding="utf-8-sig")
    if text.lstrip().startswith("{"):
        return json.loads(text)
    try:
        import tomllib
    except ImportError:
        sys.exit("a TOML config needs Python 3.11 or newer")
    return tomllib.loads(text)


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


def switch_off(config):
    """Sets to false what makes `weflow serve` hook WeChat or call a model API; returns the keys it set, for the report.

    The profiles' `ai_insight_enabled` is also set under the name the desktop app gave it, which a profile may still
    hold in `extra`.
    """
    serve = config.get("serve")
    if serve is None:
        serve = config["serve"] = {}
    if not isinstance(serve, dict):
        sys.exit("`serve` in the config is not a table")
    serve["image_auto_download"] = False
    serve["insight"] = False
    keys = ["serve.image_auto_download", "serve.insight"]
    named = config.get("profiles")
    profiles = [p for p in named.values() if isinstance(p, dict)] if isinstance(named, dict) else []
    for profile in profiles:
        profile["ai_insight_enabled"] = False
        if isinstance(profile.get("extra"), dict) and "aiInsightEnabled" in profile["extra"]:
            profile["extra"]["aiInsightEnabled"] = False
    if profiles:
        keys.append(f"ai_insight_enabled (in {len(profiles)} profiles)")
    return keys


def make_private_dir(path):
    """Creates `path` and its missing parents, each its owner's only on Linux and macOS like the folders weflow
    makes; `path` itself is tightened when it is there already. On Windows the folders take the access of where they
    are made."""
    for folder in reversed([p for p in (path, *path.parents) if not p.exists()]):
        folder.mkdir(mode=0o700, exist_ok=True)
    if POSIX:
        path.chmod(0o700)


def write_private(target, config):
    """Writes `config` to `target`, its owner's only on Linux and macOS: a file that is there already is tightened
    before it gets the keys."""
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        if POSIX:
            os.fchmod(f.fileno(), 0o600)
        json.dump(config, f, ensure_ascii=False, indent=2)


def main():
    parser = argparse.ArgumentParser(usage="%(prog)s [--weflow BINARY] home [config]", description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--weflow", metavar="BINARY", help="ask this weflow which config it reads")
    parser.add_argument("home", help="the new WEFLOW_HOME")
    parser.add_argument("config", nargs="?", help="the config file to copy, instead of the one weflow reads")
    args = parser.parse_args()
    if args.config and args.weflow:
        parser.error("--weflow finds the config; it is no use with a config file")
    home = Path(args.home)
    source, how = (Path(args.config), "given on the command line") if args.config else default_config(args.weflow)
    if not source.is_file():
        sys.exit(f"there is no config file {source.name} ({how})")
    if home.resolve() == source.parent.resolve():
        sys.exit("the new home must be another folder than the config's own")
    if read_pointer(home / "config_path"):
        sys.exit("the new home holds a config_path pointer, which sends weflow to another config")
    try:
        config = read_config(source)
    except ValueError as err:
        sys.exit(f"{source.name} is not a config file: {err}")
    if not isinstance(config, dict):
        sys.exit(f"{source.name} is not a config file: its top level is not a table")
    clear_caches(config)
    switched = switch_off(config)
    make_private_dir(home)
    write_private(home / "config.json", config)
    print(f"isolated home ready, copied {source.name} ({how})")
    print("set to false: " + ", ".join(switched))


if __name__ == "__main__":
    main()
