#!/usr/bin/env python3
"""
Shared Slack transport — the *route* every Ocean courier runs on.

Pure stdlib (no slack_sdk / requests / curl) so every courier species — file
delivery, formatted message packs, future ones — shares ONE implementation of
auth / send / retry / channel-threading against a single destination concept.
Couriers differ only in what they ship; the route is common.

Token resolution order (first hit wins):
  1. $OCEAN_SLACK_BOT_TOKEN
  2. $SLACK_BOT_TOKEN
  3. file at $OCEAN_SLACK_TOKEN_FILE (default ~/.slack_token)

Token must be a bot token (xoxb-…). Scopes:
  chat:write           post messages
  files:write          upload files
  channels:read / groups:read / im:read / mpim:read   resolve channel names
  canvases:write       (optional) create canvases
"""
import json
import mimetypes
import os
import re
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path

API = "https://slack.com/api/{method}"

# slack_canvas edit-mode → Slack `canvases.edit` change `operation` (OCEAN-244).
# Mirrors ocean-agent-sdk `CanvasEditMode` (replace/append/prepend); `append` is
# also the dedicated `SlackCanvasOp::Append` op, which maps here too.
_CANVAS_EDIT_OP = {
    "replace": "replace",
    "append": "insert_at_end",
    "prepend": "insert_at_start",
}


def resolve_token() -> str:
    for var in ("OCEAN_SLACK_BOT_TOKEN", "SLACK_BOT_TOKEN"):
        v = os.environ.get(var)
        if v:
            return v.strip()
    path = Path(os.environ.get("OCEAN_SLACK_TOKEN_FILE", str(Path.home() / ".slack_token")))
    if path.is_file():
        return path.read_text().strip()
    raise RuntimeError(
        "No Slack bot token. Set $OCEAN_SLACK_BOT_TOKEN / $SLACK_BOT_TOKEN or create ~/.slack_token"
    )


def parse_link(link: str) -> dict:
    """Resolve a Slack archive link / raw channel id into routing parts.

    Returns {channel, thread_ts, message_ts, kind}. `channel` is the ID
    conversations.info / chat.postMessage accept; None when unresolvable.
    """
    link = link.strip()
    if re.fullmatch(r"[CGD][A-Z0-9]{6,}", link):  # raw channel/DM id
        return {"channel": link, "thread_ts": None, "message_ts": None, "kind": "raw_id"}

    thread_ts = None
    m = re.search(r"[?&]thread_ts=([0-9.]+)", link)
    if m:
        thread_ts = m.group(1)
    cid = None
    m = re.search(r"[?&]cid=([CGD][A-Z0-9]{6,})", link)
    if m:
        cid = m.group(1)

    m = re.search(r"/archives/([CGD][A-Z0-9]{6,})(?:/p(\d+))?", link)
    if m:
        channel = m.group(1)
        message_ts = None
        if m.group(2):
            p = m.group(2)  # 16-digit packed ts: seconds(10) + micros(6)
            message_ts = p[:10] + "." + p[10:]
        # If the link is explicitly a thread reply, honor its thread_ts; else the
        # message ts is offered as a thread anchor the caller may opt into.
        return {
            "channel": channel,
            "thread_ts": thread_ts or message_ts,
            "message_ts": message_ts,
            "kind": "archive",
        }
    if cid:
        return {"channel": cid, "thread_ts": thread_ts, "message_ts": None, "kind": "query"}
    return {"channel": None, "thread_ts": None, "message_ts": None, "kind": "unknown"}


