import Foundation
import FoundationModels

// Buch-Gespräch (Epic #307, Paket G6): Umsetzungen von `BookTalkModel` (Kern: BookTalkPrompt.swift).
// Der Kern bleibt unverändert; hier hängen nur die beiden Modelle der App an.

// MARK: - Apple Foundation Models

/// Standardmodell (Plan §5 G6): neue `LanguageModelSession` je Frage, Verlauf steht im Prompt.
///
/// Token-Zähler: Das SDK dieses Stands liefert keinen synchronen `tokenCount`, den `BookTalkTokenCounter`
/// braucht (Flag `FOUNDATION_MODELS_TOKEN_API` ist nicht gesetzt). Ohne Zähler und ohne gemessenes Byte-Profil
/// kann der Kern das Budget nicht prüfen und antwortet mit Fundstellen und Grund `budget`. Das ist nach Plan §3.3
/// gewollt, bis ein Zähler oder ein gemessenes Profil nachgewiesen ist.
/// TODO(Mac): Sobald `SystemLanguageModel.tokenCount(for:)` im SDK steht, hier einen Zähler bereitstellen
/// (Vorlagenkosten mitmessen) und in `BookTalkEngine.make` übergeben.
struct FoundationBookTalkModel: BookTalkModel {
    static var availability: SystemLanguageModel.Availability { SystemLanguageModel.default.availability }
    static var isAvailable: Bool { availability == .available }

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    guard Self.isAvailable else { throw BookTalkModelError.unavailable }
                    let session = LanguageModelSession(instructions: instructions)
                    var sent = ""
                    let stream = session.streamResponse(to: prompt, options: GenerationOptions(maximumResponseTokens: maxTokens))
                    for try await snapshot in stream {
                        try Task.checkCancellation()
                        // Die Schnappschüsse enthalten den bisherigen Text; der Kern will nur das Neue.
                        let full = snapshot.content
                        // Gemeinsames Präfix: wächst der Text, kommt das Neue; wird er umgeschrieben, geht nichts verloren,
                        // was hinter dem gemeinsamen Teil steht (ein Durchlauf je Schnappschuss).
                        let common = sent.commonPrefix(with: full)
                        let delta = full.dropFirst(common.count)
                        if !delta.isEmpty { continuation.yield(String(delta)) }
                        sent = full
                    }
                    continuation.finish()
                } catch let error as LanguageModelSession.GenerationError {
                    if case .exceededContextWindowSize = error { continuation.finish(throwing: BookTalkModelError.contextOverflow) }
                    else { continuation.finish(throwing: error) }
                } catch {
                    continuation.finish(throwing: error)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}

// MARK: - llama.cpp (Gemma 4 E2B)

/// Exakter Zähler über den Wortschatz des Modells (`llama_tokenize` in `LlamaBridge.mm`, nur Vokabular geladen).
/// Der Kern ruft `count(instructions) + count(prompt)`; die Kosten des Chat-Rahmens (BOS, Rollen- und
/// Zugmarker) stecken zur Hälfte in jedem Aufruf, die Summe ist damit mindestens so groß wie der echte Prompt.
/// Texte werden wie bei der Erzeugung bereinigt und beschnitten (`BookTalkChatFrame`).
final class LlamaTokenCounter: BookTalkTokenCounter, @unchecked Sendable {
    private let handle: OpaquePointer
    private let perCall: Int

    init?(modelURL: URL, frame: BookTalkChatFrame = .gemma4) {
        guard let handle = lv_llama_load(modelURL.path, 1) else { return nil }
        let head = lv_llama_count(handle, frame.head, 1), middle = lv_llama_count(handle, frame.middle, 1), tail = lv_llama_count(handle, frame.tail, 1)
        guard head >= 0, middle >= 0, tail >= 0 else { lv_llama_free(handle); return nil }
        self.handle = handle
        self.perCall = (Int(head + middle + tail) + 1) / 2
    }

    deinit { lv_llama_free(handle) }

