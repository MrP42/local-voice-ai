import Foundation
#if canImport(CryptoKit)

/// Projektion geprüfter Masterdateien in die Modellordner; keine Engine und kein Netz (#898).
public enum PhononStimmenInjektion {
    private static let sperre = NSRecursiveLock()
    private static let marker = ".lv-eigene-stimmen.json"

    /// Bundle-Austausch, Indexwechsel und Injektion dürfen sich nicht überholen.
    public static func exklusiv<T>(_ body: () throws -> T) rethrows -> T {
        sperre.lock()
        defer { sperre.unlock() }
        return try body()
    }

    private static func regulaer(_ url: URL, typ: FileAttributeType) -> Bool {
        ((try? FileManager.default.attributesOfItem(atPath: url.path))?[.type] as? FileAttributeType) == typ
    }

    private static func hash(_ url: URL) -> String? {
        guard regulaer(url, typ: .typeRegular),
              let size = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? NSNumber,
              size.intValue <= PhononStimme.maxBytes,
              let data = try? Data(contentsOf: url) else { return nil }
        return AblageKrypto.sha256Hex([UInt8](data))
    }

    private static func eigene(in modell: URL) throws -> [String: Set<String>] {
        let url = modell.appendingPathComponent("voices/" + marker)
        guard FileManager.default.fileExists(atPath: url.path) else { return [:] }
        guard regulaer(url, typ: .typeRegular) else { throw PhononStimmenFehler.ungueltig }
        let entries = try JSONDecoder().decode([String: Set<String>].self, from: Data(contentsOf: url))
        guard entries.allSatisfy({ (try? PhononStimme.name($0.key)) == $0.key && !$0.value.isEmpty && $0.value.allSatisfy(AblageValidierung.hex64) }) else {
            throw PhononStimmenFehler.ungueltig
        }
        return entries
    }

    static func eigeneNamen(in modell: URL) throws -> Set<String> {
        Set(try eigene(in: modell).filter { id, hashes in
            guard let current = hash(modell.appendingPathComponent("voices/" + id + ".safetensors")) else { return false }
            return hashes.contains(current)
        }.keys)
    }

    /// Tatsächlicher Ordnerinhalt statt der beim Paketbau eingefrorenen bundle.json-Liste.
    public static func stimmen(in modell: URL) -> [String] {
        let urls = (try? FileManager.default.contentsOfDirectory(at: modell.appendingPathComponent("voices"), includingPropertiesForKeys: nil)) ?? []
        return urls.filter { $0.pathExtension == "safetensors" && regulaer($0, typ: .typeRegular) }
            .map { $0.deletingPathExtension().lastPathComponent }.filter { $0 != "default" }.sorted()
    }

