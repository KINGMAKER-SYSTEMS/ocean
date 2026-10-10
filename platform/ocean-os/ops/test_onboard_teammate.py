#!/usr/bin/env python3
"""Exercise the real onboarding script with synthetic files and command mocks."""
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib
import unittest

SOURCE = Path(__file__).parent
MOCK = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
root = pathlib.Path(os.environ["OCEAN_ONBOARD_FIXTURE_HOME"])
name = pathlib.Path(sys.argv[0]).name
with (root / "calls").open("a") as log:
    log.write(json.dumps([name, *sys.argv[1:]]) + "\n")
if name == "uname":
    print("Darwin" if sys.argv[1] == "-s" else "arm64")
elif name == "gh":
    if sys.argv[1:3] == ["auth", "status"]: print("logged in read:packages")
    elif sys.argv[1:3] == ["auth", "token"]: print("fake_fixture_token123")
    else: sys.exit(91)
elif name in {"bun", "npm"}:
    sys.exit(int(os.environ.get("OCEAN_FIXTURE_PACKAGE_EXIT", "0")))
elif name in {"ocean", "ocean-daemon", "ocean-update"}:
    sys.exit(92) # Binaries may be located, never executed by preparation.
else:
    sys.exit(93) # Any service/network/login operation is a fixture failure.
