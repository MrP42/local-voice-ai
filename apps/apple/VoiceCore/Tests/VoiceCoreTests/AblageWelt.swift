import XCTest
@testable import VoiceCore
#if canImport(CryptoKit)
import CryptoKit

// Gemeinsame Testhilfen der Ablage-Tests ab D5a-3 (lva#441): ein synthetisches Konto (Schlüssel 00..1f, Benutzer 7,
// nur `example.test`), gültige Kennungen und versiegelte Hub-Objekte wie der Desktop sie schickt. Nur synthetische
// Namen und Inhalte.

enum AblageWelt {
    static let key: [UInt8] = (0..<32).map { UInt8($0) }
    static let andererKey: [UInt8] = (0..<32).map { UInt8($0 == 0 ? 99 : $0) }
    static let userId: Int64 = 7
    static let hubUrl = "https://hub.example.test"
    static var konto: AblageKonto { AblageKonto(key: key, userId: userId, hubUrl: hubUrl) }
    static var keyId: String { SyncCrypto.keyId(key) }
    static let geraet = "g_01HZX4Q8E5K2V7M3N9P6R1S0TA"
    static let geraetB = "g_01HZX4Q8E5K2V7M3N9P6R1S0TB"

    // MARK: Kennungen (gültige ULIDs, `n` bis 255)

    private static func ulid(_ n: Int) -> String { String(format: "01HZX4Q8E5K2V7M3N9P6R1S0%02X", n) }
    static func ordId(_ n: Int) -> String { "ord_" + ulid(n) }
    static func datId(_ n: Int) -> String { "dat_" + ulid(n) }
    static func fasId(_ n: Int) -> String { "fas_" + ulid(n) }

    // MARK: Bausteine

    static func ordner(_ n: Int, name: String = "Ordner", eltern: String? = nil, klasse: String = "erbt", nurGeraet: Bool = false,
                       korb: Int64? = nil, zaehler: UInt64 = 1, geraet: String = geraet) -> AblageOrdner {
        AblageOrdner(id: ordId(n),
                     eltern: AblageRegister(wert: eltern, zaehler: zaehler, geraet: geraet),
                     name: AblageRegister(wert: name, zaehler: zaehler, geraet: geraet),
                     datenklasse: AblageRegister(wert: klasse, zaehler: zaehler, geraet: geraet),
                     nurGeraet: AblageRegister(wert: nurGeraet, zaehler: zaehler, geraet: geraet),
                     zielordnerKi: AblageRegister(wert: false, zaehler: zaehler, geraet: geraet),
                     papierkorb: AblageRegister(wert: korb, zaehler: zaehler, geraet: geraet))
    }

    static func sha(_ inhalt: String) -> String { AblageKrypto.sha256Hex(Array(inhalt.utf8)) }

    /// Fassung mit dem Original `inhalt` (Hash = SHA-256 des Inhalts, `blob == original_sha256` wie am Desktop).
    static func fassung(_ n: Int, _ vorgaenger: Int?, inhalt: String, typ: String = "pdf", zeit: Int64 = 1_000, texte: [AblageText] = [],
                        geraet: String = geraet) -> AblageFassung {
        let h = sha(inhalt)
        return AblageFassung(fas: fasId(n), vorgaenger: vorgaenger.map(fasId), blob: h, size: UInt64(inhalt.utf8.count), originalSha256: h,
                             typ: typ, geraet: geraet, zeit: zeit, texte: texte)
    }

    /// Ein Texteintrag samt Textfassungs-Bytes für `fassung` (Markenformat v1); `textrev` aus den Feldern berechnet.
    static func text(zu fassung: AblageFassung, abschnitte: [(anker: String, text: String)] = [("zeile=1", "Die Miete beträgt 850 Euro.")],
                     extraktor: String = "lva-text", version: String = "1", stand: String = "ok") -> (eintrag: AblageText, bytes: [UInt8]) {
        let rev = AblageTextfassung.textrev(originalSha256Hex: fassung.originalSha256, extraktor: extraktor, extraktorVersion: version, markenVersion: 1)
        let bytes = textBytes(textrev: rev, original: fassung.originalSha256, extraktor: extraktor, version: version, abschnitte: abschnitte)
        let e = AblageText(textrev: rev, blob: AblageKrypto.sha256Hex(bytes), size: UInt64(bytes.count), extraktor: extraktor,
                           extraktorVersion: version, markenVersion: 1, stand: stand)
        return (e, bytes)
    }

    static func textBytes(textrev: String, original: String, extraktor: String, version: String,
                          abschnitte: [(anker: String, text: String)]) -> [UInt8] {
        AblageJSON.objekt([
            ("marken_version", .uint(1)), ("textrev", .string(textrev)), ("original_sha256", .string(original)),
            ("extraktor", .string(extraktor)), ("extraktor_version", .string(version)),
            ("abschnitte", .array(abschnitte.map { AblageJSON.objekt([("anker", .string($0.anker)), ("text", .string($0.text))]) })),
        ]).kanonisch()
    }