    /// Gibt nur Änderungen an Stimmdateien zurück. Fehlender Index entfernt die eigenen Projektionen (Abmelden).
    @discardableResult
    public static func abgleichen(wurzel: URL) throws -> Bool {
        try exklusiv {
            let fm = FileManager.default, parent = wurzel.deletingLastPathComponent()
            let indexURL = wurzel.appendingPathComponent("stimmen.json")
            var desired: [String: [String: (sha: String, bytes: Data)]] = [:]
            var unvollstaendig = Set<String>()
            if fm.fileExists(atPath: indexURL.path) {
                let index = try JSONDecoder().decode(PhononStimmenSpeicher.Index.self, from: Data(contentsOf: indexURL))
                guard index.version == 1 else { throw PhononStimmenFehler.ungueltig }
                for entry in index.stimmen {
                    guard (try? PhononStimme.name(entry.id)) == entry.id,
                          let d = try? AblageDatei(nutzlast: entry.datei),
                          case .array(let meta)? = AblageJSON.object(d.extra)["phonon_voice"], meta.count == 2,
                          let uid: String = try? meta[1].pflicht("uid"), uid.utf8.count == 32,
                          uid.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { continue }
                    let heads = AblageZusammenfuehren.koepfe(d)
                    guard let f = d.fassungen.filter({ heads.contains($0.fas) }).max(by: {
                        if $0.zeit != $1.zeit { return $0.zeit < $1.zeit }
                        if $0.blob != $1.blob { return $0.blob < $1.blob }
                        return $0.fas < $1.fas
                    }), AblageValidierung.hex64(f.blob) else { continue }
                    // Die Auswahl braucht hier keinen Kontoschlüssel; der Speicher hat ihn beim Pull geprüft.
                    let k = PhononStimme.Kandidat(datei: d, updated: 0, generation: 0, uid: uid,
                        id: entry.id, name: entry.name, created: 0, fassung: f, fingerprint: "")
                    for language in PhononStimme.modelle.keys.sorted() {
                        guard let emb = try? PhononStimme.embedding(k, language: language) else {
                            unvollstaendig.insert(language); continue
                        }
                        let source = wurzel.appendingPathComponent(uid + "-" + emb.sha256 + ".safetensors")
                        guard hash(source) == emb.sha256, let data = try? Data(contentsOf: source),
                              (try? PhononStimme.pruefen([UInt8](data), sha256: emb.sha256)) != nil else {
                            unvollstaendig.insert(language); continue
                        }
                        desired[language, default: [:]][entry.id] = (emb.sha256, data)
                    }
                }
            }
            var changed = false
            for language in ["de", "en"] {
                let model = parent.appendingPathComponent("models-" + language), voices = model.appendingPathComponent("voices")
                guard regulaer(model, typ: .typeDirectory), regulaer(voices, typ: .typeDirectory) else { continue }
                let old = try eigene(in: model), wanted = desired[language] ?? [:]
                var next: [String: String] = [:]
                var copies: [String: Data] = [:]
                let existing = stimmen(in: model)
                var reserved = Set(wanted.keys.map { $0.lowercased() })
                for base in wanted.keys.sorted() {
                    let value = wanted[base]!
                    var id = base, n = 1
                    while true {
                        let dest = voices.appendingPathComponent(id + ".safetensors")
                        let owned = hash(dest).map { old[id]?.contains($0) == true } ?? false
                        let occupied = fm.fileExists(atPath: dest.path) || existing.contains { $0.lowercased() == id.lowercased() }
                        if !occupied || owned { break }
                        // Auch neue Bundle-Stimmen und nachträglich fremd ersetzte Dateien schützen.
                        repeat {
                            let suffix = n == 1 ? "-eigen" : "-eigen-\(n)"
                            id = String(base.prefix(40 - suffix.count)) + suffix
                            n += 1
                        } while reserved.contains(id.lowercased())
                    }
                    next[id] = value.sha
                    copies[id] = value.bytes
                    reserved.insert(id.lowercased())
                }
                let encoder = JSONEncoder()
                encoder.outputFormatting = [.sortedKeys]
                let markerURL = voices.appendingPathComponent(marker)
                // Besitzabsicht VOR der atomaren Kopie: nach Absturz lässt sich auch diese Kopie entfernen.
                var pending = old
                for (id, sha) in next { pending[id, default: []].insert(sha) }
                if pending != old { try encoder.encode(pending).write(to: markerURL, options: .atomic) }
                for (id, hashes) in old where next[id] == nil {
                    // Fehlende emb-Zusätze oder lokale Bytes sind keine Löschanweisung.
                    if unvollstaendig.contains(language) { continue }
                    let dest = voices.appendingPathComponent(id + ".safetensors")
                    if let current = hash(dest), hashes.contains(current) { try fm.removeItem(at: dest); changed = true }
                }
                for (id, sha) in next {
                    let dest = voices.appendingPathComponent(id + ".safetensors")
                    if hash(dest) != sha { try copies[id]!.write(to: dest, options: .atomic); changed = true }
                }
                var final = next.mapValues { Set([$0]) }
                if unvollstaendig.contains(language) { for (id, hashes) in old where final[id] == nil { final[id] = hashes } }
                if final != pending { try encoder.encode(final).write(to: markerURL, options: .atomic) }
            }
            return changed
        }
    }
}
#endif
