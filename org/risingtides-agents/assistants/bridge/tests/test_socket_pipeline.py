#!/usr/bin/env python3
"""
Unit tests for the content-agent Slack bridge pipeline (OCEAN-112).

Proves the LIVE message-handling loop end-to-end with a MOCKED Slack client and a
MOCKED daemon — a real Socket Mode connection needs an xapp- app token we don't
have, so we don't fabricate one. We exercise the exact code the live loop runs:

    inbound event → EventDeduper → resolve_context → build_turn
                  → dispatch_to_daemon (OCEAN-84 router.call_daemon, mocked)
                  → reply.deliver (mocked transport)

Pure stdlib + unittest (no pytest dep, no slack_sdk). Run:
    python3 -m unittest assistants.bridge.tests.test_socket_pipeline -v
or directly:
    python3 assistants/bridge/tests/test_socket_pipeline.py
"""
import sys
import unittest
from pathlib import Path

BRIDGE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BRIDGE_DIR))

import socket_listener as sl  # noqa: E402

MANIFEST = sl.ASSISTANTS_ROOT / "content-agent" / "content-agent.toml"


def _ok_daemon(reply="Here's your clip ✨"):
    """A stand-in for router.call_daemon returning a healthy PromptResponse."""
    return lambda payload: {
        "ok": True,
        "response": {"ok": True, "stdout": reply, "session_id": payload["session_id"]},
        "_payload": payload,
    }


def _down_daemon(error="daemon unreachable"):
    return lambda payload: {"ok": False, "kind": "daemon_down", "error": error}


class RecordingDeliver:
    """Mock reply.deliver — records calls, returns a transport-shaped ok result."""

    def __init__(self, ok=True):
        self.calls = []
        self._ok = ok

    def __call__(self, reply_target, output):
        self.calls.append((reply_target, output))
        return {"ok": self._ok, "mode": "message"}


class EventDeduperTest(unittest.TestCase):
    def test_first_seen_then_duplicate(self):
        d = sl.EventDeduper()
        self.assertFalse(d.is_duplicate("Ev1"))
        self.assertTrue(d.is_duplicate("Ev1"))
        self.assertFalse(d.is_duplicate("Ev2"))

    def test_empty_id_never_duplicate(self):
        d = sl.EventDeduper()
        self.assertFalse(d.is_duplicate(""))
        self.assertFalse(d.is_duplicate(""))

    def test_bounded_memory_evicts_oldest(self):
        d = sl.EventDeduper(capacity=3)
        for i in range(5):
            d.is_duplicate(f"E{i}")
        # E0/E1 evicted (capacity 3, FIFO) → seen again as fresh.
        self.assertFalse(d.is_duplicate("E0"))


class ResolveContextTest(unittest.TestCase):
    def test_app_mention_threads_to_message_ts(self):
        ev = {"type": "app_mention", "channel": "C1", "ts": "111.0", "text": "hi"}
        ctx = sl.resolve_context(ev)
        self.assertEqual(ctx["thread_ts"], "111.0")
        self.assertFalse(ctx["is_dm"])
        self.assertEqual(ctx["thread_or_dm"], "111.0")

    def test_existing_thread_ts_wins(self):
        ev = {"channel": "C1", "ts": "222.0", "thread_ts": "111.0", "text": "hi"}
        self.assertEqual(sl.resolve_context(ev)["thread_ts"], "111.0")

    def test_dm_detected_by_channel_type(self):
        ev = {"channel": "D9", "ts": "1.0", "text": "yo", "channel_type": "im"}
        ctx = sl.resolve_context(ev)
        self.assertTrue(ctx["is_dm"])
        self.assertEqual(ctx["thread_or_dm"], "dm")


class BuildTurnTest(unittest.TestCase):
    def test_build_turn_scopes_cwd_and_session_key(self):
        ev = {"type": "app_mention", "channel": "C1", "ts": "9.0", "text": "make a clip"}
        turn = sl.build_turn(MANIFEST, ev)
        self.assertTrue(turn["cwd"].endswith("assistants/content-agent"))
        self.assertEqual(turn["client_type"], "surface-slack")
        self.assertEqual(turn["session_key"], "slack:content-agent:C1:9.0")
        self.assertEqual(turn["_reply"]["channel"], "C1")

    def test_session_id_is_deterministic_uuid(self):
        a = sl._session_id_for("slack:content-agent:C1:9.0")
        b = sl._session_id_for("slack:content-agent:C1:9.0")
        c = sl._session_id_for("slack:content-agent:C1:OTHER")
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)
        self.assertEqual(len(a), 36)  # UUID string


class DispatchTest(unittest.TestCase):
    def test_dispatch_reuses_router_and_folds_session_id(self):
        captured = {}

        def fake_call(payload):
            captured.update(payload)
            return {"ok": True, "response": {"ok": True, "stdout": "done"}}

        orig = sl._router_call_daemon
        sl._router_call_daemon = fake_call
        try:
            turn = sl.build_turn(MANIFEST, {"channel": "C1", "ts": "9.0", "text": "x"})
            sl.dispatch_to_daemon(turn)
        finally:
            sl._router_call_daemon = orig
        # OCEAN-84 contract: cwd, prompt, client_type, UUID session_id, create_if_missing.
        self.assertEqual(captured["client_type"], "surface-slack")
        self.assertTrue(captured["create_if_missing"])
        self.assertEqual(captured["session_id"],
                         sl._session_id_for("slack:content-agent:C1:9.0"))


