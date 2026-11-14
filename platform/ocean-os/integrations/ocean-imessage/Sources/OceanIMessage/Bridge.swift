import Darwin
import Foundation
import SQLite3

private let sqliteTransient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

/// Hard bounds that no operator configuration can widen.
public enum BridgeLimits {
    public static let maximumInboundCharacters = 4_096
    public static let maximumBubbleCharacters = 1_000
    public static let maximumBubbles = 8
    public static let maximumMembers = 64
    public static let defaultRepliesPerHour = 30
    public static let maximumRepliesPerHour = 120
    public static let retainedAdmissions = 256
}

// MARK: - Roster

/// One teammate the operator enrolled. A member is identified to Ocean by
/// `id`; the phone number never leaves the bridge.
public struct RosterMember: Equatable, Sendable {
    public let id: String
    public let phone: String
    public let role: String
    public let name: String?
}

/// What a role may do. The agent is a daemon folder-as-agent name; its tool
/// allowlist is the permission boundary for every member holding the role.
public struct RolePolicy: Equatable, Sendable {
    public let agent: String
    public let repliesPerHour: Int
}

/// The operator-owned allowlist: the agent's own iMessage identity, the
/// teammates who may text it, and the role each one holds. Construction is the
/// only validation point and every anomaly fails closed.
public struct Roster: Equatable, Sendable {
    public let localIdentity: String
    public let members: [RosterMember]
    public let roles: [String: RolePolicy]

    public init(localIdentity: String, members: [RosterMember], roles: [String: RolePolicy]) throws {
        guard let local = normalizeE164(localIdentity), local == localIdentity,
              (1...BridgeLimits.maximumMembers).contains(members.count),
              roles.isEmpty == false
        else { throw BridgeError.invalidRoster }

        for (role, policy) in roles {
            guard isSlug(role, maximumLength: 32), isSlug(policy.agent, maximumLength: 64),
                  (1...BridgeLimits.maximumRepliesPerHour).contains(policy.repliesPerHour)
            else { throw BridgeError.invalidRoster }
        }

        var ids = Set<String>()
        var phones = Set<String>()
        for member in members {
            guard isSlug(member.id, maximumLength: 32),
                  let phone = normalizeE164(member.phone), phone == member.phone,
                  phone != local,
                  roles[member.role] != nil,
                  ids.insert(member.id).inserted,
                  phones.insert(phone).inserted
            else { throw BridgeError.invalidRoster }
            if let name = member.name {
                guard name.isEmpty == false, name.count <= 64,
                      name.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x7F })
                else { throw BridgeError.invalidRoster }
            }
        }

        self.localIdentity = local
        self.members = members
        self.roles = roles
    }

    public func member(forPhone phone: String) -> RosterMember? {
        guard let normalized = normalizeE164(phone) else { return nil }
        return members.first { $0.phone == normalized }
    }

    public func member(id: String) -> RosterMember? {
        members.first { $0.id == id }
    }

    /// Every member's role is proven present at construction.
    public func policy(for member: RosterMember) -> RolePolicy? {
        roles[member.role]
    }
}

/// Lowercase ASCII identifier: `[a-z0-9][a-z0-9_-]*`. Used for member ids, role
/// names, and agent names so none of them can carry separators, whitespace, or
/// path components into a prompt, a URL path, or a log line.
func isSlug(_ value: String, maximumLength: Int) -> Bool {
    let scalars = Array(value.unicodeScalars)
    guard (1...maximumLength).contains(scalars.count), let first = scalars.first else { return false }
    func isLowerAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
        (scalar.value >= 0x61 && scalar.value <= 0x7A) || (scalar.value >= 0x30 && scalar.value <= 0x39)
    }
    guard isLowerAlphanumeric(first) else { return false }
    return scalars.allSatisfy { isLowerAlphanumeric($0) || $0 == "-" || $0 == "_" }
}

/// Strict E.164: a leading `+` followed by 8–15 ASCII digits. Unicode digit
/// look-alikes are rejected so an identifier can never compare unequal to the
/// handle Messages reports while still looking like the enrolled number.
public func normalizeE164(_ value: String) -> String? {
    let scalars = value.unicodeScalars
    guard scalars.first == "+" else { return nil }
    let digits = scalars.dropFirst()
    guard (8...15).contains(digits.count),
          digits.allSatisfy({ $0.value >= 0x30 && $0.value <= 0x39 })
    else { return nil }
    return "+" + String(String.UnicodeScalarView(digits))
}

