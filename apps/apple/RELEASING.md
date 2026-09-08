# iPhone / Apple Watch: Build und spätere Veröffentlichung

Die native App gehört unter `apps/apple` zum Hauptprojekt. Desktop-Releases bleiben
bei `app-v*` und ihren bisherigen Windows-/macOS-Workflows. Ein Merge veröffentlicht
keine App und löst kein Desktop-Release aus. Apple hat aktuell die eigene Version
0.2.0 / Build 2. Vor jeder Apple-Verteilung müssen der Projektgenerator und alle drei Info.plist-Dateien
(Phone, Watch und Komplikationen) gemeinsam eine neue Version/Buildnummer erhalten.

## Bei jeder Apple-Integration

Der Workflow `Apple validation` prüft VoiceCore, die reproduzierbare Projektdatei,
den iPhone-/Watch-Simulatorbuild und ein Release-Archiv für Geräte. Abhängigkeiten
sind an SHA-256 gebunden; Modellgewichte werden nicht in die App eingebaut. Das
CI-Archiv ist **unsigniert und nicht installierbar**; es ist ein Buildnachweis,
kein fertiges IPA und kein App-Store-Release.

## Release-Kandidat aus aktuellem main

1. Offene PRs prüfen und die Release-Auswahl dokumentieren. Sauberen Worktree auf
   dem aktuellen `origin/main` anlegen; Versionsänderung über separaten PR mergen.
2. `python3 apps/apple/scripts/setup_native_engines.py` und
   `swift test --package-path apps/apple/VoiceCore` ausführen. Mit Xcode 26.3 bauen.
3. Reale Lifecycle-, Audio-, Modell-, Berechtigungs- und Akkutests abnehmen. Die
   historischen Messungen in `docs/apple-evidence` ersetzen keine Release-Abnahme.
4. Für öffentliche Distribution Apple Developer Program und App Store Connect
   einrichten. Explizite App-IDs für Phone, Watch und Widgets sowie passende
   Distribution-Profile bereitstellen. Die bisherigen Personal-Team-Installationen
   sind Entwicklungsinstallationen, keine öffentliche Distribution.
5. In Xcode `VoicePhone` → `Any iOS Device` → `Product > Archive`, Konfiguration
   Release, korrektes Distribution-Team. Das Archiv enthält die Companion Watch.
6. Im Organizer das Archiv validieren, dann bewusst an App Store Connect/TestFlight
   verteilen. Erst nach Beta-Abnahme App Review und Veröffentlichung veranlassen.
   Datenschutzangaben, Mikrofonzwecktexte, Modelllizenzen, App-Icons, Screenshots und
   Geräteanforderungen (derzeit iOS/watchOS 26) vor Einreichung prüfen.
7. Den Apple-Release mit eigenem `apple-v<Version>`-Tag am geprüften Main-Commit
   dokumentieren. Dieser Tag löst derzeit **keinen automatischen Upload** aus und
   verändert weder Desktop-Release noch `latest.json`. GitHub-Release-Notizen können
   später auf den tatsächlich verfügbaren TestFlight-/App-Store-Eintrag verweisen.

Noch nicht eingerichtet: Distribution-Zertifikate/Profile, App-Store-Connect-App,
Upload-Automation, öffentliche Beta/App-Store-Freigabe. Keine Zugangsdaten committen.
Das Einrichten/Veröffentlichen ist ein eigener expliziter Release-Schritt.

Apple-Referenzen:
- https://developer.apple.com/documentation/xcode/distributing-your-app-for-beta-testing-and-releases
- https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/
