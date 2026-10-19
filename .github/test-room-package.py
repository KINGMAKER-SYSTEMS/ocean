#!/usr/bin/env python3
"""Exercise production Room package capture in a disposable daemon profile."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import uuid

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("revision")
args = parser.parse_args()
binary = args.binary.resolve(strict=True)
root = Path(tempfile.mkdtemp(prefix="ocean-room-package-"))
config = root / "config"
config.mkdir(mode=0o700)
member = "package-probe-" + uuid.uuid4().hex
(config / "member.toml").write_text(f'member_id = "{member}"\ndisplay_name = "Package probe"\n')
(config / "member.toml").chmod(0o600)
package = config / "agents" / "fixture"
(package / "skills").mkdir(parents=True)
files = {"agent.toml": b'description = "Fixture"\ntools = []\n',
         "instructions.md": b"Do not perform external actions.\n",
         "skills/\u00e9.md": b"First version\n"}
for name, data in files.items():
    (package / name).write_bytes(data)


def expected_digest():
    digest = hashlib.sha256(b"ocean-room-agent-definition-v1\0")
    for name, data in sorted(files.items()):
        encoded = name.encode("utf-8")
        digest.update(len(encoded).to_bytes(8, "big"))
        digest.update(encoded)
        digest.update(len(data).to_bytes(8, "big"))
        digest.update(data)
    return "sha256:" + digest.hexdigest()


with socket.socket() as reservation:
    reservation.bind(("127.0.0.1", 0))
    port = reservation.getsockname()[1]
base = f"http://127.0.0.1:{port}"
# Neither provider keys nor live auth/state paths reach the child. OS identity
# stays unchanged, and every configured profile path belongs to this fixture.
env = {key: os.environ[key] for key in (
    "PATH", "HOME", "USERPROFILE", "SYSTEMROOT", "TEMP", "TMP", "TMPDIR", "LANG", "LC_ALL"
) if key in os.environ}
env.update(OCEAN_CONFIG_DIR=str(config), XDG_CONFIG_HOME=str(root / "xdg"),
           OCEAN_AUTH_FILE=str(config / "auth.json"),
           OCEAN_CODEX_AUTH_FILE=str(config / "codex-auth.json"),
           OCEAN_AGENTS_DIR=str(config / "agents"), OCEAN_BIND=f"127.0.0.1:{port}",
           OCEAN_UNSUPERVISED="1", OCEAN_MODEL="gpt-5.5")


def request(path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    req = urllib.request.Request(base + path, data=data,
                                 headers={} if data is None else {"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            return response.status, json.load(response)
    except urllib.error.HTTPError as error:
        return error.code, json.load(error)


with (root / "daemon.log").open("wb") as log:
    child = subprocess.Popen([str(binary)], cwd=root, env=env, stdin=subprocess.DEVNULL,
                             stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 45
        while True:
            if child.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError(f"candidate unavailable; log: {root / 'daemon.log'}")
            try:
                status, health = request("/health")
                break
            except OSError:
                time.sleep(0.2)
        assert status == 200 and health["ok"] is True
        assert health["rev"] and args.revision.startswith(health["rev"])
        status, identity = request("/v1/identity")
        assert status == 200 and identity["member_id"] == member
        # Never create even fixture data until this exact child profile is proven.
        status, created = request("/v1/rooms/persistent", {"key": "package-probe", "name": "Package probe"})
        assert status == 201 and created["ok"] is True
        preview = "/v1/rooms/persistent/package-probe/agents/preview/fixture"
        status, first = request(preview)
        assert status == 200, (status, first)
        assert first["definition_digest"] == expected_digest()
        assert first["binding"] is None and first["owner_eligible"] is False
        files["skills/\u00e9.md"] = b"Second version\n"
        (package / "skills/\u00e9.md").write_bytes(files["skills/\u00e9.md"])
        status, changed = request(preview)
        assert status == 200 and changed["definition_digest"] == expected_digest()
        assert changed["definition_digest"] != first["definition_digest"]
        status, missing = request(preview.replace("/fixture", "/missing"))
        assert status == 404 and missing["error"] == "agent_package_not_found"
        outside = root / "outside.md"
        outside.write_bytes(b"outside fixture bytes")
        link = package / "linked.md"
        link.symlink_to(outside)
        status, linked = request(preview)
        assert status == 409 and linked["error"] in {"agent_package_unavailable", "agent_package_symlink_refused"}
        assert outside.read_bytes() == b"outside fixture bytes"
        link.unlink()
        with (package / "oversized").open("wb") as oversized:
            oversized.truncate(8 * 1024 * 1024 + 1)
        status, bounded = request(preview)
        assert status == 409 and bounded["error"] == "agent_package_limit_exceeded"
        print(json.dumps({"result": "PASS", "revision": health["rev"],
                          "checks": ["isolated identity", "real Room package preview", "complete-map digest",
                                     "nested byte change", "missing package", "symlink refusal", "file size limit"],
                          "provider_reply": "not tested", "profile": str(root)}))
    finally:
        if child.poll() is None:
            child.terminate()
            try:
                child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