// MARK: - Admission

public struct IncomingMessage: Equatable {
    public let id: String
    public let remote: String
    public let local: String
    public let service: String
    public let text: String
    public let participantCount: Int
    public let isFromMe: Bool
    public let hasAttachment: Bool
    public let isReaction: Bool

    public init(
        id: String,
        remote: String,
        local: String,
        service: String,
        text: String,
        participantCount: Int,
        isFromMe: Bool,
        hasAttachment: Bool,
        isReaction: Bool
    ) {
        self.id = id
        self.remote = remote
        self.local = local
        self.service = service
        self.text = text
        self.participantCount = participantCount
        self.isFromMe = isFromMe
        self.hasAttachment = hasAttachment
        self.isReaction = isReaction
    }
}

public enum Admission: Equatable {
    case accepted(AllowedMessage)
    case rejected
}

/// An admitted message carries the roster member id, never the phone number.
public struct AllowedMessage: Equatable, Codable, Sendable {
    public let id: String
    public let member: String
    public let text: String

    public init(id: String, member: String, text: String) {
        self.id = id
        self.member = member
        self.text = text
    }
}

/// This is deliberately the only content admission point. Every failure is
/// indistinguishable to callers so rejected rows are not reflected in logs,
/// output, or daemon prompts.
public func admit(_ message: IncomingMessage, roster: Roster) -> Admission {
    guard message.id.isEmpty == false,
          message.isFromMe == false,
          message.service == "iMessage",
          message.participantCount == 1,
          message.hasAttachment == false,
          message.isReaction == false,
          normalizeE164(message.local) == roster.localIdentity,
          let member = roster.member(forPhone: message.remote),
          message.text.isEmpty == false,
          message.text.count <= BridgeLimits.maximumInboundCharacters
    else {
        return .rejected
    }
    return .accepted(AllowedMessage(id: message.id, member: member.id, text: message.text))
}

public enum BridgeError: LocalizedError, Equatable {
    case unsafeDatabaseSchema
    case databaseUnavailable
    case invalidState
    case invalidRoster
    case unknownMessage
    case alreadyReplied
    case rateLimited
    case unsafeReply
    case agentUnavailable
    case daemonFailure
    case daemonTimeout
    case senderFailure

    public var errorDescription: String? {
        // Never include database, prompt, recipient, or Apple-event details.
        switch self {
        case .unsafeDatabaseSchema: return "Messages database is not a supported safe shape"
        case .databaseUnavailable: return "Messages database is unavailable"
        case .invalidState: return "iMessage bridge state or configuration is invalid"
        case .invalidRoster: return "iMessage bridge roster is invalid"
        case .unknownMessage: return "message is not admitted"
        case .alreadyReplied: return "message already has a reply"
        case .rateLimited: return "member reply rate limit reached"
        case .unsafeReply: return "reply violates bridge policy"
        case .agentUnavailable: return "role agent is missing or has no tool allowlist"
        case .daemonFailure: return "Ocean did not accept the message"
        case .daemonTimeout: return "Ocean did not finish the message in time"
        case .senderFailure: return "Messages did not accept the reply"
        }
    }
}

// MARK: - State

/// A member's ongoing daemon session, bound to the agent that owns it. A role
/// change that selects a different agent must never inherit the old session.
public struct SessionBinding: Codable, Equatable, Sendable {
    public let agent: String
    public let session: String

    public init(agent: String, session: String) {
        self.agent = agent
        self.session = session
    }
}

public struct BridgeState: Codable, Equatable, Sendable {
    public var cursor: Int64
    public var admitted: [String]
    public var replied: [String]
    /// Admitted message id → roster member id. A reply is deliverable only to
    /// the member whose message was admitted.
    public var owners: [String: String]
    public var replyTimes: [String: [Date]]
    public var sessions: [String: SessionBinding]

    public static let empty = BridgeState(
        cursor: 0, admitted: [], replied: [], owners: [:], replyTimes: [:], sessions: [:]
    )

