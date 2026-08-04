#!/usr/bin/env python3
"""
Courier-hub slash-command intake — the Slack front door, over Socket Mode.

couriers/ARCHITECTURE.md planned the Intake layer as an outward-facing HTTPS
adapter (Slack signing secret + public ingress). We deliberately DON'T build
that: this bridge already holds a Socket Mode WebSocket to Slack for the
content-agent (socket_listener.py), and Slack delivers slash commands over the
SAME socket as `slash_commands` envelopes. Riding the existing connection needs
no public URL, no signing secret, and no new Slack app — just one more envelope
type. (builds.toml A5: this extends the transitional Python bridge surface,
retired later by ocean-slack in Rust.)

ONE registered Slack command fronts the entire hub:

    /ocean <courier-slash> [args]     e.g.  /ocean ship --channel C0…
    /ocean help   (or bare /ocean)    →  the live command table (router.py list)

Direct per-slash registration also works with no code change: if a payload's
`command` (e.g. `/ship`) matches a courier slash in the registry, it routes —
so per-command Slack registration can be added later without touching this file.

Dispatch REUSES couriers/hub/router.py — the registry (load_registry), the
session fold (session_key / session_id_for), and the hardened daemon invocation
(call_daemon). We never re-parse manifests or re-implement the /v1/prompt path:

    slash_commands envelope → ack ≤3s (socket_listener) → parse_invocation
      deterministic → subprocess the harness (router's exact argv shape),
                      reply in-channel with the captured stdout+stderr in a
                      code block, truncated at ~3,500 chars
      agentic       → router.call_daemon with the router's exact payload shape,
                      reply with a short "dispatched to daemon" note + session id
      help / usage  → ephemeral reply (only the invoker sees it)

Replies go to the payload's `response_url` (valid 30 min, needs NO token —
pure stdlib urllib, same as the router's daemon POSTs). If it's ever missing we
fall back to a plain reply over the shared transport via router.deliver_reply.
Every handler takes injectable seams (registry / responder / runner /
daemon_call) so the whole intake is unit-testable with no network and no Slack
(tests/test_slash_intake.py).

DORMANT until the `/ocean` command is added to the Slack app's manifest (see
RUN.md §7) — merging this changes nothing live.
"""
import json
import logging
import os
import shlex
import subprocess
import sys
import urllib.request
from pathlib import Path

ASSISTANTS_ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = ASSISTANTS_ROOT.parent

# Reuse the courier hub router (registry + session fold + hardened daemon POST).
# Import directly — if this fails the intake is useless, and socket_listener's
# own guarded `import slash_intake` turns the failure into a logged no-op for
# the chat bridge rather than a crash.
sys.path.insert(0, str(REPO_ROOT / "couriers" / "hub"))
import router as _router  # noqa: E402

log = logging.getLogger("ocean.bridge.slash")

# The ONE hub command registered with Slack. Override only if the workspace
# already owns /ocean for something else.
HUB_COMMAND = os.environ.get("OCEAN_SLASH_COMMAND", "/ocean")
# Slack messages cap out around 4,000 chars; leave headroom for the code fences
# and the status line around the captured harness output.
REPLY_LIMIT = int(os.environ.get("OCEAN_SLASH_REPLY_LIMIT", "3500"))
# A hung deterministic harness must not pin a worker thread forever. The
# response_url stays valid for 30 min; default well inside that.
EXEC_TIMEOUT = float(os.environ.get("OCEAN_SLASH_EXEC_TIMEOUT", "600"))


def split_args(text: str) -> list[str]:
    """Split operator text into argv, honoring quotes (`--caption "two words"`).
    Slack text can contain smart quotes / unbalanced quotes shlex chokes on —
    fall back to whitespace split rather than failing the whole invocation."""
    try:
        return shlex.split(text)
    except ValueError:
        return text.split()


def parse_invocation(payload: dict, registry: dict) -> dict:
    """Map a slash-command payload onto the router's `/<slash>` table. Pure.

    Returns {"kind": "help"|"usage"|"deterministic"|"agentic", ...} where the
    dispatch kinds carry `slash`, `args`, and the registry `hit`. `usage`
    carries a ready-to-send `message`. Mode mirrors router.cmd_run exactly:
    mode == "deterministic" execs the harness, anything else is agentic.
    """
    command = (payload.get("command") or "").strip()
    text = (payload.get("text") or "").strip()

    if command == HUB_COMMAND:
        words = split_args(text)
        if not words or words[0].lstrip("/") in ("help", "list"):
            return {"kind": "help"}
        head, args = words[0], words[1:]
        slash = head if head.startswith("/") else f"/{head}"
    elif command in registry:
        # Direct per-slash registration (e.g. a /ship command added to the app
        # later) — the payload's own command IS the courier slash.
        slash, args = command, split_args(text)
    elif command:
        return {"kind": "usage",
                "message": (f"Unrecognized command `{command}` — this intake fronts the "
                            f"courier hub as `{HUB_COMMAND} <command> [args]`. "
                            f"Try `{HUB_COMMAND} help`.")}
    else:
        return {"kind": "usage",
                "message": f"usage: `{HUB_COMMAND} <command> [args]` — try `{HUB_COMMAND} help`"}

    hit = registry.get(slash)
    if hit is None:
        known = ", ".join(f"`{s}`" for s in sorted(registry)) or "none"
        return {"kind": "usage",
                "message": (f"No courier handles `{slash}`. Known: {known} — "
                            f"try `{HUB_COMMAND} help`.")}
    kind = "deterministic" if hit.get("mode") == "deterministic" else "agentic"
    return {"kind": kind, "slash": slash, "args": args, "hit": hit}