    func count(_ text: String) -> Int {
        let body = BookTalkChatFrame.sanitize(text).trimmingCharacters(in: .whitespacesAndNewlines)
        let n = lv_llama_count(handle, body, 0)
        // Unzählbar: lieber zu groß schätzen als das Budget zu unterschreiten (UTF-8-Bytes sind eine Obergrenze).
        return (n >= 0 ? Int(n) : body.utf8.count) + perCall
    }
}

/// Erzeugt über `lv_llama_chat`. Das Modell bleibt zwischen Fragen geladen (warme zweite Frage) und wird mit
/// `release()` freigegeben (Blatt zu, App im Hintergrund). Alles läuft auf einer seriellen Warteschlange.
final class LlamaBookTalkModel: BookTalkModel, @unchecked Sendable {
    private static let queue = DispatchQueue(label: "de.localvoice.booktalk.llama", qos: .userInitiated)
    private let url: URL
    private let contextTokens: Int
    private let frame: BookTalkChatFrame
    private var loaded: OpaquePointer?

    init(modelURL: URL, contextTokens: Int, frame: BookTalkChatFrame = .gemma4) {
        self.url = modelURL
        self.contextTokens = contextTokens
        self.frame = frame
    }

    /// Gibt das volle Modell frei (nach laufender Erzeugung).
    func release() {
        Self.queue.async { [self] in
            if let loaded { lv_llama_free(loaded) }
            loaded = nil
        }
    }

    private func model() throws -> OpaquePointer {
        if let loaded { return loaded }
        guard let handle = lv_llama_load(url.path, 0) else { throw BookTalkModelError.unavailable }
        loaded = handle
        return handle
    }

    /// Hält Strom und Bereinigung für den C-Rückruf.
    private final class Sink {
        let continuation: AsyncThrowingStream<String, Error>.Continuation
        var cleaner = BookTalkStreamCleaner()
        init(_ continuation: AsyncThrowingStream<String, Error>.Continuation) { self.continuation = continuation }
    }