    public init(
        cursor: Int64,
        admitted: [String],
        replied: [String],
        owners: [String: String],
        replyTimes: [String: [Date]],
        sessions: [String: SessionBinding]
    ) {
        self.cursor = cursor
        self.admitted = admitted
        self.replied = replied
        self.owners = owners
        self.replyTimes = replyTimes
        self.sessions = sessions
    }

    private enum CodingKeys: String, CodingKey {
        case cursor, admitted, replied, owners, replyTimes, sessions
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        cursor = try container.decode(Int64.self, forKey: .cursor)
        admitted = try container.decode([String].self, forKey: .admitted)
        replied = try container.decode([String].self, forKey: .replied)
        owners = try container.decodeIfPresent([String: String].self, forKey: .owners) ?? [:]
        // The fixed-pair slice stored one unowned list of reply times. It names
        // no member, so it cannot seed a per-member window and is dropped.
        replyTimes = (try? container.decode([String: [Date]].self, forKey: .replyTimes)) ?? [:]
        sessions = try container.decodeIfPresent([String: SessionBinding].self, forKey: .sessions) ?? [:]
    }
}

/// State contains opaque database IDs, roster member ids, daemon session ids,
/// and timestamps only. Atomic replacement avoids a partial cursor advance
/// after a crash; the file is private to the local account and never contains
/// message text or phone numbers.
public final class StateStore: @unchecked Sendable {
    /// `fcntl` record locks exclude other processes, not other threads of this
    /// one, and closing any descriptor for the lock file releases them. A
    /// process-wide lock makes concurrent member turns safe inside one bridge.
    private static let processLock = NSLock()

    private let url: URL
    private let fileManager: FileManager

    public init(url: URL, fileManager: FileManager = .default) {
        self.url = url
        self.fileManager = fileManager
    }

    public func load() throws -> BridgeState {
        try withExclusiveLock { try loadUnlocked() }
    }

    public func save(_ state: BridgeState) throws {
        try withExclusiveLock { try saveUnlocked(state) }
    }

    /// Atomically admit an opaque message id for its member. Returns false when
    /// another watcher already admitted it, so duplicate watchers cannot submit
    /// the same allowed body to Ocean twice.
    @discardableResult
    public func recordAdmission(_ message: AllowedMessage, cursor: Int64) throws -> Bool {
        try withExclusiveLock {
            var state = try loadUnlocked()
            guard cursor > state.cursor,
                  state.admitted.contains(message.id) == false
            else {
                return false
            }
            state.cursor = cursor
            state.admitted = Array((state.admitted + [message.id]).suffix(BridgeLimits.retainedAdmissions))
            state.owners[message.id] = message.member
            prune(&state)
            try saveUnlocked(state)
            return true
        }
    }

    public func advanceCursor(to cursor: Int64) throws {
        try withExclusiveLock {
            var state = try loadUnlocked()
            state.cursor = max(state.cursor, cursor)
            try saveUnlocked(state)
        }
    }

    /// Claim the single reply for an admitted message and return the member it
    /// belongs to. `repliesPerHour` resolves the owner's current limit from the
    /// roster; returning nil (the member was removed) fails closed.
    @discardableResult
    public func claimReply(
        for messageID: String,
        now: Date = Date(),
        repliesPerHour: (String) -> Int?
    ) throws -> String {
        try withExclusiveLock {
            var state = try loadUnlocked()
            guard state.admitted.contains(messageID), let member = state.owners[messageID],
                  let limit = repliesPerHour(member)
            else { throw BridgeError.unknownMessage }
            guard state.replied.contains(messageID) == false else { throw BridgeError.alreadyReplied }
            let cutoff = now.addingTimeInterval(-3600)
            var times = (state.replyTimes[member] ?? []).filter { $0 >= cutoff }
            guard times.count < min(limit, BridgeLimits.maximumRepliesPerHour) else {
                throw BridgeError.rateLimited
            }
            times.append(now)
            state.replyTimes[member] = times
            state.replied.append(messageID)
            prune(&state)
            try saveUnlocked(state)
            return member
        }
    }

    public func session(for member: String, agent: String) throws -> String? {
        try withExclusiveLock {
            guard let binding = try loadUnlocked().sessions[member], binding.agent == agent else { return nil }
            return binding.session
        }
    }

    public func setSession(_ session: String, for member: String, agent: String) throws {
        try withExclusiveLock {
            var state = try loadUnlocked()
            state.sessions[member] = SessionBinding(agent: agent, session: session)
            try saveUnlocked(state)
        }
    }

