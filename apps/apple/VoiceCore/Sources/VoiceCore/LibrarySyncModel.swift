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

/// Kurzbuch vom PC: Datei und Herkunft für die Anzeige.
public struct KurzbuchRef: Codable, Equatable, Sendable {
    /// Größte Kurzbuch-Datei, wie `MAX_TEXT_BYTES` am Desktop.
    public static let maxBytes = 2 * 1024 * 1024

    public var blob: String
    public var size: Int
    public var chunks: Int
    public var type: String?
    public var file: String?
    public var model: String?
    public var createdMs: Double?
    public var source: String?

    enum CodingKeys: String, CodingKey { case blob, size, chunks, type, file, model, createdMs = "created_ms", source }

    public init(blob: String, size: Int, chunks: Int, type: String? = "md", file: String? = nil,
                model: String? = nil, createdMs: Double? = nil, source: String? = nil) {
        self.blob = blob; self.size = size; self.chunks = chunks; self.type = type; self.file = file
        self.model = model; self.createdMs = createdMs; self.source = source
    }

    public var ref: BlobRef { BlobRef(blob: blob, size: size, chunks: chunks, type: type, file: file) }

    /// Sprache aus `kurzbuch-<sprache>.md`, streng geprüft: Sie landet in Pfaden.
    public var language: String? { LibraryText.kurzbuchLanguage(file) }

    /// Verweise kommen vom Netz: Kennung, Sprache, Größe und Stückzahl müssen stimmen, bevor geladen wird.
    public var isUsable: Bool {
        blob.count == 64 && blob.utf8.allSatisfy { ($0 >= 0x30 && $0 <= 0x39) || ($0 >= 0x61 && $0 <= 0x66) }
            && language != nil && size > 0 && size <= Self.maxBytes && chunks == LibrarySyncFormat.chunkCount(size: size)
    }
}

public struct BookFiles: Codable, Equatable, Sendable {
    public var text: BlobRef?
    public var cover: BlobRef?
    public var audio: [BlobRef]
    /// Kurzbücher vom PC, je Sprache das neueste (`kurzbuch-<sprache>.md`).
    public var kurzbuecher: [KurzbuchRef]
    /// Dateien weiterer Sprachfassungen (lva#344), je Fassungskennung; unverändert weitergereicht.
    public var editions: PassthroughJSON?
    enum CodingKeys: String, CodingKey { case text, cover, audio, kurzbuecher, editions }
    public init(text: BlobRef? = nil, cover: BlobRef? = nil, audio: [BlobRef] = [], kurzbuecher: [KurzbuchRef] = []) {
        self.text = text; self.cover = cover; self.audio = audio; self.kurzbuecher = kurzbuecher
    }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        text = try c.decodeIfPresent(BlobRef.self, forKey: .text)
        cover = try c.decodeIfPresent(BlobRef.self, forKey: .cover)
        audio = try c.decodeIfPresent([BlobRef].self, forKey: .audio) ?? []
        kurzbuecher = try c.decodeIfPresent([KurzbuchRef].self, forKey: .kurzbuecher) ?? []
        editions = try c.decodeIfPresent(PassthroughJSON.self, forKey: .editions)
    }
    /// Leere Listen fehlen wie am Desktop, sonst ändert sich der Hash aller Bücher.
    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encodeIfPresent(text, forKey: .text)
        try c.encodeIfPresent(cover, forKey: .cover)
        try c.encode(audio, forKey: .audio)
        if !kurzbuecher.isEmpty { try c.encode(kurzbuecher, forKey: .kurzbuecher) }
        try c.encodeIfPresent(editions, forKey: .editions)
    }
    public var audioBytes: Int { audio.reduce(0) { $0 + $1.size } }
}