    /// Abbruchmarke, die vor dem (langen) Laden des Modells gelesen werden kann.
    private final class Cancelled: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        func set() { lock.lock(); value = true; lock.unlock() }
        var isSet: Bool { lock.lock(); defer { lock.unlock() }; return value }
    }

    func stream(instructions: String, prompt: String, maxTokens: Int) -> AsyncThrowingStream<String, Error> {
        AsyncThrowingStream { continuation in
            let cancellation = InferenceCancellation()
            let cancelled = Cancelled()
            continuation.onTermination = { _ in cancelled.set(); cancellation.cancel() }
            Self.queue.async { [self] in
                do {
                    guard !cancelled.isSet else { throw CancellationError() }
                    let model = try model()
                    guard !cancelled.isSet else { throw CancellationError() }
                    let sink = Sink(continuation)
                    let code = lv_llama_chat(
                        model, frame.head, frame.systemText(instructions), frame.middle, frame.userText(prompt), frame.tail,
                        Int32(maxTokens), Int32(contextTokens),
                        { bytes, length, context in
                            guard let bytes, let context, length > 0 else { return 0 }
                            let sink = Unmanaged<Sink>.fromOpaque(context).takeUnretainedValue()
                            let piece = String(decoding: UnsafeBufferPointer(start: UnsafeRawPointer(bytes).assumingMemoryBound(to: UInt8.self), count: Int(length)), as: UTF8.self)
                            let text = sink.cleaner.feed(piece)
                            if !text.isEmpty { sink.continuation.yield(text) }
                            return sink.cleaner.finished ? 1 : 0
                        },
                        Unmanaged.passUnretained(sink).toOpaque(), cancellation.pointer)
                    let rest = sink.cleaner.finish()
                    if !rest.isEmpty { continuation.yield(rest) }
                    switch code {
                    case 0: continuation.finish()
                    case 4: continuation.finish(throwing: BookTalkModelError.contextOverflow)
                    case 7: continuation.finish(throwing: CancellationError())
                    default: continuation.finish(throwing: LlamaChatError(code: Int(code)))
                    }
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
    }
}

/// Fehlercode aus `lv_llama_chat`; ohne Inhalte, nur die Zahl.
struct LlamaChatError: Error { let code: Int }

// MARK: - Auswahl

enum BookTalkModelChoice: String, CaseIterable, Identifiable {
    case apple, gemma
    var id: String { rawValue }
    var title: String { self == .apple ? "Apple-Systemmodell" : "Gemma 4 E2B" }
    static let key = "book_talk_model"
    static var stored: BookTalkModelChoice {
        UserDefaults.standard.string(forKey: key).flatMap(BookTalkModelChoice.init) ?? .apple
    }
}

/// Was die Sitzung für ein Gespräch braucht.
struct BookTalkEngine {
    let model: (any BookTalkModel)?
    let counter: (any BookTalkTokenCounter)?
    let budget: BookTalkBudget
    /// Anzeige unter der Antwort („Lokal · Apple-Systemmodell“).
    let label: String
    /// Warum kein Modell läuft (Hinweis im Blatt), nil wenn eins bereitsteht.
    let note: String?
    /// Zum Freigeben beim Schließen.
    let llama: LlamaBookTalkModel?

    /// Feature-Schalter `book_talk_enabled` (Standard aus): aus = nur Fundstellen. Der Eval-Nachweis nach Plan §3.9
    /// (`book_talk_eval_pass.json`, Bindung an Modell und Prompt-Version) ist noch nicht angebunden.
    static let enabledKey = "book_talk_enabled"
    static var enabled: Bool { UserDefaults.standard.bool(forKey: enabledKey) }

    static func make(choice: BookTalkModelChoice, enabled: Bool) -> BookTalkEngine {
        guard enabled else {
            return BookTalkEngine(model: nil, counter: nil, budget: .foundationModels, label: "Fundstellen",
                                  note: "KI-Antworten sind aus. Das Blatt zeigt Fundstellen. Einschalten unter Einstellungen › Buch-Gespräch.", llama: nil)
        }
        switch choice {
        case .apple:
            guard FoundationBookTalkModel.isAvailable else {
                return BookTalkEngine(model: nil, counter: nil, budget: .foundationModels, label: "Fundstellen",
                                      note: "Das Apple-Systemmodell ist auf diesem iPhone nicht verfügbar.", llama: nil)
            }
            return BookTalkEngine(model: FoundationBookTalkModel(), counter: nil, budget: .foundationModels, label: "Lokal · Apple-Systemmodell",
                                  note: "Für das Apple-Systemmodell fehlt noch ein Token-Zähler: es gibt Fundstellen. Gemma 4 E2B beantwortet Fragen.", llama: nil)
        case .gemma:
            guard let descriptor = LocalModel.all.first(where: \.isGemma) else {
                return BookTalkEngine(model: nil, counter: nil, budget: .large, label: "Fundstellen", note: "Gemma 4 E2B ist nicht eingetragen.", llama: nil)
            }
            let file = ModelLibrary.folder.appendingPathComponent(descriptor.name)
            let size = (try? FileManager.default.attributesOfItem(atPath: file.path)[.size] as? NSNumber)?.int64Value ?? 0
            guard size == descriptor.bytes else {
                return BookTalkEngine(model: nil, counter: nil, budget: .large, label: "Fundstellen",
                                      note: "Gemma 4 E2B ist nicht geladen (Einstellungen › Lokale Sprachmodelle).", llama: nil)
            }
            // Größe und SHA-256 prüft die Installation (`ModelLibrary.install`); ein erneutes Hashen von 3 GB vor jeder Frage entfällt.
            guard let counter = LlamaTokenCounter(modelURL: file) else {
                return BookTalkEngine(model: nil, counter: nil, budget: .large, label: "Fundstellen",
                                      note: "Gemma 4 E2B lässt sich mit dieser llama.cpp-Version nicht laden. Es gibt Fundstellen.", llama: nil)
            }
            let model = LlamaBookTalkModel(modelURL: file, contextTokens: BookTalkBudget.large.contextTokens)
            return BookTalkEngine(model: model, counter: counter, budget: .large, label: "Lokal · Gemma 4 E2B", note: nil, llama: model)
        }
    }
}
