import Foundation

/// Extract the plain string from a Messages `attributedBody` blob.
///
/// Current macOS stores most message text only in this column, as an
/// `NSAttributedString` in the legacy typedstream archive format, and leaves
/// `message.text` NULL. The bridge does not unarchive the blob: that would
/// instantiate classes named by the data inside a process holding Full Disk
/// Access. It reads the single length-prefixed UTF-8 run that follows the
/// `NSString` class record and rejects any blob that is not exactly that
/// layout. A rejected blob means the message is not admitted.
func decodeAttributedBody(_ data: Data) -> String? {
    let bytes = [UInt8](data)
    let header: [UInt8] = [0x04, 0x0B] + Array("streamtyped".utf8)
    guard bytes.count <= 1_048_576, bytes.starts(with: header) else { return nil }

    let marker = Array("NSString".utf8)
    guard let markerStart = firstRange(of: marker, in: bytes) else { return nil }
    var position = markerStart + marker.count

    // Class version 1, an object-reference tag, then one `+` (UTF-8 bytes) value.
    guard position + 5 < bytes.count,
          bytes[position] == 0x01,
          bytes[position + 1] >= 0x80,
          bytes[position + 2] == 0x84,
          bytes[position + 3] == 0x01,
          bytes[position + 4] == 0x2B
    else { return nil }
    position += 5

    var length = Int(bytes[position])
    position += 1
    switch length {
    case 0x81:
        guard position + 2 <= bytes.count else { return nil }
        length = Int(bytes[position]) | Int(bytes[position + 1]) << 8
        position += 2
    case 0x82:
        guard position + 4 <= bytes.count else { return nil }
        length = Int(bytes[position]) | Int(bytes[position + 1]) << 8
            | Int(bytes[position + 2]) << 16 | Int(bytes[position + 3]) << 24
        position += 4
    case 0x80...:
        return nil
    default:
        break
    }
    guard length >= 0, position + length <= bytes.count else { return nil }
    return String(bytes: bytes[position..<(position + length)], encoding: .utf8)
}

private func firstRange(of needle: [UInt8], in haystack: [UInt8]) -> Int? {
    guard needle.isEmpty == false, haystack.count >= needle.count else { return nil }
    for start in 0...(haystack.count - needle.count)
    where haystack[start] == needle[0] && haystack[start..<(start + needle.count)].elementsEqual(needle) {
        return start
    }
    return nil
}
