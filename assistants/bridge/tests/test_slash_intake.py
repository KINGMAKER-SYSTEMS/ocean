#!/usr/bin/env python3
"""
Unit tests for the courier-hub slash-command intake (assistants/bridge/slash_intake.py).

Proves the whole post-ack pipeline with every seam injected — a synthetic
registry, a recording responder, a fake subprocess runner, and a fake daemon —
no network, no real Slack, no live daemon. We exercise the exact code the live
`slash_commands` envelope path runs:

    payload → parse_invocation → help | usage | deterministic (harness argv)
            | agentic (router-shaped daemon payload) → responder

Pure stdlib + unittest (no pytest dep, no slack_sdk). Run:
    python3 -m unittest assistants.bridge.tests.test_slash_intake -v
or directly:
    python3 assistants/bridge/tests/test_slash_intake.py
"""
import sys
import unittest
from pathlib import Path

BRIDGE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BRIDGE_DIR))

import slash_intake as si  # noqa: E402

# Synthetic registry — one deterministic + one agentic courier, shaped exactly
# like router.load_registry() entries so the dispatch code sees the real schema.
DET = {"courier": "pack-courier", "dir": "/opt/couriers/pack-courier",
       "mode": "deterministic", "entry": "bin/courier",
       "command": {"slash": "/pack", "subcommand": "send",
                   "summary": "pack things", "usage": "/pack --channel <id>"}}
AGN = {"courier": "scout", "dir": "/opt/couriers/scout",
       "mode": "agentic", "entry": "bin/courier",
       "command": {"slash": "/scout", "prompt": "Scout the thing.",
                   "summary": "scout it"}}
REG = {"/pack": DET, "/scout": AGN}


def slash_payload(command="/ocean", text="", **kw):
    p = {"command": command, "text": text, "channel_id": "C123", "user_id": "U1",
         "response_url": "https://hooks.slack.example/T0/B0/xyz"}
    p.update(kw)
    return p


class RecordingResponder:
    """Mock respond() — records (text, response_type), returns ok."""

    def __init__(self):
        self.calls = []

    def __call__(self, text, response_type):
        self.calls.append((text, response_type))
        return {"ok": True}


def make_runner(code=0, stdout="shipped 3 files", stderr=""):
    """Fake subprocess runner — records argv, returns canned (code, out, err)."""
    def run(argv):
        run.calls.append(list(argv))
        return code, stdout, stderr
    run.calls = []
    return run


def forbidden_runner(argv):
    raise AssertionError(f"harness must not be invoked, got argv={argv}")


def forbidden_daemon(payload):
    raise AssertionError(f"daemon must not be called, got payload={payload}")


class HelpTest(unittest.TestCase):
    def test_help_renders_live_table_from_manifests(self):
        # registry=None → the REAL router.load_registry() over this repo's
        # courier.toml manifests, exactly what `router.py list` shows.
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text="help"), responder=r,
                              runner=forbidden_runner, daemon_call=forbidden_daemon)
        self.assertTrue(out["ok"])
        self.assertEqual(out["kind"], "help")
        self.assertEqual(len(r.calls), 1)
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "ephemeral")
        for live_slash in ("/ship", "/say", "/resolve"):
            self.assertIn(live_slash, text)
        self.assertIn("```", text)  # table lands in a code block

    def test_bare_hub_command_is_help(self):
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text=""), registry=REG, responder=r,
                              runner=forbidden_runner, daemon_call=forbidden_daemon)
        self.assertEqual(out["kind"], "help")
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "ephemeral")
        self.assertIn("/pack", text)
        self.assertIn("/scout", text)
        self.assertIn("↳ /pack --channel <id>", text)  # usage hint line


