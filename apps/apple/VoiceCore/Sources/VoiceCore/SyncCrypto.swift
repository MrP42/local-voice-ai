import Foundation
#if canImport(CryptoKit)
import CryptoKit
#endif

// Verschlüsselung des Geräte-Syncs, bitgleich zum Desktop
// (`apps/local-voice/src-tauri/src/sync/crypto.rs`): Argon2id (64 MiB, t=3, p=1)
// → HKDF-SHA256 („lv-sync-enc-v1“) → XChaCha20-Poly1305 mit AAD
// `user_id ␟ collection ␟ object_id`, Ausgabe base64(nonce ‖ chiffrat ‖ tag).
// Ohne Fremdbibliothek: Argon2 und BLAKE2b hier nach RFC 9106/7693,
// XChaCha20 als HChaCha20 + ChaCha20-Poly1305 aus CryptoKit.

public enum SyncCryptoError: Error, Equatable { case badInput(String), openFailed }

// MARK: - BLAKE2b (RFC 7693)

public struct Blake2b {
    private static let iv: [UInt64] = [
        0x6a09e667f3bcc908, 0xbb67ae8584caa73b, 0x3c6ef372fe94f82b, 0xa54ff53a5f1d36f1,
        0x510e527fade682d1, 0x9b05688c2b3e6c1f, 0x1f83d9abfb41bd6b, 0x5be0cd19137e2179,
    ]
    private static let sigma: [[Int]] = [
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15], [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
        [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4], [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
        [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13], [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
        [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11], [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
        [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5], [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0],
    ]
    private var h: [UInt64]
    private var buffer: [UInt8] = []
    private var counter: UInt64 = 0
    private let outLen: Int

    public init(outputLength: Int) {
        precondition((1...64).contains(outputLength))
        outLen = outputLength
        h = Self.iv
        h[0] ^= 0x01010000 ^ UInt64(outputLength)
    }

    public mutating func update(_ data: [UInt8]) {
        var data = data[...]
        while !data.isEmpty {
            if buffer.count == 128 { counter &+= 128; compress(buffer, last: false); buffer.removeAll(keepingCapacity: true) }
            let take = min(128 - buffer.count, data.count)
            buffer.append(contentsOf: data.prefix(take)); data = data.dropFirst(take)
        }
    }

    public mutating func finalize() -> [UInt8] {
        counter &+= UInt64(buffer.count)
        let block = buffer + [UInt8](repeating: 0, count: 128 - buffer.count)
        compress(block, last: true)
        var out = [UInt8](); out.reserveCapacity(64)
        for w in h { for i in 0..<8 { out.append(UInt8(truncatingIfNeeded: w >> (8 * UInt64(i)))) } }
        return Array(out.prefix(outLen))
    }

    public static func hash(_ data: [UInt8], length: Int) -> [UInt8] { var b = Blake2b(outputLength: length); b.update(data); return b.finalize() }

    private mutating func compress(_ block: [UInt8], last: Bool) {
        var m = [UInt64](repeating: 0, count: 16)
        for i in 0..<16 { for j in 0..<8 { m[i] |= UInt64(block[i * 8 + j]) << (8 * UInt64(j)) } }
        var v = h + Self.iv
        v[12] ^= counter
        if last { v[14] = ~v[14] }
        @inline(__always) func g(_ a: Int, _ b: Int, _ c: Int, _ d: Int, _ x: UInt64, _ y: UInt64) {
            v[a] = v[a] &+ v[b] &+ x; v[d] = (v[d] ^ v[a]).rotr(32)
            v[c] = v[c] &+ v[d]; v[b] = (v[b] ^ v[c]).rotr(24)
            v[a] = v[a] &+ v[b] &+ y; v[d] = (v[d] ^ v[a]).rotr(16)
            v[c] = v[c] &+ v[d]; v[b] = (v[b] ^ v[c]).rotr(63)
        }
        for r in 0..<12 {
            let s = Self.sigma[r % 10]
            g(0, 4, 8, 12, m[s[0]], m[s[1]]); g(1, 5, 9, 13, m[s[2]], m[s[3]])
            g(2, 6, 10, 14, m[s[4]], m[s[5]]); g(3, 7, 11, 15, m[s[6]], m[s[7]])
            g(0, 5, 10, 15, m[s[8]], m[s[9]]); g(1, 6, 11, 12, m[s[10]], m[s[11]])
            g(2, 7, 8, 13, m[s[12]], m[s[13]]); g(3, 4, 9, 14, m[s[14]], m[s[15]])
        }
        for i in 0..<8 { h[i] ^= v[i] ^ v[i + 8] }
    }
}

extension UInt64 {
    @inline(__always) func rotr(_ n: UInt64) -> UInt64 { (self >> n) | (self << (64 - n)) }
}

@inline(__always) private func le32(_ x: Int) -> [UInt8] { (0..<4).map { UInt8(truncatingIfNeeded: x >> (8 * $0)) } }

// MARK: - Argon2id (RFC 9106, Version 0x13)

public enum Argon2id {
    /// Variable Hashlänge H' aus RFC 9106 Abschnitt 3.3.
    static func hPrime(_ input: [UInt8], length: Int) -> [UInt8] {
        let x = le32(length) + input
        if length <= 64 { return Blake2b.hash(x, length: length) }
        let r = (length + 31) / 32 - 2
        var v = Blake2b.hash(x, length: 64)
        var out = Array(v.prefix(32))
        for _ in 1..<r { v = Blake2b.hash(v, length: 64); out += v.prefix(32) }
        out += Blake2b.hash(v, length: length - 32 * r)
        return out
    }

    public static func hash(password: [UInt8], salt: [UInt8], memoryKiB m: Int, iterations t: Int, parallelism p: Int,
                            length: Int, secret: [UInt8] = [], associated: [UInt8] = []) -> [UInt8] {
        precondition(p >= 1 && t >= 1 && m >= 8 * p && length >= 4)
        var h0in = le32(p) + le32(length) + le32(m) + le32(t) + le32(0x13) + le32(2)
        h0in += le32(password.count) + password + le32(salt.count) + salt
        h0in += le32(secret.count) + secret + le32(associated.count) + associated
        let h0 = Blake2b.hash(h0in, length: 64)

        let mPrime = 4 * p * (m / (4 * p))
        let laneLength = mPrime / p
        let segmentLength = laneLength / 4
        // Speicher als ein Feld von 128 UInt64 je Block.
        var mem = [UInt64](repeating: 0, count: mPrime * 128)

        func setBlock(_ index: Int, _ bytes: [UInt8]) {
            for w in 0..<128 {
                var x: UInt64 = 0
                for b in 0..<8 { x |= UInt64(bytes[w * 8 + b]) << (8 * UInt64(b)) }
                mem[index * 128 + w] = x
            }
        }
        for lane in 0..<p {
            setBlock(lane * laneLength, hPrime(h0 + le32(0) + le32(lane), length: 1024))
            setBlock(lane * laneLength + 1, hPrime(h0 + le32(1) + le32(lane), length: 1024))
        }

        var addressBlock = [UInt64](repeating: 0, count: 128)
        var inputBlock = [UInt64](repeating: 0, count: 128)
        let zero = [UInt64](repeating: 0, count: 128)

        for pass in 0..<t {
            for slice in 0..<4 {
                for lane in 0..<p {
                    let independent = pass == 0 && slice < 2
                    if independent {
                        for i in 0..<128 { inputBlock[i] = 0 }
                        inputBlock[0] = UInt64(pass); inputBlock[1] = UInt64(lane); inputBlock[2] = UInt64(slice)
                        inputBlock[3] = UInt64(mPrime); inputBlock[4] = UInt64(t); inputBlock[5] = 2
                    }
                    let start = (pass == 0 && slice == 0) ? 2 : 0
                    if independent && start != 0 { nextAddresses(&addressBlock, &inputBlock, zero) }
                    for index in start..<segmentLength {
                        let curr = lane * laneLength + slice * segmentLength + index
                        let prev = curr % laneLength == 0 ? curr + laneLength - 1 : curr - 1
                        let rand: UInt64
                        if independent {
                            if index % 128 == 0 { nextAddresses(&addressBlock, &inputBlock, zero) }
                            rand = addressBlock[index % 128]
                        } else {
                            rand = mem[prev * 128]
                        }
                        let j1 = rand & 0xffff_ffff, j2 = rand >> 32
                        let refLane = (pass == 0 && slice == 0) ? lane : Int(j2 % UInt64(p))
                        let sameLane = refLane == lane
                        var area: Int
                        if pass == 0 {
                            area = sameLane ? slice * segmentLength + index - 1 : slice * segmentLength + (index == 0 ? -1 : 0)
                        } else {
                            area = sameLane ? laneLength - segmentLength + index - 1 : laneLength - segmentLength + (index == 0 ? -1 : 0)
                        }
                        let x = (j1 &* j1) >> 32
                        let y = (UInt64(area) &* x) >> 32
                        let relative = area - 1 - Int(y)
                        let startPos = (pass == 0 || slice == 3) ? 0 : (slice + 1) * segmentLength
                        let refIndex = refLane * laneLength + (startPos + relative) % laneLength
                        compressInto(&mem, dest: curr, x: prev, y: refIndex, xorWithDest: pass > 0)
                    }
                }
            }
        }

        var final = [UInt64](repeating: 0, count: 128)
        for lane in 0..<p { let last = lane * laneLength + laneLength - 1; for w in 0..<128 { final[w] ^= mem[last * 128 + w] } }
        var bytes = [UInt8](); bytes.reserveCapacity(1024)
        for w in final { for b in 0..<8 { bytes.append(UInt8(truncatingIfNeeded: w >> (8 * UInt64(b)))) } }
        return hPrime(bytes, length: length)
    }

    private static func nextAddresses(_ address: inout [UInt64], _ input: inout [UInt64], _ zero: [UInt64]) {
        input[6] &+= 1
        var tmp = g(zero, input)
        tmp = g(zero, tmp)
        address = tmp
    }

    /// G(X, Y) als neue Werte.
    static func g(_ x: [UInt64], _ y: [UInt64]) -> [UInt64] {
        var r = [UInt64](repeating: 0, count: 128)
        for i in 0..<128 { r[i] = x[i] ^ y[i] }
        var q = r
        permuteAll(&q)
        for i in 0..<128 { q[i] ^= r[i] }
        return q
    }

    private static func compressInto(_ mem: inout [UInt64], dest: Int, x: Int, y: Int, xorWithDest: Bool) {
        var r = [UInt64](repeating: 0, count: 128)
        for i in 0..<128 { r[i] = mem[x * 128 + i] ^ mem[y * 128 + i] }
        var q = r
        permuteAll(&q)
        for i in 0..<128 {
            let v = q[i] ^ r[i]
            mem[dest * 128 + i] = xorWithDest ? mem[dest * 128 + i] ^ v : v
        }
    }

    /// P zeilenweise (8 Zeilen à 16 Wörter), dann spaltenweise.
    @inline(__always) private static func permuteAll(_ q: inout [UInt64]) {
        for row in 0..<8 {
            let b = row * 16
            permute(&q, b, b + 1, b + 2, b + 3, b + 4, b + 5, b + 6, b + 7, b + 8, b + 9, b + 10, b + 11, b + 12, b + 13, b + 14, b + 15)
        }
        for col in 0..<8 {
            let b = col * 2
            permute(&q, b, b + 1, b + 16, b + 17, b + 32, b + 33, b + 48, b + 49, b + 64, b + 65, b + 80, b + 81, b + 96, b + 97, b + 112, b + 113)
        }
    }

    @inline(__always) private static func permute(_ v: inout [UInt64], _ i0: Int, _ i1: Int, _ i2: Int, _ i3: Int, _ i4: Int, _ i5: Int,
                                                  _ i6: Int, _ i7: Int, _ i8: Int, _ i9: Int, _ i10: Int, _ i11: Int, _ i12: Int,
                                                  _ i13: Int, _ i14: Int, _ i15: Int) {
        @inline(__always) func gb(_ a: Int, _ b: Int, _ c: Int, _ d: Int) {
            func fBlaMka(_ x: UInt64, _ y: UInt64) -> UInt64 { x &+ y &+ 2 &* (x & 0xffff_ffff) &* (y & 0xffff_ffff) }
            v[a] = fBlaMka(v[a], v[b]); v[d] = (v[d] ^ v[a]).rotr(32)
            v[c] = fBlaMka(v[c], v[d]); v[b] = (v[b] ^ v[c]).rotr(24)
            v[a] = fBlaMka(v[a], v[b]); v[d] = (v[d] ^ v[a]).rotr(16)
            v[c] = fBlaMka(v[c], v[d]); v[b] = (v[b] ^ v[c]).rotr(63)
        }
        gb(i0, i4, i8, i12); gb(i1, i5, i9, i13); gb(i2, i6, i10, i14); gb(i3, i7, i11, i15)
        gb(i0, i5, i10, i15); gb(i1, i6, i11, i12); gb(i2, i7, i8, i13); gb(i3, i4, i9, i14)
    }
}

// MARK: - XChaCha20-Poly1305

public enum XChaCha20Poly1305 {
    /// HChaCha20 (draft-irtf-cfrg-xchacha, 2.2): 32-Byte-Schlüssel, 16-Byte-Nonce → Teilschlüssel.
    public static func hChaCha20(key: [UInt8], nonce: [UInt8]) -> [UInt8] {
        precondition(key.count == 32 && nonce.count == 16)
        func w(_ b: [UInt8], _ i: Int) -> UInt32 { UInt32(b[i]) | UInt32(b[i + 1]) << 8 | UInt32(b[i + 2]) << 16 | UInt32(b[i + 3]) << 24 }
        var s: [UInt32] = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]
        for i in 0..<8 { s.append(w(key, i * 4)) }
        for i in 0..<4 { s.append(w(nonce, i * 4)) }
        @inline(__always) func rotl(_ x: UInt32, _ n: UInt32) -> UInt32 { (x << n) | (x >> (32 - n)) }
        func qr(_ a: Int, _ b: Int, _ c: Int, _ d: Int) {
            s[a] = s[a] &+ s[b]; s[d] = rotl(s[d] ^ s[a], 16)
            s[c] = s[c] &+ s[d]; s[b] = rotl(s[b] ^ s[c], 12)
            s[a] = s[a] &+ s[b]; s[d] = rotl(s[d] ^ s[a], 8)
            s[c] = s[c] &+ s[d]; s[b] = rotl(s[b] ^ s[c], 7)
        }
        for _ in 0..<10 {
            qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15)
            qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14)
        }
        var out = [UInt8]()
        for i in [0, 1, 2, 3, 12, 13, 14, 15] { for b in 0..<4 { out.append(UInt8(truncatingIfNeeded: s[i] >> (8 * UInt32(b)))) } }
        return out
    }

