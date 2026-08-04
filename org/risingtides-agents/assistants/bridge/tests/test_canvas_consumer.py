#!/usr/bin/env python3
"""
Unit tests for the Slack canvas bridge consumer (OCEAN-244).

Proves the op→Slack-API mapping + the fulfillment round-trip end-to-end with a
MOCKED Slack transport and SYNTHETIC `AgentTurnEvent::SlackCanvas` payloads — no
running daemon, no real Slack token (we don't fabricate one). We exercise the
exact code the live SSE loop runs per event:

    parse_canvas_event → apply_op (mock transport) → deliver_fulfillment (mock)

Pure stdlib + unittest (no pytest dep, no slack_sdk). Run:
    python3 -m unittest assistants.bridge.tests.test_canvas_consumer -v
or directly:
    python3 assistants/bridge/tests/test_canvas_consumer.py

What needs a LIVE Slack workspace (NOT covered here): the actual canvases.create
/ canvases.edit / files.info round-trip, and the end-to-end SSE delivery (which
also needs ocean-os to relay slack_canvas on the wire — not on main yet). Those
are integration concerns; the mapping/return contract is fully unit-tested.
"""
import sys
import unittest
from pathlib import Path

BRIDGE_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(BRIDGE_DIR))

import canvas_consumer as cc  # noqa: E402


class FakeSlack:
    """Mock couriers.transport.slack.Slack — records calls, returns canned API
    responses shaped like the real Slack Web API. Mirrors the transport surface
    the consumer uses (create/edit/read/list canvas)."""

    def __init__(self, *, create=None, edit=None, read=None, listing=None):
        self.calls = []
        self._create = create or {"ok": True, "canvas_id": "F_NEW"}
        self._edit = edit or {"ok": True}
        self._read = read or {
            "ok": True, "canvas_id": "F1", "title": "Plan",
            "sections": ["s1", "s2"], "body_available": False,
            "note": "Slack exposes no raw markdown body",
        }
        self._list = listing or {
            "ok": True, "channel": "C1",
            "canvases": [{"canvas_id": "F1", "title": "Plan"}],
        }

    def create_canvas(self, title=None, markdown=None, channel=None):
        self.calls.append(("create_canvas", {"title": title, "markdown": markdown,
                                              "channel": channel}))
        return self._create

    def edit_canvas(self, canvas_id, markdown, mode="replace"):
        self.calls.append(("edit_canvas", {"canvas_id": canvas_id,
                                           "markdown": markdown, "mode": mode}))
        return self._edit

    def read_canvas(self, canvas_id):
        self.calls.append(("read_canvas", {"canvas_id": canvas_id}))
        return {**self._read, "canvas_id": canvas_id}

    def list_canvases(self, channel):
        self.calls.append(("list_canvases", {"channel": channel}))
        return {**self._list, "channel": channel}


class RecordingDeliver:
    """Mock deliver_fulfillment — records (session_id, op, result), returns ok."""

    def __init__(self, ok=True):
        self.calls = []
        self._ok = ok

    def __call__(self, session_id, op, result):
        self.calls.append((session_id, op, result))
        return {"ok": self._ok, "status": 200}


def _evt(op: dict, session_id="sess-1") -> dict:
    """A synthetic AgentTurnEvent::SlackCanvas SSE payload (serde shape)."""
    return {"type": "slack_canvas", "session_id": session_id, "op": op}


# ---------------------------------------------------------------------------
# parse_canvas_event
# ---------------------------------------------------------------------------

