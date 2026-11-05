import Foundation

/// Hörbuch im Hintergrund laden (Epic #210, P8 Paket 3): Plan und Zustand,
/// plattformneutral (nur Foundation) und ohne Netz prüfbar.
///
/// Ein Hörbuch besteht aus Teilen (Dateien), jedes Teil aus Stücken zu je
/// `chunkPlainBytes` Klartext. Jedes Stück ist ein eigener Download-Auftrag;
/// angekommene Stücke werden in Hörreihenfolge an `<teil>.part` angehängt.
public enum AudioDownload {
    public struct Part: Codable, Equatable, Sendable {
        public var blob: String
        public var size: Int
        public var chunks: Int
        /// Dateiname im Ordner `audio/` (nie ein Pfad).
        public var file: String
        public init(blob: String, size: Int, chunks: Int, file: String) {
            self.blob = blob; self.size = size; self.chunks = chunks; self.file = file
        }
    }

    public enum Phase: String, Codable, Sendable { case running, paused, failed }

    public struct Job: Codable, Equatable, Sendable {
        public var bookId: String
        public var parts: [Part]
        public var phase: Phase
        /// „Jetzt mobil laden“: für dieses Buch auch über Mobilfunk.
        public var allowCellular: Bool
        public var message: String?
        public init(bookId: String, parts: [Part], phase: Phase = .running, allowCellular: Bool = false, message: String? = nil) {
            self.bookId = bookId; self.parts = parts; self.phase = phase; self.allowCellular = allowCellular; self.message = message
        }
        public var totalBytes: Int { parts.reduce(0) { $0 + $1.size } }
    }

    /// Ein Stück eines Teils.
    public struct ChunkKey: Hashable, Codable, Sendable, Comparable {
        public var part: Int
        public var index: Int
        public init(part: Int, index: Int) { self.part = part; self.index = index }
        public static func < (a: ChunkKey, b: ChunkKey) -> Bool { a.part != b.part ? a.part < b.part : a.index < b.index }
    }

    /// Stand eines Teils auf dem Gerät.
    public struct PartState: Equatable, Sendable {
        /// Fertige Datei liegt mit passender Größe vor.
        public var complete: Bool
        /// Bytes in `<teil>.part`.
        public var partialBytes: Int
        public init(complete: Bool, partialBytes: Int) { self.complete = complete; self.partialBytes = partialBytes }
    }

    /// Höchstens so viele Stücke gleichzeitig unterwegs; begrenzt auch den
    /// Platz für Stücke, die vor ihrem Vorgänger ankommen.
    public static let window = 8
    /// Freier Platz, der nach dem Laden mindestens bleiben soll.
    public static let reserveBytes = 200 * 1024 * 1024

    /// Nächstes Stück, das an ein Teil angehängt werden muss (`nil`: Teil fertig).
    public static func nextChunk(_ part: Part, _ state: PartState, chunkPlainBytes: Int) -> Int? {
        if state.complete { return nil }
        return LibrarySyncFormat.resumeChunk(size: part.size, chunks: part.chunks, partialBytes: state.partialBytes, chunkPlainBytes: chunkPlainBytes)
    }

    /// Stück 1 fehlt am Hub: Es war ein Alt-Blob aus einem Stück, die Teildatei ist abgerissen
    /// und muss neu von Stück 0 geladen werden.
    public static func restartsWhenMissing(_ part: Part, index: Int) -> Bool {
        index == 1 && LibrarySyncFormat.mayBeLegacySingleChunk(size: part.size)
    }

    /// Merkt sich je Auftrag, ob der Rückfall auf Stück 0 schon genutzt ist. Unabhängig von den
    /// allgemeinen Fehlversuchen: Ein Netzfehler vorher darf ihn nicht verbrauchen.
    public struct LegacyFallback: Sendable {
        private var used: Set<String> = []
        public init() {}