/// Inhalt eines `book`-Objekts: das Buch ohne lokale Felder, Dateien, Zeitpunkt.
/// `<buch>/sync.json`: Dateiverweise im Konto und lokaler Stand.
public struct LibrarySyncMeta: Codable, Equatable, Sendable {
    public var files: BookFiles
    public var updated: Double
    /// Dateiname → Blob der geladenen Hörbuchdatei.
    public var localAudio: [String: String]?
    /// Noch nicht gesendeter Stern. Beim Holen bleibt er, der Rest kommt aus dem Konto.
    public var pendingFavorite: Bool?
    /// Angaben ohne Stern, als der Stern gesetzt wurde; nur bei abgeglichenem Stand.
    public var pendingFields: String?

    public init(files: BookFiles = BookFiles(), updated: Double = 0, localAudio: [String: String]? = nil,
                pendingFavorite: Bool? = nil, pendingFields: String? = nil) {
        self.files = files; self.updated = updated; self.localAudio = localAudio
        self.pendingFavorite = pendingFavorite; self.pendingFields = pendingFields
    }

    /// Ändert nur die Dateiverweise; Stand, Stern-Merker und geladene Fassungen bleiben.
    public func updatingFiles(_ change: (inout BookFiles) -> Void) -> LibrarySyncMeta {
        var copy = self
        change(&copy.files)
        return copy
    }
}

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

/// Eintrag der Sammlung `bookmark` (Desktop: `{key, value}`); das iPhone nutzt
/// sie für die Stimmen je Buchsprache (`tts.voice.*`, lva#352).
public struct SyncedVoicePref: Codable, Equatable, Sendable {
    public var key: String
    public var value: String
    public init(key: String, value: String) { self.key = key; self.value = value }
}

public enum LibrarySyncFormat {
    public static let collectionBook = "book"
    public static let collectionProgress = "progress"
    public static let collectionBookmark = "bookmark"
    /// Klartext in den Stücken einer Datei. Das Hosting lehnt Anfragen ab etwa 5 MiB ab;
    /// base64 und JSON machen aus einem Stück rund das 1,34-Fache.
    public static let chunkPlainBytes = 3 * 1024 * 1024
    /// Bis hierher bleibt eine Datei ein einziges Stück, wie `ONE_CHUNK_MAX` am Desktop.
    /// So stimmen früher mit 4-MiB-Stücken hochgeladene Dateien weiter.
    public static let oneChunkMaxBytes = 3_827_302

    /// Größte Datei, die mit 4-MiB-Stücken noch als ein Stück durchkam (`LEGACY_ONE_CHUNK_MAX`).
    public static let legacyOneChunkMaxBytes = 3_932_054

    /// Dieselbe Kennung kann am Hub ein oder zwei Stücke haben, der Verweis sagt beides:
    /// Stück 0 lesen und nur weitergehen, wenn die Datei noch nicht vollständig ist.
    public static func mayBeLegacySingleChunk(size: Int) -> Bool { size > oneChunkMaxBytes && size <= legacyOneChunkMaxBytes }

    /// Stück, ab dem eine Teildatei weitergeladen wird; `nil`: Sie ist vollständig.
    public static func resumeChunk(size: Int, chunks: Int, partialBytes: Int, chunkPlainBytes: Int = chunkPlainBytes) -> Int? {
        if size > 0, partialBytes == size { return nil }
        if chunks <= 1 { return 0 }
        // Alt-Blob oder zwei Stücke: Abgerissenes lässt sich nicht zuordnen, also von vorn.
        if mayBeLegacySingleChunk(size: size) { return partialBytes == chunkPlainBytes ? 1 : 0 }
        return min(partialBytes / chunkPlainBytes, chunks - 1)
    }

    public static func chunkCount(size: Int) -> Int {
        size <= oneChunkMaxBytes ? 1 : (size + chunkPlainBytes - 1) / chunkPlainBytes
    }

    /// Klartext, den ein Stück dieser Datei höchstens trägt.
    public static func chunkLimit(size: Int) -> Int { size <= oneChunkMaxBytes ? size : chunkPlainBytes }
    /// Größter Klartext eines Objekts (wie `MAX_PLAINTEXT` am Desktop).
    public static let maxPlaintext = 256 * 1024

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

