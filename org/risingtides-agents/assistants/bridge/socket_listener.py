#!/usr/bin/env python3
"""
Slack inbound bridge — LIVE Socket Mode listener (design spec §3, §4, §9).

The inbound plumbing for the assistants class. couriers/transport already handles
OUTBOUND Slack I/O; this is the front door: open an `xapp-` Socket Mode WebSocket,
receive Slack events (app_mention / message.im), ack them, dedupe Slack's retries,
resolve thread/DM context, dispatch each turn to the Ocean daemon scoped to the
agent's dir, and route the reply back in-thread via reply.py.

STATUS (OCEAN-112): LIVE. `run()` opens a Socket Mode connection (slack_sdk) and
pumps every inbound event through:

    dedupe → resolve_context → build_turn → dispatch_to_daemon → reply.deliver

with reconnect/backoff on WebSocket drop and per-message fail-soft (log + skip,
never crash the loop). The dispatch reuses OCEAN-84's HARDENED daemon invocation
(couriers/hub/router.py: call_daemon / session_id_for / extract_reply) — we do not
re-implement the /v1/prompt retry/timeout/error path here.

The same socket also carries the courier hub's slash-command intake: Slack
delivers `slash_commands` envelopes over Socket Mode, so `/ocean …` needs no
public ingress and no signing secret. Those envelopes are acked immediately
(3-second rule) and handed to slash_intake.py on a worker thread — see
slash_intake.py for the parse/dispatch/reply pipeline.

The pure pieces (EventDeduper, resolve_context, build_turn, session_key,
dispatch_to_daemon) stay stdlib + token-free so the whole pipeline is unit-testable
with a MOCKED Slack client + mocked daemon (tests/test_socket_pipeline.py). A real
live connection needs an `xapp-` app token + a Slack app with Socket Mode enabled
(spec §12) — see bridge/RUN.md.

Dependency: slack_sdk (Socket Mode client). Listed in bridge/requirements.txt.
Imported lazily so the pure helpers + `resolve` self-test run without it installed.

Usage:
  socket_listener.py resolve <manifest.toml>   # show the turn-scoping contract (JSON)
  socket_listener.py run <manifest.toml>        # LIVE — opens the Socket Mode loop
"""
import json
import logging
import os
import sys
import threading
import time
import urllib.request
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    print(json.dumps({"ok": False, "error": "needs Python 3.11+ (tomllib)"}), file=sys.stderr)
    sys.exit(1)

ASSISTANTS_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = ASSISTANTS_ROOT.parent
DAEMON_URL = os.environ.get("OCEAN_DAEMON_URL", "http://127.0.0.1:4780")

# Reuse OCEAN-84's hardened daemon invocation + deterministic session-id folding
# from the courier hub router instead of duplicating the retry/timeout/error path.
sys.path.insert(0, str(REPO_ROOT / "couriers" / "hub"))
try:  # pragma: no cover - exercised indirectly; guarded for import-order safety
    from router import call_daemon as _router_call_daemon  # type: ignore
    from router import extract_reply as _router_extract_reply  # type: ignore
    from router import session_id_for as _router_session_id_for  # type: ignore
    _router_import_error: Exception | None = None
except Exception as _e:  # pragma: no cover
    # The hardened OCEAN-84 daemon path (timeout + transient retry + typed result)
    # could not be imported. We fall through to the thin `_fallback_call_daemon`
    # below, which has NO retry/timeout hardening. Don't swallow this silently:
    # stash the error so run() can WARN once at startup, so operators can tell
    # "import failed, running the un-hardened fallback" apart from "daemon
    # transiently down". Behavior is unchanged — only made observable.
    _router_call_daemon = None  # type: ignore
    _router_extract_reply = None  # type: ignore
    _router_session_id_for = None  # type: ignore
    _router_import_error = _e

# Reply path: daemon output → couriers/transport/slack.py (thread/DM/canvas).
sys.path.insert(0, str(ASSISTANTS_ROOT / "bridge"))
try:  # pragma: no cover - guarded; mocked in tests
    import reply as _reply  # type: ignore
except Exception:  # pragma: no cover
    _reply = None  # type: ignore