class ParseEventTest(unittest.TestCase):
    def test_parses_valid_canvas_event(self):
        parsed = cc.parse_canvas_event(_evt({"op": "create", "title": "X"}))
        self.assertIsNotNone(parsed)
        self.assertEqual(parsed["session_id"], "sess-1")
        self.assertEqual(parsed["op"]["op"], "create")

    def test_ignores_non_canvas_event(self):
        self.assertIsNone(cc.parse_canvas_event(
            {"type": "assistant_text_delta", "session_id": "s", "delta": "hi"}))

    def test_ignores_event_without_op_object(self):
        self.assertIsNone(cc.parse_canvas_event(
            {"type": "slack_canvas", "session_id": "s"}))

    def test_ignores_op_missing_discriminant(self):
        self.assertIsNone(cc.parse_canvas_event(
            {"type": "slack_canvas", "session_id": "s", "op": {"title": "x"}}))

    def test_ignores_non_dict(self):
        self.assertIsNone(cc.parse_canvas_event("nope"))
        self.assertIsNone(cc.parse_canvas_event(None))


# ---------------------------------------------------------------------------
# apply_op — the op → Slack API mapping (the heart of OCEAN-244)
# ---------------------------------------------------------------------------

class ApplyOpTest(unittest.TestCase):
    def test_create_maps_to_create_canvas_and_returns_canvas_id(self):
        slack = FakeSlack(create={"ok": True, "canvas_id": "F_MINTED"})
        res = cc.apply_op(slack, {"op": "create", "title": "Campaign Plan",
                                  "markdown": "# Plan", "channel_id": "C9"})
        self.assertEqual(slack.calls[0][0], "create_canvas")
        self.assertEqual(slack.calls[0][1], {"title": "Campaign Plan",
                                             "markdown": "# Plan", "channel": "C9"})
        self.assertTrue(res["ok"])
        self.assertEqual(res["op"], "create")
        self.assertEqual(res["canvas_id"], "F_MINTED")
        self.assertTrue(res["bridged"])

    def test_update_maps_to_edit_canvas_with_mode(self):
        slack = FakeSlack()
        res = cc.apply_op(slack, {"op": "update", "canvas_id": "F1",
                                  "markdown": "new body", "mode": "prepend"})
        self.assertEqual(slack.calls[0][0], "edit_canvas")
        self.assertEqual(slack.calls[0][1]["mode"], "prepend")
        self.assertEqual(slack.calls[0][1]["canvas_id"], "F1")
        self.assertTrue(res["ok"])
        self.assertEqual(res["canvas_id"], "F1")

    def test_update_defaults_mode_to_replace(self):
        slack = FakeSlack()
        cc.apply_op(slack, {"op": "update", "canvas_id": "F1", "markdown": "x"})
        self.assertEqual(slack.calls[0][1]["mode"], "replace")

    def test_append_maps_to_edit_canvas_append_mode(self):
        slack = FakeSlack()
        res = cc.apply_op(slack, {"op": "append", "canvas_id": "F1",
                                  "markdown": "more"})
        self.assertEqual(slack.calls[0][0], "edit_canvas")
        self.assertEqual(slack.calls[0][1]["mode"], "append")
        self.assertEqual(res["op"], "append")

    def test_read_maps_to_read_canvas_and_renders_contents(self):
        slack = FakeSlack(read={"ok": True, "title": "Roadmap",
                                "sections": ["s1", "s2", "s3"],
                                "note": "no raw body via API"})
        res = cc.apply_op(slack, {"op": "read", "canvas_id": "F7"})
        self.assertEqual(slack.calls[0][0], "read_canvas")
        self.assertEqual(slack.calls[0][1]["canvas_id"], "F7")
        self.assertTrue(res["ok"])
        self.assertEqual(res["canvas_id"], "F7")
        # Awareness contents render the section skeleton + the honesty note,
        # never a fabricated markdown body.
        self.assertIn("# Roadmap", res["contents"])
        self.assertIn("3 canvas section(s)", res["contents"])
        self.assertIn("no raw body via API", res["contents"])

    def test_list_maps_to_list_canvases(self):
        slack = FakeSlack(listing={"ok": True,
                                   "canvases": [{"canvas_id": "F1", "title": "A"},
                                                {"canvas_id": "F2", "title": "B"}]})
        res = cc.apply_op(slack, {"op": "list", "channel_id": "C5"})
        self.assertEqual(slack.calls[0][0], "list_canvases")
        self.assertEqual(slack.calls[0][1]["channel"], "C5")
        self.assertTrue(res["ok"])
        self.assertEqual(len(res["canvases"]), 2)

    def test_slack_error_is_captured_not_raised(self):
        slack = FakeSlack(edit={"ok": False, "error": "canvas_not_found"})
        res = cc.apply_op(slack, {"op": "update", "canvas_id": "FX",
                                  "markdown": "x"})
        self.assertFalse(res["ok"])
        self.assertEqual(res["error"], "canvas_not_found")
        self.assertTrue(res["bridged"])  # we DID attempt the round-trip

    def test_missing_required_field_is_failsoft(self):
        slack = FakeSlack()
        # update without canvas_id: the daemon should have validated, but the
        # consumer must not crash if it didn't.
        res = cc.apply_op(slack, {"op": "update", "markdown": "x"})
        self.assertFalse(res["ok"])
        self.assertIn("canvas_id", res["error"])

    def test_unknown_op_is_failsoft(self):
        slack = FakeSlack()
        res = cc.apply_op(slack, {"op": "obliterate"})
        self.assertFalse(res["ok"])
        self.assertIn("obliterate", res["error"])

    def test_transport_exception_is_failsoft(self):
        class Boom:
            def create_canvas(self, **_):
                raise RuntimeError("slack 500")
        res = cc.apply_op(Boom(), {"op": "create", "title": "x"})
        self.assertFalse(res["ok"])
        self.assertIn("slack 500", res["error"])


