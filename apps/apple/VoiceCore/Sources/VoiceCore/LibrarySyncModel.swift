import Foundation
#if canImport(CryptoKit)
import CryptoKit

// Datenformate des Bibliothek-Syncs (Spezifikation
// docs/superpowers/specs/2026-11-02-bibliothek-sync.md), plattformneutral und testbar.

public struct HubPushObject: Codable, Equatable, Sendable {
    public var collection: String
    public var objectId: String
    public var baseRevision: Int64
    public var payload: String
    public var keyId: String
    public var deleted: Bool
    public init(collection: String, objectId: String, baseRevision: Int64, payload: String, keyId: String, deleted: Bool) {
        self.collection = collection; self.objectId = objectId; self.baseRevision = baseRevision
        self.payload = payload; self.keyId = keyId; self.deleted = deleted
    }
    enum CodingKeys: String, CodingKey { case collection, objectId = "object_id", baseRevision = "base_revision", payload, keyId = "key_id", deleted }
}

public struct HubRemoteObject: Codable, Equatable, Sendable {
    public var collection: String
    public var objectId: String
    public var revision: Int64
    public var payload: String
    public var keyId: String
    public var deleted: Bool
    public var deviceId: String
    public var serverSeq: Int64
    enum CodingKeys: String, CodingKey { case collection, objectId = "object_id", revision, payload, keyId = "key_id", deleted, deviceId = "device_id", serverSeq = "server_seq" }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        collection = try c.decode(String.self, forKey: .collection)
        objectId = try c.decode(String.self, forKey: .objectId)
        revision = try c.decodeIfPresent(Int64.self, forKey: .revision) ?? 0
        payload = try c.decodeIfPresent(String.self, forKey: .payload) ?? ""
        keyId = try c.decodeIfPresent(String.self, forKey: .keyId) ?? ""
        deleted = try c.decodeIfPresent(Bool.self, forKey: .deleted) ?? false
        deviceId = try c.decodeIfPresent(String.self, forKey: .deviceId) ?? ""
        serverSeq = try c.decodeIfPresent(Int64.self, forKey: .serverSeq) ?? 0
    }
}

public struct HubPushResult: Codable, Sendable {
    public var collection: String?
    public var objectId: String?
    public var outcome: String
    public var revision: Int64?
    public var current: HubRemoteObject?
    public var reason: String?
    enum CodingKeys: String, CodingKey { case collection, objectId = "object_id", outcome, revision, current, reason }
}

public struct HubPullResponse: Codable, Sendable {
    public var objects: [HubRemoteObject]
    public var cursor: Int64
    public var more: Bool
}

/// Verweis auf eine Datei in der Ablage des Hubs.
public struct BlobRef: Codable, Equatable, Sendable {
    public var blob: String
    public var size: Int
    public var chunks: Int
    public var type: String?
    public var file: String?
    public init(blob: String, size: Int, chunks: Int, type: String? = nil, file: String? = nil) {
        self.blob = blob; self.size = size; self.chunks = chunks; self.type = type; self.file = file
    }
}

public struct BookFiles: Codable, Equatable, Sendable {
    public var text: BlobRef?
    public var cover: BlobRef?
    public var audio: [BlobRef]
    public init(text: BlobRef? = nil, cover: BlobRef? = nil, audio: [BlobRef] = []) { self.text = text; self.cover = cover; self.audio = audio }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decodeIfPresent(BlobRef.self, forKey: .text)
        cover = try c.decodeIfPresent(BlobRef.self, forKey: .cover)
        audio = try c.decodeIfPresent([BlobRef].self, forKey: .audio) ?? []
    }
    public var audioBytes: Int { audio.reduce(0) { $0 + $1.size } }
}

/// Inhalt eines `book`-Objekts: das Buch ohne lokale Felder, Dateien, Zeitpunkt.
public struct SyncedBook: Codable, Equatable, Sendable {
    public var book: LibraryBook
    public var files: BookFiles
    public var updated: Double

    public init(book: LibraryBook, files: BookFiles, updated: Double) { self.book = book; self.files = files; self.updated = updated }

    public func json() throws -> Data {
        var dict = try JSONSerialization.jsonObject(with: JSONEncoder().encode(book)) as! [String: Any]
        dict["files"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(files))
        dict["updated"] = updated
        return try JSONSerialization.data(withJSONObject: dict, options: [.sortedKeys])
    }

