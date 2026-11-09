import Foundation
#if canImport(CryptoKit)
import CryptoKit

// Krypto der Dokumentenablage, bitgleich zum Desktop (D5a-1, lva#441):
// `sync/crypto.rs` (`aad_ablage`, `seal_deterministisch`), `sync/ablage.rs` (`hub_blob`, `hub_refs`, `grab_*`,
// `standard_id`), `sync/blobs.rs` (`blob_id`). Reine Funktionen, kein Zustand, kein I/O, keine Protokollierung.
// Schlüssel sind 32 Byte; jede andere Länge ergibt `nil`, `false` oder `AblageError.schluessel`, nie einen Absturz.

public enum AblageKrypto {
    public static let schluesselLaenge = 32

    public static func sha256(_ bytes: [UInt8]) -> [UInt8] { Array(SHA256.hash(data: Data(bytes))) }
    public static func sha256Hex(_ bytes: [UInt8]) -> String { sha256(bytes).map { String(format: "%02x", $0) }.joined() }

    // MARK: AAD

    /// AAD der Ablage wie `crypto::aad_ablage`:
    /// `user ␟ collection ␟ object ␟ "blob_refs" ␟ refs,… ␟ "key_id" ␟ key_id ␟ "deleted" ␟ 0|1`.
    /// `hubBlobRefs` sind die Hub-Kennungen (nicht die Inhalts-Hashes), sortiert und eindeutig; für Ordner und
    /// Grabsteine leer.
    public static func aadAblage(userId: Int64, collection: String, objectId: String, hubBlobRefs: [String],
                                 keyId: String, deleted: Bool) -> [UInt8] {
        SyncCrypto.aad(userId: userId, collection: collection, objectId: objectId)
            + Array("\u{1f}blob_refs\u{1f}\(hubBlobRefs.joined(separator: ","))\u{1f}key_id\u{1f}\(keyId)\u{1f}deleted\u{1f}\(deleted ? 1 : 0)".utf8)
    }

    // MARK: Hub-Kennungen

    /// 64 Hex-Zeichen → 32 Rohbytes wie `unhex32` (auch Großbuchstaben; Rusts `from_str_radix` nimmt je Paar
    /// zusätzlich `+` und eine Ziffer an), sonst `nil`.
    static func rohbytes32(_ s: String) -> [UInt8]? {
        let u = Array(s.utf8)
        guard u.count == 64 else { return nil }
        func ziffer(_ c: UInt8) -> UInt8? {
            switch c {
            case UInt8(ascii: "0")...UInt8(ascii: "9"): return c - UInt8(ascii: "0")
            case UInt8(ascii: "a")...UInt8(ascii: "f"): return c - UInt8(ascii: "a") + 10
            case UInt8(ascii: "A")...UInt8(ascii: "F"): return c - UInt8(ascii: "A") + 10
            default: return nil
            }
        }
        var out: [UInt8] = []
        out.reserveCapacity(32)
        for i in 0..<32 {
            let a = u[2 * i], b = u[2 * i + 1]
            if let x = ziffer(a), let y = ziffer(b) { out.append(x << 4 | y) }
            else if a == UInt8(ascii: "+"), let y = ziffer(b) { out.append(y) }
            else { return nil }
        }
        return out
    }

    /// Hub-Kennung eines Inhalts-Hashes (hex) wie `hub_blob`: `SHA256(key ‖ "lv-blob-v1" ‖ sha[32 Rohbytes])`.
    public static func hubBlobId(key: [UInt8], sha256Hex: String) -> String? {
        guard key.count == schluesselLaenge, let raw = rohbytes32(sha256Hex) else { return nil }
        return LibrarySyncFormat.blobId(key: key, plaintextSha256: Data(raw))
    }

    /// Hub-Kennungen aller Originale und aller Texte mit nicht leerem `blob` wie `hub_refs`: sortiert, eindeutig.
    /// Ein `blob`, der kein 64-stelliger Hex-Hash ist, fällt wie am Desktop still heraus; der Vergleich mit den
    /// vom Hub gelieferten `blob_refs` (D5a-2) macht daraus „unlesbar“.
    public static func hubRefs(datei: AblageDatei, key: [UInt8]) -> [String] {
        var ids: [String] = []
        for f in datei.fassungen {
            if let id = hubBlobId(key: key, sha256Hex: f.blob) { ids.append(id) }
            for t in f.texte where !t.blob.isEmpty {
                if let id = hubBlobId(key: key, sha256Hex: t.blob) { ids.append(id) }
            }
        }
        return AblageBytes.sortedUnique(ids)
    }

    // MARK: Deterministisches Siegel und Grabsteine