class Slack:
    """Thin Slack Web API client with built-in rate-limit + transient retry."""

    def __init__(self, token: str | None = None):
        self.token = token or resolve_token()

    # ---- low-level ------------------------------------------------------
    def _api(self, method: str, params: dict | None = None, json_body: dict | None = None) -> dict:
        url = API.format(method=method)
        if json_body is not None:
            data = json.dumps(json_body).encode()
            ctype = "application/json; charset=utf-8"
        else:
            data = urllib.parse.urlencode(params or {}).encode()
            ctype = "application/x-www-form-urlencoded"
        req = urllib.request.Request(url, data=data, method="POST")
        req.add_header("Authorization", f"Bearer {self.token}")
        req.add_header("Content-Type", ctype)
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return json.loads(r.read())
        except urllib.error.HTTPError as e:
            # 429 carries Retry-After in the header, not the body.
            if e.code == 429:
                return {"ok": False, "error": "ratelimited",
                        "retry_after": int(e.headers.get("Retry-After", "5"))}
            try:
                return json.loads(e.read())
            except Exception:
                return {"ok": False, "error": f"http_{e.code}"}

    def call(self, method, params=None, json_body=None, max_tries=3) -> dict:
        """Slack API call with retry on ratelimited + transient errors."""
        attempt = 0
        while True:
            attempt += 1
            resp = self._api(method, params, json_body)
            if resp.get("ok"):
                return resp
            err = resp.get("error")
            if err == "ratelimited":
                time.sleep(resp.get("retry_after", 5))
                continue
            # Slack 'ok:false' errors are mostly terminal (bad scope, channel not
            # found, etc.) — only retry obvious transients.
            if err in ("service_unavailable", "internal_error", "fatal_error") and attempt < max_tries:
                time.sleep(5)
                continue
            return resp

    # ---- identity / channel --------------------------------------------
    def auth_test(self) -> dict:
        return self.call("auth.test")

    def conversations_info(self, channel) -> dict:
        return self.call("conversations.info", {"channel": channel})

    # ---- messages -------------------------------------------------------
    def post_message(self, channel, text=None, blocks=None, thread_ts=None) -> dict:
        body = {"channel": channel}
        if text is not None:
            body["text"] = text
        if blocks is not None:
            body["blocks"] = blocks
        if thread_ts:
            body["thread_ts"] = thread_ts
        return self.call("chat.postMessage", json_body=body)

    # ---- files (modern 3-step external upload) --------------------------
    def upload_file(self, channel, path, title=None, initial_comment=None, thread_ts=None) -> dict:
        path = Path(path)
        size = path.stat().st_size
        # 1) reserve an upload URL
        got = self.call("files.getUploadURLExternal",
                        {"filename": path.name, "length": size})
        if not got.get("ok"):
            return got
        # Guard the fields explicitly: an ok:true response *should* carry both,
        # but a malformed body must not None-subscript downstream (Pyright-safe).
        upload_url = got.get("upload_url")
        file_id = got.get("file_id")
        if not isinstance(upload_url, str) or not isinstance(file_id, str):
            return {"ok": False, "error": "upload_url_missing", "detail": got}
        # 2) PUT/POST the bytes to the (unauthenticated) presigned URL
        put = self._post_bytes(upload_url, path)
        if put is not True:
            return {"ok": False, "error": "upload_post_failed", "detail": put}
        # 3) complete + share into the channel
        f = {"id": file_id}
        if title:
            f["title"] = title
        body = {"files": [f], "channel_id": channel}
        if initial_comment:
            body["initial_comment"] = initial_comment
        if thread_ts:
            body["thread_ts"] = thread_ts
        return self.call("files.completeUploadExternal", json_body=body)

    @staticmethod
    def _post_bytes(upload_url: str, path: Path):
        """multipart POST of the file to Slack's presigned upload URL. True on 200."""
        boundary = "----OceanCourier" + uuid.uuid4().hex
        nl = b"\r\n"
        ctype = mimetypes.guess_type(path.name)[0] or "application/octet-stream"
        body = bytearray()
        body += b"--" + boundary.encode() + nl
        body += f'Content-Disposition: form-data; name="file"; filename="{path.name}"'.encode() + nl
        body += f"Content-Type: {ctype}".encode() + nl + nl
        body += path.read_bytes() + nl
        body += b"--" + boundary.encode() + b"--" + nl
        req = urllib.request.Request(upload_url, data=bytes(body), method="POST")
        req.add_header("Content-Type", f"multipart/form-data; boundary={boundary}")
        try:
            with urllib.request.urlopen(req, timeout=300) as r:
                return True if r.status == 200 else f"status_{r.status}"
        except urllib.error.HTTPError as e:
            return f"http_{e.code}"
        except Exception as e:  # noqa: BLE001
            return str(e)

    # ---- canvas -----------------------------------------------------------
    # Slack canvas ops, the route the ocean-os `slack_canvas` agent tool round-
    # trips onto (OCEAN-214 / OCEAN-244). The agent emits structured ops
    # (create/read/update/append/list); the bridge consumer maps each to one of
    # these. Scopes: canvases:write (create/edit), canvases:read (sections
    # lookup), files:read (file lookup / channel listing).
    #
    # HONESTY on `read`: Slack exposes **no** "give me the canvas markdown body"
    # endpoint. `canvases.sections.lookup` returns section *ids* (optionally
    # filtered), not their text, and the canvas file object carries metadata, not
    # an inline body. So `read_canvas` returns the section structure + file
    # metadata it CAN get, and is explicit that the raw markdown body is not
    # retrievable via the public API. create/edit are fully round-trippable.

    def create_canvas(self, title, markdown, channel=None) -> dict:
        """Create a canvas. Returns Slack's response incl. `canvas_id` on ok.

        Channel-scoped create uses `conversations.canvases.create` (attaches the
        canvas to the channel); unscoped uses `canvases.create` (a standalone
        canvas). Both return `{"ok": true, "canvas_id": "F…"}`.
        """
        document_content = {"type": "markdown", "markdown": markdown or ""}
        if channel:
            body = {"channel_id": channel, "document_content": document_content}
            # conversations.canvases.create takes no `title` (the channel names it);
            # the standalone canvases.create does.
            return self.call("conversations.canvases.create", json_body=body)
        body = {"document_content": document_content}
        if title:
            body["title"] = title
        return self.call("canvases.create", json_body=body)

    def edit_canvas(self, canvas_id, markdown, mode="replace") -> dict:
        """Edit a canvas via `canvases.edit` (powers update/append/prepend).

        `mode` maps onto Slack's change `operation`:
          replace → "replace" (rewrite the whole body)
          append  → "insert_at_end"
          prepend → "insert_at_start"
        """
        operation = _CANVAS_EDIT_OP.get(mode)
        if operation is None:
            return {"ok": False, "error": "bad_edit_mode",
                    "detail": f"unknown mode {mode!r}; expected one of "
                              f"{sorted(_CANVAS_EDIT_OP)}"}
        body = {
            "canvas_id": canvas_id,
            "changes": [{
                "operation": operation,
                "document_content": {"type": "markdown", "markdown": markdown},
            }],
        }
        return self.call("canvases.edit", json_body=body)

    def lookup_canvas_sections(self, canvas_id, section_types=None,
                               contains_text=None) -> dict:
        """`canvases.sections.lookup` — enumerate a canvas's sections (ids only).

        Slack returns section *ids*, not their content. With no criteria we ask
        for `any_header` so we surface the canvas's heading skeleton; callers can
        narrow with `section_types`/`contains_text`.
        """
        criteria: dict = {}
        criteria["section_types"] = section_types or ["any_header"]
        if contains_text:
            criteria["contains_text"] = contains_text
        return self.call("canvases.sections.lookup",
                         json_body={"canvas_id": canvas_id, "criteria": criteria})

    def file_info(self, file_id) -> dict:
        """`files.info` for a canvas file id — metadata (title, type, channels).

        Used by the read op to confirm the canvas exists and surface its title /
        linked channel. Does NOT return the canvas body (Slack carries none inline).
        """
        return self.call("files.info", {"file": file_id})

    def read_canvas(self, canvas_id) -> dict:
        """Best-effort canvas read: file metadata + section skeleton.

        Returns a NORMALIZED dict the bridge maps to the agent's read result:
          {ok, canvas_id, title?, sections: [...ids], body_available: False,
           note: "<why no raw markdown>"}

        `body_available` is always False: the public Slack API exposes section
        structure (`canvases.sections.lookup`) and file metadata (`files.info`),
        but not the rendered markdown body. We surface what's retrievable and say
        so, rather than fabricate a body.
        """
        info = self.file_info(canvas_id)
        title = None
        if info.get("ok"):
            title = (info.get("file") or {}).get("title")
        sections = self.lookup_canvas_sections(canvas_id)
        section_ids = []
        if sections.get("ok"):
            section_ids = [s.get("id") for s in sections.get("sections", [])
                           if isinstance(s, dict) and s.get("id")]
        ok = bool(info.get("ok") or sections.get("ok"))
        return {
            "ok": ok,
            "canvas_id": canvas_id,
            "title": title,
            "sections": section_ids,
            "body_available": False,
            "note": ("Slack's public API exposes canvas section structure + file "
                     "metadata but not the raw markdown body; returning what is "
                     "retrievable."),
            "_file_info": info,
            "_sections": sections,
        }

    def list_canvases(self, channel) -> dict:
        """List canvases in a channel via `files.list` filtered to canvas files.

        Slack categorizes canvases under the `spaces` file family (they descend
        from Posts), so we filter `types=spaces` and keep canvas-ish filetypes.
        Returns a NORMALIZED dict: {ok, channel, canvases: [{canvas_id, title}]}.
        """
        resp = self.call("files.list", {"channel": channel, "types": "spaces"})
        canvases = []
        if resp.get("ok"):
            for f in resp.get("files", []):
                if not isinstance(f, dict):
                    continue
                ftype = f.get("filetype") or ""
                # canvas files report filetype "canvas" (and historically "quip").
                if ftype in ("canvas", "quip") or f.get("is_channel_canvas"):
                    fid = f.get("id")
                    if fid:
                        canvases.append({"canvas_id": fid, "title": f.get("title")})
        return {"ok": bool(resp.get("ok")), "channel": channel,
                "canvases": canvases, "_files_list": resp}
