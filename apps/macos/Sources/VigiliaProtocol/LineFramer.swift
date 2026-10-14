import Foundation

/// Splits a byte stream into newline-delimited lines.
public struct LineFramer: Sendable {
    /// Longest line accepted, matching the service's own cap.
    public static let defaultMaxLine = 64 * 1024

    public struct LineTooLong: Error, Equatable {}

    public let maxLine: Int
    private var buffer = Data()

    public init(maxLine: Int = LineFramer.defaultMaxLine) {
        self.maxLine = maxLine
    }

    /// Adds `chunk` and returns every line it completed, without the newline. Blank lines are
    /// skipped. Throws once a line grows past `maxLine`; the connection should then be dropped.
    public mutating func push(_ chunk: Data) throws -> [Data] {
        buffer.append(chunk)
        var lines: [Data] = []
        while let newline = buffer.firstIndex(of: 0x0A) {
            var line = Data(buffer[buffer.startIndex..<newline])
            buffer = Data(buffer[buffer.index(after: newline)...])
            if line.last == 0x0D { line.removeLast() }
            guard line.count <= maxLine else { throw LineTooLong() }
            if line.contains(where: { !Self.isSpace($0) }) {
                lines.append(line)
            }
        }
        guard buffer.count <= maxLine else { throw LineTooLong() }
        return lines
    }

    private static func isSpace(_ byte: UInt8) -> Bool {
        byte == 0x20 || byte == 0x09 || byte == 0x0D
    }
}