    public func clearSession(for member: String) throws {
        try withExclusiveLock {
            var state = try loadUnlocked()
            guard state.sessions.removeValue(forKey: member) != nil else { return }
            try saveUnlocked(state)
        }
    }

    /// Forget sessions and rate windows for anyone no longer on the roster.
    public func retainMembers(_ members: Set<String>) throws {
        try withExclusiveLock {
            var state = try loadUnlocked()
            state.sessions = state.sessions.filter { members.contains($0.key) }
            state.replyTimes = state.replyTimes.filter { members.contains($0.key) }
            try saveUnlocked(state)
        }
    }

    /// Reply markers and owner bindings live exactly as long as their retained
    /// admission; never evict one independently while its message is claimable.
    private func prune(_ state: inout BridgeState) {
        let retained = Set(state.admitted)
        state.replied = state.replied.filter { retained.contains($0) }
        state.owners = state.owners.filter { retained.contains($0.key) }
    }

    private func loadUnlocked() throws -> BridgeState {
        guard fileManager.fileExists(atPath: url.path) else { return .empty }
        do {
            return try JSONDecoder().decode(BridgeState.self, from: Data(contentsOf: url))
        } catch {
            throw BridgeError.invalidState
        }
    }

    private func saveUnlocked(_ state: BridgeState) throws {
        let directory = url.deletingLastPathComponent()
        let data = try JSONEncoder().encode(state)
        let temporary = directory.appendingPathComponent(".state-\(UUID().uuidString)")
        try data.write(to: temporary, options: [.atomic])
        try fileManager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: temporary.path)
        if fileManager.fileExists(atPath: url.path) {
            _ = try fileManager.replaceItemAt(
                url,
                withItemAt: temporary,
                backupItemName: nil,
                options: []
            )
        } else {
            try fileManager.moveItem(at: temporary, to: url)
        }
    }

    private func withExclusiveLock<T>(_ body: () throws -> T) throws -> T {
        Self.processLock.lock()
        defer { Self.processLock.unlock() }
        let directory = url.deletingLastPathComponent()
        try fileManager.createDirectory(
            at: directory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        let lockURL = directory.appendingPathComponent(".state.lock")
        let descriptor = Darwin.open(lockURL.path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw BridgeError.invalidState }
        defer { Darwin.close(descriptor) }
        var lock = flock()
        lock.l_type = Int16(F_WRLCK)
        lock.l_whence = Int16(SEEK_SET)
        guard Darwin.fcntl(descriptor, F_SETLKW, &lock) != -1 else {
            throw BridgeError.invalidState
        }
        defer {
            lock.l_type = Int16(F_UNLCK)
            _ = Darwin.fcntl(descriptor, F_SETLK, &lock)
        }
        return try body()
    }
}

// MARK: - Messages reader

public struct MessageScan {
    public let cursor: Int64
    public let admitted: [(rowID: Int64, message: AllowedMessage)]
}

public struct MessagesDatabase: Sendable {
    private let path: String

    public init(path: String = ("~/Library/Messages/chat.db" as NSString).expandingTildeInPath) {
        self.path = path
    }