# ---------------------------------------------------------------------------
# handle_canvas_event — full per-event pipeline (parse → apply → deliver)
# ---------------------------------------------------------------------------

class HandlePipelineTest(unittest.TestCase):
    def test_create_round_trips_and_delivers_fulfillment(self):
        slack = FakeSlack(create={"ok": True, "canvas_id": "F_DONE"})
        deliver = RecordingDeliver()
        out = cc.handle_canvas_event(
            _evt({"op": "create", "title": "Plan", "markdown": "# Plan"}),
            client=slack, deliver=deliver)
        self.assertTrue(out["handled"])
        self.assertEqual(out["op"], "create")
        self.assertTrue(out["result"]["ok"])
        self.assertEqual(out["result"]["canvas_id"], "F_DONE")
        # Fulfillment posted with session + op + result.
        self.assertEqual(len(deliver.calls), 1)
        sid, op, result = deliver.calls[0]
        self.assertEqual(sid, "sess-1")
        self.assertEqual(op["op"], "create")
        self.assertEqual(result["canvas_id"], "F_DONE")

    def test_read_round_trips_real_contents_to_daemon(self):
        slack = FakeSlack(read={"ok": True, "title": "T", "sections": ["s1"],
                                "note": "n"})
        deliver = RecordingDeliver()
        out = cc.handle_canvas_event(_evt({"op": "read", "canvas_id": "F1"}),
                                     client=slack, deliver=deliver)
        self.assertTrue(out["handled"])
        # The delivered result carries awareness contents (not the pending
        # placeholder the daemon emitted).
        _, _, result = deliver.calls[0]
        self.assertIn("contents", result)
        self.assertTrue(result["bridged"])

    def test_non_canvas_event_is_skipped(self):
        slack = FakeSlack()
        deliver = RecordingDeliver()
        out = cc.handle_canvas_event({"type": "turn_finished", "session_id": "s"},
                                     client=slack, deliver=deliver)
        self.assertFalse(out["handled"])
        self.assertTrue(out["skipped"])
        self.assertEqual(len(slack.calls), 0)
        self.assertEqual(len(deliver.calls), 0)

    def test_delivery_failure_does_not_lose_the_result(self):
        slack = FakeSlack()
        deliver = RecordingDeliver(ok=False)  # daemon route absent / down
        out = cc.handle_canvas_event(_evt({"op": "append", "canvas_id": "F1",
                                           "markdown": "x"}),
                                     client=slack, deliver=deliver)
        # The Slack-side effect still happened; we just couldn't return it.
        self.assertTrue(out["handled"])
        self.assertTrue(out["result"]["ok"])
        self.assertFalse(out["delivery"]["ok"])


