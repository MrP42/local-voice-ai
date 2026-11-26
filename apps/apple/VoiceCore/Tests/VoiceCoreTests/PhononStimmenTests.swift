import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)

private typealias PW = AblageWelt

// Swift-/Rust-Gegenproben für den anschließenden Mac-/Thread-Claude-Lauf:
// - Größen-/Hash-/Kopf-/Offset-/Namensprüfung einzeln entfernen -> jeweilige Ablehnung in den ersten Tests rot.
// - Kandidat.kleiner: Generation vor updated/SHA -> LWW- und verspätete-Generation-Tests rot.
// - AblageSystem-Filter entfernen -> Systembaum-/Index-/Detail-/Originaltest rot.
// - tot ignorieren / Verwerfliste ignorieren -> Grabstein- bzw. Einmalig-Tests rot.
// - Cache-Wiederaufnahme entfernen -> Wiederanlauftest rot; Indexreihenfolge prüft der Python-Vertrag.
final class PhononStimmenTests: XCTestCase {
    static let uid = "00112233445566778899aabbccddeeff"
    static let hash = "143e40a6fe706240586e1fa8f426d40b81d9e05546f2d21c20bd53feb3449b87"
    static let header = #"{"speaker_wavs":{"dtype":"F32","shape":[1,512,125],"data_offsets":[0,256000]},"__metadata__":{"sig":"ba7a5a45","epoch":"300"}}"#

    /// Bitgleicher Null-Tensorvektor in Rust phonon_tests und Python-Vertrag; keine echte Stimme.
    static func bytes(header: String = PhononStimmenTests.header, payload: Int = 256000) -> [UInt8] {
        var h = Array(header.utf8)
        while h.count % 8 != 0 { h.append(32) }
        return (0..<8).map { UInt8(truncatingIfNeeded: UInt64(h.count) >> ($0 * 8)) } + h + [UInt8](repeating: 0, count: payload)
    }

    private func replacing(_ j: AblageJSON, _ key: String, _ value: AblageJSON) -> AblageJSON {
        guard case .object(let m) = j else { return j }
        return .objekt(m.filter { $0.key != key }.map { ($0.key, $0.value) } + [(key, value)])
    }

    private func datei(uid: String = PhononStimmenTests.uid, generation: UInt32 = 0, updated: Int64 = 1000,
                       name: String = "Probe", language: String = "de", bytes: [UInt8] = PhononStimmenTests.bytes()) -> AblageDatei {
        let sha = AblageKrypto.sha256Hex(bytes)
        let folder = AblageSystem.kennung(key: PW.key, ordner: true, generation: generation)!
        let f = AblageFassung(fas: PW.fasId(1), vorgaenger: nil, blob: sha, size: UInt64(bytes.count), originalSha256: sha,
                             typ: "lvstimme", geraet: PW.geraet, zeit: updated, texte: [])
        var d = PW.datei(1, ordner: folder, name: "voice.safetensors", fassungen: [f], klasse: "internal")
        d.id = AblageSystem.kennung(key: PW.key, ordner: false, generation: generation, salt: uid)!
        let meta = AblageJSON.objekt([
            ("id", .string(name)), ("uid", .string(uid)), ("name", .string(name)), ("language", .string(language)),
            ("origin", .string("import")), ("voice_id", .string("")), ("source_note", .null), ("consent_at", .null),
            ("created", .string("2026-11-25T00:00:00Z")), ("sha256", .string(sha)), ("fas", .string(f.fas)),
            ("generation", .uint(UInt64(generation))),
        ])
        d.extra = [.init(key: "phonon_voice", value: .array([.string(String(format: "%020lld", updated)), meta])),
                   .init(key: "phonon_name", value: .array([.string(String(format: "%020lld", updated)), .string(name), .string(name)]))]
        return d
    }

    private func ordner(_ d: AblageDatei) -> AblageOrdner {
        var o = PW.ordner(1, name: ".lv-stimmen", klasse: "internal")
        o.id = d.ordner.wert
        return o
    }

