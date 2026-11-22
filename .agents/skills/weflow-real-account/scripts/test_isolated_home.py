"""Tests of isolated_home.py with made-up configs, never a real one: `python -m unittest` in this folder.

Every run gets its own environment (WEFLOW_HOME, the platform's config folder), so what the script picks is the file
the test put there. Set WEFLOW_BIN to a weflow binary to also check that --weflow agrees with the script's own rules.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import isolated_home  # noqa: E402

SCRIPT = HERE / "isolated_home.py"
POSIX = os.name == "posix"


def config(marker, **serve):
    """A config whose `current_profile` says which file it is; the key and the other values are made up."""
    return {
        "current_profile": marker,
        "serve": {"host": "127.0.0.1", **serve},
        "profiles": {
            "a": {"decrypt_key": "k" * 8, "cache_path": "/somewhere/cache", "ai_insight_enabled": True},
            "b": {"wxid": "w中", "extra": {"aiInsightEnabled": True, "cachePath": "x", "keep": 1}},
            "c": {},
        },
    }


class Case(unittest.TestCase):
    """A scratch folder with its own platform config folder, and a way to run the script in it."""

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.root = Path(scratch.name).resolve()
        self.env = {k: v for k, v in os.environ.items() if k != "WEFLOW_HOME"}
        self.env.update(APPDATA=str(self.root / "appdata"), XDG_CONFIG_HOME=str(self.root / "xdg"),
                        HOME=str(self.root / "home"), USERPROFILE=str(self.root / "home"))
        with mock.patch.dict(os.environ, self.env, clear=True):
            self.platform = isolated_home.platform_folder()
        self.platform.mkdir(parents=True)
        self.out = self.root / "out"

    def write(self, path, value, text=None):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text if text is not None else json.dumps(value), encoding="utf-8")
        return path

    def run_script(self, *args, home=None, check=True):
        env = dict(self.env)
        if home is not None:
            env["WEFLOW_HOME"] = str(home)
        done = subprocess.run([sys.executable, str(SCRIPT), *args], env=env, capture_output=True, text=True)
        if check:
            self.assertEqual(done.returncode, 0, done.stderr)
        return done

    def copied(self):
        return json.loads((self.out / "config.json").read_text(encoding="utf-8"))


class WhichConfig(Case):
    def test_the_platform_folder_without_a_pointer(self):
        self.write(self.platform / "config.json", config("platform"))
        self.run_script(self.out)
        self.assertEqual(self.copied()["current_profile"], "platform")

    def test_the_file_a_pointer_names(self):
        self.write(self.platform / "config.json", config("stale"))
        chosen = self.write(self.root / "elsewhere" / "chosen.json", config("chosen"))
        self.write(self.platform / "config_path", None, f"\n  {chosen}  \r\n")
        done = self.run_script(self.out)
        self.assertEqual(self.copied()["current_profile"], "chosen")
        self.assertIn("chosen.json", done.stdout)
        self.assertIn("config_path", done.stdout)

    def test_a_blank_pointer_is_no_pointer(self):
        self.write(self.platform / "config.json", config("platform"))
        for blank in ("", "  \r\n\t"):
            self.write(self.platform / "config_path", None, blank)
            self.run_script(self.out)
            self.assertEqual(self.copied()["current_profile"], "platform")

    def test_weflow_home_comes_before_the_platform_folder(self):
        self.write(self.platform / "config.json", config("platform"))
        chosen = self.write(self.root / "elsewhere" / "chosen.json", config("platform pointer"))
        self.write(self.platform / "config_path", None, str(chosen))
        self.write(self.root / "custom" / "config.json", config("custom"))
        self.run_script(self.out, home=self.root / "custom")
        self.assertEqual(self.copied()["current_profile"], "custom")

    def test_the_pointer_of_weflow_home_comes_first(self):
        self.write(self.platform / "config.json", config("platform"))
        self.write(self.root / "custom" / "config.json", config("custom"))
        chosen = self.write(self.root / "elsewhere" / "chosen.json", config("custom pointer"))
        self.write(self.root / "custom" / "config_path", None, str(chosen))
        self.run_script(self.out, home=self.root / "custom")
        self.assertEqual(self.copied()["current_profile"], "custom pointer")

    def test_weflow_does_not_look_in_the_platform_folder_when_weflow_home_is_set(self):
        # a pointer in the platform folder is not weflow's when WEFLOW_HOME is set
        chosen = self.write(self.root / "elsewhere" / "chosen.json", config("platform pointer"))
        self.write(self.platform / "config_path", None, str(chosen))
        self.write(self.root / "custom" / "config.json", config("custom"))
        self.run_script(self.out, home=self.root / "custom")
        self.assertEqual(self.copied()["current_profile"], "custom")

    def test_a_blank_weflow_home_is_not_set(self):
        self.write(self.platform / "config.json", config("platform"))
        self.run_script(self.out, home="  ")
        self.assertEqual(self.copied()["current_profile"], "platform")

    def test_a_config_on_the_command_line_wins(self):
        self.write(self.platform / "config.json", config("platform"))
        given = self.write(self.root / "given" / "any-name.json", config("given"))
        self.run_script(self.out, str(given))
        self.assertEqual(self.copied()["current_profile"], "given")

    def test_a_missing_file_is_told_by_name_only(self):
        done = self.run_script(self.out, check=False)
        self.assertNotEqual(done.returncode, 0)
        self.assertIn("no config file config.json", done.stderr)
        self.assertNotIn(str(self.root), done.stderr)
        self.assertFalse(self.out.exists())

    def test_a_toml_config(self):
        if sys.version_info < (3, 11):
            self.skipTest("tomllib is in Python 3.11 and later")
        self.write(self.platform / "config.json", None,
                   'current_profile = "toml"\n[serve]\nport = 7777\n[profiles.a]\ncache_path = "c"\n')
        self.run_script(self.out)
        copied = self.copied()
        self.assertEqual((copied["current_profile"], copied["serve"]["port"]), ("toml", 7777))
        self.assertIsNone(copied["profiles"]["a"]["cache_path"])

    def test_the_new_home_is_not_the_configs_own_folder(self):
        self.write(self.platform / "config.json", config("platform"))
        done = self.run_script(self.platform, check=False)
        self.assertNotEqual(done.returncode, 0)

    def test_the_new_home_has_no_pointer_to_another_config(self):
        self.write(self.platform / "config.json", config("platform"))
        self.write(self.out / "config_path", None, str(self.root / "elsewhere.json"))
        done = self.run_script(self.out, check=False)
        self.assertNotEqual(done.returncode, 0)
        self.assertFalse((self.out / "config.json").exists())

    def test_the_default_rules_name_the_pointer_and_trim_it(self):
        chosen = self.root / "chosen.json"
        self.write(self.platform / "config_path", None, f" {chosen}\n")
        with mock.patch.dict(os.environ, self.env, clear=True):
            self.assertEqual(isolated_home.default_config()[0], chosen)


class AskWeflow(Case):
    def answered(self, **kwargs):
        done = subprocess.CompletedProcess([], **kwargs)
        with mock.patch.dict(os.environ, self.env, clear=True), mock.patch.object(isolated_home.subprocess, "run",
                                                                     return_value=done):
            return isolated_home.default_config("weflow")[0]

    def test_the_path_weflow_names(self):
        named = self.root / "named.json"
        answer = json.dumps({"success": True, "data": str(named)}).encode()
        self.assertEqual(self.answered(returncode=0, stdout=answer), named)

    def test_the_script_s_own_rules_when_weflow_cannot_say(self):
        own = self.platform / "config.json"
        for bad in (dict(returncode=1, stdout=b'{"success":false,"data":null}'),
                    dict(returncode=0, stdout=b"not json"),
                    dict(returncode=0, stdout=b'{"success":true,"data":7}'),
                    dict(returncode=0, stdout=b'{"success":true,"data":""}')):
            self.assertEqual(self.answered(**bad), own, bad)

    def test_a_binary_that_is_not_there(self):
        with mock.patch.dict(os.environ, self.env, clear=True):
            self.assertEqual(isolated_home.default_config(str(self.root / "no-such-weflow"))[0],
                             self.platform / "config.json")

    @unittest.skipUnless(os.environ.get("WEFLOW_BIN"), "set WEFLOW_BIN to a weflow binary")
    def test_the_real_binary_agrees_with_the_rules(self):
        chosen = self.root / "chosen.json"
        for pointer in (None, f" {chosen}\n"):
            home = self.root / "bin-home"
            home.mkdir(exist_ok=True)
            (home / "config_path").unlink(missing_ok=True)
            if pointer:
                (home / "config_path").write_text(pointer, encoding="utf-8")
            with mock.patch.dict(os.environ, dict(self.env, WEFLOW_HOME=str(home)), clear=True):
                asked = isolated_home.ask_weflow(os.environ["WEFLOW_BIN"])
                own = isolated_home.default_config()[0]
            self.assertEqual(asked, own)


class WhatIsCopied(Case):
    def test_what_reaches_outside_is_switched_off(self):
        source = config("x", image_auto_download=True, insight=True)
        self.write(self.platform / "config.json", source)
        done = self.run_script(self.out)
        copied = self.copied()
        self.assertEqual(copied["serve"], {"host": "127.0.0.1", "image_auto_download": False, "insight": False})
        for name in "abc":
            self.assertIs(copied["profiles"][name]["ai_insight_enabled"], False, name)
        self.assertIs(copied["profiles"]["b"]["extra"]["aiInsightEnabled"], False)
        self.assertEqual(copied["profiles"]["b"]["extra"]["keep"], 1)
        self.assertEqual(copied["profiles"]["b"]["wxid"], "w中")
        # the caches go beside the copy, and the rest is as it was
        self.assertIsNone(copied["profiles"]["a"]["cache_path"])
        self.assertIsNone(copied["profiles"]["b"]["extra"]["cachePath"])
        self.assertEqual(copied["profiles"]["a"]["decrypt_key"], source["profiles"]["a"]["decrypt_key"])
        # the report names the keys and carries no value of the config
        for key in ("serve.image_auto_download", "serve.insight", "ai_insight_enabled"):
            self.assertIn(key, done.stdout)
        self.assertNotIn("decrypt_key", done.stdout)
        self.assertNotIn("kkkk", done.stdout + done.stderr)

    def test_a_config_without_serve_or_profiles(self):
        self.write(self.platform / "config.json", {"lang": "en"})
        done = self.run_script(self.out)
        self.assertEqual(self.copied(), {"lang": "en", "serve": {"image_auto_download": False, "insight": False}})
        self.assertNotIn("ai_insight_enabled", done.stdout)

    def test_the_source_is_left_as_it_is(self):
        path = self.write(self.platform / "config.json", config("x", insight=True))
        before = path.read_bytes()
        self.run_script(self.out)
        self.assertEqual(path.read_bytes(), before)


@unittest.skipUnless(POSIX, "modes are for Linux and macOS")
class WhoCanRead(Case):
    def mode(self, path):
        return stat.S_IMODE(path.stat().st_mode)

    def test_new_folders_and_the_file_are_the_owner_s_only(self):
        self.write(self.platform / "config.json", config("x"))
        deep = self.root / "new" / "deeper" / "home"
        old = os.umask(0o022)
        try:
            self.run_script(deep)
        finally:
            os.umask(old)
        for folder in (deep, deep.parent, deep.parent.parent):
            self.assertEqual(self.mode(folder), 0o700, folder.name)
        self.assertEqual(self.mode(deep / "config.json"), 0o600)

    def test_what_is_there_already_is_tightened(self):
        self.write(self.platform / "config.json", config("x"))
        self.out.mkdir()
        self.out.chmod(0o755)
        old = self.write(self.out / "config.json", {"old": True})
        old.chmod(0o644)
        self.run_script(self.out)
        self.assertEqual(self.mode(self.out), 0o700)
        self.assertEqual(self.mode(self.out / "config.json"), 0o600)
        self.assertEqual(self.copied()["current_profile"], "x")


if __name__ == "__main__":
    unittest.main()
