import Foundation
import XCTest
@testable import VoiceCore

// Gesprächsmodus (Paket M): OpenAI-kompatibler Stromparser, nur Foundation.

final class AssistantSSETests: XCTestCase {
    func testPlainDelta() {
        XCTAssertEqual(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":"Hallo"}}]}"#), "Hallo")
    }

    func testNoSpaceAfterColonAndCarriageReturn() {
        XCTAssertEqual(AssistantSSE.delta(fromLine: "data:{\"choices\":[{\"delta\":{\"content\":\" Welt\"}}]}\r"), " Welt")
    }

    func testIgnoresBlankCommentAndOtherFields() {
        XCTAssertNil(AssistantSSE.delta(fromLine: ""))
        XCTAssertNil(AssistantSSE.delta(fromLine: "   "))
        XCTAssertNil(AssistantSSE.delta(fromLine: ": keep-alive"))
        XCTAssertNil(AssistantSSE.delta(fromLine: "event: message"))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"{"choices":[{"delta":{"content":"ohne data"}}]}"#))
    }

    func testDone() {
        XCTAssertNil(AssistantSSE.delta(fromLine: "data: [DONE]"))
        XCTAssertNil(AssistantSSE.delta(fromLine: "data:[DONE]"))
        XCTAssertTrue(AssistantSSE.isDone("data: [DONE]"))
        XCTAssertTrue(AssistantSSE.isDone("data:[DONE]\r"))
        XCTAssertFalse(AssistantSSE.isDone(#"data: {"choices":[]}"#))
        XCTAssertFalse(AssistantSSE.isDone(""))
        XCTAssertFalse(AssistantSSE.isDone(": [DONE]"))
    }

    func testRoleDeltaWithoutContent() {
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"role":"assistant"}}]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":null}}]}"#))
    }

    func testMissingFields() {
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{}]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"id":"x","usage":{"total_tokens":3}}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#))
    }

    func testReasoningContentIsIgnored() {
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"reasoning_content":"Ich überlege"}}]}"#))
        XCTAssertEqual(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"reasoning_content":"x","content":"Antwort"}}]}"#), "Antwort")
    }

    func testUmlautsAndUnicodeEscapes() {
        XCTAssertEqual(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":"Größe für ß"}}]}"#), "Größe für ß")
        XCTAssertEqual(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":"Käse über ß"}}]}"#), "Käse über ß")
        // Surrogatpaar (Emoji) und Zeilenumbruch
        XCTAssertEqual(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":"😀\n"}}]}"#), "😀\n")
    }

    func testBrokenJSONGivesNilWithoutCrash() {
        XCTAssertNil(AssistantSSE.delta(fromLine: "data: {"))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":"abgeschnitten"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: "data: not json"))
        XCTAssertNil(AssistantSSE.delta(fromLine: "data: [1,2,3]"))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":"kaputt"}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":42}}]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: #"data: {"choices":[{"delta":{"content":["a"]}}]}"#))
        XCTAssertNil(AssistantSSE.delta(fromLine: "data:"))
    }

    func testRequestBody() throws {
        let data = try AssistantSSE.requestBody(model: "llama3.1:8b", instructions: "Sei kurz „bitte“.", prompt: "Wie spät?\nÜberleg.", maxTokens: 400)
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        XCTAssertEqual(json["model"] as? String, "llama3.1:8b")
        XCTAssertEqual(json["stream"] as? Bool, true)
        XCTAssertEqual(json["max_tokens"] as? Int, 400)
        XCTAssertEqual(try XCTUnwrap(json["temperature"] as? Double), 0.6, accuracy: 0.0001)
        let messages = try XCTUnwrap(json["messages"] as? [[String: String]])
        XCTAssertEqual(messages.count, 2)
        XCTAssertEqual(messages[0], ["role": "system", "content": "Sei kurz „bitte“."])
        XCTAssertEqual(messages[1], ["role": "user", "content": "Wie spät?\nÜberleg."])
    }

    func testRequestBodyRoundTripsIntoDelta() throws {
        // Was wir senden, ist gültiges JSON; ein Server-Chunk mit demselben Text wird unverändert zurückgelesen.
        let text = "Zeile 1\n\"Zitat\" \\ Rückstrich"
        let data = try AssistantSSE.requestBody(model: "m", instructions: text, prompt: text, maxTokens: 1)
        let json = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
        let messages = try XCTUnwrap(json["messages"] as? [[String: String]])
        XCTAssertEqual(messages[0]["content"], text)
        let chunk = String(data: try JSONSerialization.data(withJSONObject: ["choices": [["delta": ["content": text]]]]), encoding: .utf8)!
        XCTAssertEqual(AssistantSSE.delta(fromLine: "data: " + chunk), text)
    }

    func testContextOverflowClassification() {
        XCTAssertTrue(AssistantSSE.isContextOverflow(status: 400, body: #"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#))
        XCTAssertTrue(AssistantSSE.isContextOverflow(status: 400, body: "exceed_context_size_error"))
        XCTAssertTrue(AssistantSSE.isContextOverflow(status: 400, body: "Prompt has too many TOKENS"))
        XCTAssertFalse(AssistantSSE.isContextOverflow(status: 400, body: #"{"error":"Unsupported parameter: 'max_tokens'"}"#))
        XCTAssertFalse(AssistantSSE.isContextOverflow(status: 400, body: "model not loaded"))
        XCTAssertFalse(AssistantSSE.isContextOverflow(status: 400, body: ""))
        XCTAssertFalse(AssistantSSE.isContextOverflow(status: 500, body: "context length"))
        XCTAssertFalse(AssistantSSE.isContextOverflow(status: 401, body: "token"))
    }
}
