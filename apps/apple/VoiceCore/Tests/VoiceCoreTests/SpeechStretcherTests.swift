import XCTest
@testable import VoiceCore

final class SpeechStretcherTests: XCTestCase {
    private let rate = 24_000.0

    /// Stimmähnlich: Pulsfolge mit gleitender Grundfrequenz durch einen Formantfilter.
    private func voice(_ f0: Double, seconds: Double) -> [Float] {
        let n = Int(rate * seconds)
        var phase = 0.0, y1 = 0.0, y2 = 0.0
        let r = 0.97, w = 2 * Double.pi * 700 / rate
        return (0..<n).map { i in
            let t = Double(i) / rate
            phase += f0 * (1 + 0.08 * sin(2 * Double.pi * 3 * t)) / rate
            var pulse = 0.0
            if phase >= 1 { phase -= 1; pulse = 1 }
            let y = pulse + 2 * r * cos(w) * y1 - r * r * y2
            y2 = y1; y1 = y
            return Float(y * 0.05)
        }
    }

    private func maxJump(_ s: [Float]) -> Float {
        zip(s, s.dropFirst()).map { abs($1 - $0) }.max() ?? 0
    }

    func testLengthIsExact() {
        let input = voice(110, seconds: 3)
        for speed in [0.5, 0.75, 1.25, 1.5, 1.75, 2.0, 2.5] {
            let out = SpeechStretcher.stretch(input, sampleRate: rate, speed: speed)
            let want = Double(input.count) / speed
            XCTAssertLessThan(abs(Double(out.count) - want) / rate, 0.005, "Tempo \(speed)")
        }
    }

    func testStreamingEqualsWhole() {
        let input = voice(120, seconds: 2)
        let whole = SpeechStretcher.stretch(input, sampleRate: rate, speed: 1.5)
        var s = SpeechStretcher(sampleRate: rate, speed: 1.5)
        var parts: [Float] = []
        var i = 0
        while i < input.count { parts += s.push(Array(input[i..<min(i + 1920, input.count)])); i += 1920 }
        parts += s.flush()
        XCTAssertEqual(parts, whole)
    }

    func testNoJumps() {
        let input = voice(100, seconds: 2)
        let limit = maxJump(input) * 1.05
        for speed in [0.75, 1.5, 2.0] {
            XCTAssertLessThanOrEqual(maxJump(SpeechStretcher.stretch(input, sampleRate: rate, speed: speed)), limit, "Tempo \(speed)")
        }
    }

    func testPitchIsKept() {
        // 200 Hz Sinus: Nulldurchgänge je Sekunde bleiben bei jedem Tempo gleich.
        let input = (0..<Int(rate * 2)).map { Float(0.5 * sin(2 * Double.pi * 200 * Double($0) / rate)) }
        for speed in [0.75, 1.5, 2.0] {
            let out = SpeechStretcher.stretch(input, sampleRate: rate, speed: speed)
            let mid = Array(out[out.count / 4..<out.count * 3 / 4])
            let crossings = zip(mid, mid.dropFirst()).filter { ($0 < 0) != ($1 < 0) }.count
            let f = Double(crossings) * rate / Double(mid.count) / 2
            XCTAssertEqual(f, 200, accuracy: 5, "Tempo \(speed)")
        }
    }

    func testSpeedOneIsUntouched() {
        let input = voice(150, seconds: 0.5)
        XCTAssertEqual(SpeechStretcher.stretch(input, sampleRate: rate, speed: 1), input)
    }
}
