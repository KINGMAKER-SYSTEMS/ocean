import Foundation

/// Marks a reply the bridge cut because it exceeded the bubble budget.
let replyTruncationMarker = "\n[cut: the rest was too long for a text]"

/// Exact text a member sends to drop their ongoing session and start clean.
/// Handled by the bridge; it never reaches Ocean.
func isResetCommand(_ text: String) -> Bool {
    text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() == "/new"
}

/// Model output is cosmetic input to Messages: it travels as a positional
/// AppleScript argument, so it cannot change the script. Normalize whitespace
/// and drop control characters instead of failing the whole reply over a tab.
public func sanitizeReply(_ text: String) -> String {
    var scalars = String.UnicodeScalarView()
    for scalar in text.unicodeScalars {
        switch scalar.value {
        case 0x0A: scalars.append(scalar)
        case 0x09: scalars.append(" ")
        case 0x00...0x1F, 0x7F...0x9F: continue
        default: scalars.append(scalar)
        }
    }
    return String(scalars).trimmingCharacters(in: .whitespacesAndNewlines)
}

/// Split a reply into text-message bubbles, preferring paragraph, then line,
/// then word boundaries. Returns an empty array for an empty reply. A reply
/// that needs more than `maximumBubbles` is cut and says so.
public func splitReply(
    _ raw: String,
    bubbleLimit: Int = BridgeLimits.maximumBubbleCharacters,
    maximumBubbles: Int = BridgeLimits.maximumBubbles
) -> [String] {
    let clean = sanitizeReply(raw)
    guard clean.isEmpty == false, bubbleLimit > replyTruncationMarker.count, maximumBubbles > 0 else { return [] }
    var bubbles = chunk(clean, limit: bubbleLimit, separators: ["\n\n", "\n", " "])
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { $0.isEmpty == false }
    if bubbles.count > maximumBubbles {
        bubbles = Array(bubbles.prefix(maximumBubbles))
        let last = bubbles[maximumBubbles - 1]
        bubbles[maximumBubbles - 1] = String(last.prefix(bubbleLimit - replyTruncationMarker.count))
            .trimmingCharacters(in: .whitespacesAndNewlines) + replyTruncationMarker
    }
    return bubbles
}

/// Greedy packer: split on the coarsest separator present, pack units up to
/// `limit`, and recurse with finer separators for any unit still too long.
private func chunk(_ text: String, limit: Int, separators: ArraySlice<String>) -> [String] {
    if text.count <= limit { return [text] }
    var remaining = separators
    while let separator = remaining.popFirst() {
        let units = text.components(separatedBy: separator)
        guard units.count > 1 else { continue }
        var packed: [String] = []
        var current = ""
        for unit in units {
            for part in chunk(unit, limit: limit, separators: remaining) {
                if current.isEmpty {
                    current = part
                } else if current.count + separator.count + part.count <= limit {
                    current += separator + part
                } else {
                    packed.append(current)
                    current = part
                }
            }
        }
        if current.isEmpty == false { packed.append(current) }
        return packed
    }
    // No boundary left: hard split by character.
    var pieces: [String] = []
    var rest = Substring(text)
    while rest.isEmpty == false {
        pieces.append(String(rest.prefix(limit)))
        rest = rest.dropFirst(limit)
    }
    return pieces
}