class DeterministicDispatchTest(unittest.TestCase):
    def test_dispatch_invokes_harness_argv_like_router(self):
        runner = make_runner(stdout="sent 2 files")
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text="pack --channel C9"),
                              registry=REG, responder=r, runner=runner)
        # entry + manifest subcommand + operator args — router.cmd_run's shape.
        self.assertEqual(runner.calls,
                         [["/opt/couriers/pack-courier/bin/courier", "send",
                           "--channel", "C9"]])
        self.assertTrue(out["ok"])
        self.assertEqual(out["exit"], 0)
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "in_channel")
        self.assertIn("sent 2 files", text)
        self.assertIn("```", text)  # output wrapped in a code block

    def test_direct_per_slash_payload_routes_without_hub_prefix(self):
        # A future per-command Slack registration (/pack itself) needs no code
        # change: the payload's command matches a registry slash → routes.
        runner = make_runner()
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(command="/pack", text="--channel C9"),
                              registry=REG, responder=r, runner=runner)
        self.assertTrue(out["ok"])
        self.assertEqual(runner.calls,
                         [["/opt/couriers/pack-courier/bin/courier", "send",
                           "--channel", "C9"]])

    def test_quoted_args_survive_shlex(self):
        runner = make_runner()
        si.handle_slash(slash_payload(text='pack --caption "two words"'),
                        registry=REG, responder=RecordingResponder(), runner=runner)
        self.assertEqual(runner.calls[0][-2:], ["--caption", "two words"])

    def test_stderr_and_nonzero_exit_surface_in_reply(self):
        runner = make_runner(code=2, stdout="", stderr="boom: bad channel")
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text="pack"), registry=REG,
                              responder=r, runner=runner)
        self.assertFalse(out["ok"])
        self.assertEqual(out["exit"], 2)
        text, _ = r.calls[0]
        self.assertIn("exit 2", text)
        self.assertIn("boom: bad channel", text)

    def test_long_output_is_truncated_with_note(self):
        runner = make_runner(stdout="x" * 20000)
        r = RecordingResponder()
        si.handle_slash(slash_payload(text="pack"), registry=REG,
                        responder=r, runner=runner)
        text, _ = r.calls[0]
        self.assertIn("truncated", text)
        # limit + fences + status line, nowhere near the raw 20k.
        self.assertLess(len(text), si.REPLY_LIMIT + 300)


class AgenticDispatchTest(unittest.TestCase):
    def test_dispatch_builds_router_shaped_daemon_payload(self):
        captured = {}

        def fake_daemon(payload):
            captured.update(payload)
            return {"ok": True, "response": {"ok": True, "stdout": "done",
                                             "session_id": "SID-1"}}

        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text="scout find the artifact"),
                              registry=REG, responder=r, daemon_call=fake_daemon,
                              runner=forbidden_runner)
        # router.cmd_run's exact agentic contract: cwd-scoped, surface-slack,
        # deterministic UUIDv5 session (router's own fold), create_if_missing.
        self.assertEqual(captured["cwd"], "/opt/couriers/scout")
        self.assertEqual(captured["client_type"], "surface-slack")
        self.assertTrue(captured["create_if_missing"])
        expected_sid = si._router.session_id_for(
            si._router.session_key("scout", "C123", None))
        self.assertEqual(captured["session_id"], expected_sid)
        self.assertTrue(captured["prompt"].startswith("Scout the thing."))
        self.assertIn("Operator args: find the artifact", captured["prompt"])
        # Short dispatched note + the daemon's response id, in-channel.
        self.assertTrue(out["ok"])
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "in_channel")
        self.assertIn("dispatched to daemon", text)
        self.assertIn("SID-1", text)

    def test_daemon_down_replies_ephemeral_error(self):
        r = RecordingResponder()
        out = si.handle_slash(
            slash_payload(text="scout go"), registry=REG, responder=r,
            daemon_call=lambda p: {"ok": False, "kind": "daemon_down",
                                   "error": "connection refused"},
            runner=forbidden_runner)
        self.assertFalse(out["ok"])
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "ephemeral")
        self.assertIn("connection refused", text)


class UsageTest(unittest.TestCase):
    def test_unknown_command_gets_usage_with_known_table(self):
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(text="bogus"), registry=REG,
                              responder=r, runner=forbidden_runner,
                              daemon_call=forbidden_daemon)
        self.assertEqual(out["kind"], "usage")
        text, rtype = r.calls[0]
        self.assertEqual(rtype, "ephemeral")
        self.assertIn("/pack", text)   # known commands listed
        self.assertIn("help", text)    # points at /ocean help

    def test_foreign_slash_command_gets_usage(self):
        r = RecordingResponder()
        out = si.handle_slash(slash_payload(command="/unrelated", text="x"),
                              registry=REG, responder=r, runner=forbidden_runner,
                              daemon_call=forbidden_daemon)
        self.assertEqual(out["kind"], "usage")
        self.assertEqual(r.calls[0][1], "ephemeral")


class BuildAckTest(unittest.TestCase):
    def test_dispatch_kinds_ack_with_ephemeral_running_note(self):
        ack = si.build_ack(slash_payload(text="pack --channel C9"), registry=REG)
        self.assertIsNotNone(ack)
        self.assertEqual(ack["response_type"], "ephemeral")
        self.assertIn("/pack", ack["text"])

    def test_help_and_unknown_ack_bare(self):
        self.assertIsNone(si.build_ack(slash_payload(text="help"), registry=REG))
        self.assertIsNone(si.build_ack(slash_payload(text="bogus"), registry=REG))


if __name__ == "__main__":
    unittest.main(verbosity=2)