def render_help(registry: dict) -> str:
    """The live command table — same rows/format as `router.py list`, rendered
    to a Slack code block so columns stay aligned."""
    if not registry:
        return "No couriers registered (no courier.toml manifests under couriers/)."
    width = max(len(s) for s in registry)
    lines = []
    for slash in sorted(registry):
        e = registry[slash]
        c = e["command"]
        lines.append(f"{slash:<{width}}  [{e['mode']:^13}] {e['courier']:<14} {c.get('summary', '')}")
        if c.get("usage"):
            lines.append(f"{'':<{width}}  {'':15} {'':14} ↳ {c['usage']}")
    table = "\n".join(lines)
    return f"Courier hub — `{HUB_COMMAND} <command> [args]`\n```\n{table}\n```"


def truncate(text: str, limit: int = REPLY_LIMIT) -> str:
    """Cap harness output for a Slack message; be explicit that it was cut."""
    if len(text) <= limit:
        return text
    return text[:limit] + f"\n… (truncated at {limit} chars; full output on the bridge host)"


def _run_subprocess(argv: list[str]) -> tuple[int, str, str]:
    """Exec a deterministic harness, capturing output (router.cmd_run inherits
    stdio + sys.exits — fine for a terminal, useless for a Slack reply, so the
    capture wrapper lives here). Never raises: every failure maps to a
    conventional exit code + message so the reply path stays uniform."""
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=EXEC_TIMEOUT)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired as e:
        out = e.stdout if isinstance(e.stdout, str) else ""
        return 124, out, f"timed out after {EXEC_TIMEOUT:.0f}s"
    except FileNotFoundError:
        return 127, "", f"harness not found: {argv[0]}"
    except Exception as e:  # noqa: BLE001 - fail-soft; surfaced in the reply
        return 1, "", f"exec failed: {e}"


def run_deterministic(slash: str, hit: dict, args: list[str], runner=None) -> dict:
    """Exec the courier harness exactly as router.cmd_run builds its argv
    (entry + optional manifest subcommand + operator args), capture
    stdout+stderr, and format the Slack reply. `runner` is injectable for tests."""
    entry = str(Path(hit["dir"]) / hit.get("entry", "bin/courier"))
    sub = hit["command"].get("subcommand")
    argv = [entry] + ([sub] if sub else []) + list(args)
    code, out, err = (runner or _run_subprocess)(argv)

    body = (out or "").strip()
    if (err or "").strip():
        body = (body + "\n--- stderr ---\n" + err.strip()).strip()
    if not body:
        body = "(no output)"
    status = "ok" if code == 0 else f"exit {code}"
    text = f"`{slash}` → {status}\n```\n{truncate(body)}\n```"
    return {"ok": code == 0, "argv": argv, "exit": code, "text": text}


def run_agentic(slash: str, hit: dict, args: list[str], channel_id: str,
                daemon_call=None) -> dict:
    """POST the courier's scoped prompt to the daemon with router.cmd_run's
    exact payload shape (cwd-scoped, surface-slack, deterministic UUIDv5
    session per channel via the router's own session fold), through the
    hardened router.call_daemon. Reply is a short dispatched note — the agentic
    courier delivers its real results itself, over transport, from inside the
    daemon. `daemon_call` is injectable for tests."""
    prompt = (hit["command"].get("prompt", "") or "").strip()
    if args:
        prompt = (prompt + "\n\nOperator args: " + " ".join(args)).strip()
    skey = _router.session_key(hit["courier"], channel_id or None, None)
    sid = _router.session_id_for(skey)
    payload = {
        "cwd": hit["dir"],
        "prompt": prompt or f"Run {slash} with the given args.",
        "client_type": "surface-slack",
        "session_id": sid,
        "create_if_missing": True,
    }
    result = (daemon_call or _router.call_daemon)(payload)
    if not result.get("ok"):
        return {"ok": False, "payload": payload, "session_id": sid,
                "text": (f":warning: `{slash}` couldn't reach the agent: "
                         f"{result.get('error', 'unknown error')}")}
    response = result.get("response") or {}
    out_sid = response.get("session_id") or sid
    return {"ok": True, "payload": payload, "session_id": out_sid,
            "text": f"`{slash}` dispatched to daemon — session `{out_sid}`"}


