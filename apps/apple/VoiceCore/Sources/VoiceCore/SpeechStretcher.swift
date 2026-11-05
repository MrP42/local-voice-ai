import Foundation

/// Sprechtempo ändern, ohne dass die Stimme blechern klingt (#314, Runde 2).
///
/// Gleiches Verfahren wie am Desktop (`managers/tts/stretch.rs`): tonhöhensynchrones
/// Überspringen bzw. Einfügen ganzer Stimmperioden (Prinzip PICOLA/Sonic, eigenständig
/// umgesetzt). Die Grundperiode wird per normierter Korrelation gesucht, genau eine
/// Periode wird überblendet entfernt (schneller) oder verdoppelt (langsamer), alles bis
/// zur nächsten Schnittstelle bleibt Sample für Sample unverändert. Bei 1,5x bleibt so
/// die Hälfte des Signals unangetastet.
///
/// Ersetzt auf dem iPhone `AVAudioUnitTimePitch`: Dessen Phasenvocoder klang auch mit
/// höchster Überlappung blechern.
///
/// Die Länge stimmt im Mittel exakt (Bruchteil-Übertrag); Mono, 24 kHz wie Phonon.
public struct SpeechStretcher {
    private static let minPitchHz = 65.0
    private static let maxPitchHz = 400.0

    public let speed: Double
    private let minPeriod: Int
    private let maxPeriod: Int
    private var input: [Float] = []
    private var pos = 0
    private var toCopy = 0.0
    private var blendCarry = 0.0
    private var out: [Float] = []
    private var totalIn = 0
    private var totalOut = 0

    public init(sampleRate: Double, speed: Double) {
        let rate = max(sampleRate, 8_000)
        self.speed = speed.isFinite ? min(max(speed, 0.25), 4) : 1
        minPeriod = Int(rate / Self.maxPitchHz)
        maxPeriod = Int(rate / Self.minPitchHz)
    }

    public var isPassthrough: Bool { abs(speed - 1) < 0.01 }

    /// Eingabe anhängen und zurückgeben, was schon fertig ist.
    public mutating func push(_ samples: [Float]) -> [Float] {
        if isPassthrough { return samples }
        input += samples
        totalIn += samples.count
        process()
        compact()
        return takeOut()
    }

    /// Rest ausgeben, so gekürzt oder (langsamer) mit auslaufender Blende aufgefüllt,
    /// dass die Gesamtlänge genau `eingespeist / speed` ist.
    public mutating func flush() -> [Float] {
        if isPassthrough { return [] }
        let rest = input.count - pos
        let target = Int((Double(totalIn) / speed).rounded())
        let want = max(0, target - totalOut)
        let take = min(want, rest)
        out += input[pos..<(pos + take)]
        if want > take {
            let last = out.last ?? 0
            let n = want - take
            for i in 0..<n { out.append(last * (1 - Float(i + 1) / Float(n))) }
        }
        let result = takeOut()
        input = []
        pos = 0
        toCopy = 0
        blendCarry = 0
        totalIn = 0
        totalOut = 0
        return result
    }

    /// Ein ganzes Stück auf einmal strecken.
    public static func stretch(_ samples: [Float], sampleRate: Double, speed: Double) -> [Float] {
        var s = SpeechStretcher(sampleRate: sampleRate, speed: speed)
        return s.push(samples) + s.flush()
    }

    private mutating func takeOut() -> [Float] {
        totalOut += out.count
        defer { out = [] }
        return out
    }

    private mutating func process() {
        let needed = 2 * maxPeriod + 1
        while true {
            let avail = input.count - pos
            if toCopy >= 1 {
                let n = min(Int(toCopy.rounded(.down)), avail)
                if n == 0 { return }
                out += input[pos..<(pos + n)]
                pos += n
                toCopy -= Double(n)
                if toCopy >= 1 { return }
                continue
            }
            if avail < needed { return }
            let period = findPeriod(at: pos)
            if speed > 1 { skipPeriod(period) } else { insertPeriod(period) }
        }
    }

    private mutating func blendLength(_ exact: Double, _ period: Int) -> Int {
        let total = exact + blendCarry
        let n = min(max(Int(total.rounded(.down)), 1), period)
        blendCarry = total - Double(n)
        return n
    }

    private mutating func skipPeriod(_ period: Int) {
        let blend: Int
        if speed >= 2 {
            blend = blendLength(Double(period) / (speed - 1), period)
        } else {
            toCopy += Double(period) * (2 - speed) / (speed - 1)
            blend = period
        }
        overlapAdd(blend, down: pos, up: pos + period)
        pos += period + blend
    }

    private mutating func insertPeriod(_ period: Int) {
        let blend: Int
        if speed < 0.5 {
            blend = blendLength(Double(period) * speed / (1 - speed), period)
        } else {
            toCopy += Double(period) * (2 * speed - 1) / (1 - speed)
            blend = period
        }
        out += input[pos..<(pos + period)]
        overlapAdd(blend, down: pos + period, up: pos)
        pos += blend
    }

    private mutating func overlapAdd(_ n: Int, down a: Int, up b: Int) {
        out.reserveCapacity(out.count + n)
        for i in 0..<n {
            let up = (Float(i) + 0.5) / Float(n)
            out.append(input[a + i] * (1 - up) + input[b + i] * up)
        }
    }

    /// Kürzeste Periode, deren normierte Korrelation mit der folgenden Periode ein lokales
    /// Maximum ist und mindestens 95 % des Bestwerts erreicht (Vielfache passen fast genauso).
    private func findPeriod(at at: Int) -> Int {
        let lo = minPeriod, hi = min(maxPeriod, (input.count - at) / 2)
        guard hi > lo else { return max(lo, 1) }
        var ncc = [Float](repeating: 0, count: hi - lo + 1)
        input.withUnsafeBufferPointer { x in
            for p in lo...hi {
                var ab: Float = 0, aa: Float = 0, bb: Float = 0
                for i in 0..<p {
                    let u = x[at + i], v = x[at + i + p]
                    ab += u * v; aa += u * u; bb += v * v
                }
                ncc[p - lo] = aa <= 1e-12 || bb <= 1e-12 ? 0 : ab / (aa * bb).squareRoot()
            }
        }
        let best = ncc.max() ?? 0
        let limit = best * 0.95
        for i in ncc.indices where ncc[i] >= limit {
            let left = i == 0 || ncc[i] >= ncc[i - 1]
            let right = i + 1 == ncc.count || ncc[i] >= ncc[i + 1]
            if left && right { return lo + i }
        }
        return lo
    }

    private mutating func compact() {
        if pos > 4 * maxPeriod {
            input.removeFirst(pos)
            pos = 0
        }
    }
}
