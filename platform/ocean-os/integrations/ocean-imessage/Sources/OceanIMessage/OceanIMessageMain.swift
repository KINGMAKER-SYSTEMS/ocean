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
    case messages
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
/// Every line is timestamped so the tail of the service log can be trusted.
private func log(_ line: String) {
    let stamp = ISO8601DateFormatter().string(from: Date())
    FileHandle.standardError.write(Data("\(stamp) ocean-imessage: \(line)\n".utf8))
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
                let live = LiveConfig(url: paths.config)
                while true {
                    guard let roster = live.refresh().config?.roster else { throw BridgeError.invalidState }
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
                let sink: ReplySink
                if arguments.contains("--print-replies") {
                    // A rehearsal must never write its cursor and sessions into
                    // the live ledger, so it has to name its own state file.
                    guard paths.state.standardizedFileURL != defaultStateURL().standardizedFileURL else {
                        throw Usage.error
                    }
                    sink = .standardOutput
                } else {
                    sink = .messages
                }
                try await run(
                    database: paths.database,
                    store: store,
                    stateURL: paths.state,
                    live: LiveConfig(url: paths.config),
                    sink: sink
                )
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
    /// pass/fail; never a phone number or message. Run from a terminal it
    /// exercises the terminal's privacy grants, not the installed app's: the
    /// service log is the authority on whether the app itself can read Messages.
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
            print("messages database: readable by this process")
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
            do {
                guard try store.recordAdmission(message, cursor: rowID) else { continue }
            } catch BridgeError.backlogFull {
                // In this inspection mode nothing closes an admission except
                // `send` or the next `run`, so say when a member hits the cap.
                log("member \(message.member): \(describe(BridgeError.backlogFull))")
                continue
            }
            let line = try JSONEncoder().encode(message)
            FileHandle.standardOutput.write(line)
            FileHandle.standardOutput.write(Data([0x0A]))
        }
        try store.advanceCursor(to: scan.cursor)
    }

    private static func run(
        database: String,
        store: StateStore,
        stateURL: URL,
        live: LiveConfig,
        sink: ReplySink
    ) async throws {
        let queues = MemberQueues()
        let reader = MessagesDatabase(path: database)
        // Hold the ledger for the life of this process. Only then is it true
        // that messages still waiting were left by a runner that has exited,
        // with no turn left to claim them.
        let custody = try RunnerLock(stateURL: stateURL)
        defer { withExtendedLifetime(custody) {} }
        let orphans = try store.resolvePending()
        if orphans > 0 {
            log("closed \(orphans) messages a previous run admitted but never answered")
        }

        // Neither a missing roster nor an unreadable database ends the process:
        // under a supervisor that becomes a crash loop. The bridge stays up,
        // admits nothing, says so once, and recovers on its own.
        var rosterReady: Bool?
        var seenGeneration = 0
        var readerHealthy = true
        var readerEverWorked = false
        var readerFailures = 0
        while true {
            let (loaded, generation) = live.refresh()
            guard let config = loaded, sink == .standardOutput || config.messagesAccountID != nil else {
                if rosterReady != false {
                    log("configuration is missing, not private, or invalid; admitting nothing until it is fixed")
                    rosterReady = false
                }
                seenGeneration = generation
                try await Task.sleep(for: .seconds(5))
                continue
            }
            if generation != seenGeneration || rosterReady != true {
                log("roster loaded: \(config.roster.members.count) members, \(config.roster.roles.count) roles")
                try store.retainMembers(Set(config.roster.members.map(\.id)))
                seenGeneration = generation
                rosterReady = true
            }

            do {
                // A ledger that has never scanned (first run, or it was
                // removed) starts from now, not from the last 15 minutes and
                // never from the beginning of the account's history.
                let state = try store.primeIfUnscanned { try reader.latestRowID() }
                let scan = try reader.scan(after: state.cursor, roster: config.roster)
                if readerHealthy == false {
                    log("Messages reader recovered")
                    readerHealthy = true
                }
                readerEverWorked = true
                readerFailures = 0
                var refused = Set<String>()
                for (rowID, message) in scan.admitted {
                    do {
                        guard try store.recordAdmission(message, cursor: rowID) else { continue }
                    } catch BridgeError.backlogFull {
                        if refused.insert(message.member).inserted {
                            log("member \(message.member): \(describe(BridgeError.backlogFull))")
                        }
                        continue
                    }
                    let admittedAt = Date()
                    await queues.enqueue(member: message.member) {
                        await handle(message, admittedAt: admittedAt, store: store, live: live, sink: sink)
                    }
                }
                try store.advanceCursor(to: scan.cursor)
                try await Task.sleep(for: .seconds(2))
            } catch let error as BridgeError where error == .databaseUnavailable || error == .unsafeDatabaseSchema {
                if readerHealthy {
                    log("\(describe(error)); admitting nothing, retrying every 60s")
                    readerHealthy = false
                }
                readerFailures += 1
                // A newly granted Full Disk Access permission may reach only a
                // fresh process. If this process has never read Messages, leave
                // every five minutes so the supervisor starts one; this is a
                // slow, quiet retry, not a crash loop. A process that was
                // working keeps running so replies in flight are still delivered.
                if readerEverWorked == false, readerFailures >= 5 {
                    log("still no Messages access; exiting so a fresh process can pick up a permission change")
                    Foundation.exit(75)
                }
                try await Task.sleep(for: .seconds(60))
            }
        }
    }

    /// One admitted message, start to finish. Failures are contained here: a
    /// bad turn for one member never stops the bridge for the others.
    private static func handle(
        _ message: AllowedMessage,
        admittedAt: Date,
        store: StateStore,
        live: LiveConfig,
        sink: ReplySink
    ) async {
        guard let config = live.refresh().config else {
            log("member \(message.member): roster is missing or invalid; admitted message dropped")
            // Close it so it does not sit in the ledger as waiting forever.
            try? store.resolve(message.id)
            return
        }
        // The age rule holds when the turn comes, not only at admission: a
        // message that queued behind long turns is not acted on late.
        guard Date().timeIntervalSince(admittedAt) <= BridgeLimits.maximumMessageAge else {
            log("member \(message.member): message waited too long for its turn; dropped")
            try? store.resolve(message.id)
            return
        }

        // Claim the single reply before any model work, so a member over their
        // hourly limit costs nothing and a duplicate watcher cannot double-run.
        // The ledger, not the caller, says who the reply belongs to.
        let owner: String
        do {
            owner = try store.claimReply(for: message.id) { repliesPerHour(for: $0, in: config.roster) }
        } catch {
            log("member \(message.member): \(describe(error))")
            return
        }
        guard let member = config.roster.member(id: owner), let policy = config.roster.policy(for: member) else {
            return
        }

        if isResetCommand(message.text) {
            try? store.clearSession(for: member.id)
            await deliver([BridgeText.reset], toMember: owner, live: live, sink: sink)
            return
        }

        let acknowledgement = Task<Void, Never> {
            guard let delay = config.acknowledgeAfter,
                  (try? await Task.sleep(for: delay)) != nil,
                  Task.isCancelled == false
            else { return }
            await deliver([BridgeText.acknowledgement], toMember: owner, live: live, sink: sink)
        }

        let bubbles: [String]
        do {
            let reply = try await OceanDaemonClient(config: config)
                .reply(to: message, member: member, policy: policy, store: store)
            bubbles = splitReply(reply)
            guard bubbles.isEmpty == false else { throw BridgeError.unsafeReply }
        } catch {
            acknowledgement.cancel()
            await acknowledgement.value
            log("member \(member.id) role \(member.role): \(describe(error))")
            await deliver([BridgeText.failure], toMember: owner, live: live, sink: sink)
            return
        }
        // Let an in-flight acknowledgement land first so bubbles stay ordered.
        acknowledgement.cancel()
        await acknowledgement.value
        await deliver(bubbles, toMember: owner, live: live, sink: sink)
    }

    /// Delivery re-reads the roster, so a member removed while their turn was
    /// running receives nothing. AppleScript delivery blocks, so it runs off
    /// the cooperative pool.
    private static func deliver(_ bubbles: [String], toMember memberID: String, live: LiveConfig, sink: ReplySink) async {
        guard let config = live.refresh().config else {
            log("member \(memberID): roster is missing or invalid; reply dropped")
            return
        }
        guard config.roster.member(id: memberID) != nil else {
            log("member \(memberID): no longer enrolled; reply dropped")
            return
        }
        if sink == .standardOutput {
            if let line = try? JSONEncoder().encode(PrintedReply(member: memberID, bubbles: bubbles)) {
                FileHandle.standardOutput.write(line + Data([0x0A]))
            }
            return
        }
        let sender = RosterSender(accountID: config.messagesAccountID, roster: config.roster)
        let result: Result<Void, Error> = await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .utility).async {
                continuation.resume(returning: Result { try sender.send(bubbles, toMember: memberID) })
            }
        }
        if case let .failure(error) = result {
            log("member \(memberID): \(describe(error))")
        }
    }

    private static func defaultStateURL() -> URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appending(path: "Library/Application Support/Ocean/iMessage/state.json")
    }

    private static func parsePaths(arguments: [String]) -> (database: String, state: URL, config: URL) {
        func value(_ flag: String, default fallback: String) -> String {
            guard let index = arguments.firstIndex(of: flag), arguments.indices.contains(index + 1) else { return fallback }
            return arguments[index + 1]
        }
        let home = FileManager.default.homeDirectoryForCurrentUser
        return (
            value("--database", default: home.appending(path: "Library/Messages/chat.db").path),
            URL(fileURLWithPath: value("--state", default: defaultStateURL().path)),
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
        "usage: ocean-imessage <accounts|check|poll|prime|watch|run> [--database PATH] [--state PATH] [--config PATH], ocean-imessage run --print-replies --state PATH, or ocean-imessage send <accepted-message-id> <text>"
    }
}