def respond(payload: dict, text: str, response_type: str = "in_channel",
            opener=None) -> dict:
    """Deliver the reply. Primary path: POST the payload's `response_url`
    (stdlib urllib, no token, response_type controllable — this is what makes
    ephemeral errors possible). Fallback if it's missing: a plain reply over
    the shared transport via router.deliver_reply. Fail-soft, never raises."""
    url = (payload.get("response_url") or "").strip()
    if url:
        body = json.dumps({"response_type": response_type, "text": text}).encode()
        req = urllib.request.Request(
            url, data=body, headers={"Content-Type": "application/json"}, method="POST")
        try:
            with (opener or urllib.request.urlopen)(req, timeout=30) as r:
                status = getattr(r, "status", 200)
            return {"ok": 200 <= status < 300, "status": status}
        except Exception as e:  # noqa: BLE001
            log.error("response_url delivery failed: %s", e)
            return {"ok": False, "error": str(e)}
    # No response_url (shouldn't happen on a real slash payload) → plain reply.
    return _router.deliver_reply(payload.get("channel_id") or "", None, text)


def build_ack(payload: dict, registry: dict | None = None) -> dict | None:
    """The optional ack body (shown to the invoker inside Slack's 3-second
    window). Dispatch kinds get an ephemeral "running…" so a slow harness or
    daemon turn isn't a silent wait; help/usage answer near-instantly via
    response_url, so they ack bare (None). Never raises — an ack must go out."""
    try:
        reg = registry if registry is not None else _router.load_registry()
        inv = parse_invocation(payload, reg)
    except Exception:  # noqa: BLE001
        log.exception("build_ack failed; acking bare")
        return None
    if inv["kind"] not in ("deterministic", "agentic"):
        return None
    running = " ".join([inv["slash"], *inv["args"]]).strip()
    return {"response_type": "ephemeral",
            "text": f":hourglass_flowing_sand: running `{running}` …"}


def handle_slash(payload: dict, *, registry: dict | None = None, responder=None,
                 runner=None, daemon_call=None) -> dict:
    """The ONE slash pipeline, post-ack:

        payload → parse_invocation → help | usage | deterministic | agentic
                → responder (response_url; errors/usage ephemeral)

    Called on its own worker thread by socket_listener (a slow harness must not
    block the WebSocket processor) and directly by tests with every seam
    injected. Fail-soft: never raises, always returns a structured result.
    """
    if responder is None:
        def responder(text: str, response_type: str) -> dict:
            return respond(payload, text, response_type)

    try:
        reg = registry if registry is not None else _router.load_registry()
    except Exception as e:  # noqa: BLE001
        log.exception("courier registry load failed")
        responder(f":warning: courier registry unavailable: {e}", "ephemeral")
        return {"ok": False, "kind": "registry_error", "error": str(e)}

    inv = parse_invocation(payload, reg)
    kind = inv["kind"]
    if kind == "help":
        responder(render_help(reg), "ephemeral")
        return {"ok": True, "kind": "help"}
    if kind == "usage":
        responder(inv["message"], "ephemeral")
        return {"ok": True, "kind": "usage"}

    try:
        if kind == "deterministic":
            r = run_deterministic(inv["slash"], inv["hit"], inv["args"], runner=runner)
            responder(r["text"], "in_channel")
            return {"ok": r["ok"], "kind": "deterministic",
                    "argv": r["argv"], "exit": r["exit"]}
        r = run_agentic(inv["slash"], inv["hit"], inv["args"],
                        payload.get("channel_id") or "", daemon_call=daemon_call)
        responder(r["text"], "in_channel" if r["ok"] else "ephemeral")
        return {"ok": r["ok"], "kind": "agentic",
                "payload": r["payload"], "session_id": r.get("session_id")}
    except Exception as e:  # noqa: BLE001 - the worker thread must never die silent
        log.exception("slash dispatch failed")
        try:
            responder(f":warning: `{inv.get('slash', '?')}` failed: {e}", "ephemeral")
        except Exception:  # noqa: BLE001
            log.exception("failed to deliver dispatch-failure notice")
        return {"ok": False, "kind": "dispatch_error", "error": str(e)}


if __name__ == "__main__":
    # Self-describe: no token, no network, no send — print the live command
    # table this intake would serve for `/ocean help` (reads the manifests).
    print(json.dumps({
        "ok": True,
        "status": "dormant until the Slack app registers the command (RUN.md §7)",
        "hub_command": HUB_COMMAND,
        "entrypoint": "handle_slash(payload) — called by socket_listener on slash_commands envelopes",
        "help": render_help(_router.load_registry()),
    }, indent=2))
