import Foundation
import Testing

@testable import VigiliaProtocol

struct FramerTests {
    private func strings(_ lines: [Data]) -> [String] {
        lines.map { String(decoding: $0, as: UTF8.self) }
    }

    @Test func keepsPartialLinesUntilTheNewline() throws {
        var framer = LineFramer()
        #expect(try framer.push(Data(#"{"id":1,"#.utf8)).isEmpty)
        #expect(strings(try framer.push(Data("\"ok\":true}\n{\"ev".utf8))) == [#"{"id":1,"ok":true}"#])
        #expect(strings(try framer.push(Data("ent\":\"x\"}\n".utf8))) == [#"{"event":"x"}"#])
    }

    @Test func splitsSeveralLinesInOneChunk() throws {
        var framer = LineFramer()
        #expect(strings(try framer.push(Data("a\nb\r\n\n  \nc\n".utf8))) == ["a", "b", "c"])
    }

    @Test func rejectsLinesOverTheCap() throws {
        var framer = LineFramer(maxLine: 8)
        #expect(strings(try framer.push(Data("12345678\n".utf8))) == ["12345678"])
        #expect(throws: LineFramer.LineTooLong.self) { try framer.push(Data("123456789".utf8)) }

        var complete = LineFramer(maxLine: 8)
        #expect(throws: LineFramer.LineTooLong.self) { try complete.push(Data("123456789\n".utf8)) }
    }
}
