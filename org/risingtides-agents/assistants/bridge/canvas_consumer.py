#!/usr/bin/env python3
"""
Slack canvas bridge consumer — daemon `slack_canvas` ops → Slack Canvas API (OCEAN-244).

The agent's `slack_canvas` tool (ocean-os runtime, OCEAN-214) emits validated
canvas ops (create / read / update / append / list). The ocean-os daemon relays
each as an `AgentTurnEvent::SlackCanvas` over the `/v1/agent/events` SSE stream
(the seam OCEAN-235/#165 defines). Nothing on the risingtides-agents side consumed
them: a `create`/`update` dead-ended, and a `read` handed the agent the daemon's
`pending_bridge` placeholder forever. This module is that missing consumer.

What it does
------------
  subscribe /v1/agent/events (SSE)
    → filter to slack_canvas events
    → map the op to couriers/transport/slack.py (create/edit/read/list)
    → round-trip the fulfilled result back to the daemon (fulfillment seam)

This is the OTHER half of the daemon's POST-and-block reply path (router.py /
socket_listener.py): those consume `/v1/prompt` synchronously for chat replies;
canvas ops are async side-effects that arrive on the SSE bus, so they need a
*subscriber*, not a request. This module is the first SSE consumer in the bridge.

The op→API mapping and result-building are PURE (stdlib, token-free) so the whole
contract is unit-testable with a mocked Slack transport + synthetic events
(tests/test_canvas_consumer.py). Only `run()` opens the live SSE connection and
needs a running daemon + a real Slack token.

Cross-repo status (verified against ocean-os `main`, 2026-06)
-------------------------------------------------------------
Both halves of the seam are now LIVE on ocean-os `main`:

  1. The daemon EMITS `slack_canvas` on the SSE wire — it has a dedicated
     `AgentTurnEvent::SlackCanvas` relay arm, so canvas ops reach this consumer.
  2. The daemon mounts the fulfillment-return ENDPOINT at `/v1/agent/canvas/fulfill`,
     so `deliver_fulfillment()` posts the bridged read/list result back and the
     agent's `pending_bridge` placeholder resolves to real content.

`deliver_fulfillment()` still fail-softs (logs, never crashes) on a 404 or an
unreachable daemon, so the consumer degrades gracefully if the daemon is down or
running an older build.

Usage:
  canvas_consumer.py describe                 # JSON self-description (no I/O)
  canvas_consumer.py run                       # LIVE — open the SSE subscription
"""
import json
import logging
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

ASSISTANTS_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = ASSISTANTS_ROOT.parent
DAEMON_URL = os.environ.get("OCEAN_DAEMON_URL", "http://127.0.0.1:4780")

# The SSE event `type` (serde snake_case tag on AgentTurnEvent) we consume. The
# daemon also sets this as the SSE `event:` name (agent_event_type_name), so we
# match on either the parsed JSON `type` or the SSE event line.
SLACK_CANVAS_EVENT_TYPE = "slack_canvas"

# Where we POST a fulfilled read/list result back so the agent's pending op
# resolves to real content. This route is mounted on the daemon (ocean-os main);
# deliver_fulfillment still fail-softs on 404 for older daemon builds. Override
# via env if the path ever changes.
FULFILLMENT_PATH = os.environ.get("OCEAN_CANVAS_FULFILL_PATH", "/v1/agent/canvas/fulfill")

# Reply path reuse: couriers/transport/slack.py owns all Slack auth/retry/I/O.
sys.path.insert(0, str(REPO_ROOT / "couriers" / "transport"))
try:  # pragma: no cover - guarded; mocked in tests
    from slack import Slack  # type: ignore  # couriers/transport/slack.py
except Exception:  # pragma: no cover
    Slack = None  # type: ignore

log = logging.getLogger("ocean.bridge.canvas")


# ---------------------------------------------------------------------------
# Event parsing — pure
# ---------------------------------------------------------------------------

def parse_canvas_event(raw: dict) -> dict | None:
    """Extract the canvas op + session from an SSE `AgentTurnEvent` payload.

    The daemon serializes `AgentTurnEvent` internally-tagged on `type`; the
    `slack_canvas` variant carries `session_id` and an `op` object shaped by the
    SDK `SlackCanvasOp` (internally tagged on `op`, e.g.
    `{"op": "create", "title": "…", "markdown": "…"}`).

    Returns `{session_id, op}` where `op` is the inner op dict, or `None` if this
    isn't a slack_canvas event / is malformed. Pure: no I/O.
    """
    if not isinstance(raw, dict):
        return None
    if raw.get("type") != SLACK_CANVAS_EVENT_TYPE:
        return None
    op = raw.get("op")
    if not isinstance(op, dict) or not isinstance(op.get("op"), str):
        return None
    return {"session_id": raw.get("session_id"), "op": op}


