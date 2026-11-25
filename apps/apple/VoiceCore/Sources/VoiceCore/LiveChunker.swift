// nach Yogurt (MIT), FrigadeHQ — FrigadeHQ/yogurt @76e5595
import Foundation

public struct LiveChunk: Sendable {
    public let track: String
    public let startFrame: Int64
    public let samples: [Float]
    public let clearMs: Int
    let clearFrames: [Range<Int>]
    public var start: Double { Double(startFrame) / 16000 }
    public var end: Double { start + Double(samples.count) / 16000 }

    /// Ungültige Zeitangaben sind keine akustische Evidenz für eine Löschung.
    public func clearMilliseconds(from start: Double, to end: Double) -> Int {
        guard start.isFinite, end.isFinite, start >= 0, end > start,
              end <= Double(samples.count) / 16000 else { return 300 }
        let lower = Int((start * 16000).rounded(.down))
        let upper = Int((end * 16000).rounded(.up))
        return clearFrames.reduce(0) { $0 + max(0, min(upper, $1.upperBound) - max(lower, $1.lowerBound)) } * 1000 / 16000
    }
}

/// 16-kHz-Float32; nur auf der seriellen Verbraucherqueue verwenden.
public struct LiveChunker {
    private struct Window {
        var samples: [Float]
        var rms: Double
        var voiced: Bool
        var clear: Bool
    }
    private struct Track {
        var next: Int64?
        var rest: [Float] = []
        var winStart: Int64 = 0
        var start: Int64 = 0
        var open: [Window] = []
        var history: [Double] = []
    }
    private var tracks: [String: Track] = [:]
    public init() {}

    private func emit(_ track: String, _ state: Track, _ windows: [Window]) -> LiveChunk? {
        guard windows.filter({ $0.voiced }).count >= 3 else { return nil }
        var offset = 0
        var clearFrames: [Range<Int>] = []
        for window in windows {
            if window.clear { clearFrames.append(offset..<offset + window.samples.count) }
            offset += window.samples.count
        }
        return LiveChunk(track: track, startFrame: state.start, samples: windows.flatMap(\.samples),
                         clearMs: windows.filter { $0.clear }.reduce(0) { $0 + $1.samples.count * 1000 / 16000 },
                         clearFrames: clearFrames)
    }

    private func close(_ track: String, _ state: inout Track) -> LiveChunk? {
        var end = state.open.count
        while end > 0 && !state.open[end - 1].voiced { end -= 1 }
        let result = emit(track, state, Array(state.open.prefix(min(state.open.count, end + 2))))
        state.open = []
        return result
    }

    private func add(_ track: String, _ state: inout Track, _ samples: [Float]) -> [LiveChunk] {
        let rms = sqrt(samples.reduce(0.0) { $0 + Double($1) * Double($1) } / Double(samples.count))
        state.history.append(rms)
        if state.history.count > 600 { state.history.removeFirst() }
        let floor = state.history.sorted()[Int(Double(state.history.count) * 0.1)]
        func level(_ factor: Double) -> Double { min(0.02, max(0.0015, floor * factor)) }
        let voiced = rms >= (state.history.count < 20 ? 0.004 : level(1.5))
        let clear = rms >= (state.history.count < 20 ? 0.012 : level(3) * 3)
        defer { state.winStart += Int64(samples.count) }
        if state.open.isEmpty {
            guard voiced else { return [] }
            state.start = state.winStart
        }
        state.open.append(Window(samples: samples, rms: rms, voiced: voiced, clear: clear))
        if state.open.suffix(10).count == 10 && state.open.suffix(10).allSatisfy({ !$0.voiced }) {
            return close(track, &state).map { [$0] } ?? []
        }
        if state.open.count >= 150 {
            var cut = state.open.count - 20
            for i in cut..<state.open.count where state.open[i].rms < state.open[cut].rms { cut = i }
            let head = Array(state.open.prefix(cut + 1))
            let result = emit(track, state, head)
            state.open = Array(state.open.dropFirst(cut + 1))
            state.start += Int64(head.reduce(0) { $0 + $1.samples.count })
            return result.map { [$0] } ?? []
        }
        return []
    }

    /// Leerer Puffer ist eine explizite Aufnahmegrenze, auch ohne Zeitlücke.
    public mutating func push(track: String, startFrame: Int64, samples: [Float]) -> [LiveChunk] {
        var state = tracks[track] ?? Track()
        var out: [LiveChunk] = []
        if samples.isEmpty || state.next.map({ abs(startFrame - $0) > 3200 }) == true {
            out += finish(track, &state)
        }
        guard !samples.isEmpty else { tracks[track] = state; return out }
        if state.next == nil { state.winStart = startFrame }
        state.next = startFrame + Int64(samples.count)
        let buffer = state.rest + samples
        var offset = 0
        while offset + 1600 <= buffer.count {
            out += add(track, &state, Array(buffer[offset..<offset + 1600]))
            offset += 1600
        }
        state.rest = Array(buffer.dropFirst(offset))
        tracks[track] = state
        return out
    }

    private func finish(_ track: String, _ state: inout Track) -> [LiveChunk] {
        var out: [LiveChunk] = []
        let rest = state.rest
        if !rest.isEmpty { out += add(track, &state, rest) }
        if let chunk = close(track, &state) { out.append(chunk) }
        state.rest = []; state.next = nil
        return out
    }

    public mutating func flush() -> [LiveChunk] {
        var out: [LiveChunk] = []
        for key in tracks.keys.sorted() {
            var state = tracks[key]!
            out += finish(key, &state)
            tracks[key] = state
        }
        return out
    }
}
