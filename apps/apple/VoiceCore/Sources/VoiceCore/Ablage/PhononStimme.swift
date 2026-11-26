import Foundation
#if canImport(CryptoKit)

/// Reine Prüfung des S1a-Formats. Fehler tragen weder Namen noch Stimmbytes.
enum PhononStimmenFehler: Error { case ungueltig }

enum PhononStimme {
    static let minBytes = 16 * 1024
    static let maxBytes = 4 * 1024 * 1024

    static func name(_ raw: String) throws -> String {
        let display = raw.trimmingCharacters(in: .whitespacesAndNewlines)
            .split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        guard display == raw, !raw.isEmpty, raw.unicodeScalars.count <= 40,
              !raw.unicodeScalars.contains(where: { $0.properties.generalCategory == .control }),
              raw.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) || " -_.'()".unicodeScalars.contains($0) }),
              !raw.hasPrefix("."), !raw.hasSuffix(".") else { throw PhononStimmenFehler.ungueltig }
        // Dieselbe explizite Faltung wie phonon_voices::parse_name (keine localeabhängige Transliteration).
        let groups: [(String, String)] = [
            ("äæ", "ae"), ("ÄÆ", "Ae"), ("öœ", "oe"), ("ÖŒ", "Oe"), ("ü", "ue"), ("Ü", "Ue"), ("ß", "ss"),
            ("àáâãåā", "a"), ("ÀÁÂÃÅ", "A"), ("èéêë", "e"), ("ÈÉÊË", "E"),
            ("ìíîï", "i"), ("ÌÍÎÏ", "I"), ("òóôõø", "o"), ("ÒÓÔÕØ", "O"),
            ("ùúû", "u"), ("ÙÚÛ", "U"), ("ýÿ", "y"), ("Ý", "Y"), ("ñ", "n"), ("Ñ", "N"), ("ç", "c"), ("Ç", "C"),
        ]
        var id = ""
        for c in raw.unicodeScalars {
            if c.isASCII && (CharacterSet.alphanumerics.contains(c) || c == "_") { id.unicodeScalars.append(c) }
            else if let g = groups.first(where: { $0.0.unicodeScalars.contains(c) }) { id += g.1 }
            else if !id.hasSuffix("-") { id += "-" }
        }
        id = String(id.trimmingCharacters(in: CharacterSet(charactersIn: "-_")).prefix(40))
            .trimmingCharacters(in: CharacterSet(charactersIn: "-_"))
        let up = id.uppercased()
        guard !id.isEmpty, !["CON", "PRN", "AUX", "NUL", "DEFAULT"].contains(up),
              !((up.hasPrefix("COM") || up.hasPrefix("LPT")) && up.utf8.count == 4 && up.last?.isNumber == true) else {
            throw PhononStimmenFehler.ungueltig
        }
        return id
    }

    static func pruefen(_ bytes: [UInt8], sha256: String) throws {
        guard (minBytes...maxBytes).contains(bytes.count), AblageValidierung.hex64(sha256),
              AblageKrypto.sha256Hex(bytes) == sha256 else { throw PhononStimmenFehler.ungueltig }
        let n = bytes.prefix(8).enumerated().reduce(UInt64(0)) { $0 | UInt64($1.element) << ($1.offset * 8) }
        guard (2...65536).contains(n), n <= UInt64(bytes.count - 8),
              case .object(let tensors) = try AblageJSON.parse(Array(bytes[8..<8 + Int(n)])) else { throw PhononStimmenFehler.ungueltig }
        let dataCount = UInt64(bytes.count - 8 - Int(n))
        var spans: [(UInt64, UInt64)] = [], speaker = false
        for entry in tensors {
            let t = entry.value
            if entry.key == "__metadata__" {
                guard case .object(let m) = t, m.allSatisfy({ if case .string = $0.value { return true }; return false }) else { throw PhononStimmenFehler.ungueltig }
                continue
            }
            let dtype: String = try t.pflicht("dtype")
            let sizes: [String: UInt64] = ["F64": 8, "I64": 8, "U64": 8, "F32": 4, "I32": 4, "U32": 4,
                "F16": 2, "BF16": 2, "I16": 2, "U16": 2, "I8": 1, "U8": 1, "BOOL": 1, "F8_E4M3": 1, "F8_E5M2": 1]
            let shape: [UInt64] = try t.pflicht("shape"), offsets: [UInt64] = try t.pflicht("data_offsets")
            guard let size = sizes[dtype], offsets.count == 2, offsets[0] <= offsets[1], offsets[1] <= dataCount else { throw PhononStimmenFehler.ungueltig }
            var count: UInt64 = 1
            for d in shape {
                let r = count.multipliedReportingOverflow(by: d)
                guard !r.overflow else { throw PhononStimmenFehler.ungueltig }
                count = r.partialValue
            }
            let byteCount = count.multipliedReportingOverflow(by: size)
            guard !byteCount.overflow, byteCount.partialValue == offsets[1] - offsets[0] else { throw PhononStimmenFehler.ungueltig }
            if entry.key == "speaker_wavs" {
                guard dtype == "F32", shape == [1, 512, 125] else { throw PhononStimmenFehler.ungueltig }
                speaker = true
            }
            spans.append((offsets[0], offsets[1]))
        }
        var end: UInt64 = 0
        for s in spans.sorted(by: { $0.0 == $1.0 ? $0.1 < $1.1 : $0.0 < $1.0 }) {
            guard s.0 == end else { throw PhononStimmenFehler.ungueltig }
            end = s.1
        }
        guard speaker, end == dataCount else { throw PhononStimmenFehler.ungueltig }
    }

    struct Kandidat: Sendable {
        var datei: AblageDatei
        var updated: Int64
        var generation: UInt32
        var uid: String
        var id: String
        var name: String
        var created: Int64
        var fassung: AblageFassung
        var fingerprint: String

        func kleiner(als b: Kandidat) -> Bool {
            if updated != b.updated { return updated < b.updated }
            if fassung.blob != b.fassung.blob { return fassung.blob < b.fassung.blob }
            if generation != b.generation { return generation < b.generation }
            return datei.id < b.datei.id
        }
    }

    private static func rang(_ j: AblageJSON) throws -> Int64 {
        let s = try String(ablageJSON: j)
        guard s.utf8.count == 20, s.utf8.allSatisfy({ (48...57).contains($0) }), let n = Int64(s), n >= 0 else { throw PhononStimmenFehler.ungueltig }
        return n
    }

    private static func idGueltig(_ id: String) -> Bool {
        !id.isEmpty && id.utf8.count <= 40 && id.utf8.allSatisfy {
            (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 45 || $0 == 95
        }
    }

    private static func createdMillis(_ raw: String) throws -> Int64 {
        let format = ISO8601DateFormatter()
        format.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        guard format.date(from: raw) != nil || ISO8601DateFormatter().date(from: raw) != nil else { throw PhononStimmenFehler.ungueltig }
        var whole = raw, fraction: Int64 = 0
        if let dot = raw.firstIndex(of: ".") {
            let digits = raw[raw.index(after: dot)...].prefix { $0.isASCII && $0.isNumber }
            fraction = Int64((String(digits.prefix(3)) + "000").prefix(3)) ?? 0
            whole = String(raw[..<dot]) + String(raw[digits.endIndex...])
        }
        guard let date = ISO8601DateFormatter().date(from: whole) else { throw PhononStimmenFehler.ungueltig }
        // RFC3339-Nachkommastellen ohne Double-Rundung auf Millisekunden abschneiden wie chrono.
        return Int64(date.timeIntervalSince1970) * 1000 + fraction
    }

    static func kandidat(_ d: AblageDatei, key: [UInt8]) throws -> Kandidat {
        let extra = AblageJSON.object(d.extra)
        guard case .array(let v)? = extra["phonon_voice"], v.count == 2,
              case .array(let names)? = extra["phonon_name"], names.count == 3 else { throw PhononStimmenFehler.ungueltig }
        var updated = try rang(v[0])
        let m = v[1]
        let id: String = try m.pflicht("id"), display: String = try m.pflicht("name"), uid: String = try m.pflicht("uid")
        let generation: UInt32 = try m.pflicht("generation"), sha: String = try m.pflicht("sha256"), fas: String = try m.pflicht("fas")
        let lang: String = try m.pflicht("language"), origin: String = try m.pflicht("origin"), created: String = try m.pflicht("created")
        let _: String = try m.pflicht("voice_id")
        let _: String? = try m.pflicht("source_note")
        let _: String? = try m.pflicht("consent_at")
        let creation = try createdMillis(created)
        let nameID = try String(ablageJSON: names[1]), nameDisplay = try String(ablageJSON: names[2])
        _ = try rang(names[0])
        guard idGueltig(id), idGueltig(nameID),
              try name(display).lowercased() == id.lowercased(), try name(nameDisplay).lowercased() == nameID.lowercased(),
              uid.utf8.count == 32, uid.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              AblageValidierung.hex64(sha), ["und", "de", "en", "fr", "es", "pt"].contains(lang),
              ["import", "gradium-clone"].contains(origin),
              d.id == AblageSystem.kennung(key: key, ordner: false, generation: generation, salt: uid),
              d.ordner.wert == AblageSystem.kennung(key: key, ordner: true, generation: generation),
              d.papierkorb.wert == nil, !d.fassungen.isEmpty,
              d.fassungen.allSatisfy({ $0.typ == "lvstimme" && AblageValidierung.hex64($0.blob) && $0.blob == $0.originalSha256
                  && (UInt64(minBytes)...UInt64(maxBytes)).contains($0.size) && $0.texte.isEmpty }),
              d.fassungen.contains(where: { $0.fas == fas && $0.blob == sha }) else { throw PhononStimmenFehler.ungueltig }
        let heads = AblageZusammenfuehren.koepfe(d)
        guard let selected = d.fassungen.filter({ heads.contains($0.fas) }).max(by: {
            if $0.zeit != $1.zeit { return $0.zeit < $1.zeit }
            if $0.blob != $1.blob { return $0.blob < $1.blob }
            return $0.fas < $1.fas
        }) else { throw PhononStimmenFehler.ungueltig }
        if selected.fas != fas {
            guard updated < Int64.max else { throw PhononStimmenFehler.ungueltig }
            updated += 1 // gleiche Fassungswahl wie Desktop, hier ohne Push
        }
        return Kandidat(datei: d, updated: updated, generation: generation, uid: uid, id: nameID, name: nameDisplay,
            created: creation, fassung: selected,
            fingerprint: AblageKrypto.sha256Hex(d.kanonischeNutzlast(ordnen: false)))
    }
}
#endif
