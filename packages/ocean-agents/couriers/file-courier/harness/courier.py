#!/usr/bin/env python3
"""
file-courier — deliver the contents of a dropbox folder to a Slack channel.

A courier *species*: it ships arbitrary files (any type) and can post messages.
All Slack mechanics (auth, upload flow, retry, threading, link parsing) live in
the shared route at ../../transport/slack.py — this file only orchestrates:
enumerate, confirm, send, ledger, delete.

Subcommands:
  resolve   Verify a destination link (channel name + bot identity). Prints JSON.
  send      Upload every file in a directory to a channel, paced + ledgered,
            deleting each file on confirmed delivery.
  message   Post a text message (optionally threaded) to a channel.
"""
import argparse
import json
import sys
import time
from pathlib import Path

# Locate the shared transport (couriers/transport) regardless of CWD.
HERE = Path(__file__).resolve()
sys.path.insert(0, str(HERE.parents[2] / "transport"))
import slack as sl  # noqa: E402


def die(msg: str, code: int = 1):
    print(json.dumps({"ok": False, "error": msg}), file=sys.stderr)
    sys.exit(code)


def list_files(d: Path) -> list[Path]:
    return [
        p for p in sorted(d.rglob("*"))
        if p.is_file() and not p.name.startswith(".") and p.name != "courier.ledger"
    ]


def client_or_die() -> "sl.Slack":
    try:
        return sl.Slack()
    except RuntimeError as e:
        die(str(e))


def cmd_resolve(args):
    info = sl.parse_link(args.link)
    if not info["channel"]:
        die(f"Could not parse a Slack channel from: {args.link}  "
            "(paste a channel link like https://workspace.slack.com/archives/C0123ABCD)")
    c = client_or_die()

    me = c.auth_test()
    if not me.get("ok"):
        die(f"auth.test failed: {me.get('error')}  (check the bot token)")
    out = {
        "ok": True,
        "bot_user": me.get("user"),
        "team": me.get("team"),
        "channel": info["channel"],
        "channel_name": None,
        "thread_ts": info["thread_ts"],
    }
    ci = c.conversations_info(info["channel"])
    if ci.get("ok"):
        ch = ci["channel"]
        out["channel_name"] = ch.get("name")
        out["is_private"] = ch.get("is_private")
        out["is_member"] = ch.get("is_member")
    else:
        # Not fatal — may just lack the read scope; sending can still work.
        out["channel_info_error"] = ci.get("error")
    print(json.dumps(out, indent=2))


def cmd_send(args):
    d = Path(args.dir).expanduser()
    if not d.is_dir():
        die(f"Not a directory: {d}")
    files = list_files(d)
    if not files:
        die(f"No files to send in {d}")
    c = client_or_die()

    ledger = d / "courier.ledger"
    sent_already = set(ledger.read_text().splitlines()) if ledger.exists() else set()

    total = len(files)
    ok_count, fail = 0, []
    print(f"[courier] uploading {total} file(s) -> {args.channel}"
          f"{' thread ' + args.thread_ts if args.thread_ts else ''}", flush=True)

    for i, f in enumerate(files, 1):
        key = str(f)
        if key in sent_already:
            print(f"[{i}/{total}] SKIP (ledger): {f.name}", flush=True)
            continue
        resp = c.upload_file(args.channel, f, thread_ts=args.thread_ts,
                             initial_comment=(args.comment if i == 1 else None))
        if resp.get("ok"):
            ok_count += 1
            with ledger.open("a") as lg:
                lg.write(key + "\n")
            print(f"[{i}/{total}] OK: {f.name}", flush=True)
            if args.delete:
                try:
                    f.unlink()
                except OSError as e:
                    print(f"   warn: could not delete {f.name}: {e}", flush=True)
        else:
            print(f"[{i}/{total}] FAIL: {f.name} :: {resp.get('error')}", flush=True)
            fail.append(f.name)
        time.sleep(args.pace)

    if args.delete and not fail and ledger.exists():
        ledger.unlink()
    print(f"[courier] DONE sent={ok_count} failed={len(fail)} of {total}", flush=True)
    if fail:
        print("[courier] failed: " + ", ".join(fail), flush=True)
        sys.exit(2)


def cmd_message(args):
    c = client_or_die()
    text = args.text
    if text == "-":  # read from stdin
        text = sys.stdin.read()
    resp = c.post_message(args.channel, text=text, thread_ts=args.thread_ts)
    if not resp.get("ok"):
        die(f"chat.postMessage failed: {resp.get('error')}")
    print(json.dumps({"ok": True, "channel": resp.get("channel"), "ts": resp.get("ts")}, indent=2))


def main():
    ap = argparse.ArgumentParser(prog="courier", description="Ocean file-courier (Slack route)")
    sub = ap.add_subparsers(dest="cmd", required=True)

    r = sub.add_parser("resolve", help="resolve + verify a destination link")
    r.add_argument("link", help="Slack channel link or raw channel id")
    r.set_defaults(func=cmd_resolve)

    s = sub.add_parser("send", help="upload a directory's files to a channel")
    s.add_argument("--channel", required=True)
    s.add_argument("--thread-ts", default=None, dest="thread_ts")
    s.add_argument("--dir", required=True)
    s.add_argument("--comment", default=None, help="initial_comment on the first file")
    s.add_argument("--pace", type=float, default=1.0, help="seconds between uploads")
    s.add_argument("--delete", action="store_true", default=True, help="delete on confirmed send (default)")
    s.add_argument("--keep", action="store_false", dest="delete", help="keep files after send")
    s.set_defaults(func=cmd_send)

    m = sub.add_parser("message", help="post a text message to a channel")
    m.add_argument("--channel", required=True)
    m.add_argument("--thread-ts", default=None, dest="thread_ts")
    m.add_argument("text", help="message text, or '-' to read stdin")
    m.set_defaults(func=cmd_message)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
