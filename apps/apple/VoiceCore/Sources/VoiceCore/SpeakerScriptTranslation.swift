import Foundation

extension SpeakerScript {
    private enum TranslationPiece {
        case literal(String)
        case slot(Int)
    }
    /// `suggestSwap` vom Desktop: bereits passende Stimmen behalten, danach freie
    /// Zielstimmen in Auftretensreihenfolge; bei mehr Sprechern zyklisch verteilen.
    public func translatedSpeakers(targets: [Speaker], fallbackVoice: String) -> [Speaker] {
        let used = segments.compactMap(\.voice).reduce(into: [String]()) { ids, id in
            if !ids.contains(id) { ids.append(id) }
        }
        var mapping: [String: String] = [:], assigned: Set<String> = []
        for id in used where targets.contains(where: { $0.id == id }) { mapping[id] = id; assigned.insert(id) }
        var next = 0
        for id in used where mapping[id] == nil && !targets.isEmpty {
            let pick: Speaker
            if let free = targets.first(where: { !assigned.contains($0.id) }) { pick = free }
            else { pick = targets[next % targets.count]; next += 1 }
            mapping[id] = pick.id; assigned.insert(pick.id)
        }
        return speakers.map { speaker in
            var result = speaker; result.id = mapping[speaker.id] ?? fallbackVoice; return result
        }
    }

    /// Marken und Regieanweisungen gehen nie an den Übersetzer. Die unveränderten
    /// Literale werden zwischen den übersetzten Textstücken wieder eingesetzt.
    /// Ohne Marken bleibt es bei genau einem Übersetzungsstück pro Eingangstext.
    public static func translate(_ texts: [String], speakers: [Speaker] = [],
                                 startsAtLineStart: [Bool]? = nil,
                                 using translator: ([String]) async throws -> [String]) async throws -> [String] {
        guard startsAtLineStart == nil || startsAtLineStart?.count == texts.count else { throw LiveTranslationError.countMismatch }
        var requests: [String] = []
        var plans: [[TranslationPiece]] = []
        for (index, text) in texts.enumerated() {
            let all = text as NSString
            var plan: [TranslationPiece] = []
            var merged: [NSRange] = []
            for range in SpeakerScript(text, speakers: speakers, startsAtLineStart: startsAtLineStart?[index] ?? true).hiddenRanges {
                if let last = merged.last, NSMaxRange(last) >= range.location {
                    merged[merged.count - 1] = NSUnionRange(last, range)
                } else { merged.append(range) }
            }
            var start = 0
            func add(_ end: Int) {
                let piece = all.substring(with: NSRange(location: start, length: end - start))
                guard !piece.isEmpty else { return }
                if piece.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { plan.append(.literal(piece)) }
                else { plan.append(.slot(requests.count)); requests.append(piece) }
            }
            for range in merged {
                add(range.location)
                plan.append(.literal(all.substring(with: range))); start = NSMaxRange(range)
            }
            add(all.length); plans.append(plan)
        }
        let translated = requests.isEmpty ? [] : try await translator(requests)
        guard translated.count == requests.count else { throw LiveTranslationError.countMismatch }
        // Ein brauchbarer Gesamttext darf nicht nur noch aus geretteten Marken bestehen.
        guard translated.allSatisfy(LiveTranslation.usable) else { throw LiveTranslationError.unusable }
        return plans.map { plan in plan.map { item in
            switch item {
            case .literal(let value): return value
            case .slot(let index): return translated[index]
            }
        }.joined() }
    }
}
