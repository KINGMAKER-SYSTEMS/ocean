#!/usr/bin/env python3
"""Non-live installer fixtures: redirect only the copied script's HOME token.

The process HOME is preserved. All service/build/network commands resolve to
fixture executables; file operations remain real inside one temporary directory.
"""
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import unittest

SOURCE = Path(__file__).with_name("install-ocean-daemon.sh")
NEW_SHA = "1" * 40
NEW_REV = NEW_SHA[:12]
OLD_REV = "2" * 12
LABEL = "dev.risingtides.ocean-daemon"

MOCK = r"""
import json, os, pathlib, subprocess, sys
root = pathlib.Path(os.environ["OCEAN_INSTALL_FIXTURE_HOME"])
state_path = root / "state.json"
state = json.loads(state_path.read_text())
args = sys.argv[1:]
name = pathlib.Path(sys.argv[0]).name
state["calls"].append([name] + args)
scenario = state["scenario"]
new = state.get("running_rev") == "1" * 12
current = root / ".local/libexec/ocean-daemon/current"
selected_new = current.is_symlink() and pathlib.Path(os.readlink(current)).name == "ocean-daemon-" + "1" * 40
def save():
    state_path.write_text(json.dumps(state))
def end(code=0, value=None):
    save()
    if value is not None:
        print(value)
    sys.exit(code)
if name == "git":
    command = args[2:]
    if command[0] == "fetch":
        end(12 if scenario == "fetch_failure" else 0)
    if command[0] == "status":
        if scenario == "status_failure" or (scenario == "postbuild_status_failure" and state.get("built")):
            end(128)
        dirty = scenario == "dirty" or (scenario == "postbuild_dirty" and state.get("built"))
        end(value=" M Cargo.lock" if dirty else "")
    if command[0] == "rev-parse":
        value = "1" * (12 if "--short=12" in command else 40)
        if command[-1] == "origin/main" and scenario == "old_main":
            value = "3" * 40
        if command[-1] == "HEAD" and scenario == "changed_head" and state.get("built"):
            value = "3" * 40
        end(value=value)
    end(98)
elif name == "cargo":
    if scenario == "build_failure":
        end(101)
    state["built"] = True
    target = pathlib.Path(args[args.index("--target-dir") + 1]) / "release/ocean-daemon"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(b"#!/bin/sh\nexit 0\n")
    target.chmod(0o755)
    end()
elif name == "launchctl":
    action = args[0]
    if action == "print-disabled":
        if scenario == "disabled_inspection_failure":
            end(17)
        if scenario == "malformed_disabled":
            end(value='disabled services = {\n "dev.risingtides.ocean-daemon" => trueXYZ\n}')
        disabled = "true" if state.get("disabled", False) else "false"
        end(value='disabled services = {\n "dev.risingtides.ocean-daemon" => ' + disabled + '\n}')
    if action == "print":
        end(0 if state["loaded"] else 113)
    if action == "bootout":
        state["bootouts"] = state.get("bootouts", 0) + 1
        if scenario == "unload_failure" and state["bootouts"] == 1:
            end(23)
        state["loaded"] = False
        state["running_rev"] = None
        end()
    if action == "bootstrap":
        if state.get("disabled", False):
            end(9)
        if selected_new and scenario == "bootstrap_failure":
            end(9)
        if not selected_new and scenario == "rollback_failure":
            end(24)
        state["loaded"] = True
        state["running_rev"] = ("1" if selected_new else "2") * 12
        end()
    if action == "enable":
        state["disabled"] = False
        end()
    if action == "disable":
        state["disabled"] = True
        end()
    if action == "kickstart":
        if selected_new and scenario == "kickstart_failure":
            end(23)
        if not state["loaded"]:
            end(6)
        state["running_rev"] = ("1" if selected_new else "2") * 12
        end()
    end(97)
elif name == "curl":
    if not state["loaded"]:
        end(22)
    endpoint = args[-1].rsplit("/", 1)[-1]
    if endpoint == "metrics":
        if scenario == "missing_gauge":
            end(value="another_metric 0")
        end(value="ocean_turns_in_flight " + ("1" if scenario == "active_turn" else "0"))
    if endpoint == "requests":
        pending = scenario[len("request_"):] if scenario.startswith("request_") else None
        end(value=json.dumps({"ok": True, "requests": [{"state": pending}] if pending else []}))
    if new and scenario == "health_failure" and endpoint == "health":
        end(22)
    rev = state["running_rev"]
    if new and ((scenario == "stale_health" and endpoint == "health") or
                (scenario == "stale_ready" and endpoint == "ready")):
        rev = "2" * 12
    ok = not (new and endpoint == "ready" and scenario in ("not_ready", "rollback_failure"))
    end(value=json.dumps({"ok": ok, "rev": rev}))
elif name == "plutil":
    end(17 if scenario == "plist_failure" else 0)
elif name == "sleep":
    end()
elif name == "mv":
    if scenario == "publication_failure" and args[-1].endswith("/launch.sh") and not state.get("publication_failed"):
        state["publication_failed"] = True
        end(31)
    save()
    sys.exit(subprocess.call(["/bin/mv"] + args))
end(96)
"""


class InstallerFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ocean-daemon-install-")
        self.root = Path(self.temp.name)
        self.fixture_home = self.root / "operator & home"
        self.fixture_home.mkdir()
        self.repo = self.root / "repo" / "platform" / "ocean-os"
        (self.repo / "ops").mkdir(parents=True)
        (self.repo / "deploy").mkdir()
        source = SOURCE.read_text()
        source = re.sub(r"(?<=\$)HOME\b|(?<=\$\{)HOME\b", "OCEAN_INSTALL_FIXTURE_HOME", source)
        self.script = self.repo / "ops/install-ocean-daemon.sh"
        self.script.write_text(source)
        (self.repo / "deploy/ocean-daemon.sh").write_text("#!/bin/sh\n# candidate launcher\n")
        (self.repo / f"deploy/{LABEL}.plist").write_text("<plist>__OCEAN_HOME__</plist>\n")
        self.libexec = self.fixture_home / ".local/libexec/ocean-daemon"
        self.libexec.mkdir(parents=True)
        self.old_binary = self.libexec / f"ocean-daemon-{OLD_REV}"
        self.old_binary.write_bytes(b"known working binary\n")
        self.old_binary.chmod(0o755)
        self.current = self.libexec / "current"
        # Relative old target proves rollback preserves the exact symlink bytes.
        self.current.symlink_to(self.old_binary.name)
        self.launcher = self.libexec / "launch.sh"
        self.launcher.write_bytes(b"known working launcher\n")
        self.launcher.chmod(0o751)
        self.plist = self.fixture_home / f"Library/LaunchAgents/{LABEL}.plist"
        self.plist.parent.mkdir(parents=True)
        self.plist.write_bytes(b"known working plist\n")
        self.plist.chmod(0o640)
        self.command = self.fixture_home / ".local/bin/ocean-daemon"
        self.command.parent.mkdir(parents=True)
        self.command.symlink_to("../../libexec/ocean-daemon/current")
        self.state_file = self.fixture_home / "state.json"
        self.state = {"scenario": "success", "loaded": True, "running_rev": OLD_REV, "calls": []}
        mocks = self.fixture_home / ".cargo/bin"
        mocks.mkdir(parents=True)
        for name in ("git", "cargo", "launchctl", "curl", "plutil", "sleep", "mv"):
            path = mocks / name
            path.write_text(f"#!{sys.executable}\n" + MOCK)
            path.chmod(0o755)
        self.baseline = self.snapshot()

    def tearDown(self):
        self.temp.cleanup()

    def snapshot(self):
        result = {}
        for path in (self.current, self.launcher, self.plist, self.command, self.old_binary):
            if path.is_symlink():
                result[str(path)] = ("link", os.readlink(path))
            elif path.exists():
                result[str(path)] = ("file", path.read_bytes(), stat.S_IMODE(path.stat().st_mode))
            else:
                result[str(path)] = None
        return result

    def run_install(self, scenario="success"):
        self.state["scenario"] = scenario
        self.state_file.write_text(json.dumps(self.state))
        env = dict(os.environ)
        env["OCEAN_INSTALL_FIXTURE_HOME"] = str(self.fixture_home)
        env["CARGO_TARGET_DIR"] = str(self.root / "wrong inherited target")
        completed = subprocess.run(["bash", str(self.script)], env=env, capture_output=True,
                                   text=True, timeout=20)
        self.state = json.loads(self.state_file.read_text())
        self.last_output = completed.stdout + completed.stderr
        self.assertFalse((self.libexec / ".install-lock").exists())
        return completed

    def assert_restored(self, completed, expected_status, recovered=True):
        self.assertEqual(completed.returncode, expected_status, completed.stdout + completed.stderr)
        self.assertEqual(self.snapshot(), self.baseline)
        self.assertEqual(self.state["loaded"], recovered)
        if recovered:
            self.assertEqual(self.state["running_rev"], OLD_REV)

    def test_success_uses_locked_real_configuration_and_keeps_recovery_artifact(self):
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(os.readlink(self.current), str(self.libexec / f"ocean-daemon-{NEW_SHA}"))
        self.assertEqual(self.old_binary.read_bytes(), b"known working binary\n")
        self.assertEqual(self.state["running_rev"], NEW_REV)
        cargo = next(call for call in self.state["calls"] if call[0] == "cargo")
        self.assertEqual(cargo[1:], ["build", "--locked", "-p", "ocean-daemon", "--release",
                                  "--features", "legacy-chromium", "--target-dir", str(self.repo / "target")])
        self.assertEqual(self.plist.read_text(), f"<plist>{self.fixture_home}</plist>\n")

    def test_prepublication_failures_leave_prior_installation_untouched(self):
        for scenario, code in (("fetch_failure", 12), ("old_main", 64), ("dirty", 64),
                               ("build_failure", 101), ("postbuild_dirty", 64),
                               ("status_failure", 128), ("postbuild_status_failure", 128),
                               ("changed_head", 64), ("plist_failure", 17),
                               ("disabled_inspection_failure", 17), ("malformed_disabled", 70),
                               ("active_turn", 75), ("missing_gauge", 75), ("request_queued", 75),
                               ("request_waiting_for_permission", 75), ("request_cancelling", 75)):
            with self.subTest(scenario=scenario):
                self.state = {"loaded": True, "running_rev": OLD_REV, "calls": []}
                self.assert_restored(self.run_install(scenario), code)
                self.assertFalse(any(call[0] == "launchctl" and call[1] == "bootout"
                                     for call in self.state["calls"]))

    def test_immutable_collision_never_overwrites_artifact(self):
        destination = self.libexec / f"ocean-daemon-{NEW_SHA}"
        destination.write_bytes(b"different existing revision artifact")
        destination.chmod(0o755)
        self.assert_restored(self.run_install(), 65)
        self.assertEqual(destination.read_bytes(), b"different existing revision artifact")

    def test_reinstall_reuses_identical_artifact(self):
        destination = self.libexec / f"ocean-daemon-{NEW_SHA}"
        destination.write_bytes(b"#!/bin/sh\nexit 0\n")
        destination.chmod(0o755)
        inode = destination.stat().st_ino
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(destination.stat().st_ino, inode)

    def test_failed_partial_publication_restores_files_without_restarting_prior_job(self):
        self.assert_restored(self.run_install("publication_failure"), 31)
        self.assertFalse(any(call[0] == "launchctl" and call[1] == "bootout"
                             for call in self.state["calls"]))

    def test_failed_restart_and_readiness_restore_prior_tuple_and_job(self):
        for scenario, code in (("unload_failure", 75), ("bootstrap_failure", 70),
                               ("kickstart_failure", 23),
                               ("health_failure", 70), ("stale_health", 70),
                               ("stale_ready", 70), ("not_ready", 70)):
            with self.subTest(scenario=scenario):
                self.state = {"loaded": True, "running_rev": OLD_REV, "calls": []}
                self.assert_restored(self.run_install(scenario), code)
                self.assertIn("previous installation restored", self.last_output)

    def test_rollback_failure_retains_private_backup_and_original_status(self):
        result = self.run_install("rollback_failure")
        self.assert_restored(result, 70, recovered=False)
        self.assertIn("rollback could not prove recovery", result.stderr)
        stages = list(self.libexec.glob(".install.*"))
        self.assertEqual(len(stages), 1)
        self.assertEqual(stat.S_IMODE(stages[0].stat().st_mode), 0o700)
        self.assertEqual((stages[0] / "previous-launcher").read_bytes(), self.launcher.read_bytes())

    def test_first_install_failure_removes_partial_selection_and_stays_unloaded(self):
        for path in (self.current, self.launcher, self.plist, self.command, self.old_binary):
            path.unlink()
        self.state = {"loaded": False, "running_rev": None, "calls": []}
        self.baseline = self.snapshot()
        self.assert_restored(self.run_install("not_ready"), 70, recovered=False)

    def test_first_install_success_bootstraps_the_new_release(self):
        for path in (self.current, self.launcher, self.plist, self.command, self.old_binary):
            path.unlink()
        self.state = {"loaded": False, "running_rev": None, "calls": []}
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.state["loaded"])
        self.assertEqual(self.state["running_rev"], NEW_REV)

    def test_failed_install_preserves_disabled_override_and_prior_loaded_state(self):
        self.state["disabled"] = True
        self.assert_restored(self.run_install("not_ready"), 75)
        self.assertTrue(self.state["disabled"])
        self.assertFalse(any(call[0] == "launchctl" and call[1] != "print-disabled"
                             for call in self.state["calls"]))

    def test_disabled_first_install_failure_restores_override_without_loading_job(self):
        for path in (self.current, self.launcher, self.plist, self.command, self.old_binary):
            path.unlink()
        self.state = {"loaded": False, "disabled": True, "running_rev": None, "calls": []}
        self.baseline = self.snapshot()
        self.assert_restored(self.run_install("not_ready"), 75, recovered=False)
        self.assertTrue(self.state["disabled"])
        self.assertFalse(any(call[0] == "launchctl" and call[1] != "print-disabled"
                             for call in self.state["calls"]))

    def test_cleanup_keeps_prior_relative_target_even_outside_newest_three(self):
        os.utime(self.old_binary, (1, 1))
        for number in range(4):
            artifact = self.libexec / f"ocean-daemon-retained-{number}"
            artifact.write_bytes(b"older staged artifact")
            os.utime(artifact, (number + 2, number + 2))
        result = self.run_install()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(self.old_binary.exists())


if __name__ == "__main__":
    unittest.main()