    /// Kanonischer Hash eines Klartext-JSON für das Ledger: Schlüssel sortiert,
    /// `null`-Felder entfernt (Rust schreibt `null`, Swift lässt sie weg).
    public static func canonicalHash(_ json: Data) -> String {
        func strip(_ v: Any) -> Any? {
            if v is NSNull { return nil }
            if let d = v as? [String: Any] { return d.compactMapValues(strip) }
            if let a = v as? [Any] { return a.map { strip($0) ?? NSNull() } }
            return v
        }
        guard let obj = try? JSONSerialization.jsonObject(with: json), let clean = strip(obj),
              let data = try? JSONSerialization.data(withJSONObject: clean, options: [.sortedKeys]) else { return sha256Hex(json) }
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
    /// Blobs, die im Konto vollständig vorliegen: von diesem Gerät hochgeladen,
    /// vollständig von dort geladen oder vor dem Entfernen beim Hub bestätigt.
    public var uploaded: Set<String> = []
    /// Abgewiesene Stände (Objekt → Hash); erst ein geänderter Stand wird erneut gesendet.
    public var dead: [String: String] = [:]
    public init() {}
    enum CodingKeys: String, CodingKey { case cursor, entries, uploaded, dead }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        cursor = try c.decodeIfPresent(Int64.self, forKey: .cursor) ?? 0
        entries = try c.decodeIfPresent([String: Entry].self, forKey: .entries) ?? [:]
        uploaded = try c.decodeIfPresent(Set<String>.self, forKey: .uploaded) ?? []
        dead = try c.decodeIfPresent([String: String].self, forKey: .dead) ?? [:]
    }
    public static func key(_ collection: String, _ objectId: String) -> String { collection + "/" + objectId }

    /// Ein Grabstein löscht nur, was dem Abgleich bekannt ist. Weicht der lokale Stand vom zuletzt
    /// abgeglichenen ab oder war nie im Konto, gibt es ungesicherte Änderungen: Das Buch bleibt.
    /// Nur wenn sich außer dem Stern nichts geändert hat, darf der Stand aus dem Konto den Rest ersetzen.
    /// `pendingFields` hielt den Stand beim Stern fest, und zwar nur, wenn er damals abgeglichen war.
    public static func onlyFavoriteChanged(pendingFields: String?, localFields: String) -> Bool { pendingFields == localFields }

    /// Merker erst löschen, wenn das Gesendete angekommen ist und lokal nichts Neueres dazukam.
    public static func clearsPendingFavorite(accepted: String?, sent: String, local: String) -> Bool { accepted == sent && local == sent }

    /// Stand aus dem Konto mit einem noch nicht gesendeten Stern: Alles Übrige kommt aus dem Konto,
    /// damit ein Stern keine neueren Änderungen anderer Geräte überschreibt.
    public static func mergingPendingFavorite(_ favorite: Bool, into remote: LibraryBook, local: LibraryBook) -> LibraryBook {
        var book = remote
        book.favorite = favorite
        book.finished = local.finished
        if book.text == nil { book.text = local.text }
        return book
    }

    /// Kennung aller Angaben außer Stern und Zeitstempel.
    public static func fieldsHash(_ book: LibraryBook, files: BookFiles) throws -> String {
        var plain = book
        plain.favorite = false
        return LibrarySyncFormat.canonicalHash(try SyncedBook(book: plain, files: files, updated: 0).json())
    }

    public static func keepsAgainstTombstone(_ entry: Entry?, localHash: String) -> Bool {
        guard let entry, !entry.deleted else { return true }
        return entry.hash != localHash
    }
    public subscript(collection: String, objectId: String) -> Entry? {
        get { entries[Self.key(collection, objectId)] }
        set { entries[Self.key(collection, objectId)] = newValue }
    }
}
#endif