# ---------------------------------------------------------------------------
# Op → Slack transport mapping — pure (transport is injected)
# ---------------------------------------------------------------------------

def apply_op(client, op: dict) -> dict:
    """Map ONE validated `slack_canvas` op to its Slack Canvas API call.

    `client` is a couriers.transport.slack.Slack (or a mock with the same
    surface). `op` is the inner op dict (`{"op": "...", ...}`). Returns a
    NORMALIZED fulfillment result the agent can reason over:

        {ok, op, canvas_id?, contents?, canvases?, bridged: True, error?, raw}

    Shape mirrors the SDK `SlackCanvasResult` (ok/op/canvas_id/contents/canvases)
    with `bridged: True` — the marker meaning "the bridge round-tripped this to
    the live Slack API", flipping the daemon's `bridged:false` / `pending_bridge`
    placeholder. Never raises: a transport/Slack error is captured into `error`
    with `ok:false` so one bad op can't crash the consumer loop.
    """
    name = op.get("op")
    try:
        if name == "create":
            res = client.create_canvas(
                title=op.get("title"),
                markdown=op.get("markdown") or "",
                channel=op.get("channel_id"),
            )
            return _result("create", ok=res.get("ok", False),
                           canvas_id=res.get("canvas_id"),
                           error=None if res.get("ok") else res.get("error"),
                           raw=res)

        if name in ("update", "append"):
            # `append` is its own op; `update` carries a mode (default replace).
            mode = "append" if name == "append" else (op.get("mode") or "replace")
            res = client.edit_canvas(op["canvas_id"], op["markdown"], mode=mode)
            return _result(name, ok=res.get("ok", False),
                           canvas_id=op.get("canvas_id"),
                           error=None if res.get("ok") else res.get("error"),
                           raw=res)

        if name == "read":
            res = client.read_canvas(op["canvas_id"])
            # `read_canvas` already normalizes; surface its structure as the
            # awareness `contents`. Slack exposes no raw markdown body, so
            # `contents` is the section skeleton + an honesty note, never a
            # fabricated body (see read_canvas docstring).
            contents = _render_read_contents(res)
            return _result("read", ok=res.get("ok", False),
                           canvas_id=op.get("canvas_id"), contents=contents,
                           error=None if res.get("ok") else "read failed",
                           raw=res)

        if name == "list":
            res = client.list_canvases(op["channel_id"])
            return _result("list", ok=res.get("ok", False),
                           canvases=res.get("canvases", []),
                           error=None if res.get("ok") else "list failed",
                           raw=res)

        return _result(name or "unknown", ok=False,
                       error=f"unknown slack_canvas op {name!r}", raw={})
    except KeyError as e:  # a required field the daemon should have validated
        return _result(name or "unknown", ok=False,
                       error=f"missing field {e} for op {name!r}", raw={})
    except Exception as e:  # noqa: BLE001 - transport blew up; fail soft
        log.exception("apply_op(%s) raised", name)
        return _result(name or "unknown", ok=False, error=str(e), raw={})


def _result(op: str, *, ok: bool, canvas_id=None, contents=None, canvases=None,
            error=None, raw=None) -> dict:
    """Build the normalized, bridged fulfillment result (SDK-result shaped)."""
    out: dict = {"op": op, "ok": ok, "bridged": True}
    if canvas_id is not None:
        out["canvas_id"] = canvas_id
    if contents is not None:
        out["contents"] = contents
    if canvases is not None:
        out["canvases"] = canvases
    if error:
        out["ok"] = False
        out["error"] = error
    out["raw"] = raw or {}
    return out


def _render_read_contents(read_result: dict) -> str:
    """Render `read_canvas`'s normalized structure into an awareness string.

    Slack gives section structure + metadata, not the markdown body, so the
    awareness payload is the heading skeleton plus an explicit note about the
    limitation — honest about what the agent is and isn't seeing.
    """
    title = read_result.get("title")
    sections = read_result.get("sections") or []
    lines = []
    if title:
        lines.append(f"# {title}")
    if sections:
        lines.append(f"({len(sections)} canvas section(s) found)")
        lines.extend(f"- section {sid}" for sid in sections)
    else:
        lines.append("(no sections returned)")
    note = read_result.get("note")
    if note:
        lines.append("")
        lines.append(f"NOTE: {note}")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Fulfillment return — POST the bridged result back to the daemon