# ---------------------------------------------------------------------------
# deliver_fulfillment — fail-soft when the daemon route is absent
# ---------------------------------------------------------------------------

class DeliverFulfillmentTest(unittest.TestCase):
    def test_404_route_absent_is_failsoft(self):
        import urllib.error

        def fake_urlopen(req, timeout=None):
            raise urllib.error.HTTPError(req.full_url, 404, "Not Found", {}, None)

        orig = cc.urllib.request.urlopen
        cc.urllib.request.urlopen = fake_urlopen
        try:
            out = cc.deliver_fulfillment("s", {"op": "read"}, {"ok": True})
        finally:
            cc.urllib.request.urlopen = orig
        self.assertFalse(out["ok"])
        self.assertEqual(out["kind"], "route_absent")
        self.assertEqual(out["status"], 404)

    def test_daemon_unreachable_is_failsoft(self):
        def fake_urlopen(req, timeout=None):
            raise OSError("connection refused")

        orig = cc.urllib.request.urlopen
        cc.urllib.request.urlopen = fake_urlopen
        try:
            out = cc.deliver_fulfillment("s", {"op": "list"}, {"ok": True})
        finally:
            cc.urllib.request.urlopen = orig
        self.assertFalse(out["ok"])
        self.assertEqual(out["kind"], "daemon_unreachable")

    def test_success_returns_ok(self):
        class FakeResp:
            status = 200
            def read(self): return b'{"ok":true}'
            def __enter__(self): return self
            def __exit__(self, *a): return False

        def fake_urlopen(req, timeout=None):
            return FakeResp()

        orig = cc.urllib.request.urlopen
        cc.urllib.request.urlopen = fake_urlopen
        try:
            out = cc.deliver_fulfillment("s", {"op": "create"},
                                         {"ok": True, "canvas_id": "F1"})
        finally:
            cc.urllib.request.urlopen = orig
        self.assertTrue(out["ok"])
        self.assertEqual(out["status"], 200)


# ---------------------------------------------------------------------------
# SSE framing — the minimal stdlib event parser
# ---------------------------------------------------------------------------

class SseFramingTest(unittest.TestCase):
    def test_iter_sse_events_parses_data_frames(self):
        # Two events + a keep-alive comment, CRLF line endings like a real stream.
        stream = [
            b": keep-alive\r\n",
            b"event: slack_canvas\r\n",
            b'data: {"type":"slack_canvas","op":{"op":"create"}}\r\n',
            b"\r\n",
            b"event: turn_finished\r\n",
            b'data: {"type":"turn_finished"}\r\n',
            b"\r\n",
        ]
        events = list(cc._iter_sse_events(iter(stream)))
        self.assertEqual(len(events), 2)
        self.assertEqual(events[0]["type"], "slack_canvas")
        self.assertEqual(events[0]["op"]["op"], "create")
        self.assertEqual(events[1]["type"], "turn_finished")

    def test_iter_sse_events_handles_multiline_data(self):
        stream = [
            b'data: {"type":"slack_canvas",\n',
            b'data: "op":{"op":"list","channel_id":"C1"}}\n',
            b"\n",
        ]
        events = list(cc._iter_sse_events(iter(stream)))
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["op"]["channel_id"], "C1")

    def test_iter_sse_events_skips_bad_json(self):
        stream = [b"data: not json\n", b"\n",
                  b'data: {"type":"slack_canvas","op":{"op":"read","canvas_id":"F1"}}\n',
                  b"\n"]
        events = list(cc._iter_sse_events(iter(stream)))
        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["op"]["canvas_id"], "F1")


if __name__ == "__main__":
    unittest.main(verbosity=2)
