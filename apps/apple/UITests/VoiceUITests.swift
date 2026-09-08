import XCTest

@MainActor
final class VoiceUITests: XCTestCase {
    private func keepScreenshot(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name; attachment.lifetime = .keepAlways; add(attachment)
    }
    func testDeleteNoteRequiresConfirmationAndCancelKeepsOriginal() throws {
        let app = XCUIApplication(); app.launchArguments = ["--original-playback-probe"]; app.launch()
        let note = app.buttons["historyEntry"].firstMatch
        XCTAssertTrue(note.waitForExistence(timeout: 10)); note.tap()
        #if os(watchOS)
        keepScreenshot(app, name: "Watch-Notiz-Inhalt-zuerst")
        XCTAssertFalse(app.buttons["deleteNote"].isHittable)
        for _ in 0..<8 {
            if app.buttons["deleteNote"].isHittable { break }
            app.swipeUp()
        }
        keepScreenshot(app, name: "Watch-Loeschen-am-Ende")
        #endif
        app.buttons["deleteNote"].tap()
        keepScreenshot(app, name: "Sprachnotiz-Loeschbestaetigung")
        app.buttons["Abbrechen"].tap()
        XCTAssertTrue(app.buttons["originalPlayback"].exists)
        app.buttons["deleteNote"].tap()
        app.buttons["Endgültig löschen"].firstMatch.tap()
        XCTAssertTrue(note.waitForNonExistence(timeout: 5))
    }
    #if targetEnvironment(simulator)
    func testOriginalAudioCanPlayPauseResumeAndStop() throws {
        let app = XCUIApplication(); app.launchArguments = ["--original-playback-probe"]; app.launch()
        let entry = app.buttons["historyEntry"].firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 10)); entry.tap()
        let play = app.buttons["originalPlayback"]
        XCTAssertTrue(play.waitForExistence(timeout: 5)); play.tap()
        XCTAssertEqual(play.label, "Aufnahme pausieren")
        play.tap()
        XCTAssertEqual(play.label, "Aufnahme fortsetzen")
        keepScreenshot(app, name: "Originalaufnahme-pausiert")
        play.tap()
        XCTAssertEqual(play.label, "Aufnahme pausieren")
        app.buttons["originalStop"].tap()
        XCTAssertEqual(play.label, "Aufnahme anhören")
        XCTAssertFalse(app.buttons["originalStop"].isEnabled)
        play.tap()
        XCUIDevice.shared.press(.home); app.activate()
        XCTAssertEqual(play.label, "Aufnahme anhören")
    }
    func testUnreadableOriginalShowsErrorWithoutLosingTranscript() throws {
        let app = XCUIApplication(); app.launchArguments = ["--transparency-ui-probe"]; app.launch()
        app.buttons["historyEntry"].firstMatch.tap()
        app.buttons["originalPlayback"].tap()
        XCTAssertTrue(app.staticTexts["originalPlaybackError"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.buttons["originalPlayback"].label, "Aufnahme anhören")
        XCTAssertTrue(app.staticTexts["Darstellung prüfen"].exists)
    }
    #endif

    func testVoiceCanBePreviewedSelectedAndRestoredAfterRelaunch() throws {
        let app = XCUIApplication(); app.launch()
        func openVoices() {
            #if os(iOS)
            app.buttons["settings"].tap()
            #else
            app.buttons["conversationControls"].tap()
            #endif
            app.buttons["voiceSettings"].tap()
        }
        openVoices()
        let voice = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "voice-com.")).firstMatch
        XCTAssertTrue(voice.waitForExistence(timeout: 10))
        let voiceID = voice.identifier
        voice.tap()
        XCTAssertEqual(voice.value as? String, "Ausgewählt")
        let preview = app.buttons["preview-" + voiceID]
        preview.tap()
        XCTAssertEqual(preview.label, "Hörprobe stoppen")
        preview.tap()
        XCTAssertEqual(preview.label, "Hörprobe anhören")
        keepScreenshot(app, name: "Stimmenauswahl")
        app.terminate(); app.launch(); openVoices()
        XCTAssertEqual(app.buttons[voiceID].value as? String, "Ausgewählt")
        app.buttons["voice-system"].tap()
    }
    func testStartStopAndReturnFromHome() throws {
        let app = XCUIApplication()
        app.launch()
        let record = app.buttons["record"]
        XCTAssertTrue(record.waitForExistence(timeout: 15))
        record.tap()
        let permission = app.alerts.firstMatch
        if permission.waitForExistence(timeout: 2) {
            let allow = permission.buttons.matching(NSPredicate(format: "label IN %@", ["Erlauben", "Allow"])).firstMatch
            XCTAssertTrue(allow.exists)
            allow.tap()
        }
        let recording = NSPredicate(format: "label == %@", "Aufnahme sichern")
        expectation(for: recording, evaluatedWith: record)
        waitForExpectations(timeout: 15)
        record.tap()
        expectation(for: NSPredicate(format: "label == %@", "Sprechen"), evaluatedWith: record)
        waitForExpectations(timeout: 10)
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertTrue(record.waitForExistence(timeout: 10))
        XCTAssertEqual(record.label, "Sprechen")
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.lifetime = .keepAlways
        add(screenshot)
    }
    #if os(watchOS)
    func testWatchHasLocalMediaVolumeControl() throws {
        let app = XCUIApplication(); app.launchArguments = ["--original-playback-probe"]; app.launch()
        app.buttons["historyEntry"].firstMatch.tap()
        app.buttons["originalPlayback"].tap()
        app.buttons["watchVolume"].tap()
        XCTAssertTrue(app.navigationBars["Lautstärke"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["watchVolumeControl"].firstMatch.exists)
        keepScreenshot(app, name: "Watch-Lautstaerke")
    }
    func testWatchConversationOptionsAreAvailableWithoutStartingRecording() throws {
        let app = XCUIApplication(); app.launch()
        XCTAssertEqual(app.buttons["record"].label, "Sprechen")
        app.buttons["conversationControls"].tap()
        XCTAssertTrue(app.switches["autoPlayReplies"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.switches["handsFreeEnabled"].exists)
        keepScreenshot(app, name: "Watch-Gespraech-Optionen")
    }
    func testConfiguredHistoryComplicationOpensHistory() throws {
        let app = XCUIApplication()
        let spring = XCUIApplication(bundleIdentifier: "com.apple.Carousel")
        if spring.staticTexts["Switcher Face Title"].exists { XCUIDevice.shared.press(.home) }
        let face = spring.otherElements["Watch Face"]
        guard face.waitForExistence(timeout: 5) else { throw XCTSkip("Requires the configured Modular simulator face") }
        keepScreenshot(spring, name: "Watch-Komplikation")
        // The setup places the history widget in the Modular center slot.
        face.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
        XCTAssertTrue(app.navigationBars["Verlauf"].waitForExistence(timeout: 15))
        keepScreenshot(app, name: "Watch-Komplikation-Verlauf")
    }
    #endif
    #if os(iOS)
    #if !targetEnvironment(simulator)
    /// A coordinated device-test window; send the Watch fixture separately during this interval.
    func testDeviceBackgroundWindowForWatchTransfer() throws {
        let app = XCUIApplication(); app.launch()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 15))
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5))
        print("PHONE_BACKGROUND_WINDOW_STARTED")
        let window = expectation(description: "Paired Watch transfer window")
        DispatchQueue.main.asyncAfter(deadline: .now() + 90) { window.fulfill() }
        wait(for: [window], timeout: 95)
        XCTAssertEqual(app.state, .runningBackground)
        print("PHONE_BACKGROUND_WINDOW_FINISHED")
    }
    #endif
    #if targetEnvironment(simulator)
    func testReplyCompletesWhilePhoneStaysInBackground() throws {
        let app = XCUIApplication(); app.launchArguments = ["--background-processing-probe"]; app.launch()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 15))
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5))
        let time = expectation(description: "Allow the actual background lease to process the fixture")
        DispatchQueue.main.asyncAfter(deadline: .now() + 8) { time.fulfill() }
        wait(for: [time], timeout: 10)
        XCTAssertEqual(app.state, .runningBackground)
        app.activate()
        let entry = app.buttons["historyEntry"].firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 10)); entry.tap()
        XCTAssertTrue(app.staticTexts["Im Hintergrund fertiggestellt"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["Erst im Vordergrund fertiggestellt"].exists)
        keepScreenshot(app, name: "Hintergrund-Antwort")
    }
    #endif
    func testConversationOptionsDoNotRecordUntilStartedAndCanStop() throws {
        let app = XCUIApplication(); app.launch()
        app.buttons["Gespräch"].firstMatch.tap()
        let handsFree = app.switches["handsFreeEnabled"]
        XCTAssertTrue(handsFree.waitForExistence(timeout: 5))
        let original = handsFree.value as? String
        if original != "1" { handsFree.tap() }
        let record = app.buttons["record"]
        XCTAssertEqual(record.label, "Sprechen")
        XCTAssertTrue(app.switches["autoPlayReplies"].exists)
        keepScreenshot(app, name: "Gespraech-Optionen")
        record.tap()
        XCTAssertTrue(app.buttons["Gespräch beenden"].waitForExistence(timeout: 5))
        app.buttons["record"].tap()
        XCTAssertEqual(record.label, "Sprechen")
        // Delivery/processing may replace the status immediately. The recording intent must stay stopped.
        let settled = expectation(description: "Stopped conversation must not restart the microphone")
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { settled.fulfill() }
        wait(for: [settled], timeout: 4)
        XCTAssertEqual(record.label, "Sprechen")
        XCTAssertFalse(app.staticTexts["Mikrofon aktiv"].exists)
        if original != "1" { handsFree.tap() }
    }
    #if targetEnvironment(simulator)
    func testConversationRearmsAfterEmptyTranscription() throws {
        let app = XCUIApplication(); app.launchArguments = ["--conversation-no-speech-probe"]; app.launch()
        app.buttons["Gespräch"].firstMatch.tap()
        let handsFree = app.switches["handsFreeEnabled"]
        if handsFree.value as? String != "1" { handsFree.tap() }
        app.buttons["record"].tap()
        XCTAssertTrue(app.staticTexts["Keine Sprache erkannt · Du kannst weitersprechen"].firstMatch.waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Mikrofon aktiv"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts["Unexpected generation"].exists)
        keepScreenshot(app, name: "Gespraech-nach-leerer-Transkription")
        app.buttons["record"].tap()
        XCTAssertEqual(app.buttons["record"].label, "Sprechen")
        handsFree.tap()
    }
    func testConversationRearmsAfterReplyAndPausesWhenLeavingApp() throws {
        let app = XCUIApplication(); app.launchArguments = ["--conversation-cycle-probe"]; app.launch()
        app.buttons["Gespräch"].firstMatch.tap()
        let handsFree = app.switches["handsFreeEnabled"], playback = app.switches["autoPlayReplies"]
        let originalHandsFree = handsFree.value as? String, originalPlayback = playback.value as? String
        if originalHandsFree != "1" { handsFree.tap() }
        if originalPlayback != "1" { playback.tap() }
        app.buttons["record"].tap()
        XCTAssertTrue(app.staticTexts["Testantwort."].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Mikrofon aktiv"].waitForExistence(timeout: 10))
        keepScreenshot(app, name: "Gespraech-naechster-Beitrag")
        XCUIDevice.shared.press(.home)
        app.activate()
        XCTAssertEqual(app.buttons["record"].label, "Sprechen")
        XCTAssertFalse(app.staticTexts["Mikrofon aktiv"].exists)
        if originalHandsFree != "1" { handsFree.tap() }
        if originalPlayback != "1" { playback.tap() }
    }
    #endif
    #if os(iOS)
    func testSwipeDeleteWaitsForConfirmation() throws {
        let app = XCUIApplication(); app.launchArguments = ["--original-playback-probe"]; app.launch()
        app.tabBars.buttons["Verlauf"].tap()
        let note = app.buttons["historyEntry"].firstMatch
        XCTAssertTrue(note.waitForExistence(timeout: 10)); note.swipeLeft()
        app.buttons["swipeDeleteNote"].tap()
        app.buttons["Abbrechen"].tap()
        XCTAssertTrue(note.exists)
        note.swipeLeft(); app.buttons["swipeDeleteNote"].tap()
        app.buttons["Endgültig löschen"].firstMatch.tap()
        XCTAssertTrue(note.waitForNonExistence(timeout: 5))
    }
    #endif
    func testTransparencyAndMarkdown() throws {
        let app = XCUIApplication(); app.launchArguments = ["--transparency-ui-probe"]; app.launch()
        let entry = app.buttons["historyEntry"].firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 15)); entry.tap()
        XCTAssertTrue(app.staticTexts["Wichtig: Formatierter Text."].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Ergebnis"].exists)
        XCTAssertFalse(app.staticTexts["**Wichtig:** Formatierter Text."].exists)
        let info = app.buttons["processingDisclosure"].firstMatch
        XCTAssertTrue(info.exists)
        keepScreenshot(app, name: "Antwort-Transparenz")
        info.tap()
        XCTAssertTrue(app.staticTexts["UI-Testmodell"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["Dauer: 0.12 s"].exists)
        keepScreenshot(app, name: "Verarbeitungsdetails")
    }
    private func allowDownloadNotificationsIfPrompted() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let alert = springboard.alerts.firstMatch
        if alert.waitForExistence(timeout: 3) {
            let allow = alert.buttons.matching(NSPredicate(format: "label IN %@", ["Erlauben", "Allow"])).firstMatch
            if allow.exists { allow.tap() }
        }
    }
    func testModelDownloadStartsFromModelCard() throws {
        let app = XCUIApplication(); app.launch()
        app.buttons["settings"].tap()
        let download = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "download-")).firstMatch
        guard download.waitForExistence(timeout: 10) else { throw XCTSkip("All models already installed") }
        let modelID = String(download.identifier.dropFirst("download-".count))
        XCTAssertTrue(download.isEnabled); download.tap()
        allowDownloadNotificationsIfPrompted()
        let status = app.staticTexts["download-status-" + modelID]
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        XCTAssertTrue(status.isHittable)
        XCTAssertFalse(app.staticTexts["Herunterladen"].exists)
        keepScreenshot(app, name: "Modell-Status-direkt-am-Download")
        app.buttons["cancel-download-" + modelID].tap()
        XCTAssertTrue(app.buttons["download-" + modelID].waitForExistence(timeout: 5))
    }
    func testModelDownloadCompletesAfterLeavingApp() throws {
        let app = XCUIApplication()
        let modelID = "qwen2.5-0.5b-instruct-q4_k_m.gguf"
        app.launchArguments = ["--download-model-probe", modelID]; app.launch()
        allowDownloadNotificationsIfPrompted()
        app.buttons["settings"].tap()
        let status = app.staticTexts["download-status-" + modelID]
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        keepScreenshot(app, name: "Modell-Hintergrundauftrag")
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5))
        print("MODEL_DOWNLOAD_BACKGROUND_WINDOW_STARTED")
        let window = expectation(description: "Allow actual OS download and model verification in background")
        DispatchQueue.main.asyncAfter(deadline: .now() + 100) { window.fulfill() }
        wait(for: [window], timeout: 110)
        print("MODEL_DOWNLOAD_BACKGROUND_WINDOW_FINISHED")
        app.activate()
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        XCTAssertTrue(status.label.contains("bereit zur Verwendung"), status.label)
        XCTAssertTrue(app.images["installed-" + modelID].exists)
        keepScreenshot(app, name: "Modell-geprueft-und-bereit")
    }
    #if targetEnvironment(simulator)
    func testModelDownloadIntentSurvivesProcessRestart() throws {
        let app = XCUIApplication(), modelID = "qwen2.5-0.5b-instruct-q4_k_m.gguf"
        app.launchArguments = ["--download-model-probe", modelID]; app.launch()
        allowDownloadNotificationsIfPrompted()
        app.buttons["settings"].tap()
        XCTAssertTrue(app.buttons["cancel-download-" + modelID].waitForExistence(timeout: 10))
        app.terminate()
        app.launchArguments = []; app.launch()
        app.buttons["settings"].tap()
        let status = app.staticTexts["download-status-" + modelID]
        XCTAssertTrue(status.waitForExistence(timeout: 10))
        XCTAssertFalse(status.label.contains("Abgebrochen"))
        XCUIDevice.shared.press(.home)
        let window = expectation(description: "Restored background transfer finishes")
        DispatchQueue.main.asyncAfter(deadline: .now() + 100) { window.fulfill() }
        wait(for: [window], timeout: 110)
        app.activate()
        XCTAssertTrue(status.label.contains("bereit zur Verwendung"), status.label)
    }
    #endif
    func testMeetingResultsCopyAndExport() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--meeting-import-probe", "TEST-grounded.aiff"]
        app.launch()
        XCTAssertTrue(app.navigationBars["Transkripte"].waitForExistence(timeout: 15))
        let ready = app.staticTexts["Transkript & Auswertung"].firstMatch
        XCTAssertTrue(ready.waitForExistence(timeout: 180))
        app.buttons.matching(identifier: "meetingEntry").firstMatch.tap()
        XCTAssertTrue(app.navigationBars["Ergebnisse"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Zusammenfassung"].exists)
        keepScreenshot(app, name: "Aufzeichnung-Ergebnis")
        app.buttons["Kopieren"].tap()
        XCTAssertTrue(app.buttons["Kopiert"].exists)
        app.buttons["Exportieren"].tap()
        XCTAssertTrue(app.buttons["JSON"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Originaldatei"].exists)
        keepScreenshot(app, name: "Aufzeichnung-Export")
    }
    func testRetryReconstructsComponentsAfterSetupFailure() throws {
        let app = XCUIApplication(); app.launchArguments = ["--setup-failure-probe"]; app.launch()
        let status = app.staticTexts["status"]
        XCTAssertTrue(status.waitForExistence(timeout: 15))
        XCTAssertTrue(status.label.contains("Speicherprüfung"))
        app.buttons["Erneut versuchen"].tap()
        let record = app.buttons["record"]; record.tap()
        expectation(for: NSPredicate(format: "label == %@", "Aufnahme sichern"), evaluatedWith: record)
        waitForExpectations(timeout: 15)
        record.tap()
        XCTAssertEqual(record.label, "Sprechen")
    }
    func testHistoryNavigationAndModelDismissal() throws {
        let app = XCUIApplication(); app.launch()
        app.buttons["settings"].tap()
        XCTAssertTrue(app.buttons["Fertig"].waitForExistence(timeout: 10))
        app.buttons["Fertig"].tap()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 10))
        app.tabBars.buttons["Verlauf"].tap()
        XCTAssertTrue(app.navigationBars["Verlauf"].waitForExistence(timeout: 10))
        let entry = app.buttons.matching(identifier: "historyEntry").firstMatch
        XCTAssertTrue(entry.waitForExistence(timeout: 10)); entry.tap()
        XCTAssertTrue(app.staticTexts["Deine Aufnahme"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Antwort"].exists)
        keepScreenshot(app, name: "Sprachnotiz")
        app.navigationBars.buttons.firstMatch.tap()
        app.swipeDown()
        let search = app.textFields["historySearch"]
        XCTAssertTrue(search.waitForExistence(timeout: 10))
        search.tap(); search.typeText("zz_nonexistent_voice_note_zz")
        XCTAssertTrue(app.staticTexts["Keine passende Aufnahme"].waitForExistence(timeout: 10))
        keepScreenshot(app, name: "Verlauf-Suche")
        let cancelSearch = app.buttons["closeSearch"]
        XCTAssertTrue(cancelSearch.waitForExistence(timeout: 5)); cancelSearch.tap()
        app.tabBars.buttons["Sprechen"].tap()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 10))
    }
    func testModelPanelShowsLocalModelChoices() throws {
        let app = XCUIApplication(); app.launch()
        let button = app.buttons["settings"]
        XCTAssertTrue(button.waitForExistence(timeout: 15)); button.tap()
        XCTAssertTrue(app.navigationBars["Lokale Sprachmodelle"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Base"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Whisper Small"].exists)
        XCTAssertTrue(app.staticTexts["Qwen – kurze Antworten"].exists)
        keepScreenshot(app, name: "Sprachmodelle")
    }
    func testDamagedHistoryIsVisibleAndCanBeRecovered() throws {
        let app = XCUIApplication()
        app.launchArguments = ["--corrupt-history-probe"]
        app.launch()
        defer { app.launchArguments = ["--restore-history-probe"]; app.launch() }
        XCTAssertTrue(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].waitForExistence(timeout: 15))
        app.swipeUp()
        XCTAssertTrue(app.buttons["Audiodatei sichern"].exists)
        XCTAssertTrue(app.buttons["Wiederherstellung versuchen"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot()); screenshot.lifetime = .keepAlways; add(screenshot)
        app.launchArguments = ["--restore-history-probe"]; app.launch()
        XCTAssertTrue(app.buttons["record"].waitForExistence(timeout: 15))
        XCTAssertFalse(app.staticTexts["Verlauf beschädigt – Originaldateien erhalten"].exists)
    }
    #endif

}