    public func latestRowID() throws -> Int64 {
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK, let database else {
            sqlite3_close(database)
            throw BridgeError.databaseUnavailable
        }
        defer { sqlite3_close(database) }
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "SELECT COALESCE(MAX(ROWID), 0) FROM message", -1, &statement, nil) == SQLITE_OK, let statement else {
            throw BridgeError.unsafeDatabaseSchema
        }
        defer { sqlite3_finalize(statement) }
        guard sqlite3_step(statement) == SQLITE_ROW else { throw BridgeError.unsafeDatabaseSchema }
        return sqlite3_column_int64(statement, 0)
    }

    public func scan(after cursor: Int64, roster: Roster) throws -> MessageScan {
        var database: OpaquePointer?
        guard sqlite3_open_v2(path, &database, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX, nil) == SQLITE_OK, let database else {
            sqlite3_close(database)
            throw BridgeError.databaseUnavailable
        }
        defer { sqlite3_close(database) }

        let requiredMessageColumns = [
            "ROWID", "guid", "text", "handle_id", "is_from_me", "service",
            "destination_caller_id", "cache_has_attachments", "associated_message_guid",
            "associated_message_type", "item_type"
        ]
        guard try hasColumns(requiredMessageColumns, table: "message", database: database),
              try hasColumns(["ROWID", "id"], table: "handle", database: database),
              try hasColumns(["chat_id", "message_id"], table: "chat_message_join", database: database),
              try hasColumns(["chat_id", "handle_id"], table: "chat_handle_join", database: database)
        else { throw BridgeError.unsafeDatabaseSchema }

        // Do not select attributedBody, attachments, account metadata, chat names,
        // or any row that fails the SQL pre-filter. The sender must be an
        // enrolled handle, the destination must be the agent's own identity, and
        // the conversation must have exactly that one participant. Swift repeats
        // the roster check before a body can leave this reader.
        let phones = roster.members.map(\.phone)
        let placeholders = Array(repeating: "?", count: phones.count).joined(separator: ",")
        let sql = """
        SELECT m.ROWID, m.guid, m.text, h.id, m.destination_caller_id, m.service,
               m.is_from_me, m.cache_has_attachments, m.associated_message_guid,
               m.associated_message_type, m.item_type,
               COUNT(DISTINCT participant.ROWID), GROUP_CONCAT(DISTINCT participant.id)
        FROM message AS m
        JOIN handle AS h ON h.ROWID = m.handle_id
        JOIN chat_message_join AS cmj ON cmj.message_id = m.ROWID
        JOIN chat_handle_join AS chj ON chj.chat_id = cmj.chat_id
        JOIN handle AS participant ON participant.ROWID = chj.handle_id
        WHERE m.ROWID > ?
          AND h.id IN (\(placeholders))
          AND m.destination_caller_id = ?
          AND m.is_from_me = 0
          AND m.service = 'iMessage'
          AND m.text IS NOT NULL
          AND m.cache_has_attachments = 0
          AND m.associated_message_guid IS NULL
          AND COALESCE(m.associated_message_type, 0) = 0
          AND COALESCE(m.item_type, 0) = 0
        GROUP BY m.ROWID
        HAVING COUNT(DISTINCT participant.ROWID) = 1
           AND GROUP_CONCAT(DISTINCT participant.id) = h.id
        ORDER BY m.ROWID ASC
        LIMIT 128
        """
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK, let statement else {
            throw BridgeError.unsafeDatabaseSchema
        }
        defer { sqlite3_finalize(statement) }
        var index: Int32 = 1
        sqlite3_bind_int64(statement, index, cursor)
        for phone in phones {
            index += 1
            sqlite3_bind_text(statement, index, phone, -1, sqliteTransient)
        }
        sqlite3_bind_text(statement, index + 1, roster.localIdentity, -1, sqliteTransient)

        var admitted: [(Int64, AllowedMessage)] = []
        var scannedCursor = cursor
        while sqlite3_step(statement) == SQLITE_ROW {
            let rowID = sqlite3_column_int64(statement, 0)
            scannedCursor = max(scannedCursor, rowID)
            guard let id = text(statement, 1), let body = text(statement, 2),
                  let remote = text(statement, 3), let local = text(statement, 4),
                  let service = text(statement, 5), let participants = text(statement, 12)
            else { continue }
            let candidate = IncomingMessage(
                id: id,
                remote: remote,
                local: local,
                service: service,
                text: body,
                participantCount: Int(sqlite3_column_int(statement, 11)),
                isFromMe: sqlite3_column_int(statement, 6) != 0,
                hasAttachment: sqlite3_column_int(statement, 7) != 0,
                isReaction: sqlite3_column_type(statement, 8) != SQLITE_NULL
                    || sqlite3_column_int(statement, 9) != 0
                    || sqlite3_column_int(statement, 10) != 0
            )
            // SQL prefilters the enrolled sender and one-to-one shape before
            // selecting text; Swift repeats both the participant and admission
            // checks before content can leave the reader boundary.
            guard participants == remote else { continue }
            if case let .accepted(message) = admit(candidate, roster: roster) {
                admitted.append((rowID, message))
            }
        }
        return MessageScan(cursor: scannedCursor, admitted: admitted)
    }

    private func hasColumns(_ required: [String], table: String, database: OpaquePointer) throws -> Bool {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(database, "PRAGMA table_info(\(table))", -1, &statement, nil) == SQLITE_OK, let statement else {
            throw BridgeError.unsafeDatabaseSchema
        }
        defer { sqlite3_finalize(statement) }
        var available = Set<String>()
        while sqlite3_step(statement) == SQLITE_ROW {
            if let name = text(statement, 1) { available.insert(name) }
        }
        return Set(required).isSubset(of: available)
    }

    private func text(_ statement: OpaquePointer, _ column: Int32) -> String? {
        guard let pointer = sqlite3_column_text(statement, column) else { return nil }
        return String(cString: pointer)
    }
}