    #if canImport(CryptoKit)
    /// Gibt Chiffrat ‖ Tag zurück (ohne Nonce).
    public static func seal(key: [UInt8], nonce: [UInt8], plaintext: [UInt8], aad: [UInt8]) throws -> [UInt8] {
        guard key.count == 32, nonce.count == 24 else { throw SyncCryptoError.badInput("Schlüssel/Nonce") }
        let sub = SymmetricKey(data: hChaCha20(key: key, nonce: Array(nonce[0..<16])))
        let n = try ChaChaPoly.Nonce(data: [0, 0, 0, 0] + nonce[16..<24])
        let box = try ChaChaPoly.seal(plaintext, using: sub, nonce: n, authenticating: aad)
        return Array(box.ciphertext) + Array(box.tag)
    }

    public static func open(key: [UInt8], nonce: [UInt8], sealed: [UInt8], aad: [UInt8]) throws -> [UInt8] {
        guard key.count == 32, nonce.count == 24, sealed.count >= 16 else { throw SyncCryptoError.badInput("Schlüssel/Nonce") }
        let sub = SymmetricKey(data: hChaCha20(key: key, nonce: Array(nonce[0..<16])))
        let n = try ChaChaPoly.Nonce(data: [0, 0, 0, 0] + nonce[16..<24])
        let box = try ChaChaPoly.SealedBox(nonce: n, ciphertext: sealed.dropLast(16), tag: sealed.suffix(16))
        do { return Array(try ChaChaPoly.open(box, using: sub, authenticating: aad)) } catch { throw SyncCryptoError.openFailed }
    }
    #endif
}