# Courier-hub slash intake (`slash_commands` envelopes on this same socket).
# Guarded like reply: an import failure disables the intake, never the chat
# bridge — and we stash the error so run() can say WHY intake is off.
try:  # pragma: no cover - exercised via tests/test_slash_intake.py
    import slash_intake as _slash  # type: ignore
    _slash_import_error: Exception | None = None
except Exception as _se:  # pragma: no cover
    _slash = None  # type: ignore
    _slash_import_error = _se

log = logging.getLogger("ocean.bridge.slack")

# Known surface client_types — mirrors ocean-os surface_flag()/surface_dir()
# (crates/ocean-agent/src/lib.rs). A manifest declaring a client_type OUTSIDE this
# set would silently degrade to the daemon's generic "unknown client" fallback
# (losing the [SLACK]/surface profile), so we validate at startup (fix 4). Keep this
# in sync with the daemon's match arms.
KNOWN_CLIENT_TYPES = frozenset({
    "surface-extension",  # BRWSR
    "tui",                # TUI
    "surface-web",        # WEB
    "surface-gpui",       # GUI
    "surface-native",     # GUI
    "cli",                # CLI
    "leo-voice",          # VOX
    "acp-zed",            # ACP
    "surface-slack",      # SLACK
    "surface-canvas",     # CNVS
    "surface-mobile",     # MOBL
})


class EventDeduper:
    """Slack re-delivers events when the app doesn't ack in time (spec §9). Drop a
    repeat by Slack event id so a turn never double-fires. Bounded memory: keep the
    last N ids. Pure, testable, no I/O."""

    def __init__(self, capacity: int = 4096):
        self.capacity = capacity
        self._seen: dict[str, None] = {}

    def is_duplicate(self, event_id: str) -> bool:
        if not event_id:
            return False
        if event_id in self._seen:
            return True
        self._seen[event_id] = None
        if len(self._seen) > self.capacity:
            # FIFO evict oldest (dict preserves insertion order in 3.7+).
            oldest = next(iter(self._seen))
            del self._seen[oldest]
        return False


def session_key(agent: str, channel_id: str, thread_or_dm: str) -> str:
    """Bind a Slack session to (agent, channel/DM, thread) so threads stay isolated
    and resumable (spec §5). Mirrors content-agent.toml `session_key`."""
    return f"slack:{agent}:{channel_id}:{thread_or_dm}"


def resolve_context(event: dict) -> dict:
    """Turn a raw Slack event into the daemon-turn context. Pure: no I/O.

    Handles app_mention (channel/thread) and message.im (DM). The reply target is
    the thread_ts when present (keep threaded replies in-thread, spec §comms),
    else the message ts (start a thread), else the DM channel.
    """
    channel = event.get("channel", "")
    user = event.get("user", "")
    text = event.get("text", "")
    # In-thread reply target: existing thread_ts wins, else this message's ts.
    thread_ts = event.get("thread_ts") or event.get("ts") or ""
    is_dm = event.get("channel_type") == "im" or str(channel).startswith("D")
    thread_or_dm = "dm" if is_dm else thread_ts
    return {
        "channel": channel,
        "user": user,
        "prompt": text,
        "thread_ts": thread_ts,
        "is_dm": is_dm,
        "thread_or_dm": thread_or_dm,
    }


def build_turn(manifest_path: Path, event: dict) -> dict:
    """Compose the POST /v1/prompt body for a Slack event, scoped to the agent dir.

    The daemon maps client_type=surface-slack → [SLACK] + the Slack surface profile
    (ocean-os surface_dir), and load_project_prompt ancestor-walks the scoped cwd to
    pick up the agent's CLAUDE.md/AGENTS.md identity (spec §3 key seam).
    """
    with manifest_path.open("rb") as fh:
        m = tomllib.load(fh)
    surface = m.get("surface", {}).get("SLACK", {})
    agent = m.get("name", manifest_path.parent.name)
    ctx = resolve_context(event)
    cwd = surface.get("cwd") or str(manifest_path.parent.relative_to(REPO_ROOT))
    skey = session_key(agent, ctx["channel"], ctx["thread_or_dm"])
    return {
        "cwd": str((REPO_ROOT / cwd)),
        "prompt": ctx["prompt"],
        "client_type": surface.get("client_type", "surface-slack"),
        # human-readable spec §5 key (logs/observability); the daemon keys on the
        # UUIDv5 fold of it (see dispatch_to_daemon), matching OCEAN-84's router.
        "session_key": skey,
        # carried for the reply path (not part of the daemon contract, used by reply.py)
        "_reply": {"channel": ctx["channel"], "thread_ts": ctx["thread_ts"], "is_dm": ctx["is_dm"]},
    }


