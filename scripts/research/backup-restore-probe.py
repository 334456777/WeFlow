"""Synthetic backup validation matrix, with no account data or saved keys.

Usage: python scripts/research/backup-restore-probe.py /path/to/weflow[.exe]
All writes, including the deliberately escaping marker, stay inside a fresh temp root.
Results describe current behavior; accepting an invalid archive is not a passing check.
"""
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import zipfile


def main():
    executable = str(Path(sys.argv[1]).resolve())
    root = Path(tempfile.mkdtemp(prefix="weflow-backup-probe-"))
    env = dict(os.environ, WEFLOW_HOME=str(root / "home"), WEFLOW_LANG="en")
    entry = {
        "path": "db_storage/synthetic.db",
        "sha256": hashlib.sha256(b"synthetic").hexdigest(),
        "size": 9,
    }
    cases = [
        ("valid", [entry], "1", {entry["path"]: b"synthetic"}),
        ("extra-entry", [], "1", {"extra.txt": b"synthetic"}),
        ("missing-listed-entry", [entry], "1", {}),
        ("unknown-version", [], "999", {"extra.txt": b"synthetic"}),
        ("wrong-size", [dict(entry, size=123)], "1", {entry["path"]: b"synthetic"}),
        ("parent-path", [], "1", {"../escape-marker.txt": b"synthetic"}),
        ("hash-mismatch", [entry], "1", {entry["path"]: b"wrong"}),
    ]
    for tag, entries, version, payloads in cases:
        archive = root / (tag + ".zip")
        manifest = dict(version=version, created_at=0, wxid="synthetic", weflow_version="test", entries=entries)
        with zipfile.ZipFile(archive, "w") as container:
            container.writestr("weflow_backup_manifest.json", json.dumps(manifest))
            for name, payload in payloads.items():
                container.writestr(name, payload)
        target = root / tag / "target"
        for name in payloads:
            assert (target / name).resolve().is_relative_to(root.resolve())
        result = subprocess.run(
            [executable, "backup", "restore", str(archive), "--target", str(target), "--json"],
            env=env, capture_output=True, text=True, encoding="utf-8",
        )
        print(json.dumps(dict(case=tag, exit=result.returncode, wrote_payload=any((target / p).exists() for p in payloads))), flush=True)
    archive = root / "desktop-format.tar"
    with tarfile.open(archive, "w") as container:
        payload = json.dumps(dict(version=1, type="weflow-db-snapshots", databases=[])).encode()
        info = tarfile.TarInfo("manifest.json")
        info.size = len(payload)
        container.addfile(info, io.BytesIO(payload))
    result = subprocess.run([executable, "backup", "inspect", str(archive), "--json"], env=env, capture_output=True)
    print(json.dumps(dict(case="desktop-tar-inspect", exit=result.returncode)))


if __name__ == "__main__":
    main()
