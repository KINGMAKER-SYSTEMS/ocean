import Foundation

/// Validated operator configuration. Built only by `parseConfig`.
struct BridgeConfig: Sendable {
    let daemonURL: URL
    let cwd: String
    let messagesAccountID: String?
    let roster: Roster
    let turnTimeout: Duration
    /// Send a short "working on it" bubble when a turn runs past this. `nil`
    /// disables the acknowledgement.
    let acknowledgeAfter: Duration?
}

private struct RawConfig: Decodable {
    struct Role: Decodable {
        let agent: String
        let repliesPerHour: Int?
    }

    struct Member: Decodable {
        let id: String
        let phone: String
        let role: String
        let name: String?
    }

    let daemonURL: URL
    let cwd: String
    let messagesAccountID: String?
    let localIdentity: String
    let roles: [String: Role]
    let members: [Member]
    let turnTimeoutSeconds: Int?
    let acknowledgeAfterSeconds: Int?
}

/// Message bodies may be delivered only to the local Ocean daemon. A private
/// mode-0600 config file is not authority to exfiltrate them to another host.
func isAllowedOceanDaemonURL(_ url: URL) -> Bool {
    guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
        return false
    }
    return components.scheme?.lowercased() == "http"
        && components.host == "127.0.0.1"
        && components.port == 4780
        && (components.path.isEmpty || components.path == "/")
        && components.user == nil
        && components.password == nil
        && components.query == nil
        && components.fragment == nil
}

/// Decode and validate configuration. Any unknown shape, out-of-range bound,
/// or roster anomaly fails closed; nothing here reads message content.
func parseConfig(_ data: Data) throws -> BridgeConfig {
    guard let raw = try? JSONDecoder().decode(RawConfig.self, from: data) else {
        throw BridgeError.invalidState
    }
    guard isAllowedOceanDaemonURL(raw.daemonURL),
          (raw.cwd as NSString).isAbsolutePath
    else { throw BridgeError.invalidState }

    let timeoutSeconds = raw.turnTimeoutSeconds ?? 600
    guard (30...1_800).contains(timeoutSeconds) else { throw BridgeError.invalidState }

    let acknowledgeSeconds = raw.acknowledgeAfterSeconds ?? 25
    guard acknowledgeSeconds == 0 || (5...300).contains(acknowledgeSeconds) else {
        throw BridgeError.invalidState
    }

    let roles = raw.roles.mapValues {
        RolePolicy(agent: $0.agent, repliesPerHour: $0.repliesPerHour ?? BridgeLimits.defaultRepliesPerHour)
    }
    let members = raw.members.map {
        RosterMember(id: $0.id, phone: $0.phone, role: $0.role, name: $0.name)
    }
    let roster = try Roster(localIdentity: raw.localIdentity, members: members, roles: roles)

    return BridgeConfig(
        daemonURL: raw.daemonURL,
        cwd: raw.cwd,
        messagesAccountID: raw.messagesAccountID?.isEmpty == false ? raw.messagesAccountID : nil,
        roster: roster,
        turnTimeout: .seconds(timeoutSeconds),
        acknowledgeAfter: acknowledgeSeconds == 0 ? nil : .seconds(acknowledgeSeconds)
    )
}

/// The roster is the permission boundary, so the file that holds it must be
/// private to the operator account before it is trusted.
func loadConfig(_ url: URL) throws -> BridgeConfig {
    guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path),
          let permissions = (attributes[.posixPermissions] as? NSNumber)?.intValue,
          permissions & 0o077 == 0,
          let data = try? Data(contentsOf: url)
    else { throw BridgeError.invalidState }
    return try parseConfig(data)
}

/// The roster on disk is the authority. The run loop re-reads it whenever the
/// file changes, so removing or demoting a member takes effect on the next poll
/// without a restart. A missing, non-private, or invalid file means there is no
/// roster: the bridge admits nothing and sends nothing until it is valid again.
final class LiveConfig: @unchecked Sendable {
    private struct Stamp: Equatable {
        let modified: Date?
        let size: Int?
        let inode: Int?
        let permissions: Int?
    }

    private let lock = NSLock()
    private let url: URL
    private var stamp: Stamp?
    private var value: BridgeConfig?
    private var generation = 0

    init(url: URL) {
        self.url = url
    }

    /// The config as of now, and a counter that rises each time the file was
    /// re-read, whichever caller noticed the change.
    @discardableResult
    func refresh() -> (config: BridgeConfig?, generation: Int) {
        lock.lock()
        defer { lock.unlock() }
        let attributes = try? FileManager.default.attributesOfItem(atPath: url.path)
        let observed = Stamp(
            modified: attributes?[.modificationDate] as? Date,
            size: (attributes?[.size] as? NSNumber)?.intValue,
            inode: (attributes?[.systemFileNumber] as? NSNumber)?.intValue,
            permissions: (attributes?[.posixPermissions] as? NSNumber)?.intValue
        )
        if let stamp, stamp == observed { return (value, generation) }
        stamp = observed
        value = try? loadConfig(url)
        generation += 1
        return (value, generation)
    }
}
