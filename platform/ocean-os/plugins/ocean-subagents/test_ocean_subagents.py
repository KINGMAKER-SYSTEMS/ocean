import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import tomllib
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("ocean_subagents", ROOT / "ocean-subagents.py")
module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(module)

WORKER_TOOLS = ["read", "ls", "grep", "glob", "lsp", "web_fetch", "bash", "edit", "write", "hashline_edit"]
TERMINAL_REQUEST_STATES = {"completed", "errored", "cancelled"}


class FakeDaemon:
    """Loopback stand-in for the daemon routes the plugin calls.

    It mirrors the real control contract where that contract matters to the
    plugin: the request registry is volatile, cancelling an unknown or finished
    request answers HTTP 200 with `ok:false`, an accepted cancel only moves the
    request to `cancelling`, and an unknown session is a 404.
    """

    def __init__(self):
        self.lock = threading.Lock()
        self.requests = {}
        self.sessions = {}
        self.permissions = {}
        self.decisions = []
        self.payloads = []
        self.cancel_calls = []
        # (status, body) to answer the next cancels with, instead of the
        # contract above; the request is left untouched. Bodies may be bytes.
        self.cancel_reply = None
        # Called with the request id before a cancel is processed, so a test
        # can let the request finish in the window after the plugin's refresh.
        self.before_cancel = None
        self.unavailable = False
        self.sessions_unreadable = False
        self.requests_malformed = False
        self.session_claims_active = {}
        self.worker = {"config": {"tools": list(WORKER_TOOLS)}, "tools": []}
        self.models = [
            {"id": "deepseek-v4-pro", "ready": True},
            {"id": "gpt-5.5", "ready": True},
            {"id": "glm-5.3", "ready": False},
        ]
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def read_json(self):
                length = int(self.headers.get("Content-Length", "0"))
                return json.loads(self.rfile.read(length) or b"{}")

            def reply(self, status, body):
                encoded = body if isinstance(body, bytes) else json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(encoded)))
                self.end_headers()
                self.wfile.write(encoded)

            def do_POST(self):
                if owner.unavailable:
                    self.reply(503, {"ok": False})
                    return
                if self.path == "/v1/agent/turns":
                    payload = self.read_json()
                    with owner.lock:
                        turn_id = str(uuid.uuid4())
                        session_id = payload.get("session_id") or str(uuid.uuid4())
                        owner.payloads.append(payload)
                        owner.requests[turn_id] = {
                            "request_id": turn_id,
                            "session_id": session_id,
                            "state": "running",
                            "message": "agent turn running",
                        }
                        # The daemon saves the accepted prompt before any
                        # provider call.
                        owner.sessions.setdefault(session_id, []).append(
                            {"role": "user", "text": payload["prompt"]}
                        )
                    self.reply(
                        202,
                        {
                            "ok": True,
                            "turn_id": turn_id,
                            "session_id": session_id,
                            "status": "running",
                            "event_id_prefix": turn_id[:8],
                        },
                    )
                    return
                if self.path.startswith("/v1/requests/") and self.path.endswith("/cancel"):
                    request_id = self.path.split("/")[3]
                    if owner.before_cancel is not None:
                        owner.before_cancel(request_id)
                    with owner.lock:
                        owner.cancel_calls.append(request_id)
                        request = owner.requests.get(request_id)
                        if owner.cancel_reply is not None:
                            status, body = owner.cancel_reply
                        elif request is None:
                            status, body = 200, {
                                "ok": False,
                                "request_id": request_id,
                                "state": "errored",
                                "message": "request not found",
                            }
                        elif request["state"] in TERMINAL_REQUEST_STATES:
                            status, body = 200, {
                                "ok": False,
                                "request_id": request_id,
                                "state": request["state"],
                                "message": "request is already terminal; cancel ignored",
                            }
                        else:
                            request["state"] = "cancelling"
                            request["message"] = "cancel requested; cancellation token sent"
                            status, body = 200, {
                                "ok": True,
                                "request_id": request_id,
                                "state": "cancelling",
                                "message": "cancel requested; runtime cancellation token signalled",
                            }
                    self.reply(status, body)
                    return
                if self.path.startswith("/v1/permissions/") and self.path.endswith("/decision"):
                    permission_id = self.path.split("/")[3]
                    payload = self.read_json()
                    with owner.lock:
                        permission = owner.permissions.get(permission_id)
                        if permission is None:
                            self.reply(404, {"ok": False})
                            return
                        request = owner.requests[permission["request_id"]]
                        expected = next(
                            item["decision_token"]
                            for item in owner.payloads
                            if item.get("session_id", request["session_id"])
                            == request["session_id"]
                        )
                        if payload.get("decision_token") != expected:
                            self.reply(403, {"ok": False})
                            return
                        owner.decisions.append(payload)
                        del owner.permissions[permission_id]
                        request["state"] = "running"
                        request.pop("permission_id", None)
                    self.reply(200, {"ok": True, "message": "permission resolved"})
                    return
                self.reply(404, {"ok": False})

            def do_GET(self):
                if owner.unavailable:
                    self.reply(503, {"ok": False})
                    return
                if self.path == "/v1/requests":
                    with owner.lock:
                        requests = [dict(item) for item in owner.requests.values()]
                    if owner.requests_malformed:
                        requests = None
                    self.reply(200, {"ok": True, "requests": requests})
                    return
                if self.path == "/v1/permissions":
                    with owner.lock:
                        permissions = list(owner.permissions.values())
                    self.reply(200, {"ok": True, "permissions": permissions})
                    return
                if self.path == "/v1/models":
                    self.reply(200, {"ok": True, "models": owner.models})
                    return
                if self.path == f"/v1/agents/{module.WORKER_AGENT}":
                    if owner.worker is None:
                        self.reply(200, {"ok": False, "error": "agent not found"})
                    else:
                        self.reply(200, {"ok": True, "agent": owner.worker})
                    return
                if self.path.startswith("/v1/sessions/"):
                    session_id = self.path.rsplit("/", 1)[1]
                    if owner.sessions_unreadable:
                        self.reply(500, {"ok": False, "error": "session could not be read"})
                        return
                    with owner.lock:
                        if session_id not in owner.sessions:
                            self.reply(404, {"ok": False, "error": "session not found"})
                            return
                        transcript = list(owner.sessions[session_id])
                        active = [
                            item["request_id"]
                            for item in owner.requests.values()
                            if item["session_id"] == session_id
                            and item["state"] not in TERMINAL_REQUEST_STATES
                        ] + owner.session_claims_active.get(session_id, [])
                    self.reply(
                        200,
                        {
                            "ok": True,
                            "session": {
                                "id": session_id,
                                "transcript": transcript,
                                "active_requests": active,
                            },
                        },
                    )
                    return
                self.reply(404, {"ok": False})

            def log_message(self, *_args):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    @property
    def url(self):
        host, port = self.server.server_address
        return f"http://{host}:{port}"

    def add_permission(self, request_id, tool="bash", args=None):
        permission_id = str(uuid.uuid4())
        with self.lock:
            request = self.requests[request_id]
            self.permissions[permission_id] = {
                "permission_id": permission_id,
                "request_id": request_id,
                "session_id": request["session_id"],
                "tool": tool,
                "reason": "test permission",
                "args": args or {"command": "true"},
                "created_at": "2026-07-29T12:00:00Z",
            }
            request["state"] = "waiting_for_permission"
            request["permission_id"] = permission_id
        return permission_id

    def complete(self, request_id, output="worker result", finished_at="2026-07-29T12:00:00Z"):
        with self.lock:
            request = self.requests[request_id]
            request.update(
                state="completed",
                message="prompt completed",
                finished_at=finished_at,
            )
            self.sessions[request["session_id"]].append(
                {"role": "assistant", "text": output}
            )

    def fail(self, request_id, message):
        with self.lock:
            self.requests[request_id].update(
                state="errored", message=message, finished_at="2026-07-29T12:00:00Z"
            )

    def finish_cancel(self, request_id):
        with self.lock:
            self.requests[request_id].update(
                state="cancelled", message="cancelled", finished_at="2026-07-29T12:00:00Z"
            )

    def say(self, request_id, text):
        with self.lock:
            session_id = self.requests[request_id]["session_id"]
            self.sessions[session_id].append({"role": "assistant", "text": text})

    def forget_requests(self):
        """What a daemon restart, or registry eviction, does to every request."""
        with self.lock:
            self.requests.clear()

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *_args):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


