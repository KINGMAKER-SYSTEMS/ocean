import importlib.util
import json
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
        self.unavailable = False
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
                encoded = json.dumps(body).encode()
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
                        owner.sessions.setdefault(session_id, [])
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
                    with owner.lock:
                        owner.cancel_calls.append(request_id)
                        request = owner.requests.get(request_id)
                        if request is None:
                            body = {"ok": False, "state": "errored", "message": "request not found"}
                        elif request["state"] in TERMINAL_REQUEST_STATES:
                            body = {
                                "ok": False,
                                "state": request["state"],
                                "message": "request is already terminal; cancel ignored",
                            }
                        else:
                            request["state"] = "cancelling"
                            request["message"] = "cancel requested; cancellation token sent"
                            body = {"ok": True, "state": "cancelling"}
                    self.reply(200, body)
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
                        ]
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
        return permission_id

    def complete(self, request_id, output="worker result"):
        with self.lock:
            request = self.requests[request_id]
            request.update(
                state="completed",
                message="prompt completed",
                finished_at="2026-07-29T12:00:00Z",
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
            run = manager.spawn({"task": "first", "cwd": root, "thinking_level": "low"})
            self.assertEqual(daemon.payloads[-1]["thinking_level"], "low")
            daemon.complete(run["turn_id"])
            manager.refresh(run["run_id"])

            followed = manager.send({"run_id": run["run_id"], "message": "second"})
            self.assertEqual(followed["session_id"], run["session_id"])
            self.assertEqual(daemon.payloads[-1]["session_id"], run["session_id"])
            self.assertEqual(daemon.payloads[-1]["thinking_level"], "low")
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

    def test_wait_returns_when_a_permission_prompt_appears_but_does_not_spin_on_it(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "needs approval", "cwd": root})
            threading.Timer(0.2, daemon.add_permission, args=(run["turn_id"],)).start()
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 10})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertLess(time.monotonic() - started, 5)

            # The prompt is still pending: a second wait keeps its full budget
            # instead of returning at once and burning the parent's rounds.
            started = time.monotonic()
            result = manager.wait({"run_id": run["run_id"], "timeout_seconds": 0.6})
            self.assertEqual(result["status"], "waiting_for_permission")
            self.assertGreaterEqual(time.monotonic() - started, 0.5)

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
            ]
            for worker, expected in cases:
                daemon.worker = worker
                with self.assertRaisesRegex(module.PluginError, expected):
                    manager.spawn({"task": "must not start", "cwd": root})
            self.assertEqual(daemon.payloads, [])

            # Allowlist entries from the agent's tools/ folder count too.
            daemon.worker = {"config": {"tools": []}, "tools": ["read"]}
            self.assertEqual(manager.spawn({"task": "ok", "cwd": root})["status"], "running")

    def test_unroutable_model_failure_lists_what_the_daemon_can_route(self):
        with tempfile.TemporaryDirectory() as root, FakeDaemon() as daemon:
            manager = self.manager(root, daemon)
            run = manager.spawn({"task": "typo", "cwd": root, "model": "gpt-5.6-mini"})
            daemon.fail(run["turn_id"], "failed to resolve model `gpt-5.6-mini`: unknown model")
            result = manager.refresh(run["run_id"])
            self.assertEqual(result["status"], "failed")
            self.assertIn("failed to resolve model `gpt-5.6-mini`", result["error"])
            self.assertIn("ready catalog ids: deepseek-v4-pro, gpt-5.5.", result["error"])
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
                finished = []
                for number in range(4):
                    run = manager.spawn({"task": f"done {number}", "cwd": root})
                    daemon.complete(run["turn_id"], f"output {number}")
                    manager.refresh(run["run_id"])
                    finished.append(run["run_id"])
                    time.sleep(1.05)  # finished_at has one-second resolution
                active = manager.spawn({"task": "still running", "cwd": root})
            kept = set(json.loads((Path(root) / "state/runs.json").read_text())["runs"])
            self.assertEqual(kept, {finished[2], finished[3], active["run_id"]})

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
            with mock.patch.object(module, "WATCHDOG_RETRY_SECONDS", 0.05):
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