    public static func decode(_ data: Data) throws -> SyncedBook {
        let book = try JSONDecoder().decode(LibraryBook.self, from: data)
        let raw = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        let files = try raw["files"].map { try JSONDecoder().decode(BookFiles.self, from: JSONSerialization.data(withJSONObject: $0)) } ?? BookFiles()
        return SyncedBook(book: book, files: files, updated: (raw["updated"] as? NSNumber)?.doubleValue ?? 0)
    }
}

public struct SyncedProgress: Codable, Equatable, Sendable {
    public var key: String
    public var record: ProgressRecord
    public init(key: String, record: ProgressRecord) { self.key = key; self.record = record }
}

public enum LibrarySyncFormat {
    public static let collectionBook = "book"
    public static let collectionProgress = "progress"
    public static let collectionBookmark = "bookmark"
    /// Klartext in den Stücken einer Datei.
    public static let chunkPlainBytes = 4 * 1024 * 1024

    static func sha256Hex(_ data: Data) -> String { SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined() }

    public static func progressObjectId(_ key: String) -> String { "p_" + String(sha256Hex(Data(key.utf8)).prefix(40)) }
    public static func bookmarkObjectId(_ key: String) -> String { "b_" + String(sha256Hex(Data(key.utf8)).prefix(40)) }

    /// Inhaltsadresse, die der Server keinem Inhalt zuordnen kann.
    public static func blobId(key: [UInt8], plaintextSha256: Data) -> String {
        sha256Hex(Data(key) + Data("lv-blob-v1".utf8) + plaintextSha256)
    }

    public static func chunkAAD(userId: Int64, blobId: String, index: Int) -> [UInt8] {
        Array("\(userId)\u{1f}blob\u{1f}\(blobId)\u{1f}\(index)".utf8)
    }

    /// Stück = nonce ‖ chiffrat ‖ tag (roh).
    public static func sealChunk(key: [UInt8], userId: Int64, blobId: String, index: Int, plain: [UInt8], nonce: [UInt8]? = nil) throws -> [UInt8] {
        var n = nonce ?? [UInt8](repeating: 0, count: 24)
        if nonce == nil { guard SecRandomCopyBytes(kSecRandomDefault, 24, &n) == errSecSuccess else { throw SyncCryptoError.badInput("Zufall") } }
        return n + (try XChaCha20Poly1305.seal(key: key, nonce: n, plaintext: plain, aad: chunkAAD(userId: userId, blobId: blobId, index: index)))
    }

    public static func openChunk(key: [UInt8], userId: Int64, blobId: String, index: Int, sealed: [UInt8]) throws -> [UInt8] {
        guard sealed.count >= 40 else { throw SyncCryptoError.openFailed }
        return try XChaCha20Poly1305.open(key: key, nonce: Array(sealed[0..<24]), sealed: Array(sealed[24...]),
                                          aad: chunkAAD(userId: userId, blobId: blobId, index: index))
    }

    /// Kanonischer Hash eines Klartext-JSON (Schlüssel sortiert), für das Ledger.
    public static func canonicalHash(_ json: Data) -> String {
        guard let obj = try? JSONSerialization.jsonObject(with: json),
              let data = try? JSONSerialization.data(withJSONObject: obj, options: [.sortedKeys]) else { return sha256Hex(json) }
        return sha256Hex(data)
    }
}

/// Zuletzt abgeglichener Stand je Objekt und Pull-Cursor.
public struct SyncLedger: Codable, Equatable, Sendable {
    public struct Entry: Codable, Equatable, Sendable {
        public var revision: Int64
        public var hash: String
        public var deleted: Bool
        public init(revision: Int64, hash: String, deleted: Bool) { self.revision = revision; self.hash = hash; self.deleted = deleted }
    }
    public var cursor: Int64 = 0
    public var entries: [String: Entry] = [:]
    /// Blobs, die dieses Gerät vollständig hochgeladen hat.
    public var uploaded: Set<String> = []
    public init() {}
    public static func key(_ collection: String, _ objectId: String) -> String { collection + "/" + objectId }
    public subscript(collection: String, objectId: String) -> Entry? {
        get { entries[Self.key(collection, objectId)] }
        set { entries[Self.key(collection, objectId)] = newValue }
    }
}
#endif