def wait_until(predicate, timeout=3.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return True
        time.sleep(0.02)
    return predicate()


class OceanSubagentTests(unittest.TestCase):
    def setUp(self):
        # Runs in these tests are seconds old; the out-of-order-read grace would
        # otherwise hide every lost-request path.
        patcher = mock.patch.object(module, "LOST_GRACE_SECONDS", 0)
        patcher.start()
        self.addCleanup(patcher.stop)

    def manager(self, root, daemon):
        return module.Subagents(
            module.DaemonClient(daemon.url), module.JsonStore(Path(root) / "state")
        )

    def stored(self, root, run_id):
        return json.loads((Path(root) / "state/runs.json").read_text())["runs"][run_id]

    def unwatched_run(self, manager, root, task="unwatched"):
        """A run whose elapsed-time watchdog is not armed, so a test controls
        exactly when, and against which request, the watchdog fires."""
        with mock.patch.object(manager, "_start_watchdog"):
            return manager.spawn({"task": task, "cwd": root})

    def expire_watchdog(self, manager, run):
        """Arm the run's watchdog with its ceiling already passed and return
        the stored run once that watchdog thread has finished."""
        manager.store.update(run["run_id"], started_at="2000-01-01T00:00:00Z", timeout_seconds=30)
        manager._start_watchdog(run["run_id"])
        key = (run["run_id"], run["turn_id"])
        self.assertTrue(wait_until(lambda: key not in manager.watchdogs), "watchdog did not finish")
        return manager.store.get(run["run_id"])

    def wire_responses(self, root, daemon, messages):
        """Drive the real plugin executable over stdio against the loopback
        daemon and return its JSON-RPC responses."""
        env = dict(
            os.environ,
            OCEAN_DAEMON_URL=daemon.url,
            OCEAN_SUBAGENT_STATE_DIR=str(Path(root) / "state"),
        )
        process = subprocess.run(
            [sys.executable, str(ROOT / "ocean-subagents.py")],
            input="".join(json.dumps(message) + "\n" for message in messages),
            capture_output=True,
            text=True,
            env=env,
            timeout=10,
            check=True,
        )
        self.assertEqual(process.stderr, "")
        return [json.loads(line) for line in process.stdout.splitlines()]

    @staticmethod
    def rpc(message_id, name, **args):
        return {
            "jsonrpc": "2.0",
            "id": message_id,
            "method": "invoke_tool",
            "params": {"name": name, "args": args},
        }

    def test_manifest_and_live_tools_match(self):
        manifest = tomllib.loads((ROOT / "plugin.toml").read_text())
        declared = [
            (tool["name"], tool["description"], tool["input_schema"]) for tool in manifest["tool"]
        ]
        live = [(tool["name"], tool["description"], tool["inputSchema"]) for tool in module.TOOLS]
        self.assertEqual(declared, live)

    def test_spawn_is_real_child_turn_and_completion_returns_output(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn(
                {
                    "task": "Inspect the repository",
                    "role": "reviewer",
                    "cwd": root,
                    "model": "deepseek-v4-pro",
                }
            )
            self.assertEqual(run["status"], "running")
            payload = daemon.payloads[-1]
            self.assertEqual(payload["agent"], module.WORKER_AGENT)
            self.assertEqual(payload["client_type"], "ocean-subagent")
            self.assertEqual(payload["model_id"], "deepseek-v4-pro")
            self.assertNotIn("thinking_level", payload)
            self.assertGreaterEqual(len(payload["decision_token"]), 48)
            self.assertIn("Do not delegate", payload["prompt"])

            daemon.complete(run["turn_id"], "review complete")
            finished = manager.refresh(run["run_id"])
            self.assertEqual(finished["status"], "completed")
            self.assertEqual(finished["output"], "review complete")
            self.assertEqual(self.stored(root, run["run_id"])["session_id"], run["session_id"])

    def test_follow_up_reuses_session_and_cancel_uses_turn_id(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "first", "cwd": root, "thinking_level": "max"})
            self.assertEqual(daemon.payloads[-1]["thinking_level"], "max")
            daemon.complete(run["turn_id"])
            manager.refresh(run["run_id"])

            followed = manager.send({"run_id": run["run_id"], "message": "second"})
            self.assertEqual(followed["session_id"], run["session_id"])
            self.assertEqual(daemon.payloads[-1]["session_id"], run["session_id"])
            self.assertEqual(daemon.payloads[-1]["thinking_level"], "max")
            cancelling = manager.cancel({"run_id": run["run_id"]})
            self.assertEqual(cancelling["status"], "cancelling")
            self.assertEqual(daemon.cancel_calls, [followed["turn_id"]])
            daemon.finish_cancel(followed["turn_id"])
            cancelled = manager.refresh(run["run_id"])
            self.assertEqual(cancelled["status"], "cancelled")

    def test_thinking_level_is_validated(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            with self.assertRaisesRegex(module.PluginError, "thinking_level must be one of"):
                manager.spawn({"task": "bad effort", "cwd": root, "thinking_level": "turbo"})
            self.assertEqual(daemon.payloads, [])
            manager.spawn({"task": "blank effort", "cwd": root, "thinking_level": ""})
            self.assertNotIn("thinking_level", daemon.payloads[-1])

    def test_child_permissions_are_scoped_and_token_bound(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "permission test", "cwd": root})
            permission_id = daemon.add_permission(run["turn_id"], "bash")
            pending = manager.permissions({"run_id": run["run_id"]})
            self.assertEqual(pending["status"], "waiting_for_permission")
            self.assertEqual(pending["permissions"][0]["permission_id"], permission_id)
            with self.assertRaisesRegex(module.PluginError, "expected_tool"):
                manager.decide(
                    {
                        "run_id": run["run_id"],
                        "permission_id": permission_id,
                        "expected_tool": "write",
                        "decision": "allow",
                    }
                )
            resolved = manager.decide(
                {
                    "run_id": run["run_id"],
                    "permission_id": permission_id,
                    "expected_tool": "bash",
                    "decision": "allow_session",
                }
            )
            self.assertTrue(resolved["ok"])
            self.assertEqual(daemon.decisions[0]["decision"], "allow_session")
            self.assertIn("decision_token", self.stored(root, run["run_id"]))

    def test_wait_refreshes_terminal_state(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "wait test", "cwd": root})
            daemon.complete(run["turn_id"], "done")
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 1})
            self.assertEqual(result["status"], "completed")
            self.assertEqual(result["output"], "done")

    def test_wait_reports_each_permission_prompt_once(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "needs approval", "cwd": root})
            threading.Timer(0.2, daemon.add_permission, args=(run["turn_id"],)).start()
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 5)

            # Still the same prompt: a second wait keeps its full budget
            # instead of returning at once and burning the parent's rounds.
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 0.6})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertGreaterEqual(time.monotonic() - started, 0.5)

            # The parent decides, and the child raises another prompt before
            # the next wait begins. That one is new, so it is reported at once
            # even though the wait starts with the run already blocked.
            first = manager.permissions({"run_id": run["run_id"]})["permissions"][0]
            manager.decide(
                {
                    "run_id": run["run_id"],
                    "permission_id": first["permission_id"],
                    "expected_tool": "bash",
                    "decision": "allow",
                }
            )
            daemon.add_permission(run["turn_id"], "write")
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 5)

            # Listing prompts reports them too.
            pending = manager.permissions({"run_id": run["run_id"]})["permissions"][0]
            manager.decide(
                {
                    "run_id": run["run_id"],
                    "permission_id": pending["permission_id"],
                    "expected_tool": "write",
                    "decision": "allow",
                }
            )
            daemon.add_permission(run["turn_id"], "edit")
            manager.permissions({"run_id": run["run_id"]})
            started = time.monotonic()
            manager.wait({"run_id": run["run_id"], "timeout_seconds": 0.6})
            self.assertGreaterEqual(time.monotonic() - started, 0.5)

    def test_a_repeated_identical_tool_call_is_reported_again(self):
        # The daemon mints one permission id per (tool, arguments) within a
        # turn, so a child that re-runs the same command raises a prompt with
        # an id the parent has already seen.
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "runs the tests twice", "cwd": root})
            reused = daemon.add_permission(run["turn_id"], "bash")
            first = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(first["status"], "waiting_for_permission")
            manager.decide(
                {
                    "run_id": run["run_id"],
                    "permission_id": reused,
                    "expected_tool": "bash",
                    "decision": "allow",
                }
            )

            # The same call again: same id, new prompt.
            with daemon.lock:
                request = daemon.requests[run["turn_id"]]
                daemon.permissions[reused] = {
                    "permission_id": reused,
                    "request_id": run["turn_id"],
                    "session_id": request["session_id"],
                    "tool": "bash",
                    "reason": "test permission",
                    "args": {"command": "true"},
                    "created_at": "2026-07-29T12:00:01Z",
                }
                request["state"] = "waiting_for_permission"
                request["permission_id"] = reused
            started = time.monotonic()
            again = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(again["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 5)

    def test_a_prompt_answered_elsewhere_and_raised_again_is_reported_again(self):
        # Another client can answer a child's prompt, so `decide` never runs
        # here. Seeing the run move on is what forgets the report.
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "approved from the cockpit", "cwd": root})
            reused = daemon.add_permission(run["turn_id"], "bash")
            first = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(first["status"], "waiting_for_permission")
            with daemon.lock:
                prompt = daemon.permissions.pop(reused)
                request = daemon.requests[run["turn_id"]]
                request["state"] = "running"
                request.pop("permission_id", None)
            self.assertEqual(manager.refresh(run["run_id"])["status"], "running")

            with daemon.lock:
                daemon.permissions[reused] = prompt
                request["state"] = "waiting_for_permission"
                request["permission_id"] = reused
            started = time.monotonic()
            again = manager.wait({"run_id": run["run_id"], "timeout_seconds": 3})
            self.assertEqual(again["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 2)

    def test_permissions_never_marks_a_prompt_it_did_not_list(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)

            # The request already says it is blocked, but the prompt is not in
            # the permission list yet. Nothing was shown, so nothing is marked.
            run = manager.spawn({"task": "prompt not listed yet", "cwd": root})
            with daemon.lock:
                request = daemon.requests[run["turn_id"]]
                request["state"] = "waiting_for_permission"
                request["permission_id"] = "not-listed-yet"
            listed = manager.permissions({"run_id": run["run_id"]})
            self.assertEqual(listed["permissions"], [])
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 3})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 2)

            # A prompt raised between the status read and the list read is in
            # the response, and the next wait still reports it.
            other = manager.spawn({"task": "prompt raised mid-call", "cwd": root})
            raised = []

            def raise_after_first_read(real):
                def read(*args, **kwargs):
                    result = real(*args, **kwargs)
                    if not raised:
                        raised.append(daemon.add_permission(other["turn_id"], "write"))
                    return result

                return read

            with mock.patch.object(
                manager.client,
                "request_status",
                side_effect=raise_after_first_read(manager.client.request_status),
            ), mock.patch.object(
                manager.client,
                "pending_permissions",
                side_effect=raise_after_first_read(manager.client.pending_permissions),
            ):
                listed = manager.permissions({"run_id": other["run_id"]})
            self.assertEqual(
                [item["permission_id"] for item in listed["permissions"]], raised
            )
            started = time.monotonic()
            result = manager.wait({"run_id": other["run_id"], "timeout_seconds": 3})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 2)

    def test_concurrency_is_bounded(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            for number in range(module.MAX_ACTIVE):
                manager.spawn({"task": f"task {number}", "cwd": root})
            with self.assertRaisesRegex(module.PluginError, "concurrency limit"):
                manager.spawn({"task": "one too many", "cwd": root})

    def test_runs_the_daemon_forgot_release_their_slots(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            runs = [
                manager.spawn({"task": f"task {number}", "cwd": root})
                for number in range(module.MAX_ACTIVE)
            ]
            daemon.say(runs[0]["turn_id"], "partial findings")
            daemon.forget_requests()

            # Before the fix all four stayed active forever and this raised
            # "subagent concurrency limit reached (4)" for every session.
            replacement = manager.spawn({"task": "after restart", "cwd": root})
            self.assertEqual(replacement["status"], "running")
            lost = [manager.refresh(run["run_id"]) for run in runs]
            self.assertEqual({run["status"] for run in lost}, {"lost"})
            self.assertEqual(lost[0]["output"], "partial findings")
            self.assertIsNone(lost[1]["output"])
            self.assertIn("no longer tracks this turn", lost[0]["error"])
            self.assertIsNotNone(lost[0]["finished_at"])

            # A lost run can be continued in its durable child session.
            followed = manager.send({"run_id": runs[0]["run_id"], "message": "continue"})
            self.assertEqual(followed["status"], "running")
            self.assertEqual(followed["session_id"], runs[0]["session_id"])

    def test_a_request_missing_moments_after_spawn_is_not_declared_lost(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "fresh", "cwd": root})
            daemon.forget_requests()
            with mock.patch.object(module, "LOST_GRACE_SECONDS", 60):
                self.assertEqual(manager.refresh(run["run_id"])["status"], "running")

    def test_cancel_of_an_unknown_request_settles_instead_of_cancelling_forever(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "orphan", "cwd": root})
            daemon.forget_requests()
            # The daemon's explicit "request not found" is authoritative, so the
            # grace period does not apply.
            with mock.patch.object(module, "LOST_GRACE_SECONDS", 60):
                result = manager.cancel({"run_id": run["run_id"]})
            self.assertEqual(result["status"], "lost")
            self.assertEqual(manager.list_runs({"active_only": True})["runs"], [])

    def test_cancel_of_a_finished_request_reports_how_it_finished(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "race", "cwd": root})
            daemon.complete(run["turn_id"], "finished first")
            result = manager.cancel({"run_id": run["run_id"]})
            self.assertEqual(result["status"], "completed")
            self.assertEqual(result["output"], "finished first")

    def test_a_malformed_cancel_acknowledgement_changes_nothing(self):
        # Only the daemon's own acknowledgement (`ok` exactly true, the same
        # request id, state cancelling) means a cancellation is in flight.
        # Anything else is neither acceptance nor refusal: an error, no write.
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = self.unwatched_run(manager, root)
            accepted = {"ok": True, "request_id": run["turn_id"], "state": "cancelling"}
            responses = [
                {},
                dict(accepted, ok=1),
                dict(accepted, ok="true"),
                {"ok": True, "state": "cancelling"},
                dict(accepted, request_id="another-request"),
                {"ok": True, "request_id": run["turn_id"]},
                dict(accepted, state="running"),
                dict(accepted, state="completed"),
                dict(accepted, state="cancelled"),
                [],
                b"not JSON: private daemon details",
            ]
            before = manager.store.path.read_bytes()
            for response in responses:
                with self.subTest(response=response):
                    daemon.cancel_reply = (200, response)
                    with self.assertRaises(module.PluginError) as raised:
                        manager.cancel({"run_id": run["run_id"]})
                    self.assertNotIn("private daemon details", str(raised.exception))
                    self.assertEqual(manager.store.path.read_bytes(), before)
                    self.assertEqual(manager.store.get(run["run_id"])["status"], "running")
                    self.assertEqual(daemon.requests[run["turn_id"]]["state"], "running")
            self.assertEqual(len(daemon.cancel_calls), len(responses))

            # The daemon's real acknowledgement still works afterwards.
            daemon.cancel_reply = None
            self.assertEqual(manager.cancel({"run_id": run["run_id"]})["status"], "cancelling")
            self.assertEqual(daemon.requests[run["turn_id"]]["state"], "cancelling")

    def test_a_failed_cancel_call_never_exposes_the_daemon_response_body(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            manager.watchdog_attempts = 1
            run = self.unwatched_run(manager, root)
            daemon.cancel_reply = (503, {"error": "private daemon details"})
            with self.assertRaises(module.PluginError) as raised:
                manager.cancel({"run_id": run["run_id"]})
            self.assertEqual(str(raised.exception), "Ocean daemon cancellation request failed")
            self.assertEqual(manager.store.get(run["run_id"])["status"], "running")

            # The watchdog stores the same controlled reason, not the body.
            expired = self.expire_watchdog(manager, run)
            self.assertEqual(expired["status"], "running")
            self.assertEqual(expired["error"], "Ocean daemon cancellation request failed")

            responses = self.wire_responses(root, daemon, [self.rpc(1, "cancel", run_id=run["run_id"])])
            self.assertEqual(responses[0]["error"]["code"], -32602)
            self.assertNotIn("private daemon details", responses[0]["error"]["message"])
            self.assertEqual(self.stored(root, run["run_id"])["status"], "running")

    def test_the_watchdog_never_claims_a_cancellation_the_daemon_did_not_accept(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            manager.watchdog_attempts = 1

            # The child finishes between the watchdog's refresh and its
            # cancel: the daemon refuses, and the run settles as it finished.
            finished = self.unwatched_run(manager, root, "finishes first")
            daemon.before_cancel = lambda request_id: daemon.complete(request_id, "beat the clock")
            settled = self.expire_watchdog(manager, finished)
            daemon.before_cancel = None
            self.assertEqual(settled["status"], "completed")
            self.assertEqual(settled["output"], "beat the clock")
            self.assertIsNone(settled["error"])
            self.assertEqual(daemon.cancel_calls, [finished["turn_id"]])

            # A malformed acknowledgement is recorded as the error it is; the
            # run stays running rather than cancelling.
            run = self.unwatched_run(manager, root, "answered badly")
            daemon.cancel_reply = (200, {"ok": True, "state": "running"})
            expired = self.expire_watchdog(manager, run)
            self.assertEqual(expired["status"], "running")
            self.assertIn("invalid cancellation acknowledgement", expired["error"])
            self.assertEqual(daemon.cancel_calls, [finished["turn_id"], run["turn_id"]])

            # Once the daemon answers properly, a poll re-arms the ceiling and
            # the cancellation goes through.
            daemon.cancel_reply = None
            manager.refresh(run["run_id"])
            self.assertTrue(
                wait_until(lambda: manager.store.get(run["run_id"])["status"] == "cancelling")
            )
            self.assertIn("elapsed-time ceiling", manager.store.get(run["run_id"])["error"])

    def test_cancel_acknowledgement_over_real_stdio(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = self.unwatched_run(manager, root)

            daemon.cancel_reply = (200, {"ok": True, "state": "cancelling"})
            responses = self.wire_responses(root, daemon, [self.rpc(1, "cancel", run_id=run["run_id"])])
            self.assertEqual(responses[0]["error"]["code"], -32602)
            self.assertIn("invalid cancellation acknowledgement", responses[0]["error"]["message"])
            self.assertEqual(self.stored(root, run["run_id"])["status"], "running")

            daemon.cancel_reply = None
            responses = self.wire_responses(root, daemon, [self.rpc(2, "cancel", run_id=run["run_id"])])
            self.assertEqual(responses[0]["result"]["status"], "cancelling")
            self.assertEqual(self.stored(root, run["run_id"])["status"], "cancelling")
            daemon.finish_cancel(run["turn_id"])
            responses = self.wire_responses(root, daemon, [self.rpc(3, "status", run_id=run["run_id"])])
            self.assertEqual(responses[0]["result"]["status"], "cancelled")

            # A cancel the daemon refuses because the turn already finished
            # reports how it finished instead of an error.
            other = self.unwatched_run(manager, root, "finished before cancel")
            daemon.complete(other["turn_id"], "wire completion")
            responses = self.wire_responses(root, daemon, [self.rpc(4, "cancel", run_id=other["run_id"])])
            self.assertEqual(responses[0]["result"]["status"], "completed")
            self.assertEqual(responses[0]["result"]["output"], "wire completion")
            self.assertEqual(daemon.cancel_calls, [run["turn_id"], run["turn_id"], other["turn_id"]])

    def test_spawn_refuses_when_the_worker_profile_would_not_narrow_tools(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            cases = [
                (None, "does not resolve"),
                ({"config": {"tools": []}, "tools": []}, "empty tool allowlist"),
                (
                    {"config": {"tools": ["read", "plugin__ocean-subagents__spawn"]}, "tools": []},
                    "allows subagent tools",
                ),
                # Names are exact. An allowlist that matches no tool that
                # exists is no allowlist: the daemon keeps every tool.
                ({"config": {"tools": ["Read", "Bash"]}, "tools": []}, "names no built-in tool"),
                ({"config": {"tools": ["lsp"]}, "tools": ["README"]}, "names no built-in tool"),
                # Subprocess capability tools are added after narrowing.
                (
                    {
                        "config": {
                            "tools": ["read"],
                            "subprocess_capability": [{"command": "./tools/x"}],
                        },
                        "tools": [],
                    },
                    "binds subprocess capabilities",
                ),
            ]
            for worker, expected in cases:
                daemon.worker = worker
                with self.assertRaisesRegex(module.PluginError, expected):
                    manager.spawn({"task": "must not start", "cwd": root})
            self.assertEqual(daemon.payloads, [])

            # Allowlist entries from the agent's tools/ folder count too.
            daemon.worker = {"config": {"tools": []}, "tools": ["read"]}
            run = manager.spawn({"task": "ok", "cwd": root})
            self.assertEqual(run["status"], "running")

            # `send` starts a child turn as well, so it is guarded the same way.
            daemon.complete(run["turn_id"])
            manager.refresh(run["run_id"])
            daemon.worker = None
            with self.assertRaisesRegex(module.PluginError, "does not resolve"):
                manager.send({"run_id": run["run_id"], "message": "must not start"})
            self.assertEqual(len(daemon.payloads), 1)

    def test_unroutable_model_failure_lists_what_the_daemon_can_route(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "typo", "cwd": root, "model": "gpt-5.6-mini"})
            daemon.fail(run["turn_id"], "failed to resolve model `gpt-5.6-mini`: unknown model")
            result = manager.refresh(run["run_id"])
            self.assertEqual(result["status"], "failed")
            self.assertIn("failed to resolve model `gpt-5.6-mini`", result["error"])
            # `send` would reuse the bad model, so the hint points at a new spawn.
            self.assertIn("Spawn again without `model`", result["error"])
            self.assertIn("ready catalog id: deepseek-v4-pro, gpt-5.5.", result["error"])
            self.assertNotIn("glm-5.3", result["error"])

            other = manager.spawn({"task": "other failure", "cwd": root})
            daemon.fail(other["turn_id"], "provider error: 529 overloaded")
            self.assertEqual(
                manager.refresh(other["run_id"])["error"], "provider error: 529 overloaded"
            )

    def test_polling_an_unchanged_run_does_not_rewrite_state(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "slow", "cwd": root})
            with mock.patch.object(
                manager.store, "_write_locked", wraps=manager.store._write_locked
            ) as write:
                manager.wait({"run_id": run["run_id"], "timeout_seconds": 0.6})
                self.assertEqual(write.call_count, 0)
                daemon.complete(run["turn_id"], "done")
                manager.refresh(run["run_id"])
                self.assertEqual(write.call_count, 1)
                # A finished run is settled: no daemon calls, no writes.
                daemon.unavailable = True
                self.assertEqual(manager.refresh(run["run_id"])["output"], "done")
                self.assertEqual(write.call_count, 1)

    def test_finished_runs_are_pruned_but_active_runs_never_are(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            with mock.patch.object(module, "MAX_RETAINED_RUNS", 2):
                # Finish times deliberately out of insertion order: the two
                # most recently FINISHED runs survive, not the last two added.
                finish_times = [
                    "2026-07-29T12:00:03Z",
                    "2026-07-29T12:00:01Z",
                    "2026-07-29T12:00:04Z",
                    "2026-07-29T12:00:02Z",
                ]
                finished = []
                for number, finished_at in enumerate(finish_times):
                    run = manager.spawn({"task": f"done {number}", "cwd": root})
                    daemon.complete(run["turn_id"], f"output {number}", finished_at)
                    manager.refresh(run["run_id"])
                    finished.append(run["run_id"])
                active = manager.spawn({"task": "still running", "cwd": root})
            kept = set(json.loads((Path(root) / "state/runs.json").read_text())["runs"])
            self.assertEqual(kept, {finished[0], finished[2], active["run_id"]})

    def test_state_read_for_an_older_turn_cannot_overwrite_the_next_turn(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "first", "cwd": root})
            stale = manager.store.get(run["run_id"])
            daemon.complete(run["turn_id"])
            manager.refresh(run["run_id"])
            followed = manager.send({"run_id": run["run_id"], "message": "second"})

            # A watchdog that examined the first turn settles it only now.
            daemon.forget_requests()
            manager._settle_untracked(stale)
            current = manager.store.get(run["run_id"])
            self.assertEqual(current["status"], "running")
            self.assertEqual(current["request_id"], followed["turn_id"])

    def test_lost_output_is_never_an_earlier_turns_answer(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "first", "cwd": root})
            daemon.complete(run["turn_id"], "ANSWER TO FIRST TASK")
            manager.refresh(run["run_id"])
            manager.send({"run_id": run["run_id"], "message": "second"})

            # The daemon restarts before the second turn writes anything.
            daemon.forget_requests()
            lost = manager.refresh(run["run_id"])
            self.assertEqual(lost["status"], "lost")
            self.assertIsNone(lost["output"])

    def test_a_run_settled_while_its_session_was_unreadable_collects_output_later(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)

            lost = manager.spawn({"task": "lost", "cwd": root})
            daemon.say(lost["turn_id"], "partial findings")
            done = manager.spawn({"task": "done", "cwd": root})
            daemon.complete(done["turn_id"], "finished answer")
            daemon.requests.pop(lost["turn_id"])
            daemon.sessions_unreadable = True

            # Both settle at once and release their slots even though neither
            # session can be read. The completed one used to stay active.
            self.assertEqual(manager.refresh(lost["run_id"])["status"], "lost")
            settled = manager.refresh(done["run_id"])
            self.assertEqual(settled["status"], "completed")
            self.assertIsNone(settled["output"])
            self.assertEqual(manager.list_runs({"active_only": True})["runs"], [])

            daemon.sessions_unreadable = False
            self.assertEqual(manager.refresh(lost["run_id"])["output"], "partial findings")
            self.assertEqual(manager.refresh(done["run_id"])["output"], "finished answer")

    def test_a_malformed_request_list_never_settles_runs_as_lost(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "alive", "cwd": root})
            daemon.requests_malformed = True
            with self.assertRaisesRegex(module.PluginError, "no request list"):
                manager.refresh(run["run_id"])
            self.assertEqual(manager.store.get(run["run_id"])["status"], "running")

    def test_a_request_the_session_still_claims_is_not_declared_lost(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "still claimed", "cwd": root})
            daemon.requests.pop(run["turn_id"])
            daemon.session_claims_active[run["session_id"]] = [run["turn_id"]]
            self.assertEqual(manager.refresh(run["run_id"])["status"], "running")
            daemon.session_claims_active.clear()
            self.assertEqual(manager.refresh(run["run_id"])["status"], "lost")

    def test_send_respects_the_concurrency_cap(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            finished = manager.spawn({"task": "finished", "cwd": root})
            daemon.complete(finished["turn_id"])
            manager.refresh(finished["run_id"])
            for number in range(module.MAX_ACTIVE):
                manager.spawn({"task": f"task {number}", "cwd": root})
            with self.assertRaisesRegex(module.PluginError, "concurrency limit"):
                manager.send({"run_id": finished["run_id"], "message": "a fifth child"})

    def overdue_run(self, root, request_id, session_id):
        state = Path(root) / "state"
        state.mkdir(mode=0o700)
        run_id = str(uuid.uuid4())
        long_ago = "2026-07-29T12:00:00+00:00"
        run = {
            "run_id": run_id,
            "task": "overdue",
            "role": "general worker",
            "cwd": root,
            "model": None,
            "status": "running",
            "turn_id": request_id,
            "request_id": request_id,
            "session_id": session_id,
            "output": None,
            "error": None,
            "created_at": long_ago,
            "updated_at": long_ago,
            "started_at": long_ago,
            "finished_at": None,
            "timeout_seconds": 30,
            "decision_token": "token",
            "turns": [],
        }
        (state / "runs.json").write_text(
            json.dumps({"schema_version": module.SCHEMA_VERSION, "runs": {run_id: run}})
        )
        return run_id

    def test_startup_watchdog_retries_until_the_daemon_answers(self):
        # The daemon launches plugins before its listener binds, so the first
        # watchdog call for an overdue run fails. It used to give up for good.
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            run_id = self.overdue_run(root, str(uuid.uuid4()), str(uuid.uuid4()))
            daemon.unavailable = True
            # Enough attempts that the retry window cannot close before the
            # test brings the daemon back, however loaded the machine is.
            with mock.patch.object(module, "WATCHDOG_RETRY_SECONDS", 0.05), mock.patch.object(
                module, "WATCHDOG_ATTEMPTS", 2_000
            ):
                manager = self.manager(root, daemon)
                self.assertTrue(
                    wait_until(lambda: manager.store.get(run_id)["error"] is not None)
                )
                self.assertEqual(manager.store.get(run_id)["status"], "running")
                daemon.unavailable = False
                self.assertTrue(
                    wait_until(lambda: manager.store.get(run_id)["status"] == "lost")
                )

    def test_watchdog_cancels_an_overdue_turn_that_is_still_running(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            request_id, session_id = str(uuid.uuid4()), str(uuid.uuid4())
            daemon.requests[request_id] = {
                "request_id": request_id,
                "session_id": session_id,
                "state": "running",
                "message": "agent turn running",
            }
            daemon.sessions[session_id] = []
            run_id = self.overdue_run(root, request_id, session_id)
            manager = self.manager(root, daemon)
            self.assertTrue(
                wait_until(lambda: manager.store.get(run_id)["status"] == "cancelling")
            )
            self.assertEqual(daemon.cancel_calls, [request_id])
            self.assertIn("elapsed-time ceiling", manager.store.get(run_id)["error"])

            # Polling a run that is already cancelling neither asks again nor
            # starts another watchdog thread per poll.
            self.assertTrue(wait_until(lambda: not manager.watchdogs))
            real_thread, started = threading.Thread, []

            def recording_thread(*args, **kwargs):
                started.append(kwargs.get("name"))
                return real_thread(*args, **kwargs)

            with mock.patch.object(module.threading, "Thread", side_effect=recording_thread):
                for _ in range(3):
                    manager.refresh(run_id)
            self.assertEqual([name for name in started if name], [])
            self.assertEqual(daemon.cancel_calls, [request_id])

            # If the daemon then forgets the request, the reason for the
            # cancellation survives beside the lost explanation.
            daemon.forget_requests()
            lost = manager.refresh(run_id)
            self.assertEqual(lost["status"], "lost")
            self.assertIn("no longer tracks this turn", lost["error"])
            self.assertIn("elapsed-time ceiling", lost["error"])

    def test_a_cancelling_run_persisted_across_a_restart_settles_on_its_own(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            run_id = self.overdue_run(root, str(uuid.uuid4()), str(uuid.uuid4()))
            state = Path(root) / "state/runs.json"
            data = json.loads(state.read_text())
            data["runs"][run_id]["status"] = "cancelling"
            state.write_text(json.dumps(data))
            # The daemon restarted and no longer knows the request. Nobody has
            # to call a tool for the startup watchdog to notice.
            manager = self.manager(root, daemon)
            self.assertTrue(wait_until(lambda: manager.store.get(run_id)["status"] == "lost"))
            self.assertEqual(daemon.cancel_calls, [])

    def test_the_ceiling_reason_survives_a_restart_that_starts_unreachable(self):
        # The daemon launches plugins before its listener binds, so the
        # startup watchdog's first call always fails. That failure must not
        # replace the reason the run was being cancelled.
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            run_id = self.overdue_run(root, str(uuid.uuid4()), str(uuid.uuid4()))
            state = Path(root) / "state/runs.json"
            data = json.loads(state.read_text())
            data["runs"][run_id]["status"] = "cancelling"
            data["runs"][run_id]["error"] = "elapsed-time ceiling reached; cancellation requested"
            state.write_text(json.dumps(data))
            daemon.unavailable = True
            real_status, refused = module.DaemonClient.request_status, []

            def counted_status(client, request_id):
                try:
                    return real_status(client, request_id)
                except module.PluginError:
                    refused.append(request_id)
                    raise

            with mock.patch.object(module, "WATCHDOG_RETRY_SECONDS", 0.05), mock.patch.object(
                module, "WATCHDOG_ATTEMPTS", 2_000
            ), mock.patch.object(module.DaemonClient, "request_status", counted_status):
                manager = self.manager(root, daemon)
                self.assertTrue(wait_until(lambda: len(refused) >= 2))
                self.assertIn("elapsed-time ceiling", manager.store.get(run_id)["error"])
                daemon.unavailable = False
                self.assertTrue(
                    wait_until(lambda: manager.store.get(run_id)["status"] == "lost")
                )
            lost = manager.store.get(run_id)
            self.assertIn("elapsed-time ceiling", lost["error"])
            self.assertNotIn("HTTP 503", lost["error"])
            self.assertEqual(daemon.cancel_calls, [])

    def test_a_cancellation_still_in_flight_after_a_restart_is_not_asked_for_twice(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            request_id, session_id = str(uuid.uuid4()), str(uuid.uuid4())
            daemon.requests[request_id] = {
                "request_id": request_id,
                "session_id": session_id,
                "state": "cancelling",
                "message": "cancel requested; cancellation token sent",
            }
            daemon.sessions[session_id] = []
            run_id = self.overdue_run(root, request_id, session_id)
            state = Path(root) / "state/runs.json"
            data = json.loads(state.read_text())
            data["runs"][run_id]["status"] = "cancelling"
            state.write_text(json.dumps(data))
            real_status, polled = module.DaemonClient.request_status, []

            def counted_status(client, polled_id):
                polled.append(polled_id)
                return real_status(client, polled_id)

            with mock.patch.object(module.DaemonClient, "request_status", counted_status):
                manager = self.manager(root, daemon)
                self.assertTrue(wait_until(lambda: polled and not manager.watchdogs))
            self.assertEqual(manager.store.get(run_id)["status"], "cancelling")
            self.assertEqual(daemon.cancel_calls, [])

    def test_refresh_rearms_a_watchdog_that_gave_up(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            request_id, session_id = str(uuid.uuid4()), str(uuid.uuid4())
            daemon.requests[request_id] = {
                "request_id": request_id,
                "session_id": session_id,
                "state": "running",
                "message": "agent turn running",
            }
            daemon.sessions[session_id] = []
            run_id = self.overdue_run(root, request_id, session_id)
            daemon.unavailable = True
            with mock.patch.object(module, "WATCHDOG_ATTEMPTS", 1):
                manager = self.manager(root, daemon)
                self.assertTrue(wait_until(lambda: not manager.watchdogs))
            self.assertEqual(manager.store.get(run_id)["status"], "running")
            self.assertEqual(daemon.cancel_calls, [])

            # The ceiling must still be enforced once the daemon is back.
            daemon.unavailable = False
            manager.refresh(run_id)
            self.assertTrue(
                wait_until(lambda: manager.store.get(run_id)["status"] == "cancelling")
            )
            self.assertEqual(daemon.cancel_calls, [request_id])

    def test_watchdog_for_a_pruned_run_exits_quietly(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            run_id = self.overdue_run(root, str(uuid.uuid4()), str(uuid.uuid4()))
            daemon.unavailable = True
            with mock.patch.object(module, "WATCHDOG_RETRY_SECONDS", 0.05), mock.patch.object(
                module, "WATCHDOG_ATTEMPTS", 2_000
            ):
                manager = self.manager(root, daemon)
                self.assertTrue(
                    wait_until(lambda: manager.store.get(run_id)["error"] is not None)
                )
                with manager.store.lock:
                    del manager.store.data["runs"][run_id]
                self.assertTrue(wait_until(lambda: not manager.watchdogs))

    def test_check_reads_state_without_starting_watchdogs(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            self.overdue_run(root, str(uuid.uuid4()), str(uuid.uuid4()))
            environment = {
                "OCEAN_SUBAGENT_STATE_DIR": str(Path(root) / "state"),
                "OCEAN_DAEMON_URL": daemon.url,
            }
            with mock.patch.dict(module.os.environ, environment), mock.patch.object(
                module, "build_manager", side_effect=AssertionError("check must not build")
            ), mock.patch("builtins.print"):
                self.assertEqual(module.main(["--check"]), 0)
            self.assertEqual(daemon.cancel_calls, [])


if __name__ == "__main__":
    unittest.main()