class HandleEventPipelineTest(unittest.TestCase):
    """The full inbound→daemon→reply pipeline with both halves mocked."""

    def setUp(self):
        self._orig = sl._router_call_daemon

    def tearDown(self):
        sl._router_call_daemon = self._orig

    def test_happy_path_dispatches_and_replies_in_thread(self):
        sl._router_call_daemon = _ok_daemon("Your clip is ready ✨")
        deliver = RecordingDeliver()
        deduper = sl.EventDeduper()
        ev = {"type": "app_mention", "channel": "C1", "ts": "9.0",
              "text": "make a clip", "event_id": "Ev1"}

        out = sl.handle_event(MANIFEST, ev, deduper, deliver=deliver)

        self.assertTrue(out["ok"])
        self.assertTrue(out["delivered"])
        self.assertEqual(out["reply"], "Your clip is ready ✨")
        # Replied in-thread to the originating channel/thread_ts.
        self.assertEqual(len(deliver.calls), 1)
        reply_target, output = deliver.calls[0]
        self.assertEqual(reply_target["channel"], "C1")
        self.assertEqual(reply_target["thread_ts"], "9.0")
        self.assertEqual(output["text"], "Your clip is ready ✨")

    def test_dm_replies_to_dm_channel(self):
        sl._router_call_daemon = _ok_daemon("hi back")
        deliver = RecordingDeliver()
        ev = {"channel": "D9", "ts": "1.0", "text": "hey", "event_id": "Ev2",
              "channel_type": "im"}
        out = sl.handle_event(MANIFEST, ev, sl.EventDeduper(), deliver=deliver)
        self.assertTrue(out["ok"])
        self.assertEqual(deliver.calls[0][0]["channel"], "D9")
        self.assertTrue(deliver.calls[0][0]["is_dm"])

    def test_duplicate_event_is_dropped_no_dispatch(self):
        calls = {"n": 0}

        def counting(payload):
            calls["n"] += 1
            return {"ok": True, "response": {"ok": True, "stdout": "x"}}

        sl._router_call_daemon = counting
        deliver = RecordingDeliver()
        deduper = sl.EventDeduper()
        ev = {"channel": "C1", "ts": "9.0", "text": "yo", "event_id": "DUP"}

        first = sl.handle_event(MANIFEST, ev, deduper, deliver=deliver)
        second = sl.handle_event(MANIFEST, ev, deduper, deliver=deliver)

        self.assertTrue(first["ok"])
        self.assertTrue(second["skipped"])
        self.assertEqual(second["reason"], "duplicate")
        self.assertEqual(calls["n"], 1)        # daemon hit exactly once
        self.assertEqual(len(deliver.calls), 1)  # replied exactly once

    def test_empty_prompt_skips_without_dispatch(self):
        calls = {"n": 0}
        sl._router_call_daemon = lambda p: (calls.__setitem__("n", calls["n"] + 1)
                                            or {"ok": True, "response": {"ok": True, "stdout": ""}})
        deliver = RecordingDeliver()
        ev = {"channel": "C1", "ts": "9.0", "text": "   ", "event_id": "Ev3"}
        out = sl.handle_event(MANIFEST, ev, sl.EventDeduper(), deliver=deliver)
        self.assertTrue(out["skipped"])
        self.assertEqual(calls["n"], 0)
        self.assertEqual(len(deliver.calls), 0)

    def test_daemon_down_posts_failsoft_notice_in_thread(self):
        sl._router_call_daemon = _down_daemon("connection refused")
        deliver = RecordingDeliver()
        ev = {"channel": "C1", "ts": "9.0", "text": "do it", "event_id": "Ev4"}
        out = sl.handle_event(MANIFEST, ev, sl.EventDeduper(), deliver=deliver)
        self.assertFalse(out["ok"])
        self.assertEqual(out["kind"], "daemon_down")
        # Operator gets a visible in-thread warning rather than silence (spec §9).
        self.assertEqual(len(deliver.calls), 1)
        self.assertIn("couldn't reach the agent", deliver.calls[0][1]["text"])

    def test_reply_delivery_failure_is_failsoft(self):
        sl._router_call_daemon = _ok_daemon("text")
        deduper = sl.EventDeduper()

        def boom(reply_target, output):
            raise RuntimeError("slack 500")

        ev = {"channel": "C1", "ts": "9.0", "text": "hi", "event_id": "Ev5"}
        out = sl.handle_event(MANIFEST, ev, deduper, deliver=boom)
        # Pipeline survives a transport blow-up: returns structured, doesn't raise.
        self.assertFalse(out["ok"])
        self.assertFalse(out["delivered"])
        self.assertIn("deliver", out["reason"])


class ExtractInboundTest(unittest.TestCase):
    """The slack_sdk payload → flat event normalizer (no slack_sdk needed)."""

    def test_app_mention_extracted_with_event_id(self):
        payload = {"type": "events_api",
                   "payload": {"event_id": "EvX",
                               "event": {"type": "app_mention", "channel": "C1",
                                         "ts": "1.0", "text": "hi"}}}
        flat = sl._extract_inbound(payload)
        self.assertIsNotNone(flat)
        self.assertEqual(flat["event_id"], "EvX")
        self.assertEqual(flat["channel"], "C1")

    def test_bot_message_ignored(self):
        payload = {"type": "events_api",
                   "payload": {"event_id": "EvY",
                               "event": {"type": "message", "bot_id": "B1",
                                         "channel": "C1", "ts": "1.0", "text": "loop?"}}}
        self.assertIsNone(sl._extract_inbound(payload))

    def test_message_subtype_ignored(self):
        payload = {"type": "events_api",
                   "payload": {"event_id": "EvZ",
                               "event": {"type": "message", "subtype": "channel_join",
                                         "channel": "C1", "ts": "1.0"}}}
        self.assertIsNone(sl._extract_inbound(payload))

    def test_non_events_api_ignored(self):
        self.assertIsNone(sl._extract_inbound({"type": "hello", "payload": {}}))


if __name__ == "__main__":
    unittest.main(verbosity=2)