    private func umgebung() throws -> (URL, AblageSpeicher) {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: root) }
        return (root, AblageSpeicher(wurzel: root.appendingPathComponent("Ablage"),
                                    stimmenWurzel: root.appendingPathComponent("PhononTTS/eigene-stimmen")))
    }

    func testKennungsvektorenUndGegenprobeNormalerName() {
        XCTAssertEqual(AblageSystem.kennung(key: PW.key, ordner: true, generation: 0), "ord_2CAS9MY90GTRQEBVKFKP5TXZAB")
        XCTAssertEqual(AblageSystem.kennung(key: PW.key, ordner: true, generation: 1), "ord_2CAS9MYNV00X4X0TY6MQE0ECCH")
        XCTAssertEqual(AblageSystem.kennung(key: PW.key, ordner: false, generation: 0, salt: Self.uid), "dat_2CAS9MD268N1T7K9V0N4GCDVT8")
        XCTAssertTrue(AblageSystem.istOrdner("ord_2cas9my90gtrqebvkfkp5txzab"))
        XCTAssertTrue(AblageSystem.istDatei(datei().id))
        XCTAssertFalse(AblageSystem.istOrdner(".lv-stimmen"))
        XCTAssertFalse(AblageSystem.istOrdner(PW.ordId(1)))
        XCTAssertFalse(AblageSystem.istDatei("dat_../../voice"))
    }

    func testGueltigerVertragsvektor() throws {
        let b = Self.bytes()
        XCTAssertEqual(b.count, 256136)
        XCTAssertEqual(AblageKrypto.sha256Hex(b), Self.hash)
        XCTAssertNoThrow(try PhononStimme.pruefen(b, sha256: Self.hash))
        XCTAssertEqual(try PhononStimme.kandidat(datei(), key: PW.key).uid, Self.uid)
        var kaputt = b
        kaputt[kaputt.count - 1] = 1
        XCTAssertThrowsError(try PhononStimme.pruefen(kaputt, sha256: Self.hash))
    }

    func testGeraeteUndDesktopKopfOhneFesteGesamtgroesse() throws {
        let device = Self.bytes(header: #"{"speaker_wavs":{"dtype":"F32","shape":[1,512,125],"data_offsets":[0,256000]}}"#)
        for (bytes, size) in [(device, 256088), (Self.bytes(), 256136)] {
            XCTAssertEqual(bytes.count, size)
            XCTAssertNoThrow(try PhononStimme.pruefen(bytes, sha256: AblageKrypto.sha256Hex(bytes)))
            XCTAssertThrowsError(try PhononStimme.pruefen(Array(bytes.dropLast()), sha256: AblageKrypto.sha256Hex(Array(bytes.dropLast()))))
        }
    }

    // Gegenproben: Injektionsaufruf, Sprachfilter, Hashschutz bzw. voices/-Leser entfernen -> diese Tests rot.
    func testInjektionNachPullNeuemBuiltUndGrabstein() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        let model = root.appendingPathComponent("PhononTTS/models-de"), voices = model.appendingPathComponent("voices")
        try FileManager.default.createDirectory(at: voices, withIntermediateDirectories: true)
        let modelVoice = voices.appendingPathComponent("Modell.safetensors")
        try Data([42]).write(to: modelVoice)
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let own = voices.appendingPathComponent("Probe.safetensors")
        XCTAssertEqual(try Data(contentsOf: own), Data(Self.bytes()))
        // PhononModels.install ersetzt bei neuem built den gesamten Zielordner, einschließlich Marker.
        try FileManager.default.removeItem(at: model)
        try FileManager.default.createDirectory(at: voices, withIntermediateDirectories: true)
        try Data(#"{"built":2,"voices":["default"]}"#.utf8).write(to: model.appendingPathComponent("bundle.json"))
        try Data([43]).write(to: modelVoice)
        XCTAssertTrue(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        XCTAssertEqual(try Data(contentsOf: own), Data(Self.bytes()))
        XCTAssertFalse(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).stimmen.first?.id, "Probe")
        hub.stelle(PW.remoteGrab(collection: AblageLedger.sammlungDatei, id: d.id, revision: 2))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertFalse(FileManager.default.fileExists(atPath: own.path))
        XCTAssertEqual(try Data(contentsOf: modelVoice), Data([43]))
    }

    func testInjektionSprachenKollisionDefekteBytesUndAbmelden() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau()
        let parent = root.appendingPathComponent("PhononTTS")
        for lang in ["de", "en"] {
            try FileManager.default.createDirectory(at: parent.appendingPathComponent("models-\(lang)/voices"), withIntermediateDirectories: true)
        }
        let en = datei(name: "English", language: "en")
        let unknown = datei(uid: String(repeating: "f", count: 32), name: "Unbekannt", language: "und")
        hub.stelle(PW.remote(ordner: ordner(en)))
        for d in [en, unknown] { hub.stelle(PW.remote(datei: d)) }
        _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let voice = parent.appendingPathComponent("models-en/voices/English.safetensors")
        XCTAssertEqual(try Data(contentsOf: voice), Data(Self.bytes()))
        XCTAssertEqual(PhononStimmenInjektion.stimmen(in: parent.appendingPathComponent("models-en")), ["English"])
        XCTAssertEqual(PhononStimmenInjektion.stimmen(in: parent.appendingPathComponent("models-de")), [])
        // Fremde Datei am zuvor injizierten Pfad: weder überschreiben noch beim Abmelden löschen.
        try Data([99]).write(to: voice, options: .atomic)
        XCTAssertTrue(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        XCTAssertEqual(try Data(contentsOf: voice), Data([99]))
        XCTAssertEqual(PhononStimmenInjektion.stimmen(in: parent.appendingPathComponent("models-en")), ["English", "English-eigen"])
        await AblageDienst(speicher: sp).verlassen()
        XCTAssertEqual(try Data(contentsOf: voice), Data([99]))
        XCTAssertFalse(FileManager.default.fileExists(atPath: parent.appendingPathComponent("models-en/voices/English-eigen.safetensors").path))
    }

    func testInjektionValidiertMasterUndFiltertDefault() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        let model = root.appendingPathComponent("PhononTTS/models-de"), voices = model.appendingPathComponent("voices")
        try FileManager.default.createDirectory(at: voices, withIntermediateDirectories: true)
        for name in ["default.safetensors", "Fremd.safetensors", "ignorieren.txt"] {
            try Data([42]).write(to: voices.appendingPathComponent(name))
        }
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(PhononStimmenInjektion.stimmen(in: model), ["Fremd", "Probe"])
        let k = try PhononStimme.kandidat(d, key: PW.key)
        try Data([0]).write(to: sp.stimmenWurzel!.appendingPathComponent(PhononStimmenSpeicher.blobName(k)), options: .atomic)
        XCTAssertTrue(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        XCTAssertEqual(PhononStimmenInjektion.stimmen(in: model), ["Fremd"])
        XCTAssertEqual(try Data(contentsOf: voices.appendingPathComponent("Fremd.safetensors")), Data([42]))
    }

    func testNeuesModellMitNamenskollisionUndAbsturzVorKopie() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        // Pull ohne Modell: der spätere Scan muss trotzdem injizieren können.
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let model = root.appendingPathComponent("PhononTTS/models-de"), voices = model.appendingPathComponent("voices")
        try FileManager.default.createDirectory(at: voices, withIntermediateDirectories: true)
        let foreign = voices.appendingPathComponent("Probe.safetensors")
        try Data([42]).write(to: foreign)
        XCTAssertTrue(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        XCTAssertEqual(try Data(contentsOf: foreign), Data([42]))
        let own = voices.appendingPathComponent("Probe-eigen.safetensors")
        XCTAssertEqual(try Data(contentsOf: own), Data(Self.bytes()))
        // Besitzabsicht und alter/neuer Hash stehen schon, die atomare Kopie ist noch nicht passiert.
        try FileManager.default.removeItem(at: own)
        try JSONEncoder().encode(["Probe-eigen": [Self.hash, String(repeating: "a", count: 64)]])
            .write(to: voices.appendingPathComponent(".lv-eigene-stimmen.json"), options: .atomic)
        XCTAssertTrue(try PhononStimmenInjektion.abgleichen(wurzel: sp.stimmenWurzel!))
        XCTAssertEqual(try Data(contentsOf: own), Data(Self.bytes()))
        await AblageDienst(speicher: sp).verlassen()
        XCTAssertFalse(FileManager.default.fileExists(atPath: own.path))
        XCTAssertEqual(try Data(contentsOf: foreign), Data([42]))
    }

    func testJedeBytePruefregelMitGueltigerGegenprobe() throws {
        XCTAssertNoThrow(try PhononStimme.pruefen(Self.bytes(), sha256: Self.hash))
        var tooLongHeader = Self.bytes()
        tooLongHeader.replaceSubrange(0..<8, with: [UInt8](repeating: 255, count: 8))
        let h = Self.header
        let invalid = [
            [UInt8](repeating: 0, count: 16383), [UInt8](repeating: 0, count: 4 * 1024 * 1024 + 1), tooLongHeader,
            Self.bytes(header: "[]"), Self.bytes(header: "ungültig"),
            Self.bytes(header: h.replacingOccurrences(of: "speaker_wavs", with: "fremder_tensor")),
            Self.bytes(header: h.replacingOccurrences(of: "F32", with: "I32")),
            Self.bytes(header: h.replacingOccurrences(of: "F32", with: "ZZZ")),
            Self.bytes(header: h.replacingOccurrences(of: "F32", with: "F16"), payload: 128000),
            Self.bytes(header: h.replacingOccurrences(of: "[1,512,125]", with: "[1,125,512]")),
            Self.bytes(header: h.replacingOccurrences(of: "[0,256000]", with: "[1,256000]")),
            Self.bytes(header: h.replacingOccurrences(of: "[0,256000]", with: "[0,256004]")),
            Self.bytes(header: h.replacingOccurrences(of: "[1,512,125]", with: "[18446744073709551615,512,125]")),
            Self.bytes(header: h.replacingOccurrences(of: #""epoch":"300""#, with: #""epoch":300"#)),
            Self.bytes(payload: 256001), Self.bytes(payload: 255999),
            Self.bytes(header: h.replacingOccurrences(of: "[0,256000]", with: "[0]")),
            Self.bytes(header: h.replacingOccurrences(of: "[1,512,125]", with: "[-1,512,125]")),
        ]
        for (i, b) in invalid.enumerated() {
            XCTAssertThrowsError(try PhononStimme.pruefen(b, sha256: AblageKrypto.sha256Hex(b)), "Regel \(i)")
        }
        XCTAssertThrowsError(try PhononStimme.pruefen(Self.bytes(), sha256: Self.hash.uppercased()))
        let extra = #""padding":{"dtype":"U8","shape":[8],"data_offsets":[0,8]},"#
        let two = "{" + extra + String(h.dropFirst()).replacingOccurrences(of: "[0,256000]", with: "[8,256008]")
        let good = Self.bytes(header: two, payload: 256008)
        XCTAssertNoThrow(try PhononStimme.pruefen(good, sha256: AblageKrypto.sha256Hex(good)))
        for changed in [two.replacingOccurrences(of: "[0,8]", with: "[1,9]"),
                        two.replacingOccurrences(of: "[8,256008]", with: "[0,256000]")] {
            let bad = Self.bytes(header: changed, payload: 256008)
            XCTAssertThrowsError(try PhononStimme.pruefen(bad, sha256: AblageKrypto.sha256Hex(bad)))
        }
    }

    func testNamenUndMetadatenMitGegenproben() throws {
        XCTAssertEqual(try PhononStimme.name("Jörg (Test)"), "Joerg-Test")
        for bad in ["", "../Probe", "Probe/anderer", "Probe\\anderer", " Probe", "Probe  Test", ".Probe", "Probe.",
                    "CON", "nul", "default", "COM0", "LPT9", "---", "Test\nName", String(repeating: "a", count: 41)] {
            XCTAssertThrowsError(try PhononStimme.name(bad), bad)
        }
        let good = datei()
        XCTAssertNoThrow(try PhononStimme.kandidat(good, key: PW.key))
        for (time, delta) in [("2026-11-25T00:00:00.001Z", Int64(1)), ("2026-11-25T00:00:00.0009Z", Int64(0))] {
            var d = good
            guard case .array(let v) = d.extra[0].value else { return XCTFail() }
            d.extra[0].value = .array([v[0], replacing(v[1], "created", .string(time))])
            XCTAssertEqual(try PhononStimme.kandidat(d, key: PW.key).created, try PhononStimme.kandidat(good, key: PW.key).created + delta)
        }
        for (key, bad) in [("id", AblageJSON.string("../Probe")), ("uid", .string("not-a-uid")),
                           ("id", .string("K")),
                           ("name", .string("Andere")), ("language", .string("xx")), ("origin", .string("model")),
                           ("created", .string("kein Datum")), ("sha256", .string(Self.hash.uppercased())),
                           ("fas", .string(PW.fasId(2))), ("generation", .uint(1)), ("generation", .uint(UInt64.max)),
                           ("voice_id", .uint(1)), ("source_note", .uint(1)), ("consent_at", .bool(true))] {
            var d = good
            guard case .array(let v) = d.extra[0].value else { return XCTFail() }
            d.extra[0].value = .array([v[0], replacing(v[1], key, bad)])
            XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key), key)
        }
        for badRank in ["1", "-0000000000000000001", "99999999999999999999"] {
            var d = good
            guard case .array(let v) = d.extra[0].value else { return XCTFail() }
            d.extra[0].value = .array([.string(badRank), v[1]])
            XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        }
        var d = good
        d.extra[1].value = .array([.string("00000000000000001000"), .string("Andere"), .string("Probe")])
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.id = PW.datId(1)
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.ordner.wert = PW.ordId(1)
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.fassungen[0].typ = "pdf"
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.fassungen[0].originalSha256 = String(repeating: "a", count: 64)
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.fassungen[0].size = 1
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.fassungen[0].texte = [PW.text(zu: d.fassungen[0]).eintrag]
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.papierkorb.wert = 1000
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
        d = good; d.fassungen = []
        XCTAssertThrowsError(try PhononStimme.kandidat(d, key: PW.key))
    }

    func testLWWVorGenerationUndDateikennung() throws {
        let first = try PhononStimme.kandidat(datei(updated: 2000), key: PW.key)
        let old = try PhononStimme.kandidat(datei(generation: 1, updated: 1000), key: PW.key)
        XCTAssertTrue(old.kleiner(als: first)) // Gegenprobe: Generation zuerst verliert.
        let next = try PhononStimme.kandidat(datei(generation: 1, updated: 2000), key: PW.key)
        XCTAssertTrue(first.kleiner(als: next))
        var biggerHash = old
        biggerHash.updated = first.updated
        biggerHash.fassung.blob = String(repeating: "f", count: 64)
        XCTAssertTrue(next.kleiner(als: biggerHash))
        var biggerFile = next
        biggerFile.datei.id += "Z"
        XCTAssertTrue(next.kleiner(als: biggerFile))
    }

    func testFassungskoepfeWaehlenZeitBlobFasUndNamensregister() throws {
        var d = datei()
        var second = d.fassungen[0]
        second.fas = PW.fasId(2); second.zeit += 1
        d.fassungen.append(second)
        d.extra[1].value = .array([.string("00000000000000002000"), .string("Neu"), .string("Neu")])
        let k = try PhononStimme.kandidat(d, key: PW.key)
        XCTAssertEqual(k.fassung.fas, second.fas)
        XCTAssertEqual(k.updated, 1001)
        XCTAssertEqual(k.name, "Neu")
    }

    func testSystembaumSucheDetailsOriginaleVerborgenOhneBestandZuAendern() throws {
        let d = datei(), o = ordner(d)
        var child = PW.ordner(2, eltern: o.id)
        var hiddenFassung = PW.fassung(2, nil, inhalt: "synthetisch")
        let hiddenText = PW.text(zu: hiddenFassung)
        hiddenFassung.texte = [hiddenText.eintrag]
        var ordinary = PW.datei(2, ordner: child.id, fassungen: [hiddenFassung])
        let normal = PW.ordner(3, name: ".lv-stimmen", klasse: "internal")
        let visible = PW.datei(3, ordner: normal.id, fassungen: [PW.fassung(3, nil, inhalt: "sichtbar")])
        var z = PW.neuerZustand()
        PW.einspielen(&z, ordner: [PW.remote(ordner: o), PW.remote(ordner: child), PW.remote(ordner: normal)],
                      dateien: [PW.remote(datei: d), PW.remote(datei: ordinary), PW.remote(datei: visible)])
        let before = z
        let u = AblageAnzeige.uebersicht(z, vorhanden: AblageVorhanden())
        XCTAssertEqual(u.ordner.map(\.id), [normal.id])
        XCTAssertEqual(u.dateien.map(\.id), [visible.id])
        XCTAssertFalse(AblageBaum.zeilen(u).isEmpty)
        for id in [d.id, ordinary.id] {
            XCTAssertFalse(AblageAnzeige.darfIndiziert(z, dat: id))
            XCTAssertNil(AblageAnzeige.detail(z, dat: id, vorhanden: AblageVorhanden()))
            XCTAssertNil(AblageDownloadPlan.original(zustand: z, dat: id, fas: PW.fasId(1), key: PW.key))
        }
        XCTAssertEqual(AblageAnzeige.lebendeDateien(z), [visible.id])
        var vorhanden = AblageVorhanden()
        vorhanden.setzeText(dat: ordinary.id, textrev: hiddenText.eintrag.textrev)
        XCTAssertFalse(AblageAnzeige.indexQuellen(z, vorhanden: vorhanden).contains(ordinary.id))
        XCTAssertEqual(z, before)
        // Auch ohne Elternobjekt bleibt die Systemdatei verborgen; Roh-Elternzyklen dürfen nicht freigeben.
        z.bestand.ordner = AblageOrdnerbestand()
        XCTAssertTrue(AblageSystem.datei(d.id, in: z.bestand))
        child.eltern.wert = o.id
        z.bestand.ordner[child.id] = child
        ordinary.ordner.wert = child.id
        z.bestand.dateien[ordinary.id] = ordinary
        XCTAssertTrue(AblageSystem.datei(ordinary.id, in: z.bestand))
    }

    func testNamensprojektionReserviertModellUndBasisnamen() throws {
        let a = try PhononStimme.kandidat(datei(name: "Anton"), key: PW.key)
        let b = try PhononStimme.kandidat(datei(uid: String(repeating: "f", count: 32), name: "Anton"), key: PW.key)
        let c = try PhononStimme.kandidat(datei(uid: String(repeating: "e", count: 32), name: "Anton-eigen"), key: PW.key)
        let names = try PhononStimmenSpeicher.namen([b, c, a], modellNamen: [])
        XCTAssertEqual(names[a.uid], "Anton-eigen-2")
        XCTAssertEqual(names[b.uid], "Anton-eigen-3")
        XCTAssertEqual(names[c.uid], "Anton-eigen")
        let probe = try PhononStimme.kandidat(datei(), key: PW.key)
        XCTAssertEqual(try PhononStimmenSpeicher.namen([probe], modellNamen: ["pRoBe"])[probe.uid], "Probe-eigen")
        XCTAssertEqual(try PhononStimmenSpeicher.namen([probe], modellNamen: [])[probe.uid], "Probe")
        let duplicate = try PhononStimme.kandidat(datei(uid: String(repeating: "e", count: 32)), key: PW.key)
        let pair = try PhononStimmenSpeicher.namen([duplicate, probe], modellNamen: [])
        XCTAssertEqual(pair[probe.uid], "Probe")
        XCTAssertEqual(pair[duplicate.uid], "Probe-2", "gleiche Namen/Bytes bleiben zwei UIDs")
        let long = try PhononStimme.kandidat(datei(name: String(repeating: "a", count: 40)), key: PW.key)
        let display = try XCTUnwrap(PhononStimmenSpeicher.namen([long], modellNamen: [long.id])[long.uid])
        XCTAssertEqual(display.count, 40)
        XCTAssertTrue(display.hasSuffix("-eigen"))
        var blocked = probe
        blocked.datei.extra.append(.init(key: "phonon_blocked_probe", value: .bool(true)))
        XCTAssertEqual(try PhononStimmenSpeicher.namen([blocked], modellNamen: [])[probe.uid], "Probe-eigen")
    }

    func testPullImportGrabsteinWiederanlaufUndAbmelden() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d))
        _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let root = sp.stimmenWurzel!, k = try PhononStimme.kandidat(d, key: PW.key)
        let file = root.appendingPathComponent(PhononStimmenSpeicher.blobName(k))
        XCTAssertEqual(try Data(contentsOf: file), Data(Self.bytes()))
        XCTAssertEqual(try PhononStimmenSpeicher.laden(root, konto: PW.konto.kennung).stimmen.count, 1)
        XCTAssertEqual(hub.blobAufrufe.count, 1, "ohne Öffnen automatisch laden")
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(hub.blobAufrufe.count, 1, "Wiederanlauf lädt nichts erneut")
        hub.stelle(PW.remoteGrab(collection: AblageLedger.sammlungDatei, id: d.id, revision: 2))
        var opts = AblageLaufOptionen(); opts.downloadsErlaubt = false
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto, optionen: opts)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertTrue(try PhononStimmenSpeicher.laden(root, konto: PW.konto.kennung).stimmen.isEmpty)
        await AblageDienst(speicher: sp).verlassen()
        XCTAssertFalse(FileManager.default.fileExists(atPath: root.path))
    }

    func testUngueltigeBytesEinmaligNetzfehlerWiederholbar() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau()
        let bad = Self.bytes(header: Self.header.replacingOccurrences(of: "speaker_wavs", with: "wrong_tensor"))
        let d = datei(bytes: bad)
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(bad)
        hub.blobHook = { _ in throw AblageHubFehler.netz }
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertTrue(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).verworfen.isEmpty)
        hub.blobHook = nil
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(hub.blobAufrufe.count, 2)
        let idx = try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung)
        XCTAssertEqual(idx.verworfen.count, 1)
        XCTAssertTrue(idx.stimmen.isEmpty)
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(hub.blobAufrufe.count, 2)
        XCTAssertFalse(sp.laden(konto: PW.konto.kennung).zustand.ledger.begraben.contains(d.id))
    }

    func testSpaetereGenerationVerdraengtKeinenNeuerenStandUndGrabWechseltAufLebenden() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau()
        var other = Self.bytes(); other[other.count - 1] = 1
        let newer = datei(updated: 2000), older = datei(generation: 1, updated: 1000, bytes: other)
        hub.stelle(PW.remote(ordner: ordner(newer))); hub.stelle(PW.remote(datei: newer)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        hub.stelle(PW.remote(ordner: ordner(older))); hub.stelle(PW.remote(datei: older)); _ = hub.blob(other)
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        var idx = try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung)
        var winner = try PhononStimme.kandidat(AblageDatei(nutzlast: XCTUnwrap(idx.stimmen.first).datei), key: PW.key)
        XCTAssertEqual(winner.datei.id, newer.id)
        XCTAssertEqual(hub.blobAufrufe.count, 1, "Verlierer nicht laden")
        // Ein Ordnergrab ist endgültig, auch wenn der Ordner wegen seiner Datei intern noch gehalten wird.
        hub.stelle(PW.remoteGrab(collection: AblageLedger.sammlungOrdner, id: newer.ordner.wert, revision: 2))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        idx = try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung)
        winner = try PhononStimme.kandidat(AblageDatei(nutzlast: XCTUnwrap(idx.stimmen.first).datei), key: PW.key)
        XCTAssertEqual(winner.datei.id, older.id)
        XCTAssertFalse(FileManager.default.fileExists(atPath: sp.stimmenWurzel!.appendingPathComponent(PhononStimmenSpeicher.blobName(try PhononStimme.kandidat(newer, key: PW.key))).path))
    }

    func testAbbruchZwischenBytesUndIndexHeiltUndModellDateiBleibtUnveraendert() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei(name: "Probe")
        let k = try PhononStimme.kandidat(d, key: PW.key), dir = sp.stimmenWurzel!
        let model = root.appendingPathComponent("PhononTTS/models-de/voices", isDirectory: true)
        try FileManager.default.createDirectory(at: model, withIntermediateDirectories: true)
        let foreign = model.appendingPathComponent("Probe.safetensors"), foreignBytes = Data("synthetische fremde Datei".utf8)
        try foreignBytes.write(to: foreign)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        // Zustand wie nach beendetem Byte-Schritt und vor atomarem Indexwechsel.
        try Data(Self.bytes()).write(to: dir.appendingPathComponent(PhononStimmenSpeicher.blobName(k)), options: .atomic)
        let unused = dir.appendingPathComponent(String(repeating: "e", count: 32) + "-" + Self.hash + ".safetensors")
        try Data(Self.bytes()).write(to: unused)
        let unknown = dir.appendingPathComponent("fremde-datei.safetensors")
        try foreignBytes.write(to: unknown)
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        let idx = try PhononStimmenSpeicher.laden(dir, konto: PW.konto.kennung)
        XCTAssertEqual(idx.stimmen.first?.id, "Probe-eigen")
        XCTAssertEqual(try Data(contentsOf: foreign), foreignBytes)
        XCTAssertEqual(try Data(contentsOf: unknown), foreignBytes)
        XCTAssertFalse(FileManager.default.fileExists(atPath: unused.path))
    }

    func testMobilHaeltDownloadsZurueckUndMetadatenfehlerLadenKeinenBlob() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau(), good = datei()
        var opts = AblageLaufOptionen(); opts.mobil = true; opts.bulk = true
        hub.stelle(PW.remote(ordner: ordner(good))); hub.stelle(PW.remote(datei: good)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto, optionen: opts)
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        opts.mobil = false
        hub.blobHook = { _ in throw AblageHubFehler.http(503, "unavailable") }
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto, optionen: opts)
        XCTAssertTrue(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).verworfen.isEmpty)
        hub.blobHook = nil
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto, optionen: opts)
        XCTAssertEqual(hub.blobAufrufe.count, 2)
        XCTAssertTrue(hub.blobAufrufe.last!.bulk)
        let bad = datei(uid: String(repeating: "e", count: 32), name: "default")
        hub.stelle(PW.remote(datei: bad))
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto, optionen: opts)
        XCTAssertEqual(hub.blobAufrufe.count, 2)
        XCTAssertEqual(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).verworfen.count, 1)
    }

    func testZuGrosseStueckantwortEinmaligVerworfen() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d))
        // HTTP-Adapter bildet den festen Clientfehler auf beschaedigt ab.
        hub.blobHook = { _ in throw AblageHubFehler.beschaedigt }
        for _ in 0..<2 { _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto) }
        XCTAssertEqual(hub.blobAufrufe.count, 1)
        XCTAssertEqual(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).verworfen.count, 1)
    }

    func testVerknuepfterStimmenordnerWirdNichtBeschrieben() async throws {
        let (root, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        let foreign = root.appendingPathComponent("fremd", isDirectory: true)
        try FileManager.default.createDirectory(at: foreign, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: sp.stimmenWurzel!.deletingLastPathComponent(), withIntermediateDirectories: true)
        try FileManager.default.createSymbolicLink(at: sp.stimmenWurzel!, withDestinationURL: foreign)
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        let report = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertTrue(report.downloadsUnterbrochen)
        XCTAssertEqual(hub.blobAufrufe.count, 0)
        XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: foreign.path).isEmpty)
    }

    func testKontowechselEntferntAlteStimmbytes() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let file = sp.stimmenWurzel!.appendingPathComponent(PhononStimmenSpeicher.blobName(try PhononStimme.kandidat(d, key: PW.key)))
        XCTAssertTrue(FileManager.default.fileExists(atPath: file.path))
        let other = AblageKonto(key: PW.andererKey, userId: PW.userId, hubUrl: PW.hubUrl)
        _ = try await AblageLauf.ausfuehren(hub: AblageHubNachbau(), speicher: sp, konto: other)
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
        XCTAssertTrue(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: other.kennung).stimmen.isEmpty)
    }

    func testAbgebrochenerDownloadLaesstBisherigenIndexUndBytesNutzbar() async throws {
        let (_, sp) = try umgebung(), hub = AblageHubNachbau(), d = datei()
        hub.stelle(PW.remote(ordner: ordner(d))); hub.stelle(PW.remote(datei: d)); _ = hub.blob(Self.bytes())
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        let before = try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung)
        var bytes = Self.bytes(); bytes[bytes.count - 1] = 1
        let changed = datei(updated: 2000, bytes: bytes)
        hub.stelle(PW.remote(datei: changed, revision: 2)); _ = hub.blob(bytes)
        hub.blobVorher = { _, _ in throw CancellationError() }
        do {
            _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
            XCTFail("Abbruch erwartet")
        } catch is CancellationError {} // Fehler nicht als ungültige Stimme merken.
        XCTAssertEqual(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung), before)
        let oldFile = sp.stimmenWurzel!.appendingPathComponent(PhononStimmenSpeicher.blobName(try PhononStimme.kandidat(d, key: PW.key)))
        XCTAssertEqual(try Data(contentsOf: oldFile), Data(Self.bytes()))
        hub.blobVorher = nil
        _ = try await AblageLauf.ausfuehren(hub: hub, speicher: sp, konto: PW.konto)
        XCTAssertFalse(FileManager.default.fileExists(atPath: oldFile.path))
        XCTAssertEqual(try PhononStimmenSpeicher.laden(sp.stimmenWurzel!, konto: PW.konto.kennung).stimmen.count, 1)
    }
}
#endif
