import Foundation

private enum Command: String {
    case accounts
    case check
    case poll
    case prime
    case watch
    case send
    case run
}

/// Fixed, content-free texts the bridge itself may send.
enum BridgeText {
    static let acknowledgement = "On it. I'll text you when it's done."
    static let failure = "Something went wrong on my end. Try that again in a minute."
    static let reset = "Started a new conversation."
}

/// Where finished replies go. `standardOutput` is the operator's rehearsal
/// mode: the whole pipeline runs, but nothing is handed to Messages.
private enum ReplySink: Sendable {
    case messages(RosterSender)
    case standardOutput
}

private struct PrintedReply: Encodable {
    let member: String
    let bubbles: [String]
}

/// Runs each member's messages in arrival order while different members
/// proceed concurrently, so one long task never blocks another teammate.
private actor MemberQueues {
    private var tails: [String: Task<Void, Never>] = [:]

    func enqueue(member: String, _ work: @escaping @Sendable () async -> Void) {
        let previous = tails[member]
        tails[member] = Task {
            await previous?.value
            await work()
        }
    }
}

/// Diagnostics never carry message text, phone numbers, or Apple-event detail.
/// Roster member ids and role names are operator-chosen labels and may appear.
private func log(_ line: String) {
    FileHandle.standardError.write(Data("ocean-imessage: \(line)\n".utf8))
}

private func describe(_ error: Error) -> String {
    (error as? BridgeError)?.localizedDescription ?? "unexpected failure"
}

