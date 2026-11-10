import Foundation

// Gesprächsmodus (Paket M): Zeilenparser und Anfragekörper für OpenAI-kompatibles Streaming
// (`POST /chat/completions` mit `stream: true`). Reines Foundation, ohne Netz, unter Linux testbar.

public enum AssistantSSE {
    private struct Chunk: Decodable {
        struct Choice: Decodable {
            struct Delta: Decodable { let content: String? }
            let delta: Delta?
        }
        let choices: [Choice]?
    }

    private struct Message: Encodable { let role: String; let content: String }

    private struct Body: Encodable {
        let model: String
        let stream: Bool
        let max_tokens: Int
        let temperature: Double
        let messages: [Message]
    }

    /// Nutzlast hinter `data:`, oder nil bei Leerzeilen, Kommentaren (`: keep-alive`) und anderen SSE-Feldern.
    private static func payload(_ line: String) -> String? {
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.hasPrefix("data:") else { return nil }
        return String(trimmed.dropFirst(5)).trimmingCharacters(in: .whitespaces)
    }

    /// Textzuwachs aus einer Zeile `data: {...}` (`choices[0].delta.content`), sonst nil.
    /// Alles andere (Leerzeile, Kommentar, `[DONE]`, Rollen-Delta ohne `content`, `reasoning_content`, kaputtes JSON) ergibt nil.
    public static func delta(fromLine line: String) -> String? {
        guard let body = payload(line), !body.isEmpty, body != "[DONE]", body.hasPrefix("{") else { return nil }
        guard let chunk = try? JSONDecoder().decode(Chunk.self, from: Data(body.utf8)) else { return nil }
        guard let text = chunk.choices?.first?.delta?.content, !text.isEmpty else { return nil }
        return text
    }

    /// Ende des Stroms: `data: [DONE]` (auch ohne Leerzeichen nach dem Doppelpunkt).
    public static func isDone(_ line: String) -> Bool {
        payload(line) == "[DONE]"
    }

    /// Anfragekörper: System = Anweisungen, User = Prompt.
    public static func requestBody(model: String, instructions: String, prompt: String, maxTokens: Int) throws -> Data {
        let body = Body(model: model, stream: true, max_tokens: maxTokens, temperature: 0.6,
                        messages: [Message(role: "system", content: instructions), Message(role: "user", content: prompt)])
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        return try encoder.encode(body)
    }

    /// Kontextüberlauf: HTTP 400, dessen Fehlertext von Kontext, Länge oder Token spricht. Die Parameternamen
    /// `max_tokens` und `max_completion_tokens` zählen nicht (sonst wäre „Parameter nicht unterstützt“ ein Überlauf).
    public static func isContextOverflow(status: Int, body: String) -> Bool {
        guard status == 400 else { return false }
        let text = body.lowercased()
            .replacingOccurrences(of: "max_completion_tokens", with: "")
            .replacingOccurrences(of: "max_tokens", with: "")
        return ["context", "length", "token"].contains { text.contains($0) }
    }
}
