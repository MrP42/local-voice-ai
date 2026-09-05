# Local Voice AI: Mobile First und Watch-Interaktion

Stand: 05.09.2026. Architekturvorschlag und Entwicklungsfahrplan, keine bereits implementierte Portierung.
Projekt: `C:\Users\wolff\local-voice-project`. Geprüfter Commit: `5f85a99`, Paketversion 0.14.0. Arbeitsbaum bei Prüfung ohne angezeigte Änderungen; Git meldete fehlenden Lesezugriff auf die globale Ignore-Datei. Keine Builds oder Gerätetests in dieser Planungsrunde.

## 1. Produkt und Umfang

Ein kompakter persönlicher Sprachbegleiter: auf der Watch sprechen, eine kurze gesprochene Antwort erhalten und den Verlauf im Tagebuch wiederfinden. Aufnahme und Speicherung müssen auch bei nicht verfügbarer KI funktionieren. Die Watch ist die primäre Interaktionsfläche; das iPhone übernimmt möglichst die Verarbeitung.

Vom Benutzer bestätigte Referenzgeräte: Apple Watch Ultra 3, iPhone 15 Pro Max, MacBook Pro 16 Zoll mit Intel-Prozessor. Ausbaustufe 2 umfasst ältere Apple-Geräte, Android-Smartphones, Android-Tablets und Android-Uhren. Bei Uhren ist Wear OS der erste Zielkandidat; andere proprietäre Uhrenbetriebssysteme sind keine automatische Folge der Android-Unterstützung.

Stufe 1 enthält Push-to-Talk, Antwortwiedergabe, Unterbrechen/Abbrechen, Wiederaufnahme nach Verbindungsverlust, ein einfaches lokales Tagebuch auf dem iPhone und eine vereinheitlichte, kompakte Desktop-Oberfläche. Mehrere Gesprächsrunden sind möglich; die erste Version ist halbduplex: während der Sprachausgabe wird das Mikrofon nicht dauerhaft offen gehalten. Ein neuer PTT-Start stoppt die Wiedergabe.

Ein explizit gestarteter Aufmerksamkeitsmodus wird als eigener Versuch vorbereitet. Ganztägiges Wakeword, automatische Wissensbasis, Outlook und Microsoft To Do sind nachgelagerte Meilensteine, keine Abhängigkeiten des Sprachpfads.

## 2. Tatsächlicher Bestand

- `apps/local-voice/src-tauri/Cargo.toml`: Tauri 2.11.5, Rust, lokale STT-Engines, SQLite, Desktop-Audio und zahlreiche Desktop-Abhängigkeiten. Mobile-Guards für einzelne Plugins sind vorhanden, aber kein Nachweis einer vollständigen iOS-Portierung.
- `apps/local-voice/src-tauri/src/managers/history.rs`: SQLite-Verlauf, Migrationen, Original und nachbearbeiteter Text. Der Manager hängt direkt an Tauri AppHandle; er ist noch keine mobile Kernbibliothek. Lokale numerische IDs sind für geräteübergreifende Identität zu ergänzen.
- `apps/local-voice/src-tauri/src/llm_client.rs`, `summarizer.rs`, `refinement/`: Kandidaten für wiederverwendbare Provider-Verträge, Zusammenfassung und Texttreueprüfungen. Wiederverwendbarkeit muss pro Modul durch Entkopplung und Tests belegt werden.
- `apps/local-voice/src-tauri/swift/apple_intelligence.swift`: existierende Swift-/Foundation-Models-Anbindung für macOS. Als Referenz verwendbar; die synchrone C-Brücke mit Semaphore ist kein Muster für den UI-Thread der neuen iOS-App.
- `docs/ROADMAP.md`: Meetings, TTS-Engine-Abstraktion, Fish/Piper, Windows- und macOS-Veröffentlichung bereits vorhanden. macOS-Notarisierung und echte Intel-Verifikation waren dort noch offen.
- `.github/workflows/release-windows.yml` und `release-macos.yml`: vorhandene Plattformpipelines. Keine Watch-/iOS-App in der überprüften Dateisuche, lediglich die macOS-Swift-Brücke.
- Der alte Prompt vom August ist keine aktuelle Fehlerliste: STATUS dokumentiert, dass die ursprüngliche Streaming-Defektannahme später anhand der Fixes korrigiert wurde. Keine pauschale Rücknahme dieser Arbeit.

