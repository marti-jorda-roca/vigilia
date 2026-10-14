import CoreGraphics
import Testing

@testable import Vigilia

struct PixelArtTests {
    @Test func glyphsAreSevenRowsOfEqualWidth() {
        for (character, rows) in PixelFont.glyphs {
            #expect(rows.count == PixelFont.height, "\(character)")
            #expect(Set(rows.map(\.count)).count == 1, "\(character)")
        }
    }

    @Test func layoutAddsSpacingAndUppercases() {
        let one = PixelFont.layout("A")
        #expect(one.width == 5)
        #expect(PixelFont.layout("AA").width == 5 + PixelFont.spacing + 5)
        #expect(PixelFont.layout("a").pixels == one.pixels)
        // Unknown characters fall back to `?` rather than vanishing.
        #expect(PixelFont.layout("€").pixels == PixelFont.layout("?").pixels)
    }

    @Test func everyCountdownCharacterHasAGlyph() {
        for character in "0123456789DHMS :+-%·." {
            #expect(PixelFont.glyphs[character] != nil, "\(character)")
        }
    }

    @Test func lanternRowsAreEqualWidth() {
        #expect(Set(Lantern.art.map(\.count)) == [Lantern.width])
    }

    @Test func onlyTheLitLanternGlows() {
        let frame = CGColor(gray: 1, alpha: 1)
        let unlit = Lantern.pixels(state: .unlit, frame: frame)
        let lit = Lantern.pixels(state: .lit, frame: frame)
        // Unlit: only the frame and its cross bars, all in the frame colour.
        #expect(unlit.allSatisfy { $0.color == frame })
        #expect(lit.count > unlit.count)
        #expect(lit.contains { $0.color != frame })
    }
}
