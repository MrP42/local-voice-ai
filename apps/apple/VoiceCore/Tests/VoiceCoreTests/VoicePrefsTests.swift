import XCTest
@testable import VoiceCore

final class VoicePrefsTests: XCTestCase {
    func testKeysWieAmDesktop() {
        XCTAssertEqual(VoicePrefs.languageKey("de-DE"), "tts.voice.lang.de")
        XCTAssertEqual(VoicePrefs.bookKey("book_17", "EN"), "tts.voice.book.book_17.en")
        XCTAssertNil(VoicePrefs.bookKey("a.b", "de"))
        XCTAssertNil(VoicePrefs.languageKey("deutsch"))
        // Open Library liefert ISO 639-2: gleicher Schlüssel wie am PC.
        XCTAssertEqual(VoicePrefs.languageKey("ger"), "tts.voice.lang.de")
        XCTAssertEqual(VoicePrefs.languageKey("eng"), "tts.voice.lang.en")
        XCTAssertEqual(VoicePrefs.owner("book_1_kurzbuch_de_abcdef012345"), "book_1")
        XCTAssertEqual(VoicePrefs.owner("book_1"), "book_1")
    }

    func testBuchVorSprache() {
        let store = [
            "tts.voice.lang.en": #"{"phonon":"en:Mark","updated":1}"#,
            "tts.voice.book.b1.en": #"{"phonon":"en:Zoey","updated":2}"#,
        ]
        let toId: (VoicePref) -> String? = { $0.phonon.map { "phonon:" + $0 } }
        let hit = VoicePrefs.resolve(owner: "b1", language: "en", read: { store[$0] }, toId: toId)
        XCTAssertEqual(hit?.id, "phonon:en:Zoey"); XCTAssertEqual(hit?.source, .book)
        let other = VoicePrefs.resolve(owner: "b2", language: "en", read: { store[$0] }, toId: toId)
        XCTAssertEqual(other?.id, "phonon:en:Mark"); XCTAssertEqual(other?.source, .language)
        XCTAssertNil(VoicePrefs.resolve(owner: "b1", language: "de", read: { store[$0] }, toId: toId))
    }

    func testIphoneWahlVerwirftFremdeDesktopKennung() {
        let old = VoicePref(desktop: "piper:thorsten", phonon: nil, updated: 1)
        let pref = VoicePrefs.iosPref(id: "com.apple.voice.Anna", phonon: nil, old: old, now: 5)
        XCTAssertNil(pref.desktop)
        let same = VoicePrefs.iosPref(id: "phonon:de:Anton", phonon: "de:Anton",
                                      old: VoicePref(desktop: "provider:phonon:Anton", phonon: "de:Anton", updated: 1), now: 5)
        XCTAssertEqual(same.desktop, "provider:phonon:Anton")
    }

    func testDesktopWertLesbar() {
        let pref = VoicePrefs.decode(#"{"desktop":"provider:phonon:en:Zoey","phonon":"en:Zoey","updated":1762450000000}"#)
        XCTAssertEqual(pref?.phonon, "en:Zoey")
        XCTAssertNil(VoicePrefs.decode(#"{"updated":1}"#))
        XCTAssertTrue(VoicePrefs.newer(local: #"{"phonon":"de:A","updated":1}"#, remote: #"{"phonon":"de:B","updated":2}"#))
        XCTAssertFalse(VoicePrefs.newer(local: #"{"phonon":"de:A","updated":3}"#, remote: #"{"phonon":"de:B","updated":2}"#))
    }

    /// Codex A1: Buchausnahme am PC aus → nach dem Abgleich auch am iPhone aus, Sprachstandard bleibt.
    func testAufhebungReistImAbgleich() {
        var store = [
            "tts.voice.lang.en": #"{"phonon":"en:Mark","updated":1}"#,
            "tts.voice.book.b1.en": #"{"phonon":"en:Zoey","updated":2}"#,
        ]
        let remote = #"{"updated":3}"#
        XCTAssertTrue(VoicePrefs.newer(local: store["tts.voice.book.b1.en"], remote: remote))
        store["tts.voice.book.b1.en"] = remote
        XCTAssertFalse(VoicePrefs.isSet(store["tts.voice.book.b1.en"]))
        let hit = VoicePrefs.resolve(owner: "b1", language: "en", read: { store[$0] }, toId: { $0.phonon })
        XCTAssertEqual(hit?.id, "en:Mark"); XCTAssertEqual(hit?.source, .language)
        // Eine ältere Wahl überschreibt die Aufhebung nicht.
        XCTAssertFalse(VoicePrefs.newer(local: remote, remote: #"{"phonon":"en:Zoey","updated":2}"#))
    }
}