    /// Wie `seal_deterministisch`: Nonce = erste 24 Bytes von
    /// `SHA-256("lv-ablage-siegel-nonce" ‖ 0x1F ‖ key ‖ u64_be(aad.len) ‖ aad ‖ klartext)`. Gleiche Eingabe ergibt
    /// dasselbe Chiffrat (eine Wiederholung nach verlorener Antwort trägt dieselbe Nutzlast).
    public static func sealDeterministisch(key: [UInt8], aad: [UInt8], plaintext: [UInt8]) throws -> String {
        guard key.count == schluesselLaenge else { throw AblageError.schluessel }
        var eingabe = Array("lv-ablage-siegel-nonce".utf8) + [0x1F] + key
        let laenge = UInt64(aad.count)
        for s in stride(from: 56, through: 0, by: -8) { eingabe.append(UInt8(truncatingIfNeeded: laenge >> UInt64(s))) }
        eingabe += aad
        eingabe += plaintext
        let nonce = Array(sha256(eingabe).prefix(24))
        do { return try SyncCrypto.seal(key: key, aad: aad, plaintext: plaintext, nonce: nonce) } catch { throw AblageError.krypto }
    }

    /// Klartext eines Grabsteins wie `grab_klartext`: `{"collection":"…","deleted":true,"id":"…"}` (kanonisch).
    public static func grabKlartext(collection: String, id: String) -> [UInt8] {
        AblageJSON.objekt([("collection", .string(collection)), ("deleted", .bool(true)), ("id", .string(id))]).kanonisch()
    }

    /// Versiegelte Nutzlast eines Grabsteins wie `grab_nutzlast` (AAD mit `deleted=1`, leere Refs). Nie eine leere
    /// Nutzlast senden: der Desktop ignoriert Grabsteine ohne Siegel und hält das Objekt an.
    public static func grabNutzlast(key: [UInt8], userId: Int64, collection: String, id: String) throws -> String {
        guard key.count == schluesselLaenge else { throw AblageError.schluessel }
        let aad = aadAblage(userId: userId, collection: collection, objectId: id, hubBlobRefs: [],
                            keyId: SyncCrypto.keyId(key), deleted: true)
        return try sealDeterministisch(key: key, aad: aad, plaintext: grabKlartext(collection: collection, id: id))
    }

    /// Ist der Grabstein vom Konto selbst versiegelt wie `grab_echt`: Nutzlast nicht leer, `key_id` = eigene,
    /// Öffnen mit AAD (`deleted=1`, leere Refs) gelingt, und das JSON bestätigt `id`, `collection` und
    /// `deleted == true`. Alles andere ist `false` (Grabstein nicht anwenden, Objekt anhalten).
    public static func grabEcht(key: [UInt8], userId: Int64, collection: String, id: String, keyId: String, payload: String) -> Bool {
        guard key.count == schluesselLaenge, !payload.isEmpty, AblageBytes.equal(keyId, SyncCrypto.keyId(key)) else { return false }
        let aad = aadAblage(userId: userId, collection: collection, objectId: id, hubBlobRefs: [], keyId: keyId, deleted: true)
        guard let plain = try? SyncCrypto.open(key: key, aad: aad, sealedBase64: payload),
              let v = try? AblageJSON.parse(plain),
              case .string(let jid)? = v["id"], AblageBytes.equal(jid, id),
              case .string(let jcoll)? = v["collection"], AblageBytes.equal(jcoll, collection),
              case .bool(true)? = v["deleted"] else { return false }
        return true
    }

    // MARK: Standardordner

    /// Kontoweit gleiche Kennung des Ordners „Allgemein“ wie `standard_id`:
    /// `"ord_" + ULID(SHA-256(key ‖ "lv-ablage-allgemein")[0..<16])`.
    public static func standardOrdnerId(key: [UInt8]) -> String? {
        guard key.count == schluesselLaenge,
              let u = AblageId.ulid(Array(sha256(key + Array("lv-ablage-allgemein".utf8)).prefix(16))) else { return nil }
        return AblageId.praefixOrdner + u
    }
}

// MARK: - Hash der Nutzlast

extension AblageOrdner {
    /// SHA-256 (hex) der kanonischen Nutzlast wie `hash_ordner`.
    public func hash() -> String { AblageKrypto.sha256Hex(kanonischeNutzlast()) }
}

extension AblageDatei {
    /// SHA-256 (hex) der kanonischen Nutzlast. `ordnen` ist Pflicht: `false` = Rusts `hash_datei` (Reihenfolge der
    /// Listen unverändert), `true` = Hash des Ergebnisses von `zusammenfuehren_datei` (siehe `geordnet()`).
    public func hash(ordnen: Bool) -> String { AblageKrypto.sha256Hex(kanonischeNutzlast(ordnen: ordnen)) }
}
#endif
