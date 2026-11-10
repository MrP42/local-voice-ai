import XCTest

// Release-Schutz der Ablage-Diagnose (Nachtrag zu D5a-3, lva#441): der Simulator-Test `--ablage-test` schreibt einen Bericht
// MIT Namen nach `Documents/ablage-testbericht.json`. Das darf es nur im Debug-Bau geben (Code und Aufruf in `#if DEBUG`),
// und die App darf Documents nicht freigeben (Dateien-App, iTunes-Dateifreigabe). Der Release-Bau ist der eigentliche
// Beweis (er übersetzt ohne diesen Code); diese Tests fangen Rückfälle schon unter `swift test` ab.

final class AblageDebugSchutzTests: XCTestCase {
    /// `apps/apple` (Datei, VoiceCoreTests, Tests, VoiceCore, apple).
    private static var appleURL: URL {
        var url = URL(fileURLWithPath: #filePath).resolvingSymlinksInPath()
        for _ in 0..<4 { url.deleteLastPathComponent() }
        return url
    }

    /// Zeilen, die nicht in einem `#if DEBUG`-Zweig stehen (der `#else`-Zweig eines `#if DEBUG` zählt als Release).
    private func zeilenAusserhalbVonDebug(_ text: String) -> [(nummer: Int, zeile: String)] {
        var stapel: [Bool] = []          // je offenem `#if`: gilt dieser Zweig nur im Debug-Bau?
        var aus: [(Int, String)] = []
        for (i, roh) in text.components(separatedBy: "\n").enumerated() {
            let z = roh.trimmingCharacters(in: .whitespaces)
            if z.hasPrefix("#if ") { stapel.append(z == "#if DEBUG"); continue }
            if z == "#else" || z.hasPrefix("#elseif") { if !stapel.isEmpty { stapel[stapel.count - 1] = false }; continue }
            if z == "#endif" { if !stapel.isEmpty { stapel.removeLast() }; continue }
            if !stapel.contains(true) { aus.append((i + 1, roh)) }
        }
        return aus
    }

    func testDebugBerichtMitNamenExistiertNurImDebugBau() throws {
        let dateien = ["iPhone/Library/AblageSync.swift", "iPhone/Library/LibraryViews.swift"]
        var gefunden = 0
        for rel in dateien {
            let text = try String(contentsOf: Self.appleURL.appendingPathComponent(rel), encoding: .utf8)
            if text.contains("ablage-testbericht") || text.contains("debugLauf") { gefunden += 1 }
            for (nummer, zeile) in zeilenAusserhalbVonDebug(text) where !zeile.trimmingCharacters(in: .whitespaces).hasPrefix("//") {
                XCTAssertFalse(zeile.contains("ablage-testbericht"), "\(rel):\(nummer) schreibt den Testbericht ausserhalb von #if DEBUG")
                XCTAssertFalse(zeile.contains("debugLauf"), "\(rel):\(nummer) ruft den Debug-Lauf ausserhalb von #if DEBUG")
                XCTAssertFalse(zeile.contains("--ablage-test"), "\(rel):\(nummer) wertet das Debug-Startargument ausserhalb von #if DEBUG aus")
            }
        }
        XCTAssertEqual(gefunden, 2, "Sucherfolg: Definition und Aufruf liegen in den beiden Dateien (sonst ist der Test blind)")
    }

    func testAppGibtDocumentsNichtFrei() throws {
        let data = try Data(contentsOf: Self.appleURL.appendingPathComponent("VoicePhone-Info.plist"))
        let plist = try XCTUnwrap(try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
        XCTAssertNil(plist["UIFileSharingEnabled"], "iTunes-Dateifreigabe würde Documents (und den Debug-Bericht) sichtbar machen")
        XCTAssertEqual(plist["LSSupportsOpeningDocumentsInPlace"] as? Bool, false, "Dateien-App darf Documents nicht in place öffnen")
        XCTAssertNil(plist["UISupportsDocumentBrowser"])
        let skript = try String(contentsOf: Self.appleURL.appendingPathComponent("scripts/generate_project.py"), encoding: .utf8)
        XCTAssertFalse(skript.components(separatedBy: "\n").contains { $0.contains("UIFileSharingEnabled") && !$0.trimmingCharacters(in: .whitespaces).hasPrefix("#") },
                       "generate_project.py darf UIFileSharingEnabled nicht setzen")
    }
}
