#!/usr/bin/env python3
"""
Courier hub router — the spine of the courier system.

Discovers every `courier.toml` under couriers/, builds the slash-command →
courier table, and dispatches an invocation:

  • deterministic courier → exec its harness entry directly (no LLM)
  • agentic courier        → POST /v1/prompt to the Ocean daemon, scoped to the
                             courier's dir (the daemon auto-loads its CLAUDE.md),
                             then route the agent's reply back over transport.

This is the routing layer of the hub described in ../ARCHITECTURE.md. Slack
slash-command intake (the front door) is a separate, outward-facing layer that
calls into this router; it is intentionally not built here.

Daemon invocation contract (grounded in ocean-os, read-only):
  POST /v1/prompt is **synchronous and blocking** — it returns a complete
  `PromptResponse` JSON body (ocean-core/src/lib.rs PromptResponse), NOT a
  fire-and-forget ack. The reply text the agent produced is in `stdout`; the
  resumable session id is in `session_id` (a UUID). So the reply path does not
  need to consume the `/v1/agent/events` SSE stream — the round-trip already
  lands the answer in the response body. We parse it, defensively, and hand the
  reply text to the transport for delivery.

Session-key consistency (design spec §5):
  Slack threads must reuse the same daemon session so context survives across
  messages. The spec's session key is `slack:<agent>:<channel_id>:<thread_ts|dm>`.
  The daemon's `session_id` is a UUID (ocean-core: `SessionId = Uuid`), so we
  fold the stable string key into a deterministic UUIDv5. Same thread → same
  UUID → same daemon session, with no daemon-side change required.

Pure stdlib. TOML via tomllib (Python 3.11+).

Usage:
  router.py list                       # show all couriers + commands
  router.py resolve <slash>            # which courier handles a command (JSON)
  router.py run [--dry-run] [--channel <id>] [--thread-ts <ts>] [--session-key <k>] \
                <slash> [args...]
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any, NoReturn

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    print(json.dumps({"ok": False, "error": "needs Python 3.11+ (tomllib)"}), file=sys.stderr)
    sys.exit(1)

COURIERS_ROOT = Path(__file__).resolve().parents[1]
DAEMON_URL = os.environ.get("OCEAN_DAEMON_URL", "http://127.0.0.1:4780")

# Stable namespace for folding a Slack session key into a daemon UUID session id.
# Fixed (not random) so the same (agent, channel, thread) always maps to the same
# UUID across processes/restarts — that's the whole point of session reuse.
_SESSION_NS = uuid.UUID("6f9b1e2a-0c3d-5a7b-8e4f-1a2b3c4d5e6f")

# How long to wait on the daemon. The agent turn runs server-side and the call
# blocks until it finishes, so this is generous; override via env for slow turns.
DAEMON_TIMEOUT = float(os.environ.get("OCEAN_DAEMON_TIMEOUT", "300"))
DAEMON_MAX_TRIES = int(os.environ.get("OCEAN_DAEMON_MAX_TRIES", "3"))


def session_key(courier: str, channel: str | None, thread_ts: str | None,
                explicit: str | None = None) -> str:
    """Build the spec §5 session key `slack:<agent>:<channel_id>:<thread_ts|dm>`.

    An explicit key (passed by the intake adapter) always wins. Otherwise we
    compose it from the courier name + Slack routing context. `dm` is the
    sentinel for a thread-less destination (a DM or top-level channel post), so
    a whole DM conversation shares one session.
    """
    if explicit:
        return explicit
    chan = channel or "unknown"
    thread = thread_ts or "dm"
    return f"slack:{courier}:{chan}:{thread}"


def session_id_for(key: str) -> str:
    """Fold a stable session key into a deterministic daemon session UUID.

    The daemon's `session_id` is a UUID (`SessionId = Uuid` in ocean-core), and
    UUIDv5 is content-addressed: the same key always yields the same UUID, so
    repeated messages in a Slack thread resume the same daemon session instead of
    spawning a fresh transcript each time.
    """
    return str(uuid.uuid5(_SESSION_NS, key))


def call_daemon(payload: dict) -> dict[str, Any]:
    """POST /v1/prompt with timeout, transient-failure retry, and a typed result.

    Returns a dict that ALWAYS has an "ok" bool. On success it carries the parsed
    daemon `PromptResponse` under "response"; on failure it carries an "error"
    string and "kind" classifier. Never raises — callers branch on "ok".

    /v1/prompt is synchronous: the daemon runs the whole agent turn and returns
    the reply in the response body, so a single blocking call is the reply path.
    """
    data = json.dumps(payload).encode()
    last_err = "unknown"
    for attempt in range(1, DAEMON_MAX_TRIES + 1):
        req = urllib.request.Request(
            f"{DAEMON_URL}/v1/prompt",
            data=data,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=DAEMON_TIMEOUT) as r:
                raw = r.read().decode()
        except urllib.error.HTTPError as e:
            # 5xx is transient (daemon overloaded / restarting); 4xx is a bug in
            # our request — don't hammer it.
            body = ""
            try:
                body = e.read().decode()
            except Exception:  # noqa: BLE001
                pass
            last_err = f"http_{e.code}: {body[:300]}".strip()
            if 500 <= e.code < 600 and attempt < DAEMON_MAX_TRIES:
                time.sleep(min(2 ** attempt, 8))
                continue
            return {"ok": False, "kind": "http_error", "error": last_err}
        except urllib.error.URLError as e:
            # Connection refused / DNS / timeout — daemon down or slow. Retry.
            last_err = f"daemon unreachable ({DAEMON_URL}): {e.reason}"
            if attempt < DAEMON_MAX_TRIES:
                time.sleep(min(2 ** attempt, 8))
                continue
            return {"ok": False, "kind": "daemon_down", "error": last_err}
        except TimeoutError:
            last_err = f"daemon timed out after {DAEMON_TIMEOUT:.0f}s"
            if attempt < DAEMON_MAX_TRIES:
                time.sleep(min(2 ** attempt, 8))
                continue
            return {"ok": False, "kind": "timeout", "error": last_err}

        # Parse the body defensively — the daemon is the source of truth, but a
        # truncated/garbled body must not crash the courier (Pyright-safe: every
        # field access below is guarded, no None-subscripting).
        try:
            parsed: Any = json.loads(raw)
        except json.JSONDecodeError:
            return {"ok": False, "kind": "bad_body",
                    "error": f"daemon returned non-JSON: {raw[:300]}"}
        if not isinstance(parsed, dict):
            return {"ok": False, "kind": "bad_body",
                    "error": f"daemon returned non-object JSON: {raw[:200]}"}
        if not parsed.get("ok", False):
            return {"ok": False, "kind": "daemon_reported_error",
                    "error": str(parsed.get("stderr") or parsed.get("error")
                                 or "daemon reported ok=false"),
                    "response": parsed}
        return {"ok": True, "response": parsed}

    return {"ok": False, "kind": "daemon_down", "error": last_err}


def extract_reply(response: dict[str, Any]) -> str:
    """Pull the agent's reply text out of a daemon PromptResponse, type-safely.

    The reply lives in `stdout`; fall back to `stderr` for a failed turn so the
    operator still sees *something* rather than silence. Guarded so a missing or
    non-string field can never raise (Pyright None-subscript safe).
    """
    out = response.get("stdout")
    if isinstance(out, str) and out.strip():
        return out.strip()
    err = response.get("stderr")
    if isinstance(err, str) and err.strip():
        return err.strip()
    return "(the agent returned no text for this turn)"


def deliver_reply(channel: str | None, thread_ts: str | None, text: str) -> dict:
    """Route the agent's reply back to Slack over the shared transport.

    Imported lazily so deterministic runs and `--dry-run` never need a Slack
    token. If no channel is known we can't deliver — return a structured note so
    the caller (and any wrapping intake adapter) can decide what to do.
    """
    if not channel:
        return {"ok": False, "error": "no channel to deliver reply to",
                "kind": "no_destination"}
    sys.path.insert(0, str(COURIERS_ROOT))
    try:
        from transport.slack import Slack  # type: ignore[import-not-found]
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "error": f"transport import failed: {e}",
                "kind": "transport_unavailable"}
    try:
        return Slack().post_message(channel, text=text, thread_ts=thread_ts)
    except Exception as e:  # noqa: BLE001
        return {"ok": False, "error": f"transport send failed: {e}",
                "kind": "transport_error"}


def load_registry() -> dict[str, dict[str, Any]]:
    """slash-command -> {courier, dir, mode, entry, command}."""
    table: dict[str, dict[str, Any]] = {}
    for manifest in sorted(COURIERS_ROOT.glob("*/courier.toml")):
        with manifest.open("rb") as fh:
            m = tomllib.load(fh)
        cdir = manifest.parent
        for cmd in m.get("commands", []):
            slash = cmd.get("slash")
            if not slash:
                continue
            table[slash] = {
                "courier": m.get("name", cdir.name),
                "dir": str(cdir),
                "mode": m.get("mode", "deterministic"),
                "entry": m.get("entry", "bin/courier"),
                "command": cmd,
            }
    return table


def cmd_list(_args):
    table = load_registry()
    if not table:
        print("(no courier.toml manifests found under couriers/)")
        return
    width = max(len(s) for s in table)
    for slash in sorted(table):
        e = table[slash]
        c = e["command"]
        print(f"{slash:<{width}}  [{e['mode']:^13}] {e['courier']:<14} {c.get('summary','')}")
        if c.get("usage"):
            print(f"{'':<{width}}  {'':15} {'':14} ↳ {c['usage']}")


def cmd_resolve(args):
    if not args:
        die("usage: router.py resolve <slash>")
    table = load_registry()
    hit = table.get(args[0])
    if not hit:
        die(f"no courier handles '{args[0]}'  (known: {', '.join(sorted(table)) or 'none'})")
    print(json.dumps({"ok": True, **hit}, indent=2))


def _split_router_args(args: list[str]) -> tuple[dict[str, Any], str | None, list[str]]:
    """Split intake-supplied router flags (before the slash) from courier args.

    Everything up to and including the slash command may carry router flags
    (`--dry-run`, `--channel`, `--thread-ts`, `--session-key`); everything AFTER
    the slash is the courier's own argv, passed through verbatim. This boundary
    is what keeps deterministic harness flags (e.g. `/ship --channel <id>`) from
    being swallowed by the router — those live after the slash.
    """
    opts: dict[str, Any] = {"dry": False, "channel": None,
                            "thread_ts": None, "session_key": None}
    valued = {"--channel": "channel", "--thread-ts": "thread_ts",
              "--session-key": "session_key"}
    i = 0
    while i < len(args):
        tok = args[i]
        if tok == "--dry-run":
            opts["dry"] = True
            i += 1
        elif tok in valued:
            if i + 1 >= len(args):
                die(f"{tok} needs a value")
            opts[valued[tok]] = args[i + 1]
            i += 2
        else:
            # First non-flag token is the slash command; the rest is courier argv.
            return opts, tok, args[i + 1:]
    return opts, None, []


def cmd_run(args):
    opts, slash, rest = _split_router_args(args)
    dry = opts["dry"]
    channel = opts["channel"]
    thread_ts = opts["thread_ts"]
    explicit_key = opts["session_key"]
    if not slash:
        die("usage: router.py run [--dry-run] [--channel <id>] [--thread-ts <ts>] "
            "[--session-key <k>] <slash> [args...]")
    table = load_registry()
    hit = table.get(slash)
    if not hit:
        die(f"no courier handles '{slash}'  (known: {', '.join(sorted(table)) or 'none'})")

    if hit["mode"] == "deterministic":
        entry = str(Path(hit["dir"]) / hit["entry"])
        sub = hit["command"].get("subcommand")
        argv = [entry] + ([sub] if sub else []) + rest
        if dry:
            print(json.dumps({"ok": True, "mode": "deterministic", "exec": argv}, indent=2))
            return
        sys.exit(subprocess.call(argv))

    # agentic: hand off to the daemon, scoped to the courier's dir, then route the
    # reply back over transport.
    prompt = hit["command"].get("prompt", "").strip()
    if rest:
        prompt = (prompt + "\n\nOperator args: " + " ".join(rest)).strip()

    # Stable session per Slack thread (spec §5) → daemon resumes context. We send
    # both the human-readable key (for logs/observability) and the UUID the daemon
    # actually keys on; create_if_missing so the first message in a thread reserves
    # that id rather than erroring on the strict resume path.
    skey = session_key(hit["courier"], channel, thread_ts, explicit_key)
    sid = session_id_for(skey)
    payload = {
        "cwd": hit["dir"],
        "prompt": prompt or f"Run {slash} with the given args.",
        "client_type": "surface-slack",
        "session_id": sid,
        "create_if_missing": True,
    }
    if dry:
        print(json.dumps({"ok": True, "mode": "agentic",
                          "POST": f"{DAEMON_URL}/v1/prompt",
                          "session_key": skey, "session_id": sid,
                          "channel": channel, "thread_ts": thread_ts,
                          "body": payload}, indent=2))
        return

    result = call_daemon(payload)
    if not result["ok"]:
        # Daemon down / errored. Try to tell the operator in-thread so the failure
        # isn't silent; either way exit non-zero with a structured error.
        note = f":warning: courier `{slash}` couldn't reach the agent: {result['error']}"
        delivered = deliver_reply(channel, thread_ts, note) if channel else {"ok": False}
        print(json.dumps({"ok": False, "kind": result.get("kind"),
                          "error": result["error"], "session_key": skey,
                          "reply_delivered": bool(delivered.get("ok"))}, indent=2),
              file=sys.stderr)
        sys.exit(1)

    response = result["response"]
    reply = extract_reply(response)
    out_session = response.get("session_id") or sid
    delivered = deliver_reply(channel, thread_ts, reply)
    print(json.dumps({
        "ok": True,
        "mode": "agentic",
        "session_key": skey,
        "session_id": out_session,
        "reply": reply,
        "reply_delivered": bool(delivered.get("ok")),
        "delivery": delivered if not delivered.get("ok") else {"ok": True},
        "usage": response.get("usage"),
        "wall_ms": response.get("wall_ms"),
    }, indent=2))


def die(msg: str, code: int = 1) -> NoReturn:
    print(json.dumps({"ok": False, "error": msg}), file=sys.stderr)
    sys.exit(code)


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    cmd, rest = sys.argv[1], sys.argv[2:]
    {"list": cmd_list, "resolve": cmd_resolve, "run": cmd_run}.get(cmd, lambda _a: die(
        f"unknown command '{cmd}' (try: list | resolve | run)"))(rest)


if __name__ == "__main__":
    main()
