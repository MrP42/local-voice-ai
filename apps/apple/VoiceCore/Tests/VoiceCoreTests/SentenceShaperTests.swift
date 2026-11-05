import XCTest
@testable import VoiceCore

final class SentenceShaperTests: XCTestCase {
    private let rate = 24_000.0

    /// Satz wie von Phonon: Stille vorn, Ton mit Gleichanteil, Stille hinten.
    private func sentence(leadMs: Double, toneMs: Double, tailMs: Double, dc: Float = 0.02) -> [Float] {
        let f = { (ms: Double) in Int(self.rate * ms / 1000) }
        let tone = (0..<f(toneMs)).map { i in Float(0.3 * sin(2 * Double.pi * 180 * Double(i) / rate)) + dc }
        return [Float](repeating: 0, count: f(leadMs)) + tone + [Float](repeating: 0, count: f(tailMs))
    }

    private func shape(_ s: [Float], chunk: Int = 480, paragraphEnd: Bool = false) -> [Float] {
        var shaper = SentenceShaper(sampleRate: rate)
        var out: [Float] = []
        var i = 0
        while i < s.count { out += shaper.push(Array(s[i..<min(i + chunk, s.count)])); i += chunk }
        return out + shaper.finish(paragraphEnd: paragraphEnd)
    }

    func testLongSilenceIsTrimmedToNaturalPause() {
        let out = shape(sentence(leadMs: 300, toneMs: 1000, tailMs: 600))
        let ms = Double(out.count) / rate * 1000
        // 30 ms Vorlauf + 1000 ms Laut + höchstens 140 ms Nachklang (plus Ausschwingen
        // des Gleichanteilfilters) statt 300 + 600 ms Stille.
        XCTAssertLessThan(ms, 1000 + 30 + 140 + 30)
        XCTAssertGreaterThan(ms, 1000)
    }

    func testEdgesStartAndEndNearZero() {
        let out = shape(sentence(leadMs: 0, toneMs: 500, tailMs: 0, dc: 0.1))
        XCTAssertLessThan(abs(out.first ?? 1), 0.001)
        XCTAssertLessThan(abs(out.last ?? 1), 0.01)
        // Kein Sprung größer als der Ton selbst erzeugt (Ploppen).
        let jump = zip(out, out.dropFirst()).map { abs($1 - $0) }.max() ?? 0
        XCTAssertLessThan(jump, 0.06)
    }

    func testParagraphEndGetsLongerPause() {
        let inner = shape(sentence(leadMs: 0, toneMs: 400, tailMs: 50))
        let end = shape(sentence(leadMs: 0, toneMs: 400, tailMs: 50), paragraphEnd: true)
        XCTAssertGreaterThan(end.count - inner.count, Int(rate * 0.35))
    }

    func testShortPausesInsideSentenceStay() {
        let s = sentence(leadMs: 0, toneMs: 300, tailMs: 150) + sentence(leadMs: 0, toneMs: 300, tailMs: 0)
        let out = shape(s)
        XCTAssertGreaterThan(Double(out.count) / rate, 0.74)
    }

    func testLongPausesInsideSentenceAreShortened() {
        // Phonon setzt zwischen seinen Abschnitten fast eine Satzpause.
        let s = sentence(leadMs: 0, toneMs: 300, tailMs: 600) + sentence(leadMs: 0, toneMs: 300, tailMs: 0)
        let ms = Double(shape(s).count) / rate * 1000
        XCTAssertLessThan(ms, 300 + 220 + 300 + 30)
        XCTAssertGreaterThan(ms, 300 + 200 + 300 - 30)
    }

    func testDcOffsetDoesNotKeepLeadingSilence() {
        // Gleichanteil über der Schwelle schon in der Stille (Ursache des Ploppens).
        let s = sentence(leadMs: 400, toneMs: 500, tailMs: 0).map { $0 + 0.05 }
        let ms = Double(shape(s).count) / rate * 1000
        XCTAssertLessThan(ms, 500 + 30 + 140 + 30)
    }

    func testChunkSizeDoesNotMatter() {
        let s = sentence(leadMs: 120, toneMs: 700, tailMs: 300)
        XCTAssertEqual(shape(s, chunk: 7), shape(s, chunk: 4_800))
    }
}
