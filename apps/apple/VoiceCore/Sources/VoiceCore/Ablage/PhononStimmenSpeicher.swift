import Foundation
#if canImport(CryptoKit)

/// Lesende iOS-Projektion, unabhängig von installierten Engines (#898, S2).
enum PhononStimmenSpeicher {
    struct Eintrag: Codable, Equatable {
        var datei: [UInt8]
        var id: String
        var name: String
    }

    struct Index: Codable, Equatable {
        var version = 1
        var konto: String
        var stimmen: [Eintrag] = []
        var verworfen: Set<String> = []
    }

    // Spiegel der PhononVoice-Katalogkennungen. Offline-Vertrag prüft Gleichheit mit catalog.json.
    static let katalogNamen = """
    Nele Jette Annika Jule Lotta Josefine Anton Mirko Mats Henrik Leon Mika Svea Zoey Juniper Everly Harper Hadley Delia Easton Micah Trevor Russell Preston Miles Maeve Freya Pippa Elodie-Rose Imogen Lottie Alfie Hugh Edwin Spencer Toby Teddy Bryony
    Captain-Silas-Blackwater Morwenna-of-the-Deep Old-Mortlake Azgorath Fennick-the-Young-Elf The-Thing-Under-the-Stairs Vyrmathrax-the-Ancient Grub-the-Ogre Master-Aldric The-Snow-Queen Sir-Bramwell Fairy-Godmother
    2A2fFf8SkDEJ8nzx 4Mn9VfG2wsLLEzi5 p6Uutkyi3j2iNAUu MxhgvuHuUER7qjmL MOrDPn052KUlnKXk yadpmqsHVrYC0NJp 3ZKKapPOvuWFcw9f Xn9xDzqSSr4xGTNg Kf5m22mROozoMWj3 yyS1KYWs6mXoEw7D 20zdyYrQPzKlCwkk R3uU1GTAZJotUhUs Adoh4wA9tp5QXJi9 NbpkqMVS3CJeq2j8 NIWIOv5vTi5eRMj8 vjFyp2BcCyTlUmYb 4SZHfMpw-p46Ywgs bXwrkHhURL1VFJdG nGzMHMPAWGaJFHOo LMDIYE0Oa377Qh6Z 40QrMVgAxFiUnP9f 0zLApsZfVLLNaEGl _6Aslh2DxfmnRLmP qIxlZ7Ei8KvWlyQH d9XCyhjo4JhkHF1d 6PWnV0Nq4wu7RVBT GgfEkEJtxZR7gnpy uem82D50GRv2Dwma y9bQqIDnyxwqK01t gDT1nz7Ie36ZhL-C m9MxpY6IUGXtWp76 X6NceqF285Gr04gb YyKcxPG7iIgS3jsK zUCxzrOrugTSis03 JOW0vchyswkQUMDS dME3IWyZBvmh1n1q Zn7g6NxLqXlJb7Qi gfP44RlHwr2kEwk7 xPDpsV3F592srkjA nOykTmcH9ydsGGIc lX3LavMNdlIbgEHH dGyGDKo2osj7h3s7 cXoIXJ2llS8S0WOW GXaSqMEmOg12OIYb xO8xQo3BrB8FuI6J cYtsRSxV5q9IMehn gSgG5Tor0GPEnqzC NIWoEurJHMJjyUTe y2eiKxtQo2B67deU QXNr0SvkukLB4wPq
    """.split(whereSeparator: { $0.isWhitespace }).map { $0.lowercased() }

    static func blobName(_ k: PhononStimme.Kandidat) -> String { k.uid + "-" + k.fassung.blob + ".safetensors" }