def _session_id_for(key: str) -> str:
    """Fold a stable session key → deterministic daemon UUID. Reuse OCEAN-84's
    router fold so a Slack thread resumes the SAME daemon session whether the turn
    came through the courier hub or this bridge. Mirror minimally if unavailable."""
    if _router_session_id_for is not None:
        return _router_session_id_for(key)
    import uuid  # pragma: no cover - fallback path only
    ns = uuid.UUID("6f9b1e2a-0c3d-5a7b-8e4f-1a2b3c4d5e6f")
    return str(uuid.uuid5(ns, key))


def dispatch_to_daemon(turn: dict) -> dict:
    """Dispatch a built turn to the Ocean daemon, REUSING OCEAN-84's hardened
    invocation (router.call_daemon: timeout + transient retry + typed result).

    The `_reply`/`session_key` bridge-side keys are stripped; the stable session
    key is folded into the daemon's UUID `session_id` (router.session_id_for) and
    `create_if_missing` lets the first message in a thread reserve that id rather
    than erroring on strict resume — exactly the courier router's payload shape.

    Returns the router's typed result: {"ok": bool, "response"?: PromptResponse,
    "kind"?, "error"?}. Never raises (the router never does).
    """
    skey = turn.get("session_key") or session_key(
        "content-agent", turn.get("_reply", {}).get("channel", ""), "dm")
    payload = {
        "cwd": turn["cwd"],
        "prompt": turn["prompt"],
        "client_type": turn.get("client_type", "surface-slack"),
        "session_id": _session_id_for(skey),
        "create_if_missing": True,
    }
    if _router_call_daemon is not None:
        return _router_call_daemon(payload)
    # Minimal mirror only if the router import failed (keeps the bridge functional).
    # WARN so this is observable: we're on the un-hardened fallback (no transient
    # retry, fixed 120s timeout) because the OCEAN-84 router import failed — not
    # because the daemon is transiently down. (Behavior unchanged; just surfaced.)
    log.warning(
        "router import unavailable (%s); using un-hardened _fallback_call_daemon "
        "(no retry/timeout hardening)", _router_import_error,
    )
    return _fallback_call_daemon(payload)  # pragma: no cover


def _fallback_call_daemon(payload: dict) -> dict:  # pragma: no cover - import-failure path
    """Last-resort daemon POST if the hardened router import is unavailable. Kept
    deliberately thin — the real path is router.call_daemon (OCEAN-84)."""
    req = urllib.request.Request(
        f"{DAEMON_URL}/v1/prompt",
        data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            parsed = json.loads(r.read())
        if isinstance(parsed, dict) and parsed.get("ok"):
            return {"ok": True, "response": parsed}
        return {"ok": False, "kind": "daemon_reported_error", "error": "ok=false", "response": parsed}
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "kind": "daemon_down", "error": str(e)}


def _reply_text_from(result: dict) -> str:
    """Pull the agent's reply text out of the router result, type-safely (reuse
    OCEAN-84's extract_reply over the PromptResponse)."""
    response = result.get("response") if isinstance(result, dict) else None
    if isinstance(response, dict) and _router_extract_reply is not None:
        return _router_extract_reply(response)
    if isinstance(response, dict):
        out = response.get("stdout")
        if isinstance(out, str) and out.strip():
            return out.strip()
    return "(the agent returned no text for this turn)"