# ---------------------------------------------------------------------------

def deliver_fulfillment(session_id, op: dict, result: dict,
                        daemon_url: str | None = None, timeout: float = 30.0) -> dict:
    """POST the bridged result back to the daemon's canvas-fulfillment seam.

    The agent's `read`/`list` op is left as a `pending_bridge` placeholder until
    a fulfilled result is delivered. This posts `{session_id, op, result}` to the
    daemon's `/v1/agent/canvas/fulfill` route so the placeholder resolves to real
    content. That route is mounted on ocean-os `main`.

    On 404/connection error we LOG and return a structured note — we never raise —
    so the consumer degrades gracefully against a down or older daemon build.
    The mutating ops (create/update/append) don't strictly need a return (their
    effect is already live in Slack), but we still post so the daemon can mint the
    real `canvas_id` back to the agent for `create`.
    """
    base = (daemon_url or DAEMON_URL).rstrip("/")
    url = f"{base}{FULFILLMENT_PATH}"
    payload = json.dumps({
        "session_id": session_id,
        "op": op,
        "result": result,
    }).encode()
    req = urllib.request.Request(
        url, data=payload, headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            body = r.read().decode()
        return {"ok": True, "status": r.status, "body": body[:500]}
    except urllib.error.HTTPError as e:
        # 404 = the daemon doesn't expose the fulfillment route (e.g. an older
        # build; it is mounted on current ocean-os main). Other 4xx/5xx = a real
        # delivery problem.
        if e.code == 404:
            log.info(
                "fulfillment route %s returned 404 — daemon build predates the "
                "mounted route; result computed but not returned. "
                "op=%s ok=%s", url, op.get("op"), result.get("ok"),
            )
            return {"ok": False, "status": 404, "kind": "route_absent"}
        log.warning("fulfillment POST %s failed: http_%s", url, e.code)
        return {"ok": False, "status": e.code, "kind": "http_error"}
    except Exception as e:  # noqa: BLE001 - daemon down / network; fail soft
        log.warning("fulfillment POST %s failed: %s", url, e)
        return {"ok": False, "kind": "daemon_unreachable", "error": str(e)}


# ---------------------------------------------------------------------------
# Per-event handler — pure-ish (transport + deliver injected for tests)
# ---------------------------------------------------------------------------

def handle_canvas_event(raw_event: dict, client=None, deliver=None) -> dict:
    """The ONE canvas-op pipeline, surfaced for end-to-end unit testing.

    parse → apply_op (Slack API) → deliver_fulfillment (daemon return).
    Fail-soft: any error is captured into a structured result so the live SSE
    loop never dies on one bad event. `client`/`deliver` are injected so tests
    pass a mocked transport + a recording deliver.

    Returns `{handled, result?, delivery?, skipped?, reason?}`.
    """
    parsed = parse_canvas_event(raw_event)
    if parsed is None:
        return {"handled": False, "skipped": True, "reason": "not a slack_canvas event"}

    if client is None:
        if Slack is None:  # pragma: no cover - import guard
            return {"handled": False, "skipped": True,
                    "reason": "slack transport unavailable"}
        client = Slack()
    if deliver is None:
        deliver = deliver_fulfillment

    op = parsed["op"]
    result = apply_op(client, op)
    delivery = deliver(parsed["session_id"], op, result)
    return {"handled": True, "op": op.get("op"), "result": result,
            "delivery": delivery}


# ---------------------------------------------------------------------------
# Live SSE subscription loop (stdlib urllib streaming). Only this needs a
# running daemon + a real Slack token; everything above is unit-tested with mocks.
# ---------------------------------------------------------------------------

def _iter_sse_events(resp):
    """Yield parsed SSE event dicts from a streaming urllib response.

    Minimal SSE framing: accumulate `data:` lines until a blank line, then yield
    the JSON-decoded payload. `event:`/`id:` lines are read but we route on the
    JSON `type`, so they're informational. Stdlib only — no sseclient dep,
    matching the repo's pure-stdlib transport ethos.
    """
    data_lines: list[str] = []
    for rawline in resp:
        line = rawline.decode("utf-8", "replace").rstrip("\n").rstrip("\r")
        if line == "":
            if data_lines:
                blob = "\n".join(data_lines)
                data_lines = []
                try:
                    yield json.loads(blob)
                except json.JSONDecodeError:
                    log.debug("non-JSON SSE data frame skipped: %s", blob[:120])
            continue
        if line.startswith(":"):
            continue  # SSE comment / keep-alive
        if line.startswith("data:"):
            data_lines.append(line[len("data:"):].lstrip())
        # event:/id:/retry: lines are ignored (we route on JSON `type`).


def run(daemon_url: str | None = None):
    """LIVE: subscribe to `/v1/agent/events` and round-trip canvas ops.

    Opens the SSE stream with `?all=1` (canvas events are session-scoped, so the
    firehose opt-in is required to receive them without pre-knowing a session id),
    reconnects with backoff on drop, and fail-softs per event. Requires a running
    daemon and a Slack bot token (resolved by the transport).

    NOTE: ocean-os `main` relays `slack_canvas` on the SSE wire, so a running
    daemon delivers canvas events to this subscription end-to-end.
    """
    logging.basicConfig(
        level=os.environ.get("OCEAN_LOG_LEVEL", "INFO"),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )
    if Slack is None:
        _die("couriers/transport/slack.py not importable; cannot make Slack calls.")

    base = (daemon_url or DAEMON_URL).rstrip("/")
    # `?all=1`: session-scoped events (slack_canvas carries a session_id) only
    # reach a subscriber that either names that session or opts into the firehose.
    # The bridge doesn't know session ids up front, so it takes the firehose.
    url = f"{base}/v1/agent/events?all=1"

    client = Slack()
    backoff = 1.0
    log.info("slack canvas consumer starting (daemon=%s, fulfill=%s)",
             url, FULFILLMENT_PATH)
    while True:
        try:
            req = urllib.request.Request(url, headers={"Accept": "text/event-stream"})
            with urllib.request.urlopen(req, timeout=None) as resp:
                log.info("subscribed to %s; waiting for slack_canvas events", url)
                backoff = 1.0  # healthy connect
                for event in _iter_sse_events(resp):
                    if event.get("type") != SLACK_CANVAS_EVENT_TYPE:
                        continue  # not ours; ignore the rest of the firehose
                    try:
                        out = handle_canvas_event(event, client=client)
                        log.info("handled slack_canvas op=%s ok=%s delivered=%s",
                                 out.get("op"),
                                 (out.get("result") or {}).get("ok"),
                                 (out.get("delivery") or {}).get("ok"))
                    except Exception:  # noqa: BLE001 - belt & suspenders
                        log.exception("handle_canvas_event crashed; loop continues")
        except KeyboardInterrupt:
            log.info("shutting down (KeyboardInterrupt)")
            break
        except Exception as e:  # noqa: BLE001 - SSE drop / daemon down → reconnect
            log.warning("SSE stream error: %s; reconnecting in %.1fs", e, backoff)
            time.sleep(backoff)
            backoff = min(backoff * 2, 30.0)


def cmd_describe(_args):
    """JSON self-description of the consumer's contract (no I/O)."""
    print(json.dumps({
        "ok": True,
        "consumes": f"GET {DAEMON_URL}/v1/agent/events?all=1 (SSE)",
        "event_type": SLACK_CANVAS_EVENT_TYPE,
        "op_map": {
            "create": "couriers.transport.slack.Slack.create_canvas → canvases.create",
            "update": "Slack.edit_canvas(mode) → canvases.edit (replace/insert_at_*)",
            "append": "Slack.edit_canvas('append') → canvases.edit insert_at_end",
            "read": "Slack.read_canvas → files.info + canvases.sections.lookup (no raw body)",
            "list": "Slack.list_canvases → files.list types=spaces (canvas filetype)",
        },
        "fulfillment": f"POST {DAEMON_URL}{FULFILLMENT_PATH} {{session_id, op, result}}",
        "cross_repo_notes": [
            "ocean-os main relays AgentTurnEvent::SlackCanvas over SSE (events arrive)",
            "ocean-os main mounts the /v1/agent/canvas/fulfill route (deliver_fulfillment posts back; fail-softs on 404 for older builds)",
            "Slack API has no raw-markdown canvas read; read returns section structure + note",
        ],
    }, indent=2))


def _die(msg, code=1):
    print(json.dumps({"ok": False, "error": msg}), file=sys.stderr)
    sys.exit(code)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    cmd, rest = sys.argv[1], sys.argv[2:]
    if cmd == "describe":
        cmd_describe(rest)
    elif cmd == "run":
        run()
    else:
        _die(f"unknown command '{cmd}' (try: describe | run)")


if __name__ == "__main__":
    main()
