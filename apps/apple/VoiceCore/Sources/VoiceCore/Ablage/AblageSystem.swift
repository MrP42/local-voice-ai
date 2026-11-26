import Foundation

/// Reservierter Kennungsraum aus S1a (#898). Der sichtbare Name ist keine Systemkennung.
public enum AblageSystem {
    private static func ist(_ id: String, prefix: String, magic: [UInt8]) -> Bool {
        guard AblageId.idGueltig(id, prefix: prefix) else { return false }
        let alphabet = Array("0123456789ABCDEFGHJKMNPQRSTVWXYZ".utf8)
        let text = Array(id.dropFirst(prefix.count).uppercased().utf8)
        var bytes: [UInt8] = [], acc: UInt32 = 0, bits = 0
        for (i, c) in text.enumerated() {
            guard let n = alphabet.firstIndex(of: c) else { return false }
            acc = (acc << 5) | UInt32(n)
            bits += i == 0 ? 3 : 5
            if i == 0 { acc &= 7 } // ulid::from_string wie am Desktop
            if bits >= 8 {
                bits -= 8
                bytes.append(UInt8((acc >> bits) & 255))
                acc &= (1 << bits) - 1
            }
        }
        return Array(bytes.prefix(4)) == magic
    }

    public static func istOrdner(_ id: String) -> Bool { ist(id, prefix: "ord_", magic: Array("LVSO".utf8)) }
    public static func istDatei(_ id: String) -> Bool { ist(id, prefix: "dat_", magic: Array("LVSF".utf8)) }

    /// Auch Unterbäume und verwaiste Systemdateien bleiben verborgen; der interne Bestand bleibt vollständig.
    public static func ordner(_ id: String, in b: AblageBestand) -> Bool {
        var next: String? = id, besucht = AblageMenge()
        while let p = next, !besucht.contains(p) {
            if istOrdner(p) { return true }
            besucht.insert(p)
            next = b.ordner[p]?.eltern.wert
        }
        return false
    }

    public static func datei(_ id: String, in b: AblageBestand) -> Bool {
        istDatei(id) || b.dateien[id].map { ordner($0.ordner.wert, in: b) } == true
    }
}

#if canImport(CryptoKit)
extension AblageSystem {
    public static func kennung(key: [UInt8], ordner: Bool, generation: UInt32, salt: String = "") -> String? {
        guard key.count == 32 else { return nil }
        let g = (0..<4).reversed().map { UInt8(truncatingIfNeeded: generation >> ($0 * 8)) }
        let hash = AblageKrypto.sha256(key + Array("lv-phonon-sync-v1".utf8) + [ordner ? 1 : 0] + g + Array(salt.utf8))
        return (ordner ? "ord_" : "dat_") + (AblageId.ulid(Array((ordner ? "LVSO" : "LVSF").utf8) + Array(hash.prefix(12))) ?? "")
    }
}
#endif
