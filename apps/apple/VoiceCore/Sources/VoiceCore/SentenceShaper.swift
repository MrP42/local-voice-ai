import Foundation

/// Satzränder fürs lückenlose Vorlesen (#314).
///
/// Phonon liefert jeden Satz als eigenes Tonstück: mit eigener Stille vorn und hinten und
/// mit einem Einsatz, der nicht bei null beginnt. Hintereinandergehängt ergab das ein leises
/// Ploppen am Anfang jedes Satzes (hörbar mit Kopfhörern) und lange Pausen zwischen Sätzen.
///
/// Der Former arbeitet im Strom, Stück für Stück, ohne den Satz abzuwarten:
/// - Gleichanteil weg (einpoliger Hochpass um 19 Hz bei 24 kHz).
/// - Stille vor dem ersten Laut bis auf `leadMs` kürzen, mit Kosinus einblenden.
/// - Stille nach dem letzten Laut auf höchstens `tailMs` kürzen, mit Kosinus ausblenden.
/// - Am Absatzende die Pause auf mindestens `paragraphMs` auffüllen.
/// Pausen IM Satz (Komma, Gedankenstrich) bleiben unangetastet.
public struct SentenceShaper {
    /// Pegel, ab dem ein Abtastwert als Laut zählt (rund −44 dBFS).
    public static let threshold: Float = 0.006

    private let lead: Int
    private let fadeIn: Int
    private let fadeOut: Int
    private let tail: Int
    private let paragraph: Int

    private var started = false
    private var pre: [Float] = []
    private var hold: [Float] = []
    /// Die letzten `fadeOut` Werte bleiben zurück, damit `finish` sie ausblenden kann.
    private var delay: [Float] = []
    private var fadePos = 0
    private var dcX: Float = 0
    private var dcY: Float = 0
    private let dcPole: Float

    public init(sampleRate: Double, leadMs: Double = 15, tailMs: Double = 140, paragraphMs: Double = 450) {
        let frames = { (ms: Double) in max(0, Int((sampleRate * ms / 1000).rounded())) }
        lead = frames(leadMs)
        fadeIn = max(1, frames(8))
        fadeOut = max(1, frames(12))
        tail = frames(tailMs)
        paragraph = frames(paragraphMs)
        dcPole = Float(1 - 2 * Double.pi * 19 / max(sampleRate, 1))
    }

    /// Ein Stück einspeisen; zurück kommt, was schon gespielt werden kann.
    public mutating func push(_ chunk: [Float]) -> [Float] {
        var ready: [Float] = []
        ready.reserveCapacity(chunk.count)
        for raw in chunk {
            let y = raw - dcX + dcPole * dcY
            dcX = raw
            dcY = y
            if !started {
                guard abs(y) >= Self.threshold else {
                    pre.append(y)
                    if pre.count > 2 * lead + 64 { pre.removeFirst(pre.count - lead) }
                    continue
                }
                started = true
                for v in pre.suffix(lead) { ready.append(faded(v)) }
                pre = []
            }
            if abs(y) >= Self.threshold {
                for v in hold { ready.append(faded(v)) }
                hold.removeAll(keepingCapacity: true)
                ready.append(faded(y))
            } else {
                hold.append(y)
            }
        }
        return delayed(ready)
    }

    /// Satzende: Nachklang kürzen und ausblenden, am Absatzende die Pause auffüllen.
    public mutating func finish(paragraphEnd: Bool) -> [Float] {
        var rest = delay + hold.prefix(tail).map { faded($0) }
        let n = min(fadeOut, rest.count)
        for i in 0..<n {
            let x = Double(i + 1) / Double(n)
            rest[rest.count - n + i] *= Float(0.5 + 0.5 * cos(Double.pi * x))
        }
        let silence = rest.count - delay.count
        if paragraphEnd, silence < paragraph {
            rest += [Float](repeating: 0, count: paragraph - silence)
        }
        reset()
        return rest
    }

    private mutating func reset() {
        started = false
        pre = []
        hold = []
        delay = []
        fadePos = 0
        dcX = 0
        dcY = 0
    }

    private mutating func faded(_ v: Float) -> Float {
        guard fadePos < fadeIn else { return v }
        let g = 0.5 - 0.5 * cos(Double.pi * Double(fadePos) / Double(fadeIn))
        fadePos += 1
        return v * Float(g)
    }

    private mutating func delayed(_ ready: [Float]) -> [Float] {
        delay += ready
        guard delay.count > fadeOut else { return [] }
        let out = Array(delay.prefix(delay.count - fadeOut))
        delay.removeFirst(out.count)
        return out
    }
}