// MARK: - Messages sender

/// Runs one AppleScript with positional arguments and returns its output.
public typealias AppleScriptRunner = @Sendable (_ source: String, _ arguments: [String]) throws -> String

func runMessagesAppleScript(_ source: String, arguments: [String] = []) throws -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
    process.arguments = ["-e", source] + arguments
    let output = Pipe()
    process.standardOutput = output
    // Never return AppleScript diagnostics: they can contain local account
    // metadata. Callers receive only the fixed BridgeError.
    process.standardError = Pipe()
    try process.run()
    let deadline = Date().addingTimeInterval(15)
    while process.isRunning && Date() < deadline {
        Thread.sleep(forTimeInterval: 0.05)
    }
    if process.isRunning {
        process.terminate()
        throw BridgeError.senderFailure
    }
    guard process.terminationStatus == 0 else { throw BridgeError.senderFailure }
    return String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
}

public enum MessagesAccounts {
    /// Returns enabled iMessage account identifiers only. This queries no chats,
    /// participants, or message content.
    public static func enabledIDs() throws -> [String] {
        let source = """
        tell application "Messages"
            get id of every account whose enabled is true and service type is iMessage
        end tell
        """
        return try runMessagesAppleScript(source)
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .split(separator: ",")
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }
}

/// Sends reply bubbles to an enrolled member. The caller names a roster member
/// id; the handle is read from the roster and passed to AppleScript as a
/// positional argument, never interpolated into script source. There is no way
/// to address a handle that is not on the roster.
public struct RosterSender: Sendable {
    private let configuredAccountID: String?
    private let roster: Roster
    private let bubbleDelay: TimeInterval
    private let runner: AppleScriptRunner

    /// Sending requires the operator-enrolled Messages account id for the
    /// agent's own identity. Omission always fails closed; account ordering or a
    /// sole enabled account is never treated as identity proof.
    public init(
        accountID: String?,
        roster: Roster,
        bubbleDelay: TimeInterval = 0.4,
        runner: AppleScriptRunner? = nil
    ) {
        self.configuredAccountID = accountID?.isEmpty == false ? accountID : nil
        self.roster = roster
        self.bubbleDelay = bubbleDelay
        self.runner = runner ?? { source, arguments in
            try runMessagesAppleScript(source, arguments: arguments)
        }
    }

    public func send(_ bubbles: [String], toMember memberID: String) throws {
        guard let accountID = configuredAccountID else { throw BridgeError.senderFailure }
        guard let member = roster.member(id: memberID) else { throw BridgeError.unknownMessage }
        guard (1...BridgeLimits.maximumBubbles).contains(bubbles.count),
              bubbles.allSatisfy({ bubble in
                  bubble.isEmpty == false
                      && bubble.count <= BridgeLimits.maximumBubbleCharacters
                      && bubble.unicodeScalars.allSatisfy { $0.value >= 0x20 || $0 == "\n" }
              }),
              accountID.count <= 200,
              accountID.unicodeScalars.allSatisfy({ $0.value >= 0x20 && $0.value != 0x22 })
        else { throw BridgeError.unsafeReply }

        let source = """
        on run argv
            set replyText to item 1 of argv
            set enrolledAccountID to item 2 of argv
            set enrolledHandle to item 3 of argv
            tell application "Messages"
                set targetAccount to first account whose id is enrolledAccountID and enabled is true and service type is iMessage
                set targetParticipant to participant enrolledHandle of targetAccount
                send replyText to targetParticipant
            end tell
        end run
        """
        for (index, bubble) in bubbles.enumerated() {
            if index > 0, bubbleDelay > 0 { Thread.sleep(forTimeInterval: bubbleDelay) }
            _ = try runner(source, [bubble, accountID, member.phone])
        }
    }
}