    static func datei(_ n: Int, ordner: String, name: String = "vertrag.pdf", fassungen: [AblageFassung], klasse: String = "public",
                      nurGeraetMin: Bool = false, korb: Int64? = nil, zaehler: UInt64 = 1, geraet: String = geraet) -> AblageDatei {
        AblageDatei(id: datId(n),
                    ordner: AblageRegister(wert: ordner, zaehler: zaehler, geraet: geraet),
                    name: AblageRegister(wert: name, zaehler: zaehler, geraet: geraet),
                    klasseMin: AblageRegister(wert: klasse, zaehler: zaehler, geraet: geraet),
                    nurGeraetMin: nurGeraetMin, fassungen: fassungen, wahlen: [], entfernt: [],
                    blobRefs: AblageDatei.inhaltsRefs(fassungen),
                    papierkorb: AblageRegister(wert: korb, zaehler: zaehler, geraet: geraet))
    }

    // MARK: Hub-Objekte (versiegelt)

    static func siegel(_ klar: [UInt8], collection: String, id: String, refs: [String], keyId kid: String? = nil, key k: [UInt8]? = nil,
                       deleted: Bool = false) -> String {
        let kk = k ?? key
        let aad = AblageKrypto.aadAblage(userId: userId, collection: collection, objectId: id, hubBlobRefs: refs, keyId: kid ?? SyncCrypto.keyId(kk), deleted: deleted)
        return try! SyncCrypto.seal(key: kk, aad: aad, plaintext: klar)
    }

    static func remote(ordner o: AblageOrdner, revision: Int64 = 1, objectId: String? = nil, keyId kid: String? = nil, key k: [UInt8]? = nil,
                       refs: [String] = []) -> AblagePullObjekt {
        let id = objectId ?? o.id
        return AblagePullObjekt(collection: "ablage_ordner", objectId: id, revision: revision,
                                payload: siegel(o.kanonischeNutzlast(), collection: "ablage_ordner", id: id, refs: refs, keyId: kid, key: k),
                                keyId: kid ?? SyncCrypto.keyId(k ?? key), deleted: false, deviceId: "dev", serverSeq: revision, blobRefs: nil)
    }

    static func hubRefs(_ d: AblageDatei, key k: [UInt8]? = nil) -> [String] { AblageKrypto.hubRefs(datei: d, key: k ?? key) }

    /// `refsImKopf`: was der Hub als `blob_refs` mitliefert (Vorgabe: die richtige Ableitung); `refsImSiegel`: was in der AAD steht.
    static func remote(datei d: AblageDatei, revision: Int64 = 1, objectId: String? = nil, keyId kid: String? = nil, key k: [UInt8]? = nil,
                       refsImKopf: [String]? = nil, refsImSiegel: [String]? = nil, nutzlast: [UInt8]? = nil) -> AblagePullObjekt {
        let id = objectId ?? d.id
        let refs = hubRefs(d, key: k)
        let klar = nutzlast ?? d.kanonischeNutzlast(ordnen: true)
        return AblagePullObjekt(collection: "ablage_datei", objectId: id, revision: revision,
                                payload: siegel(klar, collection: "ablage_datei", id: id, refs: refsImSiegel ?? refsImKopf ?? refs, keyId: kid, key: k),
                                keyId: kid ?? SyncCrypto.keyId(k ?? key), deleted: false, deviceId: "dev", serverSeq: revision,
                                blobRefs: refsImKopf ?? refs)
    }

    static func remoteGrab(collection: String, id: String, revision: Int64, echt: Bool = true) -> AblagePullObjekt {
        let payload = echt ? try! AblageKrypto.grabNutzlast(key: key, userId: userId, collection: collection, id: id) : ""
        return AblagePullObjekt(collection: collection, objectId: id, revision: revision, payload: payload, keyId: keyId, deleted: true,
                                deviceId: "dev", serverSeq: revision, blobRefs: nil)
    }

    /// Zustand mit den übernommenen Objekten (Ordner vor Dateien, je Sammlung eine Seite).
    @discardableResult
    static func einspielen(_ z: inout AblageZustand, ordner: [AblagePullObjekt] = [], dateien: [AblagePullObjekt] = [],
                           konto k: AblageKonto? = nil) -> (ordner: AblageSeitenBericht, dateien: AblageSeitenBericht, graeber: AblageSeitenBericht) {
        let kk = k ?? konto
        let o = AblagePullVerarbeitung.seite(sammlung: "ablage_ordner", objekte: ordner, konto: kk, zustand: &z)
        let d = AblagePullVerarbeitung.seite(sammlung: "ablage_datei", objekte: dateien, konto: kk, zustand: &z)
        let g = AblagePullVerarbeitung.ordnergraeber(zustand: &z)
        return (o, d, g)
    }

    static func neuerZustand() -> AblageZustand { AblageZustand(konto: konto.kennung) }
}
#endif