// MARK: - Sync-Schlüssel und Objekte wie am Desktop

#if canImport(CryptoKit)
public enum SyncCrypto {
    public static let argonMemoryKiB = 64 * 1024
    public static let argonIterations = 3

    /// Schlüssel aus Passwort und E-Mail; Salz = SHA-256("local-voice-ai/v1/<email>")[0..16].
    public static func deriveKey(password: String, email: String) -> [UInt8] {
        let normalized = email.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        let salt = Array(SHA256.hash(data: Data("local-voice-ai/v1/\(normalized)".utf8)).prefix(16))
        let master = Argon2id.hash(password: Array(password.utf8), salt: salt, memoryKiB: argonMemoryKiB,
                                   iterations: argonIterations, parallelism: 1, length: 32)
        return hkdf(master)
    }

    static func hkdf(_ master: [UInt8]) -> [UInt8] {
        let key = HKDF<SHA256>.deriveKey(inputKeyMaterial: SymmetricKey(data: master), info: Data("lv-sync-enc-v1".utf8), outputByteCount: 32)
        return key.withUnsafeBytes { Array($0) }
    }

    public static func keyId(_ key: [UInt8]) -> String {
        SHA256.hash(data: Data(key)).prefix(4).map { String(format: "%02x", $0) }.joined()
    }