    private static func istMasterName(_ name: String) -> Bool {
        guard name.hasSuffix(".safetensors") else { return false }
        let parts = name.dropLast(".safetensors".count).split(separator: "-", omittingEmptySubsequences: false)
        return parts.count == 2 && parts[0].utf8.count == 32
            && parts[0].utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) })
            && AblageValidierung.hex64(String(parts[1]))
    }

    private static func wurzelPruefen(_ wurzel: URL) throws {
        for url in [wurzel.deletingLastPathComponent(), wurzel] {
            if let attr = try? FileManager.default.attributesOfItem(atPath: url.path),
               (attr[.type] as? FileAttributeType) != .typeDirectory { throw AblageSpeicherFehler.schreiben }
        }
    }

    static func laden(_ wurzel: URL, konto: String) throws -> Index {
        let url = wurzel.appendingPathComponent("stimmen.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return Index(konto: konto) }
        let idx = try JSONDecoder().decode(Index.self, from: Data(contentsOf: url))
        guard idx.version == 1 else { throw PhononStimmenFehler.ungueltig }
        return idx.konto == konto ? idx : Index(konto: konto)
    }

    private static func tot(_ id: String, sammlung: String, z: AblageZustand) -> Bool {
        z.ledger.begraben.contains(id) || z.ledger.objekt(sammlung, id)?.deleted == true
            || (sammlung == AblageLedger.sammlungOrdner && z.bestand.ordner[id]?.papierkorb.wert != nil)
            || (sammlung == AblageLedger.sammlungDatei && z.bestand.dateien[id]?.papierkorb.wert != nil)
    }

    /// Gesamtplan vor dem Indexwechsel: echte Basisnamen reservieren, dann Suffixe nach UID vergeben.
    static func namen(_ ks: [PhononStimme.Kandidat], modellNamen: Set<String>) throws -> [String: String] {
        var blocked = Set(katalogNamen).union(modellNamen.map { $0.lowercased() })
        for k in ks {
            for e in k.datei.extra where e.value == .bool(true) && e.key.hasPrefix("phonon_blocked_") {
                let id = String(e.key.dropFirst("phonon_blocked_".count))
                if (try? PhononStimme.name(id)) == id { blocked.insert(id.lowercased()) }
            }
        }
        var occupied = blocked, assigned: [String: String] = [:]
        let groups = Dictionary(grouping: ks, by: { $0.id.lowercased() })
        for base in groups.keys.sorted() {
            let group = groups[base]!.sorted { $0.created == $1.created ? $0.uid < $1.uid : $0.created < $1.created }
            if occupied.insert(base).inserted, let first = group.first { assigned[first.uid] = first.name }
        }
        for k in ks.sorted(by: { $0.uid < $1.uid }) where assigned[k.uid] == nil {
            let eigen = blocked.contains(k.id.lowercased())
            var n = eigen ? 1 : 2
            while true {
                let suffix = eigen ? (n == 1 ? "-eigen" : "-eigen-\(n)") : "-\(n)"
                let display = String(k.id.prefix(40 - suffix.count)) + suffix
                let id = try PhononStimme.name(display)
                if occupied.insert(id.lowercased()).inserted { assigned[k.uid] = display; break }
                n += 1
            }
        }
        return assigned
    }

    /// Zusätzlich lokal installierte Modellstimmen schützen, ohne eine Engine anzufassen.
    private static func modellNamen(_ wurzel: URL) -> Set<String> {
        let fm = FileManager.default, parent = wurzel.deletingLastPathComponent()
        var names = Set<String>()
        for model in (try? fm.contentsOfDirectory(at: parent, includingPropertiesForKeys: nil)) ?? [] where model.lastPathComponent.hasPrefix("models-") {
            let own = (try? PhononStimmenInjektion.eigeneNamen(in: model)) ?? []
            for url in (try? fm.contentsOfDirectory(at: model.appendingPathComponent("voices"), includingPropertiesForKeys: nil)) ?? [] where url.pathExtension == "safetensors" {
                if own.contains(url.deletingPathExtension().lastPathComponent) { continue }
                names.insert(url.deletingPathExtension().lastPathComponent.lowercased())
            }
        }
        return names
    }

    /// Läuft unter der AblageLauf-Sperre und dessen Abmeldekoordination. Keine Logs, kein Push, keine Engine-Aufrufe.
    static func abgleichen(hub: any AblageHub, speicher: AblageSpeicher, konto: AblageKonto, z: AblageZustand,
                           optionen: AblageLaufOptionen, bericht: inout AblageBericht) async throws {
        guard let wurzel = speicher.stimmenWurzel else { return }
        let fm = FileManager.default
        try wurzelPruefen(wurzel)
        var idx = try laden(wurzel, konto: konto.kennung)
        let vorher = idx
        var winners: [String: PhononStimme.Kandidat] = [:]
        for e in idx.stimmen {
            let k = try PhononStimme.kandidat(AblageDatei(nutzlast: e.datei), key: konto.key)
            if !tot(k.datei.id, sammlung: AblageLedger.sammlungDatei, z: z)
                && !tot(k.datei.ordner.wert, sammlung: AblageLedger.sammlungOrdner, z: z) { winners[k.uid] = k }
        }
        var incoming: [PhononStimme.Kandidat] = []
        for d in z.bestand.dateien.values where AblageSystem.istDatei(d.id) {
            let fp = AblageKrypto.sha256Hex(d.kanonischeNutzlast(ordnen: false))
            if idx.verworfen.contains(fp) || tot(d.id, sammlung: AblageLedger.sammlungDatei, z: z)
                || tot(d.ordner.wert, sammlung: AblageLedger.sammlungOrdner, z: z)
                || z.ledger.angehalten(AblageLedger.sammlungDatei, d.id)
                || z.ledger.angehalten(AblageLedger.sammlungOrdner, d.ordner.wert) { continue }
            guard z.bestand.ordner[d.ordner.wert] != nil else { continue }
            do { incoming.append(try PhononStimme.kandidat(d, key: konto.key)) }
            catch { idx.verworfen.insert(fp) }
        }
        var budget = AblageDownloadBudget(rest: optionen.downloadBudget)
        for k in incoming.sorted(by: { $1.kleiner(als: $0) }) {
            if let old = winners[k.uid], k.kleiner(als: old) { continue }
            if winners[k.uid] == nil && winners.count >= 200 { continue }
            let dest = wurzel.appendingPathComponent(blobName(k))
            let size = ((try? fm.attributesOfItem(atPath: dest.path))?[.size] as? NSNumber)?.uint64Value
            let cached = size == k.fassung.size ? (try? Data(contentsOf: dest)).map { [UInt8]($0) } : nil
            if let cached, (try? PhononStimme.pruefen(cached, sha256: k.fassung.blob)) != nil {
                winners[k.uid] = k
                continue
            }
            guard optionen.downloadsErlaubt, !optionen.mobil, budget.darfStarten(groesse: k.fassung.size) else { continue }
            let ziel = speicher.zwischenpfad()
            defer { speicher.zwischenEntfernen(ziel) }
            do {
                guard let hubId = AblageKrypto.hubBlobId(key: konto.key, sha256Hex: k.fassung.blob) else { throw PhononStimmenFehler.ungueltig }
                try await hub.blobLaden(hubId: hubId, groesse: k.fassung.size, ziel: ziel, bulk: optionen.bulk)
            } catch AblageHubFehler.abgemeldet { throw AblageLaufFehler.abgemeldet }
            catch is CancellationError { throw CancellationError() }
            catch AblageHubFehler.netz, AblageHubFehler.wlanRegel, AblageHubFehler.speicherVoll {
                bericht.downloadsUnterbrochen = true
                break
            } catch AblageHubFehler.http(let code, _) where code == 408 || code == 429 || code >= 500 {
                bericht.downloadsUnterbrochen = true
                break
            } catch {
                idx.verworfen.insert(k.fingerprint)
                continue
            }
            try Task.checkCancellation()
            budget.verbraucht(groesse: k.fassung.size)
            // Größe vor dem Einlesen begrenzen; niemals ungeprüfte Downloadbytes veröffentlichen.
            let attrs = try fm.attributesOfItem(atPath: ziel.path)
            guard (attrs[.size] as? NSNumber)?.uint64Value == k.fassung.size else { idx.verworfen.insert(k.fingerprint); continue }
            let bytes = [UInt8](try Data(contentsOf: ziel))
            do { try PhononStimme.pruefen(bytes, sha256: k.fassung.blob) }
            catch { idx.verworfen.insert(k.fingerprint); continue }
            try fm.createDirectory(at: wurzel, withIntermediateDirectories: true)
            try wurzelPruefen(wurzel)
            try Data(bytes).write(to: dest, options: .atomic)
            winners[k.uid] = k
        }
        try PhononStimmenInjektion.exklusiv {
            let ks = winners.values.sorted { $0.uid < $1.uid }
            let assigned = try namen(ks, modellNamen: modellNamen(wurzel))
            idx.stimmen = try ks.map { k in
                let display = assigned[k.uid]!
                return Eintrag(datei: k.datei.kanonischeNutzlast(ordnen: false), id: try PhononStimme.name(display), name: display)
            }
            try Task.checkCancellation()
            if idx != vorher || !fm.fileExists(atPath: wurzel.appendingPathComponent("stimmen.json").path) {
                try fm.createDirectory(at: wurzel, withIntermediateDirectories: true)
                try wurzelPruefen(wurzel)
                let encoder = JSONEncoder()
                encoder.outputFormatting = [.sortedKeys]
                try encoder.encode(idx).write(to: wurzel.appendingPathComponent("stimmen.json"), options: .atomic)
            }
            // Nach Absturz vor dem Indexwechsel ist nur eine unreferenzierte Datei übrig. Aufräumen ist wiederholbar.
            let keep = Set(ks.map(blobName))
            for url in (try? fm.contentsOfDirectory(at: wurzel, includingPropertiesForKeys: nil)) ?? []
                where istMasterName(url.lastPathComponent) && !keep.contains(url.lastPathComponent) { try fm.removeItem(at: url) }
            try PhononStimmenInjektion.abgleichen(wurzel: wurzel)
        }
    }
}
#endif