@main
struct OceanIMessageMain {
    static func main() async {
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            guard let raw = arguments.first, let command = Command(rawValue: raw) else {
                throw Usage.error
            }
            let paths = parsePaths(arguments: Array(arguments.dropFirst()))
            let store = StateStore(url: paths.state)
            switch command {
            case .accounts:
                for accountID in try MessagesAccounts.enabledIDs() {
                    print(accountID)
                }
            case .check:
                guard try await check(database: paths.database, config: loadConfig(paths.config)) else {
                    Foundation.exit(1)
                }
            case .poll:
                try poll(database: paths.database, store: store, roster: loadConfig(paths.config).roster)
            case .prime:
                try store.advanceCursor(to: try MessagesDatabase(path: paths.database).latestRowID())
            case .watch:
                let roster = try loadConfig(paths.config).roster
                while true {
                    try poll(database: paths.database, store: store, roster: roster)
                    try await Task.sleep(for: .seconds(2))
                }
            case .send:
                let positional = positionalArguments(arguments.dropFirst())
                guard positional.count == 2 else { throw Usage.error }
                let config = try loadConfig(paths.config)
                let bubbles = splitReply(positional[1])
                guard bubbles.isEmpty == false else { throw BridgeError.unsafeReply }
                let member = try store.claimReply(for: positional[0]) { repliesPerHour(for: $0, in: config.roster) }
                try RosterSender(accountID: config.messagesAccountID, roster: config.roster)
                    .send(bubbles, toMember: member)
            case .run:
                let config = try loadConfig(paths.config)
                let sink: ReplySink
                if arguments.contains("--print-replies") {
                    sink = .standardOutput
                } else {
                    guard config.messagesAccountID != nil else { throw BridgeError.invalidState }
                    sink = .messages(RosterSender(accountID: config.messagesAccountID, roster: config.roster))
                }
                try await run(database: paths.database, store: store, config: config, sink: sink)
            }
        } catch {
            // Errors intentionally carry no iMessage content, database SQL, or
            // recipient.
            log(error.localizedDescription)
            Foundation.exit(1)
        }
    }

    private static func repliesPerHour(for memberID: String, in roster: Roster) -> Int? {
        roster.member(id: memberID).flatMap { roster.policy(for: $0) }?.repliesPerHour
    }

    /// Operator preflight. Prints only counts, role names, agent names, and
    /// pass/fail; never a phone number or message.
    private static func check(database: String, config: BridgeConfig) async throws -> Bool {
        let client = OceanDaemonClient(config: config)
        var healthy = true
        print("roster: \(config.roster.members.count) members, \(config.roster.roles.count) roles")
        print("sender account: \(config.messagesAccountID == nil ? "MISSING" : "enrolled")")
        if config.messagesAccountID == nil { healthy = false }

        if await client.daemonIsHealthy() {
            print("daemon: ok")
        } else {
            print("daemon: UNREACHABLE")
            healthy = false
        }
        for role in config.roster.roles.keys.sorted() {
            guard let policy = config.roster.roles[role] else { continue }
            if await client.agentIsUsable(policy.agent) {
                print("role \(role): agent \(policy.agent) ok")
            } else {
                print("role \(role): agent \(policy.agent) MISSING OR HAS NO TOOL ALLOWLIST")
                healthy = false
            }
        }
        do {
            _ = try MessagesDatabase(path: database).scan(after: Int64.max - 1, roster: config.roster)
            print("messages database: readable")
        } catch {
            print("messages database: \(describe(error))")
            healthy = false
        }
        return healthy
    }

    private static func poll(database: String, store: StateStore, roster: Roster) throws {
        let state = try store.load()
        let scan = try MessagesDatabase(path: database).scan(after: state.cursor, roster: roster)
        for (rowID, message) in scan.admitted {
            // Commit the opaque id before emitting text. A crash can cause at-most
            // one lost message, never duplicate auto-replies or a rejected row leak.
            guard try store.recordAdmission(message, cursor: rowID) else { continue }
            let line = try JSONEncoder().encode(message)
            FileHandle.standardOutput.write(line)
            FileHandle.standardOutput.write(Data([0x0A]))
        }
        try store.advanceCursor(to: scan.cursor)
    }

    private static func run(
        database: String,
        store: StateStore,
        config: BridgeConfig,
        sink: ReplySink
    ) async throws {
        let client = OceanDaemonClient(config: config)
        let queues = MemberQueues()
        let reader = MessagesDatabase(path: database)
        try store.retainMembers(Set(config.roster.members.map(\.id)))

        // A reader that cannot open the database stays alive and retries
        // instead of exiting, so a revoked permission does not become a
        // supervisor crash loop. It logs once per state change.
        var readerHealthy = true
        while true {
            do {
                let state = try store.load()
                let scan = try reader.scan(after: state.cursor, roster: config.roster)
                if readerHealthy == false {
                    log("Messages reader recovered")
                    readerHealthy = true
                }
                for (rowID, message) in scan.admitted {
                    guard try store.recordAdmission(message, cursor: rowID) else { continue }
                    await queues.enqueue(member: message.member) {
                        await handle(message, store: store, config: config, client: client, sink: sink)
                    }
                }
                try store.advanceCursor(to: scan.cursor)
                try await Task.sleep(for: .seconds(2))
            } catch let error as BridgeError where error == .databaseUnavailable || error == .unsafeDatabaseSchema {
                if readerHealthy {
                    log("\(describe(error)); admitting nothing, retrying every 60s")
                    readerHealthy = false
                }
                try await Task.sleep(for: .seconds(60))
            }
        }
    }

    /// One admitted message, start to finish. Failures are contained here: a
    /// bad turn for one member never stops the bridge for the others.
    private static func handle(
        _ message: AllowedMessage,
        store: StateStore,
        config: BridgeConfig,
        client: OceanDaemonClient,
        sink: ReplySink
    ) async {
        guard let member = config.roster.member(id: message.member),
              let policy = config.roster.policy(for: member)
        else { return }

        // Claim the single reply before any model work, so a member over their
        // hourly limit costs nothing and a duplicate watcher cannot double-run.
        do {
            _ = try store.claimReply(for: message.id) { repliesPerHour(for: $0, in: config.roster) }
        } catch {
            log("member \(member.id): \(describe(error))")
            return
        }

        if isResetCommand(message.text) {
            try? store.clearSession(for: member.id)
            await deliver([BridgeText.reset], to: member, sink: sink)
            return
        }

        let acknowledgement = Task<Void, Never> {
            guard let delay = config.acknowledgeAfter,
                  (try? await Task.sleep(for: delay)) != nil,
                  Task.isCancelled == false
            else { return }
            await deliver([BridgeText.acknowledgement], to: member, sink: sink)
        }

        let bubbles: [String]
        do {
            let reply = try await client.reply(to: message, member: member, policy: policy, store: store)
            bubbles = splitReply(reply)
            guard bubbles.isEmpty == false else { throw BridgeError.unsafeReply }
        } catch {
            acknowledgement.cancel()
            await acknowledgement.value
            log("member \(member.id) role \(member.role): \(describe(error))")
            await deliver([BridgeText.failure], to: member, sink: sink)
            return
        }
        // Let an in-flight acknowledgement land first so bubbles stay ordered.
        acknowledgement.cancel()
        await acknowledgement.value
        await deliver(bubbles, to: member, sink: sink)
    }

    /// AppleScript delivery blocks, so it runs off the cooperative pool.
    private static func deliver(_ bubbles: [String], to member: RosterMember, sink: ReplySink) async {
        let sender: RosterSender
        switch sink {
        case let .messages(configured):
            sender = configured
        case .standardOutput:
            if let line = try? JSONEncoder().encode(PrintedReply(member: member.id, bubbles: bubbles)) {
                FileHandle.standardOutput.write(line + Data([0x0A]))
            }
            return
        }
        let result: Result<Void, Error> = await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                continuation.resume(returning: Result { try sender.send(bubbles, toMember: member.id) })
            }
        }
        if case let .failure(error) = result {
            log("member \(member.id): \(describe(error))")
        }
    }

    private static func parsePaths(arguments: [String]) -> (database: String, state: URL, config: URL) {
        func value(_ flag: String, default fallback: String) -> String {
            guard let index = arguments.firstIndex(of: flag), arguments.indices.contains(index + 1) else { return fallback }
            return arguments[index + 1]
        }
        let home = FileManager.default.homeDirectoryForCurrentUser
        return (
            value("--database", default: home.appending(path: "Library/Messages/chat.db").path),
            URL(fileURLWithPath: value("--state", default: home.appending(path: "Library/Application Support/Ocean/iMessage/state.json").path)),
            URL(fileURLWithPath: value("--config", default: home.appending(path: "Library/Application Support/Ocean/iMessage/config.json").path))
        )
    }

    private static func positionalArguments(_ arguments: ArraySlice<String>) -> [String] {
        var values: [String] = []
        var skipNext = false
        for argument in arguments {
            if skipNext { skipNext = false; continue }
            if argument == "--database" || argument == "--state" || argument == "--config" { skipNext = true; continue }
            values.append(argument)
        }
        return values
    }
}

private enum Usage: Error, LocalizedError {
    case error
    var errorDescription: String? {
        "usage: ocean-imessage <accounts|check|poll|prime|watch|run> [--database PATH] [--state PATH] [--config PATH], ocean-imessage run --print-replies, or ocean-imessage send <accepted-message-id> <text>"
    }
}