    public static func aad(userId: Int64, collection: String, objectId: String) -> [UInt8] {
        Array("\(userId)\u{1f}\(collection)\u{1f}\(objectId)".utf8)
    }

    /// base64(nonce ‖ chiffrat ‖ tag) wie `crypto::seal`.
    public static func seal(key: [UInt8], aad: [UInt8], plaintext: [UInt8], nonce: [UInt8]? = nil) throws -> String {
        var n = nonce ?? [UInt8](repeating: 0, count: 24)
        if nonce == nil { guard SecRandomCopyBytes(kSecRandomDefault, 24, &n) == errSecSuccess else { throw SyncCryptoError.badInput("Zufall") } }
        return Data(n + (try XChaCha20Poly1305.seal(key: key, nonce: n, plaintext: plaintext, aad: aad))).base64EncodedString()
    }

    public static func open(key: [UInt8], aad: [UInt8], sealedBase64: String) throws -> [UInt8] {
        guard let raw = Data(base64Encoded: sealedBase64.trimmingCharacters(in: .whitespacesAndNewlines)), raw.count >= 40 else {
            throw SyncCryptoError.openFailed
        }
        let bytes = [UInt8](raw)
        return try XChaCha20Poly1305.open(key: key, nonce: Array(bytes[0..<24]), sealed: Array(bytes[24...]), aad: aad)
    }
}
#endif