def handle_event(manifest_path: Path, event: dict, deduper: EventDeduper,
                 deliver=None) -> dict:
    """The ONE message-handling pipeline, surfaced as a pure-ish function so it can
    be unit-tested end-to-end with a mocked daemon + mocked reply transport.

    inbound event → dedupe → resolve_context → build_turn → dispatch_to_daemon
    (OCEAN-84) → reply.deliver. Fail-soft: any error is caught and returned as a
    structured skip so the live loop never crashes on one bad message (spec §9).

    `deliver` is injected (defaults to reply.deliver) so tests pass a mock.
    """
    if deliver is None:
        if _reply is None:  # pragma: no cover - import guard
            return {"ok": False, "skipped": True, "reason": "reply transport unavailable"}
        deliver = _reply.deliver

    event_id = event.get("event_id") or event.get("client_msg_id") or ""
    if deduper.is_duplicate(event_id):
        log.info("dedupe: dropping repeat event %s", event_id)
        return {"ok": True, "skipped": True, "reason": "duplicate", "event_id": event_id}

    try:
        turn = build_turn(manifest_path, event)
    except Exception as e:  # noqa: BLE001
        log.exception("build_turn failed; skipping event %s", event_id)
        return {"ok": False, "skipped": True, "reason": f"build_turn: {e}"}

    # Empty prompt (e.g. a bare mention with no text, or a bot/system message we
    # don't want to round-trip) → skip rather than spend a daemon turn.
    if not (turn.get("prompt") or "").strip():
        return {"ok": True, "skipped": True, "reason": "empty prompt"}

    result = dispatch_to_daemon(turn)  # OCEAN-84 hardened invocation; never raises.
    reply_target = turn.get("_reply", {})

    if not result.get("ok"):
        # Daemon down / errored → tell the operator in-thread so it isn't silent.
        note = f":warning: couldn't reach the agent: {result.get('error', 'unknown error')}"
        try:
            deliver(reply_target, {"text": note})
        except Exception:  # noqa: BLE001
            log.exception("failed to deliver daemon-error notice")
        log.error("dispatch failed (%s): %s", result.get("kind"), result.get("error"))
        return {"ok": False, "kind": result.get("kind"), "error": result.get("error")}

    text = _reply_text_from(result)
    try:
        delivered = deliver(reply_target, {"text": text})
    except Exception as e:  # noqa: BLE001
        log.exception("reply delivery raised; turn completed but not delivered")
        return {"ok": False, "delivered": False, "reason": f"deliver: {e}", "reply": text}

    return {"ok": True, "delivered": bool(isinstance(delivered, dict) and delivered.get("ok")),
            "reply": text, "delivery": delivered}


# ---------------------------------------------------------------------------
# Live Socket Mode loop (slack_sdk). Imported lazily so the pure helpers above
# (and the `resolve` self-test + the unit tests) run without slack_sdk installed.
# ---------------------------------------------------------------------------

def _extract_inbound(payload: dict) -> dict | None:
    """Normalize a slack_sdk SocketModeRequest payload into the flat event dict our
    pipeline expects, or None if it's not an inbound message we handle.

    We act on `events_api` envelopes carrying an `app_mention` or a real user
    `message` (skip bot/self messages and message subtypes like edits/joins so we
    don't loop on our own replies — spec §9 'no startup auto-sends' spirit).
    """
    if payload.get("type") != "events_api":
        return None
    envelope = payload.get("payload") or {}
    event = envelope.get("event") or {}
    etype = event.get("type")
    if etype not in ("app_mention", "message"):
        return None
    # Ignore bot messages, our own messages, and non-user message subtypes.
    if event.get("bot_id") or event.get("subtype"):
        return None
    # event_id lives on the outer envelope; carry it onto the flat event for dedupe.
    flat = dict(event)
    flat["event_id"] = envelope.get("event_id") or event.get("event_id") or ""
    return flat