## 3. Architekturentscheidung

| Option | Vorteil | Kosten und Risiko | Bewertung |
|---|---|---|---|
| SwiftUI für iPhone/Watch, Tauri für Desktop, gemeinsame Verträge und kleiner Rust-Kern | Direkter Zugriff auf Apple-Audio/Lifecycle; vorhandene Desktop-App bleibt nutzbar | Zwei UI-Implementierungen; Android bekommt später eigene native Oberfläche | Empfehlung bei Watch-Priorität |
| Tauri auch für iPhone/Android, native Watch-App | Mehr React-Wiederverwendung auf Phones/Tablets | Audio, Hintergrundbetrieb und Watch-Anbindung brauchen trotzdem native Integration; Desktop-Abhängigkeiten müssen getrennt werden | Vertretbar bei höherer Priorität auf UI-Code-Sharing |
| Flutter als neue Phone-/Desktop-Oberfläche plus native Watch | Gemeinsame neue Phone-/Tablet-UI | Zusätzlicher Frameworkwechsel und größere Migration; Watch bleibt ein Sonderfall | Gegenwärtig kein ausreichender Vorteil |

Tauri dokumentiert Desktop- und Mobile-Distribution, aber daraus folgt keine watchOS-Unterstützung: [Tauri Distribution](https://v2.tauri.app/distribute/).

Empfohlene Verantwortlichkeiten:

| Baustein | Verantwortung |
|---|---|
| Watch, SwiftUI + AVFoundation | Aufnahme, kurze lokale Audio-Warteschlange, haptisches Feedback, Status, Antwortwiedergabe |
| iPhone/iPad, SwiftUI | Spracherkennung, Gesprächssteuerung, lokale KI soweit verfügbar, Tagebuch, Plattformberechtigungen |
| Windows/Mac, bestehendes Tauri | Bestehendes Diktat/Meetings/TTS, kompakte UI, später authentifizierter optionaler Rechenpartner |
| Plattformneutraler Vertrag | Versionierte Nachrichten, Session-IDs, Ereignisse, Fehlertypen, Wiederholungsregeln und Testdaten |
| Kleiner Rust-Kern, schrittweise extrahiert | Bewährte reine Validierung und später plattformfreie Domänenlogik; keine Desktop-Audio-/Tray-Abhängigkeiten |
| Android/Wear OS in Stufe 2 | Kotlin/Compose als Startempfehlung; dieselben Verträge und gegebenenfalls Rust-Kern über geprüfte Bindings |

Zunächst Verträge und Testfixtures teilen. Einen Rust-/Swift-FFI-Layer erst für konkret nützliche Module einziehen. Die Watch erhält keinen vollständigen Desktop-Rust-/ONNX-/TTS-Stack. macOS kann später eine SwiftUI-Oberfläche bekommen, wenn deren Nutzen eine zweite Migration rechtfertigt.

## 4. Audio, Transport und Offline-Verhalten

Zustände: bereit → nimmt auf → lokal gesichert → überträgt → verarbeitet → antwortet → bereit. Abbruch, Fehler und „wartet auf iPhone“ sind explizite Zustände. Sichtbare Bestätigung bedeutet lokal gesichert, nicht bloß ein erfolgreiches UI-Ereignis.

Jede Aufnahme erhält eine stabile Session-ID. Audio wird während der Aufnahme in begrenzten Segmenten auf der Watch gesichert. Das iPhone bestätigt dauerhafte Annahme. Erst nach dieser Bestätigung und gemäß Aufbewahrungsregel darf die Watch ihre Kopie entfernen. Wiederholte Zustellung erzeugt keinen doppelten Tagebucheintrag. Speicherlimit erreicht: sichtbar stoppen oder Aufnahme ablehnen, niemals alte unbestätigte Aufnahmen still löschen.

WatchConnectivity ist der erste Kommunikationsadapter für das Gerätepaar. Interaktive Nachrichten sind nur bei erreichbarem Gegenüber sinnvoll; Hintergrunddateien dienen der späteren Zustellung, nicht als garantierter Echtzeitkanal. Kleine Audiosegmente über den interaktiven Pfad werden gemessen, nicht als performanter Audiostream vorausgesetzt. Falls das nicht reicht, zunächst kurze komplette Sprachturns übertragen. Ein anderer Transport wird erst nach einem gemessenen Problem eingeführt. [Apple WatchConnectivity](https://developer.apple.com/documentation/WatchConnectivity/transferring-data-with-watch-connectivity), [Apple Datentransfer](https://developer.apple.com/videos/play/wwdc2021/10003/).

Der entscheidende Versuch prüft iPhone entsperrt, gesperrt, App im Hintergrund, App vom Nutzer beendet, Verbindung getrennt und wiederhergestellt. Die Aufweckfunktion von WatchConnectivity garantiert keine unbegrenzte Ausführung oder ML-Verfügbarkeit im Hintergrund. Scheitert der lokale gesperrte-iPhone-Pfad, muss das Produkt ehrlich „gespeichert, Verarbeitung folgt“ anzeigen. Ein expliziter Online-Modus kann später Antwortverfügbarkeit verbessern; er darf nicht unbemerkt aktiviert werden.

STT auf dem iPhone: zuerst Apple SpeechAnalyzer/SpeechTranscriber mit Laufzeitprüfung von Sprache, Gerät und heruntergeladenen Assets testen. Gegen die deutsche Desktop-Testmenge vergleichen; Desktop-CPU-Zahlen sind keine iPhone-Messwerte. Bei unzureichender Qualität einen lokalen alternativen Engine-Kandidaten vergleichen. Das Apple-API ist kein Beleg für dieselbe Inferenz direkt auf der Watch. [Apple SpeechAnalyzer](https://developer.apple.com/videos/play/wwdc2025/277/).

KI-Antwort: Foundation Models auf unterstützten und aktivierten Geräten als lokaler Kandidat; Verfügbarkeit prüfen und eigenes Qualitätsset verwenden. Nicht als gleichwertigen Ersatz für große Cloudmodelle behandeln. Ohne Modell bleibt die Sprachnotiz erhalten. Wahlweise explizit konfigurierte Cloud-KI oder später ein sicher gekoppelter Desktop-Rechenpartner. Niemals das ganze Tagebuch als Standardkontext übertragen. [Apple Foundation Models](https://developer.apple.com/videos/play/wwdc2025/286/).

TTS: zunächst verfügbare Systemstimmen und reale Watch-Ausgaberouten prüfen. Falls Synthese auf dem iPhone erfolgt, Antwortaudio zur Watch übertragen. Fish/Piper bleiben Desktop-Optionen; Stimmenklonen ist keine mobile Voraussetzung. Fehlende Stimme oder unterbrochene Audioausgabe lassen den Antworttext bestehen.

## 5. Aufmerksamkeitsmodus und Energie

PTT ist der Standard. Für längeres Zuhören ist ein eigener, sichtbar gestarteter Sitzungsmodus vorgesehen: Laufzeit sichtbar, Stop jederzeit erreichbar, begrenzte Dauer, Status bei Unterbrechung und ein konfigurierbares Energiebudget. Das iPhone ist der bevorzugte Recorder für längere Gespräche; die Watch steuert die Sitzung. Audioqualität aus Tasche/Abstand muss gesondert geprüft werden.

Eine absichtlich gestartete Hintergrundaufnahme und ein ganztägig unsichtbar wartender eigener Sprachassistent sind unterschiedliche Produkte. Extended Runtime ist zweckgebunden und kein allgemeiner Freibrief für unendliche Ausführung. Keine Schein-Workout-Sitzungen zur Laufzeitverlängerung. Daueraufnahme, Funk und Wakeword-Verarbeitung benötigen Energie; „ohne Akkunachteil“ ist kein Abnahmekriterium. [Apple Runtime-Sitzungen](https://developer.apple.com/documentation/WatchKit/using-extended-runtime-sessions), [Hintergrundmodi](https://developer.apple.com/documentation/xcode/configuring-background-execution-modes).

Wakeword aktiviert eine Antwort. Kontinuierliche Erkenntnisgewinnung benötigt zusätzlich die Analyse des Gesprächs vor und zwischen Aktivierungen; ein reiner Wakeword-Detektor liefert das nicht. Deshalb separate Schalter und Datennutzungsregeln für „auf Aktivierung warten“ und „Gespräch protokollieren“.

Messverfahren: gleiches Gerät, Akkuzustand, Verbindungen, Lautstärke und Nutzungsmuster; mindestens drei vergleichbare Läufe mit und ohne App. Watch und iPhone messen. PTT-Test: 8 Stunden mit 30 Sprachturns à ca. 10 Sekunden und kurzen Antworten. Vorläufiges Entwicklungsziel: höchstens 5 zusätzliche Akku-Prozentpunkte auf der Watch gegenüber Vergleichsnutzung; kein Leistungsversprechen. Längere Aufnahme separat pro 30/60 Minuten messen. Bei Überschreitung Routing, Übertragungsfrequenz und Features anpassen.

Im Leerlauf keine laufende Mikrofon-/ML-Schleife und kein Heartbeat-Polling. VAD kann übertragene Stille reduzieren, schaltet aber bei offener Aufnahme das Mikrofon nicht magisch stromlos. Telemetrie enthält Zeitpunkte und Fehlercodes, keine Gesprächsinhalte.

## 6. Tagebuch, Erkenntnisse und Automationen

MVP: chronologischer Verlauf mit Aufnahmezeit, Originaltranskript, Antwort und Bearbeitungsstatus. Notizen sind editierbar; Original und erzeugte Zusammenfassung bleiben unterscheidbar. Eine lokale Suche reicht zunächst; keine Vektordatenbank als Voraussetzung.

Spätere Datentypen: Capture, TranscriptRevision, ConversationTurn, JournalEntry, ProposedAction, DeliveryReceipt. Stabile globale IDs und Schema-Versionen; Desktop-History bleibt lesbar, Migration additiv. Erkenntnisse verweisen auf ihre Quelle, Zeitpunkt und Bearbeitung. Widerrufene oder korrigierte Aussagen überschreiben nicht still die Historie.

Synchronisation: MVP Watch↔iPhone mit dauerhafter Outbox. Danach gekoppelte Desktop-Geräte; keine gemeinsam geöffnete SQLite-Datei in OneDrive. Konflikte, Löschmarkierungen, Export und erneute Zustellung explizit behandeln. CloudKit kann optionaler Apple-Adapter sein, aber nicht das einzige plattformweite Domänenmodell.

Gesprächsaufnahme ist bewusst aktivierbar und sichtbar. Aufbewahrung, Löschen und Export werden direkt in der App angeboten; Audio nach erfolgreicher Verarbeitung standardmäßig zeitnah löschen, längere Aufbewahrung explizit wählbar. Datenschutzhinweise und Zustimmung für Gesprächsteilnehmer gehören vor die Veröffentlichung des Mitschnittmodus. Speicher und Transport schützen, Schlüssel in Plattform-Key-Stores. Inhaltsprotokolle sind keine Debuglogs.

Aus gesprochenen Inhalten entstehen zunächst Vorschläge. Ein beiläufiger Satz oder ein von anderen gesprochenes Kommando ist keine Ausführungsberechtigung. Wissensbasis-Übernahme und externe Aktionen werden getrennt freigegeben oder durch ausdrücklich konfigurierte, eng gefasste Automationen erlaubt.

Microsoft-Ausbau: Graph-Adapter für Outlook-Entwürfe und Microsoft To Do, OAuth mit minimalen Berechtigungen, sichere Tokenablage und idempotente Outbox. Kein automatischer Mailversand als Standard. Retries bei unklarem Ergebnis dürfen keine doppelten Entwürfe/Aufgaben erzeugen. [Microsoft To Do API](https://learn.microsoft.com/en-us/graph/api/todotasklist-post-tasks?view=graph-rest-1.0).

## 7. Bedienung und Gestaltung

Watch: eine primäre Sprechaktion, klarer Zustand, kurze Antwort, Abbrechen und Wiederholen. Haptik für Aufnahmebeginn, Ende und Fehler. PTT-Halten und ein zugänglicher Start/Stop-Modus; keine winzigen Einstellungslisten. Action Button/App Intent als geräteabhängiger Schnellzugriff nach Funktionsprobe; nicht als einzig möglicher Startweg.

iPhone: drei Hauptbereiche „Sprechen“, „Tagebuch“, „Sammlung“. Einstellungen in einer bestehenden kompakten Oberfläche, Verbindungen darin. Systemschrift, native Navigation, adaptive Farben, ausreichende Kontraste, Dynamic Type, VoiceOver und reduzierte Bewegung. Transparenz nur dort, wo lesbar und funktional sinnvoll.

Desktop: gleiche Begriffe, Zustände und visuelle Hierarchie, in schmalen Fenstern mobile Aufteilung, bei mehr Platz Sidebar/Detail. Diktat, Meetings und Vorlesen bleiben erreichbar. Vorhandene WAI-Farben als zurückhaltende Markenakzente; Apples Komponentenverhalten hat auf Apple-Geräten Vorrang vor pixelidentischer Web-Nachbildung.

## 8. Geräte und Build-Voraussetzungen

Vorläufiges Ziel Stufe 1: iOS/watchOS 26 auf den Referenzgeräten; konkrete installierte Versionen vor dem Spike erfassen. Keine Beta-Abhängigkeit für die erste nutzbare Version. Asset- und KI-Verfügbarkeit zur Laufzeit prüfen.

Der Benutzer bestätigt macOS 15.7.9 auf dem Intel-Mac. Xcode 26.3 unterstützt laut Apple macOS ab 15.6 und liefert iOS-/watchOS-26.2-SDKs: dies ist der erste zu prüfende Buildpfad, ohne OS-Upgrade. Xcode 26.6 verlangt dagegen Tahoe 26.2–26.x. Die installierten iPhone-/Watch-Versionen und tatsächliche Gerätekopplung müssen mit Xcode 26.3 geprüft werden; aus einem passenden Host-OS folgt keine uneingeschränkte Unterstützung neuerer Geräte-APIs. Apple führt das MacBook Pro 16 Zoll von 2019 bei Tahoe-Kompatibilität, falls später ein Upgrade nötig wird. Exaktes Mac-Modell, freier Speicher, Xcode-Installation und Signing sind noch nicht am Gerät geprüft. Der Intel-Mac testet keine lokale Apple-Intelligence-Inferenz wie das iPhone. [Mac-Kompatibilität](https://support.apple.com/en-euro/122867), [Xcode-Matrix](https://developer.apple.com/xcode/system-requirements).

Für echte Apple-Builds und Gerätetests braucht der ausführende Agent Zugriff auf diesen Mac; Windows allein liefert keinen Nachweis. Entwicklerkonto/Signing und TestFlight-Zugang vor externer Verteilung einrichten. Hier wurde kein Mac-Zugriff eingerichtet und keine Veröffentlichung beauftragt.

Legacy in Stufe 2 bedeutet Funktionsstaffelung: Aufnahme/Tagebuch, lokale Erkennung soweit möglich, KI optional. Falls „iPad 2“ tatsächlich das ursprüngliche Modell bezeichnet, wird es nicht als modernes natives SwiftUI-Ziel eingeplant; genaue Modellnummer klären, gegebenenfalls nur Export-/Lesefunktion prüfen. Keine universelle Unterstützung alter Geräte versprechen.

## 9. Reihenfolge und Abnahme

| Etappe | Lieferbares Ergebnis | Abnahme |
|---|---|---|
| P0: Referenzstand und Apple-Setup | dokumentierter Desktop-Stand, reproduzierbarer Apple-Testbuild auf beiden Geräten | Mikrofon und Wiedergabe auf echter Watch; SDK/Signing belegt |
| P1: Watch↔iPhone-Machbarkeit | Aufnahme, gesicherte Übergabe, lokale Transkription, kurze Antwort zurück | 100 Turns über definierte Lifecycle-Zustände; keine bestätigte Aufnahme verloren; Deferred-Fälle sichtbar |
| P2: Robuster Sprachkern | Session-Zustände, Persistenz, Abbruch, Wiederholungen und Provider-Ausfall | Neustart während Übergabe, doppelte Nachricht, voller Speicher, verweigerte Berechtigung, Audio-Unterbrechung getestet |
| P3: Mobile Produktversion | SwiftUI-Oberfläche, lokales Tagebuch, Suche und Einstellungen | VoiceOver/Dynamic Type, Offline-Start nach Assetdownload, Originaltexte erhalten |
| P4: Desktop angleichen | kompakte Desktop-UI und gemeinsame Verträge; danach optionale Gerätesynchronisation | Windows/Mac-Diktat und TTS regressionsgeprüft; Migration alter History getestet |
| P5: Zuhörversuch | begrenzte Aufnahme, sichtbarer Modus, Erkenntnisvorschläge | reale Laufzeit-/Akku-/Unterbrechungsmessung; Produktentscheidung anhand Daten |
| P6: Integrationen | bestätigte Outlook-Entwürfe, To Do und Wissensbasisadapter | authentifizierte Testkonten, keine unbestellten Aktionen, keine Duplikate bei Retry |
| P7: Stufe 2 | Android/Tablet/Wear OS und konkrete Legacy-Matrix | gleiche Vertragstests, separate Audio-/Akku-Abnahme pro Plattform |

P1 beginnt mit Audio-Loopback und einer festen Testantwort, um Transport von Modelllatenz zu trennen. Danach echte STT/LLM/TTS ergänzen. Ein positiver Vordergrundtest beendet P1 nicht: gesperrtes iPhone und gesenktes Handgelenk sind Pflichtfälle. Bei ungelöstem Hintergrundproblem wird zuerst das Verhalten eingeschränkt oder das Routing geändert, bevor die Oberfläche ausgebaut wird.

Vorläufige Performanceziele, noch keine Messwerte: Aufnahmefeedback p95 ≤150 ms; nach Sprachende finales Transkript p95 ≤1,5 s bei warmen Assets und erreichbar aktivem iPhone; erste gesprochene kurze lokale Antwort p95 ≤3 s im definierten Referenzfall. Kaltstart separat ausweisen. Zeiten aufzeichnen für Aufnahme, Sicherung, Transfer, STT, KI und TTS. Keine künstlichen Wartezeiten; keine vollständige Gesprächshistorie ungefiltert im Modellkontext.

## 10. Konkreter Startauftrag für die Umsetzung

Nur P0/P1 zuerst ausführen. Vorgeschlagene neue Bereiche relativ zum Projektroot: `apps/apple/` für Xcode-iOS-/watchOS-Targets und `packages/voice-protocol/` für versionierte Nachrichten und Fixtures. Diese Pfade sind geplant und noch nicht angelegt.

- [ ] Aktuellen HEAD, lokale Änderungen, Desktop-Startbefehle und bestehende Tests dokumentieren; isolierten Entwicklungsbranch mit sicherem Zwischenstand anlegen.
- [ ] Am Mac Modell/OS mit `system_profiler SPHardwareDataType` und `sw_vers`, Xcode mit `xcodebuild -version`, SDKs mit `xcodebuild -showsdks` erfassen. Keine Seriennummern in geteilte Reports übernehmen.
- [ ] Minimale signierte iPhone-/Watch-App mit Aufnahme, Stop und Wiedergabe auf den Referenzgeräten ausführen; keine Desktop-Engine mitportieren.
- [ ] Versioniertes Envelope mit `sessionId`, `messageId`, `schemaVersion`, `kind`, `createdAt`, `payload` und Receipt definieren. Fixtures für Duplikat, falsche Version und verspätete Antwort erstellen.
- [ ] Watch-Aufnahme dauerhaft puffern, auf iPhone empfangen und erst nach Persistenz bestätigen. Wiederholung derselben messageId muss dieselbe Quittung liefern.
- [ ] Feste Antwort zurückspielen, Transportlatenz und Unterbrechungen messen. Erreichbarkeit und Hintergrundtransfer getrennt anzeigen.
- [ ] Deutsche On-Device-STT und System-TTS ergänzen; verfügbare lokale KI für eine kurze Gesprächsrunde einsetzen. Ohne KI den erfolgreichen Notizpfad erhalten.
- [ ] Gerätematrix aus P1 mindestens mit Vordergrund, gesperrtem iPhone, Hintergrund, erzwungenem App-Ende, Verbindungsabriss und Neustart abarbeiten.
- [ ] Ergebnis mit Messdaten und Entscheidung festhalten: vollständig lokal responsive unter welchen Bedingungen, wann deferred, welche offene Lücke. Erst anschließend P2/P3 detailliert planen.

P0/P1 ist ein überprüfbarer Machbarkeitsauftrag, noch kein fertig ausgeschriebener Implementierungsplan für sämtliche Plattformen. Aufwand für die Gesamt-App erst nach diesen Ergebnissen schätzen; AI-Tokenzahlen oder pauschale Kalenderzusagen wären jetzt nicht belastbar.

## 11. Entwicklungsmodelle

Empfehlung: GPT-6 Astra in Codex als Hauptentwickler für Architektur, Rust-/Swift-Grenzen, Zustandsautomaten, Integration und Fehlersuche. Fable 5.1 in Claude Code als unabhängiger Reviewer für kritische Diffs und optional als Implementierer klar abgegrenzter SwiftUI-Arbeitspakete. Ein Verantwortlicher integriert die Änderungen. Keine gleichzeitigen konkurrierenden Änderungen derselben Dateien.

Das ist eine Arbeitsaufteilung, kein bewiesenes Ranking für Swift oder watchOS. GPT-6 ist offiziell für komplexe Coding-/End-to-End-Arbeit beschrieben; Fable 5.1 ist offiziell verfügbar und für agentische Langzeitaufgaben dokumentiert. Beide haben nach den gelesenen Standard-API-Preisangaben $10 Input/$50 Output je Million Tokens; Caching, Kontextstaffeln, Agent-Produkte und Abonnements verändern reale Kosten. [GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra), [Claude Fable 5.1](https://platform.claude.com/docs/en/models/fable-5-1/overview).

Optionaler Projektvergleich vor größerem Hybridbetrieb: beide Modelle erhalten denselben kleinen Auftrag und dieselben Tests in separaten Branches, etwa Watch-Nachrichten-Deduplizierung und einen SwiftUI-Aufnahmestatus. Bewerten: frischer Build, Lifecycle-Tests, Korrekturen, Verständlichkeit, Laufzeit und tatsächliche Nutzungskosten. Zweites Modell nur beibehalten, wenn Reviews oder Umsetzung messbar helfen.

Das Entwicklungsmodell wird nicht automatisch das Laufzeitmodell der App. Für responsive Sprache zuerst lokale STT/System-TTS und verfügbare lokale KI messen; ein großes Cloud-Codingmodell ist keine Voraussetzung. Die Modellwahl darf Hardware- und Hintergrundtests nicht ersetzen.

## 12. Noch zu ermittelnde Fakten

Die Planung ist mit den bestätigten Geräten und macOS 15.7.9 vollständig als Vorschlag nutzbar. Vor Apple-Implementierung sind die installierten iOS-/watchOS-Versionen, der genaue Intel-Mac, Signing-Zugang und der Zugriff des ausführenden Agenten auf den Mac zu ermitteln. Vor optionalem Cloudbetrieb ist die Nutzerpräferenz für Datenübertragung einzuholen; Standard bleibt ohne Cloud. Ganztägiges Watch-Zuhören und latenzarme Verarbeitung bei jedem iOS-Lifecycle-Zustand sind ausdrücklich nicht verifiziert.
