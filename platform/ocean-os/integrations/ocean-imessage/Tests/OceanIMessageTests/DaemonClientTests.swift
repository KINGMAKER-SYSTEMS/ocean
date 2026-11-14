import XCTest
@testable import OceanIMessage

/// Stands in for the local daemon at the URL-loading layer, so the real client
/// code runs: preflight, session probe, turn submission, the event stream, and
/// cancellation.
private final class StubDaemon: URLProtocol, @unchecked Sendable {
    struct Call: Equatable {
        let method: String
        let path: String
    }

    enum Stream {
        case lines([String])
        case never
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var recordedCalls: [Call] = []
    nonisolated(unsafe) private static var recordedTurnBodies: [[String: Any]] = []
    nonisolated(unsafe) private static var agentTools: [String] = ["read"]
    nonisolated(unsafe) private static var stream: Stream = .lines([])

    static let turnID = "11111111-1111-4111-8111-111111111111"
    static let sessionID = "22222222-2222-4222-8222-222222222222"

    static func reset(agentTools tools: [String] = ["read"], stream newStream: Stream) {
        lock.lock()
        defer { lock.unlock() }
        recordedCalls = []
        recordedTurnBodies = []
        agentTools = tools
        stream = newStream
    }

    static var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recordedCalls
    }

    static var turnBodies: [[String: Any]] {
        lock.lock()
        defer { lock.unlock() }
        return recordedTurnBodies
    }

    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [StubDaemon.self]
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}

    override func startLoading() {
        guard let url = request.url else { return }
        let method = request.httpMethod ?? "GET"
        let body = Self.readBody(request)
        Self.lock.lock()
        Self.recordedCalls.append(Call(method: method, path: url.path))
        if url.path == "/v1/agent/turns", let body,
           let object = try? JSONSerialization.jsonObject(with: body) as? [String: Any] {
            Self.recordedTurnBodies.append(object)
        }
        let tools = Self.agentTools
        let stream = Self.stream
        Self.lock.unlock()

        func respond(_ status: Int, _ object: [String: Any]) {
            let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
            send(status: status, contentType: "application/json", data: data, finish: true)
        }

        switch (method, url.path) {
        case ("GET", let path) where path.hasPrefix("/v1/agents/"):
            let name = String(path.dropFirst("/v1/agents/".count))
            respond(200, ["ok": true, "agent": ["name": name, "config": ["tools": tools]]])
        case ("GET", let path) where path.hasPrefix("/v1/agent/sessions/"):
            respond(200, ["ok": true, "turns": []])
        case ("POST", "/v1/agent/turns"):
            respond(202, ["ok": true, "turn_id": Self.turnID, "session_id": Self.sessionID, "status": "running"])
        case ("POST", let path) where path.hasSuffix("/cancel"):
            respond(200, ["ok": true, "state": "cancelling"])
        case ("GET", "/v1/agent/events"):
            switch stream {
            case let .lines(lines):
                send(status: 200, contentType: "text/event-stream",
                     data: Data((lines.joined(separator: "\n") + "\n").utf8), finish: true)
            case .never:
                // Headers only: the stream stays open, as a turn that never ends.
                send(status: 200, contentType: "text/event-stream", data: Data(": open\n".utf8), finish: false)
            }
        default:
            respond(404, ["ok": false])
        }
    }

    private func send(status: Int, contentType: String, data: Data, finish: Bool) {
        guard let url = request.url,
              let response = HTTPURLResponse(
                  url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": contentType]
              )
        else { return }
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        if finish { client?.urlProtocolDidFinishLoading(self) }
    }

    private static func readBody(_ request: URLRequest) -> Data? {
        if let body = request.httpBody { return body }
        guard let stream = request.httpBodyStream else { return nil }
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4_096)
        while stream.hasBytesAvailable {
            let read = stream.read(&buffer, maxLength: buffer.count)
            guard read > 0 else { break }
            data.append(buffer, count: read)
        }
        return data
    }
}

final class DaemonClientTests: XCTestCase {
    private let roster = try! Roster(
        localIdentity: "+15550100100",
        members: [RosterMember(id: "ada", phone: "+15550100101", role: "admin", name: "Ada")],
        roles: ["admin": RolePolicy(agent: "text-admin", repliesPerHour: 30)]
    )

