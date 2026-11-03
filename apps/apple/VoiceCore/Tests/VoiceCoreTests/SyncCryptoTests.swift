import XCTest
@testable import VoiceCore

private func hex(_ s: String) -> [UInt8] {
    var out = [UInt8](); var i = s.startIndex
    while i < s.endIndex { let j = s.index(i, offsetBy: 2); out.append(UInt8(s[i..<j], radix: 16)!); i = j }
    return out
}
private func hexString(_ b: [UInt8]) -> String { b.map { String(format: "%02x", $0) }.joined() }

final class SyncCryptoTests: XCTestCase {
    func testBlake2bKnownAnswer() {
        XCTAssertEqual(hexString(Blake2b.hash(Array("abc".utf8), length: 64)),
                       "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d17d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923")
        // Über mehrere 128-Byte-Blöcke.
        XCTAssertEqual(Blake2b.hash([UInt8](repeating: 0x61, count: 300), length: 32).count, 32)
    }

    func testArgon2idRfc9106Vector() {
        let tag = Argon2id.hash(password: [UInt8](repeating: 1, count: 32), salt: [UInt8](repeating: 2, count: 16),
                                memoryKiB: 32, iterations: 3, parallelism: 4, length: 32,
                                secret: [UInt8](repeating: 3, count: 8), associated: [UInt8](repeating: 4, count: 12))
        XCTAssertEqual(hexString(tag), "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659")
    }

    func testHChaCha20Vector() {
        let key = (0..<32).map { UInt8($0) }
        XCTAssertEqual(hexString(XChaCha20Poly1305.hChaCha20(key: key, nonce: hex("000000090000004a0000000031415927"))),
                       "82413b4227b27bfed30e42508a877d73a0f9e4d58a74a853c12ec41326d3ecdc")
    }

    func testXChaCha20Poly1305Vector() throws {
        let plaintext = Array("Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.".utf8)
        let key = (0x80...0x9f).map { UInt8($0) }
        let nonce = (0x40...0x57).map { UInt8($0) }
        let aad = hex("50515253c0c1c2c3c4c5c6c7")
        let sealed = try XChaCha20Poly1305.seal(key: key, nonce: nonce, plaintext: plaintext, aad: aad)
        XCTAssertEqual(hexString(sealed),
                       "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff921f9664c97637da9768812f615c68b13b52e"
                       + "c0875924c1c7987947deafd8780acf49")
        XCTAssertEqual(try XChaCha20Poly1305.open(key: key, nonce: nonce, sealed: sealed, aad: aad), plaintext)
        var tampered = sealed; tampered[0] ^= 1
        XCTAssertThrowsError(try XChaCha20Poly1305.open(key: key, nonce: nonce, sealed: tampered, aad: aad))
    }

    func testSealOpenRoundTripBoundToPlace() throws {
        let key = [UInt8](repeating: 7, count: 32)
        let a = SyncCrypto.aad(userId: 7, collection: "page", objectId: "page_1")
        let sealed = try SyncCrypto.seal(key: key, aad: a, plaintext: Array("Guten Tag".utf8))
        XCTAssertEqual(String(decoding: try SyncCrypto.open(key: key, aad: a, sealedBase64: sealed), as: UTF8.self), "Guten Tag")
        XCTAssertThrowsError(try SyncCrypto.open(key: key, aad: SyncCrypto.aad(userId: 7, collection: "page", objectId: "page_2"), sealedBase64: sealed))
        XCTAssertNotEqual(sealed, try SyncCrypto.seal(key: key, aad: a, plaintext: Array("Guten Tag".utf8)))
        XCTAssertEqual(SyncCrypto.keyId(key).count, 8)
    }

    /// Erwartet aus dem Rust-Pfad des Desktops (argon2 0.5 + hkdf 0.12, wie sync/crypto.rs).
    func testDeriveKeyMatchesDesktop() throws {
        #if DEBUG
        // 64 MiB Argon2 braucht unoptimiert ~25 s; im Release-Lauf (`swift test -c release`) unter 1 s.
        try XCTSkipUnless(ProcessInfo.processInfo.environment["LV_SLOW_TESTS"] == "1", "nur mit LV_SLOW_TESTS=1 oder -c release")
        #endif
        let started = Date()
        let key = SyncCrypto.deriveKey(password: "geheim", email: "  Mail@Example.com ")
        XCTAssertEqual(hexString(key), "f5037b3d85942b962f0b5faa05786fa3dae1cb5bb0d3068277450c3287a678ee")
        print("deriveKey dauerte \(Date().timeIntervalSince(started)) s")
    }
}