'''


class Onboarding(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="ocean-onboard-fixture-")
        self.root = Path(self.tmp.name).resolve()
        self.ops = self.root / "repo/ops"
        self.ops.mkdir(parents=True)
        text = (SOURCE / "onboard-teammate.sh").read_text()
        # Same policy as existing installer fixtures: leave process HOME intact.
        (self.ops / "onboard-teammate.sh").write_text(text.replace("$HOME", "$OCEAN_ONBOARD_FIXTURE_HOME"))
        shutil.copyfile(SOURCE / "onboarding_files.py", self.ops / "onboarding_files.py")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.bin / "python3").symlink_to(sys.executable)
        for name in ["uname", "gh", "bun", "ocean", "ocean-daemon", "ocean-update", "launchctl", "curl", "security"]:
            path = self.bin / name
            path.write_text(MOCK)
            path.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(OCEAN_ONBOARD_FIXTURE_HOME=str(self.root), PATH=f"{self.bin}:/usr/bin:/bin")
        for key in ["OCEAN_CONFIG_DIR", "XDG_CONFIG_HOME", "OCEAN_FIXTURE_PACKAGE_EXIT"]:
            self.env.pop(key, None)
        self.member = self.root / ".config/ocean-rs/member.toml"
        self.npmrc = self.root / ".npmrc"

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, *extra, success=True):
        result = subprocess.run(["/bin/bash", str(self.ops / "onboard-teammate.sh"), "--model", "openai/gpt-6.1-sol", "--member", "fixture.member", *extra], env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode == 0, success, result.stdout + result.stderr)
        self.assertNotIn("fake_fixture_token123", result.stdout + result.stderr)
        self.assertEqual(self.env.get("HOME"), os.environ.get("HOME"))
        calls = self.calls()
        self.assertFalse(any(c[0] in {"launchctl", "curl", "security", "ocean", "ocean-daemon", "ocean-update"} for c in calls), calls)
        return result

    def calls(self):
        return [json.loads(line) for line in (self.root / "calls").read_text().splitlines()] if (self.root / "calls").exists() else []

    def test_real_script_escapes_identity_and_keeps_private_modes(self):
        name = 'A "quoted" \\ name Ω'
        self.npmrc.write_text("save-exact=true\n")
        self.run_script("--display-name", name)
        self.assertEqual(tomllib.loads(self.member.read_text()), {"member_id": "fixture.member", "display_name": name})
        self.assertEqual(stat.S_IMODE(self.member.stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(self.member.parent.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE(self.npmrc.stat().st_mode), 0o600)
        self.assertIn("save-exact=true", self.npmrc.read_text())
        self.assertEqual([c for c in self.calls() if c[0] == "bun"], [["bun", "add", "-g", "@risingtides-dev/ocean@latest"]])

    def test_rerun_preserves_unrelated_npm_lines_without_duplicates(self):
        self.npmrc.write_text("other=keep\n@risingtides-dev:registry=https://old.invalid\n//npm.pkg.github.com/:_authToken=old_fixture\n")
        self.run_script()
        before = self.npmrc.read_text(), self.member.read_text()
        self.run_script()
        self.assertEqual(before, (self.npmrc.read_text(), self.member.read_text()))
        self.assertEqual(self.npmrc.read_text().count("_authToken="), 1)
        self.assertIn("other=keep", self.npmrc.read_text())

    def test_dry_run_has_no_auth_network_or_mutations(self):
        self.run_script("--dry-run")
        self.assertFalse(self.npmrc.exists())
        self.assertFalse(self.member.parent.exists())
        self.assertTrue(all(c[0] == "uname" for c in self.calls()))

    def test_malformed_identity_fails_before_package_or_auth(self):
        self.member.parent.mkdir(parents=True)
        for text in ['member_id = "fixture.member"\nunknown = "x"\n', 'member_id = "fixture.member"\nmember_id = "other"\n', 'member_id = "fixture.member"\ndisplay_name = "broken\\q"\n']:
            self.member.write_text(text)
            self.run_script(success=False)
            self.assertEqual(self.member.read_text(), text)
        self.assertTrue(all(c[0] == "uname" for c in self.calls()))
        self.run_script("--force")
        self.assertEqual(tomllib.loads(self.member.read_text())["member_id"], "fixture.member")

    def test_other_identity_requires_force(self):
        self.member.parent.mkdir(parents=True)
        self.member.write_text('member_id = "other"\n')
        self.run_script(success=False)
        self.run_script("--force")
        self.assertEqual(tomllib.loads(self.member.read_text())["member_id"], "fixture.member")

    def test_invalid_inputs_fail_without_credentials(self):
        for extra in [("--display-name", "control\rname"), ("--display-name", "x" * 81), ("--member", "with space"), ("--model",)]:
            self.run_script(*extra, success=False)
        self.assertFalse(any(c[0] == "gh" for c in self.calls()))

    def test_symlink_file_and_parent_targets_are_not_touched(self):
        victim = self.root / "victim"
        victim.write_text("preserved")
        victim.chmod(0o644)
        self.npmrc.symlink_to(victim)
        self.run_script("--force", success=False)
        self.assertEqual(victim.read_text(), "preserved")
        self.assertEqual(stat.S_IMODE(victim.stat().st_mode), 0o644)
        self.npmrc.unlink()
        self.member.parent.mkdir(parents=True)
        self.member.symlink_to(victim)
        self.run_script("--force", success=False)
        self.member.unlink()
        self.member.parent.rmdir()
        self.member.parent.symlink_to(self.root)
        self.run_script("--force", success=False)
        self.assertFalse(any(c[0] == "gh" for c in self.calls()))

    def test_package_failure_does_not_write_identity(self):
        self.env["OCEAN_FIXTURE_PACKAGE_EXIT"] = "17"
        self.run_script(success=False)
        self.assertFalse(self.member.exists())

    def test_missing_real_package_binary_refuses_completion(self):
        (self.bin / "ocean-update").unlink()
        self.run_script(success=False)
        self.assertFalse(self.member.exists())

    def test_config_override_is_used_without_service_configuration(self):
        self.env["OCEAN_CONFIG_DIR"] = str(self.root / "configured")
        self.run_script()
        self.assertFalse(self.member.exists())
        self.assertEqual(tomllib.loads((self.root / "configured/member.toml").read_text())["member_id"], "fixture.member")
        self.assertFalse((self.root / "Library").exists())


if __name__ == "__main__":
    unittest.main()