    private func client(
        timeout: Duration = .seconds(5),
        maximumLineBytes: Int = LineSplitter.defaultMaximumLineBytes
    ) -> OceanDaemonClient {
        OceanDaemonClient(
            config: BridgeConfig(
                daemonURL: URL(string: "http://127.0.0.1:4780/")!,
                cwd: "/tmp/ocean-text",
                messagesAccountID: nil,
                roster: roster,
                turnTimeout: timeout,
                acknowledgeAfter: nil
            ),
            session: StubDaemon.session(),
            maximumLineBytes: maximumLineBytes
        )
    }

    private func store() -> (StateStore, URL) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        return (StateStore(url: root.appendingPathComponent("state.json")), root)
    }

    private func frame(_ event: String, _ fields: [String: Any]) -> [String] {
        var payload = fields
        payload["turn_id"] = payload["turn_id"] ?? StubDaemon.turnID
        let json = String(decoding: try! JSONSerialization.data(withJSONObject: payload), as: UTF8.self)
        return ["event: \(event)", "data: \(json)", ""]
    }

    private func ask(_ client: OceanDaemonClient, _ store: StateStore, text: String = "what is live") async throws -> String {
        let member = try XCTUnwrap(roster.member(id: "ada"))
        let policy = try XCTUnwrap(roster.policy(for: member))
        return try await client.reply(
            to: AllowedMessage(id: "m-1", member: "ada", text: text), member: member, policy: policy, store: store
        )
    }

    func testReplyRunsTheRoleAgentStrictlyResumesTheSessionAndKeepsOnlyTheFinalAnswer() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        StubDaemon.reset(stream: .lines(
            frame("assistant_text_delta", ["delta": "Let me check the campaigns."])
                + frame("tool_call_started", ["name": "read"])
                + frame("assistant_text_delta", ["delta": "someone else's turn", "turn_id": "other"])
                + frame("assistant_text_delta", ["delta": "Twelve posts are live\u{2028}today."])
                + frame("turn_finished", ["status": "completed"])
        ))

        let first = try await ask(client(), store)
        XCTAssertEqual(first, "Twelve posts are live\u{2028}today.")

        let body = try XCTUnwrap(StubDaemon.turnBodies.first)
        XCTAssertEqual(body["agent"] as? String, "text-admin")
        XCTAssertEqual(body["require_named_agent"] as? Bool, true)
        XCTAssertEqual(body["client_type"] as? String, "ocean-imessage")
        XCTAssertNil(body["session_id"])
        let prompt = try XCTUnwrap(body["prompt"] as? String)
        XCTAssertTrue(prompt.contains("member_id: ada"))
        XCTAssertFalse(prompt.contains("+1555"))

        // The second message continues the same session.
        XCTAssertEqual(try store.session(for: "ada", agent: "text-admin"), StubDaemon.sessionID)
        _ = try await ask(client(), store)
        XCTAssertEqual(StubDaemon.turnBodies.last?["session_id"] as? String, StubDaemon.sessionID)
        XCTAssertFalse(StubDaemon.calls.contains { $0.path.hasSuffix("/cancel") })
    }

    func testATurnThatOutlastsTheLimitIsCancelledInTheDaemon() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        StubDaemon.reset(stream: .never)

        do {
            _ = try await ask(client(timeout: .milliseconds(300)), store)
            XCTFail("a turn that never finishes must time out")
        } catch {
            XCTAssertEqual(error as? BridgeError, .daemonTimeout)
        }
        XCTAssertEqual(
            StubDaemon.calls.last, .init(method: "POST", path: "/v1/requests/\(StubDaemon.turnID)/cancel")
        )
    }

    func testAFailedOrFatalStreamCancelsTheTurn() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        for lines in [
            frame("turn_finished", ["status": "failed"]),
            ["event: error", "data: {\"lagged\":3}", ""],
            frame("assistant_text_delta", ["delta": "cut off mid-stream"]),
        ] {
            StubDaemon.reset(stream: .lines(lines))
            do {
                _ = try await ask(client(), store)
                XCTFail("expected a daemon failure")
            } catch {
                XCTAssertEqual(error as? BridgeError, .daemonFailure)
            }
            XCTAssertTrue(StubDaemon.calls.contains(.init(method: "POST", path: "/v1/requests/\(StubDaemon.turnID)/cancel")))
        }
    }

    func testAnOversizedFrameThatCouldShapeTheReplyFailsClosedInsteadOfSendingAPartialAnswer() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        let huge = String(repeating: "x", count: 2_000)
        let started = frame("turn_started", [:])
        let narration = frame("assistant_text_delta", ["delta": "Let me look that up."])
        let finished = frame("turn_finished", ["status": "completed"])
        let unsafeStreams: [(String, [String])] = [
            ("current-turn delta lost", started + narration + frame("assistant_text_delta", ["delta": huge]) + finished),
            ("current-turn tool boundary lost", started + narration + frame("tool_call_started", ["arguments": huge]) + frame("assistant_text_delta", ["delta": "Done."]) + finished),
            ("terminal event lost", started + narration + frame("turn_finished", ["status": "completed", "error": huge])),
            ("turn start marker lost", frame("turn_started", ["note": huge]) + narration + finished),
            ("unidentifiable line lost", started + narration + ["event: \(huge)", "data: {}", ""] + finished),
        ]
        for (name, lines) in unsafeStreams {
            StubDaemon.reset(stream: .lines(lines))
            do {
                let reply = try await ask(client(maximumLineBytes: 512), store)
                XCTFail("\(name): sent a reply assembled across a lost frame: \(reply)")
            } catch {
                XCTAssertEqual(error as? BridgeError, .daemonFailure, name)
            }
            XCTAssertTrue(
                StubDaemon.calls.contains(.init(method: "POST", path: "/v1/requests/\(StubDaemon.turnID)/cancel")), name
            )
        }
    }

    func testOversizedFramesThatCannotShapeTheReplyAreSkippedAndTheAnswerIsIntact() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        let huge = String(repeating: "x", count: 2_000)
        // Replayed history from an earlier turn in the same session, including
        // oversized frames of every kind, then this turn with a large tool result.
        let history = frame("turn_started", ["turn_id": "earlier"])
            + frame("assistant_text_delta", ["turn_id": "earlier", "delta": huge])
            + frame("tool_call_started", ["turn_id": "earlier", "arguments": huge])
            + frame("tool_call_finished", ["turn_id": "earlier", "result": huge])
            + frame("turn_finished", ["turn_id": "earlier", "status": "completed", "error": huge])
        let current = frame("turn_started", [:])
            + frame("assistant_text_delta", ["delta": "Checking the export."])
            + frame("tool_call_started", ["name": "read"])
            + frame("tool_call_finished", ["result": huge])
            + frame("assistant_text_delta", ["delta": "Nine campaigns are live."])
            + frame("turn_finished", ["status": "completed"])
        StubDaemon.reset(stream: .lines(history + current))

        let reply = try await ask(client(maximumLineBytes: 512), store)
        XCTAssertEqual(reply, "Nine campaigns are live.")
        XCTAssertFalse(StubDaemon.calls.contains { $0.path.hasSuffix("/cancel") })
    }

    func testALedgerWriteFailureAfterTheTurnIsAcceptedCancelsThatTurn() async throws {
        let (store, root) = store()
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
            try? FileManager.default.removeItem(at: root)
        }
        // A readable but unwritable ledger: lookups work, the session write fails.
        try store.advanceCursor(to: 1)
        try FileManager.default.setAttributes([.posixPermissions: 0o500], ofItemAtPath: root.path)
        StubDaemon.reset(stream: .lines(
            frame("assistant_text_delta", ["delta": "done"]) + frame("turn_finished", ["status": "completed"])
        ))

        do {
            _ = try await ask(client(), store)
            XCTFail("the ledger write must fail")
        } catch {
            XCTAssertNotEqual(error as? BridgeError, .daemonTimeout)
        }
        XCTAssertTrue(StubDaemon.calls.contains(.init(method: "POST", path: "/v1/agent/turns")))
        XCTAssertEqual(
            StubDaemon.calls.last, .init(method: "POST", path: "/v1/requests/\(StubDaemon.turnID)/cancel")
        )
    }

    func testARoleAgentWithoutADeclaredAllowlistNeverReachesTheTurnEndpoint() async throws {
        let (store, root) = store()
        defer { try? FileManager.default.removeItem(at: root) }
        StubDaemon.reset(agentTools: [], stream: .lines([]))

        do {
            _ = try await ask(client(), store)
            XCTFail("an agent with no allowlist must be refused")
        } catch {
            XCTAssertEqual(error as? BridgeError, .agentUnavailable)
        }
        XCTAssertFalse(StubDaemon.calls.contains { $0.path == "/v1/agent/turns" })
    }
}