def run(manifest_path: Path | None = None):
    """LIVE: open the Socket Mode WebSocket and pump events through the pipeline.

    Reconnect/backoff on drop; per-message fail-soft via handle_event. Requires
    SLACK_APP_TOKEN (xapp-) + slack_sdk. The connection is the only piece that
    needs a live token — everything it calls (handle_event → OCEAN-84 dispatch →
    reply.deliver) is the same code the unit tests prove with mocks.
    """
    logging.basicConfig(
        level=os.environ.get("OCEAN_LOG_LEVEL", "INFO"),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    if manifest_path is None:
        # Default to the content-agent manifest (the v1 Slack assistant).
        manifest_path = ASSISTANTS_ROOT / "content-agent" / "content-agent.toml"
    manifest_path = Path(manifest_path).resolve()
    if not manifest_path.exists():
        _die(f"manifest not found: {manifest_path}")

    # Fix 2: warn if the daemon won't load THIS repo's authored surface profiles.
    # The daemon resolves authored profiles from OCEAN_ASSISTANTS_DIR (ocean-os
    # assistants_root()); if it's unset or points elsewhere, the daemon falls back
    # to its compiled-in seed prompts and never sees assistants/SLACK/system.md et al.
    env_assistants_dir = os.environ.get("OCEAN_ASSISTANTS_DIR", "").strip()
    if not env_assistants_dir:
        log.warning(
            "OCEAN_ASSISTANTS_DIR is unset — the daemon will use its compiled-in "
            "surface prompts and will NOT load this repo's authored profiles. "
            "Set OCEAN_ASSISTANTS_DIR=%s so the daemon loads them.", ASSISTANTS_ROOT,
        )
    else:
        try:
            mismatch = Path(env_assistants_dir).resolve() != ASSISTANTS_ROOT.resolve()
        except Exception:  # noqa: BLE001
            mismatch = True
        if mismatch:
            log.warning(
                "OCEAN_ASSISTANTS_DIR=%s differs from this bridge's ASSISTANTS_ROOT "
                "(%s) — the daemon won't load this repo's authored profiles. "
                "Set OCEAN_ASSISTANTS_DIR=%s to load them.",
                env_assistants_dir, ASSISTANTS_ROOT, ASSISTANTS_ROOT,
            )

    # Fix 4: validate the manifest's declared client_type against the known surface
    # set. A typo (e.g. "surface-slak") would silently degrade to the daemon's
    # generic "unknown client" fallback, losing the [SLACK] surface profile — fail
    # loud at startup instead.
    try:
        with manifest_path.open("rb") as _fh:
            _manifest = tomllib.load(_fh)
        _declared = (_manifest.get("surface", {}).get("SLACK", {})
                     .get("client_type", "surface-slack"))
    except Exception as e:  # noqa: BLE001
        _die(f"could not read client_type from manifest {manifest_path}: {e}")
    if _declared not in KNOWN_CLIENT_TYPES:
        log.error(
            "manifest %s declares unknown client_type %r — not in the known surface "
            "set %s; refusing to start so it doesn't silently fall back to the "
            "daemon's unknown-client prompt. Fix the manifest's [surface.SLACK] "
            "client_type.", manifest_path, _declared, sorted(KNOWN_CLIENT_TYPES),
        )
        _die(f"unknown client_type {_declared!r} in {manifest_path}")

    app_token = os.environ.get("SLACK_APP_TOKEN", "").strip()
    if not app_token.startswith("xapp-"):
        _die("SLACK_APP_TOKEN (xapp-...) is required for Socket Mode. See bridge/RUN.md "
             "and bridge.env.example.")

    try:
        from slack_sdk.socket_mode import SocketModeClient  # type: ignore
        from slack_sdk.socket_mode.request import SocketModeRequest  # type: ignore
        from slack_sdk.socket_mode.response import SocketModeResponse  # type: ignore
        from slack_sdk.web import WebClient  # type: ignore
    except Exception:  # noqa: BLE001
        _die("slack_sdk not installed. `pip install -r assistants/bridge/requirements.txt` "
             "(see bridge/RUN.md).")

    bot_token = os.environ.get("OCEAN_SLACK_BOT_TOKEN") or os.environ.get("SLACK_BOT_TOKEN")
    deduper = EventDeduper()

    def _on_request(client, req):  # SocketModeClient handler signature.
        # ACK FIRST (spec §9): ack within Slack's window so it doesn't re-deliver;
        # dedupe still guards against the races where it does. Slash commands may
        # ack WITH a body (an ephemeral "running…" the invoker sees inside the
        # 3-second window); events ack bare.
        is_slash = req.type == "slash_commands"
        ack_body = None
        if is_slash and _slash is not None:
            try:
                ack_body = _slash.build_ack(dict(req.payload or {}))
            except Exception:  # noqa: BLE001
                log.exception("build_ack failed; acking bare")
        try:
            if ack_body is not None:
                client.send_socket_mode_response(
                    SocketModeResponse(envelope_id=req.envelope_id, payload=ack_body))
            else:
                client.send_socket_mode_response(SocketModeResponse(envelope_id=req.envelope_id))
        except Exception:  # noqa: BLE001
            log.exception("failed to ack envelope %s", getattr(req, "envelope_id", "?"))

        if is_slash:
            # Courier-hub intake (/ocean …): execute OFF this thread so a slow
            # harness or daemon turn never blocks the WS processor; the result is
            # posted asynchronously via the payload's response_url.
            if _slash is None:
                log.error("slash_commands envelope received but slash_intake failed "
                          "to import (%s); ignoring", _slash_import_error)
                return
            threading.Thread(
                target=_slash.handle_slash, args=(dict(req.payload or {}),),
                daemon=True, name="slash-intake",
            ).start()
            return

        event = _extract_inbound({"type": req.type, "payload": req.payload})
        if event is None:
            return
        # Fail-soft: handle_event never raises, but belt-and-suspenders the loop.
        try:
            out = handle_event(manifest_path, event, deduper)
            if not out.get("ok") and not out.get("skipped"):
                log.warning("turn not delivered: %s", out)
        except Exception:  # noqa: BLE001
            log.exception("handle_event crashed (should be impossible); loop continues")

    web = WebClient(token=bot_token) if bot_token else None
    backoff = 1.0
    log.info("content-agent Slack bridge starting (manifest=%s, daemon=%s)",
             manifest_path, DAEMON_URL)
    while True:
        client = None
        try:
            client = SocketModeClient(app_token=app_token, web_client=web)
            client.socket_mode_request_listeners.append(_on_request)
            client.connect()
            log.info("Socket Mode connected; listening for app_mention + message "
                     "events + slash_commands (courier hub intake)")
            backoff = 1.0  # reset on a healthy connect
            # Block this thread while the WS pumps events on its own threads.
            while client.is_connected():
                time.sleep(1)
            log.warning("Socket Mode disconnected; reconnecting")
        except KeyboardInterrupt:
            log.info("shutting down (KeyboardInterrupt)")
            break
        except Exception as e:  # noqa: BLE001
            log.error("Socket Mode error: %s; reconnecting in %.1fs", e, backoff)
            time.sleep(backoff)
            backoff = min(backoff * 2, 30.0)  # exponential backoff, capped
        finally:
            try:
                if client is not None:
                    client.close()
            except Exception:  # noqa: BLE001
                pass


def cmd_resolve(args):
    """Show the turn-scoping contract for a manifest against a sample event."""
    if not args:
        _die("usage: socket_listener.py resolve <manifest.toml>")
    manifest = Path(args[0]).resolve()
    if not manifest.exists():
        _die(f"manifest not found: {manifest}")
    sample = {"type": "app_mention", "channel": "C123", "user": "U1",
              "text": "make me a clip", "ts": "1700000000.000100", "event_id": "Ev1"}
    turn = build_turn(manifest, sample)
    turn_view = {**turn, "daemon_session_id": _session_id_for(turn["session_key"])}
    print(json.dumps({"ok": True, "sample_event": sample, "turn": turn_view}, indent=2))


def _die(msg, code=1):
    print(json.dumps({"ok": False, "error": msg}), file=sys.stderr)
    sys.exit(code)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    cmd, rest = sys.argv[1], sys.argv[2:]
    if cmd == "resolve":
        cmd_resolve(rest)
    elif cmd == "run":
        manifest = Path(rest[0]) if rest else None
        run(manifest)
    else:
        _die(f"unknown command '{cmd}' (try: resolve | run)")


if __name__ == "__main__":
    main()