        /// `true`: Teildatei verwerfen und neu beginnen. Fehlt das Stück danach wieder, ist es ein Fehler.
        public mutating func shouldRestart(_ part: Part, index: Int, task: String) -> Bool {
            restartsWhenMissing(part, index: index) && used.insert(task).inserted
        }
    }

    /// Fehlende Stücke in Hörreihenfolge, ohne angekommene und laufende; höchstens `limit`.
    public static func missingChunks(_ job: Job, states: [PartState], staged: Set<ChunkKey>, inFlight: Set<ChunkKey>,
                                     chunkPlainBytes: Int, limit: Int = window) -> [ChunkKey] {
        let budget = limit - inFlight.count - staged.count
        guard budget > 0 else { return [] }
        var out: [ChunkKey] = []
        for (p, part) in job.parts.enumerated() {
            let state = p < states.count ? states[p] : PartState(complete: false, partialBytes: 0)
            guard let first = nextChunk(part, state, chunkPlainBytes: chunkPlainBytes) else { continue }
            // Ob es ein zweites Stück gibt, zeigt erst das erste.
            let last = LibrarySyncFormat.mayBeLegacySingleChunk(size: part.size) ? first + 1 : part.chunks
            for i in first..<min(last, part.chunks) {
                let key = ChunkKey(part: p, index: i)
                if staged.contains(key) || inFlight.contains(key) { continue }
                out.append(key)
                if out.count == budget { return out }
            }
        }
        return out
    }

    /// Geladene Bytes (fertige Teile plus begonnene).
    public static func doneBytes(_ job: Job, states: [PartState]) -> Int {
        zip(job.parts, states).reduce(0) { sum, pair in
            sum + (pair.1.complete ? pair.0.size : min(pair.1.partialBytes, pair.0.size))
        }
    }

    public static func isComplete(_ job: Job, states: [PartState]) -> Bool {
        states.count == job.parts.count && states.allSatisfy(\.complete)
    }

    /// Speicherprüfung vor dem Start: `nil` wenn genug frei ist, sonst die Meldung.
    public static func spaceProblem(remainingBytes: Int, freeBytes: Int64?, format: (Int64) -> String) -> String? {
        guard let freeBytes else { return nil }  // unbekannt: nicht blockieren
        let needed = Int64(remainingBytes) + Int64(reserveBytes)
        guard freeBytes < needed else { return nil }
        return "Zu wenig Speicher: Das Hörbuch braucht noch \(format(Int64(remainingBytes))), frei sind \(format(freeBytes)). "
            + "Unter „Konto & Sync › Speicher“ lassen sich fertig gehörte Hörbücher entfernen."
    }

    // MARK: Namen

    /// Kennung eines Download-Auftrags: `<buch>|<teil>|<stück>`.
    public static func taskDescription(bookId: String, _ key: ChunkKey) -> String { "\(bookId)|\(key.part)|\(key.index)" }

    public static func parseTask(_ description: String?) -> (bookId: String, key: ChunkKey)? {
        guard let parts = description?.split(separator: "|", omittingEmptySubsequences: false), parts.count == 3,
              !parts[0].isEmpty, let p = Int(parts[1]), let i = Int(parts[2]), p >= 0, i >= 0 else { return nil }
        return (String(parts[0]), ChunkKey(part: p, index: i))
    }

    /// Ordner für angekommene, noch nicht angehängte Stücke (in `audio/`).
    public static let stagingFolder = ".stuecke"

    public static func stagedName(_ key: ChunkKey) -> String { String(format: "%03d-%06d.json", key.part, key.index) }

    public static func parseStaged(_ name: String) -> ChunkKey? {
        guard name.hasSuffix(".json") else { return nil }
        let bits = name.dropLast(5).split(separator: "-")
        guard bits.count == 2, let p = Int(bits[0]), let i = Int(bits[1]) else { return nil }
        return ChunkKey(part: p, index: i)
    }
}
