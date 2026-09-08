# Modelle und Stimmen – 06.09.2026

Implementierung: `0b986fd`, Integritätskorrektur: `0b76ddd`, native Tests: `a598f10`.
Isolierter Branch: `codex/watch-conversation-background`.

## Ergebnis

Modelldownloads verwenden eine persistente native Hintergrund-URLSession. Fortschritt,
übertragene Bytes, Prüfung, Fehler, Abbruch und Bereitschaft stehen unmittelbar in
der Modellzeile. Der Downloadknopf zeigt nur ein Symbol. Vorhandene Modelle bleiben
bei fehlerhaften oder abgebrochenen Installationen erhalten. Erst Größe, erwartetes
Modell und SHA-256-Prüfung erlauben die atomare Installation und Bereitschaftsmeldung.

Auf dem echten iPhone wurde Qwen 0.5B vollständig mit 491.400.032 Bytes im Hintergrund
installiert. Der aus dem Gerätecontainer gelesene Status dokumentiert `ready`,
`completedInBackground: true` und `notified: true`. Letzteres bestätigt, dass iOS die
lokale Benachrichtigung angenommen hat; ein sichtbares Banner wurde nicht beobachtet.
Die Mitteilung lautet „Sprachmodell bereit“ und nennt das verwendbare Modell.

In Einstellungen → Stimme & Hörprobe sind die von Apple für Apps bereitgestellten
Stimmen auswählbar, mit Vorschau und dauerhaft gespeicherter Geräteauswahl. Dasselbe
ist auf der Watch über die Gesprächseinstellungen erreichbar. Die Siri-/Apple-
Intelligence-Stimmenauswahl kann von der öffentlichen Sprach-API abweichen.

## Frische Prüfungen

| Prüfung | Ergebnis |
|---|---|
| VoiceCore | 84 Tests bestanden |
| Installation mit echter 491-MB-Datei | 5 Tests bestanden: SHA, falsches Modell, Korruption, Abbruch, Bestandsschutz |
| Simulator: App verlassen, vollständiger Download | bestanden, 122,892 s Gesamttest |
| Simulator: Prozessneustart während Download | bestanden, 131,977 s Gesamttest |
| Simulator: kompakter Knopf, Inline-Fortschritt und Abbruch | bestanden, 21,902 s |
| iPhone-Simulator: Stimme, Vorschau, Auswahl nach Neustart | bestanden, 29,854 s |
| Watch-Simulator: Stimme, Vorschau, Auswahl nach Neustart | bestanden, 23,160 s |
| Echtes iPhone: Hintergrunddownload und Integritätsprüfung | durch persistierten Gerätestatus bestätigt |
| Echtes iPhone: automatisches Wiederöffnen nach 100 s | durch inzwischen gesperrtes Gerät blockiert; kompletter UI-Test deshalb nicht bestanden |
| Hell-/Dunkelansicht | Screenshots geprüft; Dark-Screenshot vor abschließender Textkürzung |

Testzeiten sind Gesamtlaufzeiten einschließlich Wartefenstern, keine reinen Download-
oder TTS-Latenzen. Akustische Qualität auf den echten Geräten, sichtbares Banner und
Akkuauswirkung wurden in diesem Durchlauf nicht gemessen.

## Lifecycle-Grenzen

Normales Verlassen der App lässt den Systemdownload weiterlaufen. Wird die App vom
Benutzer weggewischt, beendet iOS Hintergrundtransfers; der dauerhaft gespeicherte
Auftrag wird beim nächsten Öffnen erneut aufgenommen. Bytegenaues Fortsetzen wird
nicht versprochen. Bei abgelaufener Prüfzeit bleibt die heruntergeladene Datei erhalten
und die Prüfung wird beim Öffnen fortgesetzt. Benachrichtigungen erfordern die
Systemfreigabe. Ein echter Wegwisch-Gestentest wurde nicht durchgeführt; der
Simulator prüfte einen Prozessneustart.

Simulator-Builds benötigen hier Ad-hoc-Signing. Ein zuvor unsignierter Lauf scheiterte
am Hintergrundsession-XPC/Entitlement-Zugriff; der signierte Wiederholungslauf bestand.
Der dokumentierte Buildbefehl ist entsprechend korrigiert.

Quellen: [Apple Hintergrunddownloads](https://developer.apple.com/documentation/foundation/downloading-files-in-the-background),
[Apple zur Beendigung durch Benutzer](https://developer.apple.com/documentation/foundation/urlsessionconfiguration/background%28withidentifier%3A%29),
[verfügbare App-Stimmen](https://developer.apple.com/documentation/avfaudio/avspeechsynthesisvoice/speechvoices%28%29).

## Weitere Abnahme

Sichtbares Fertig-Banner und akustische Stimmenausgabe auf entsperrten echten Geräten
prüfen. Physisches Wegwischen, lange Unterbrechung, voller Speicher und Energiebedarf
bleiben gezielte Folgetests. Frühere offene Integrationen wie Sync, entfernte Mikrofone
und interaktive Watch-Komplikationen werden durch dieses Update nicht abgeschlossen.

## Installation des endgültigen Stands

Signierte iPhone- und Watch-Builds erfolgreich. Beide Apps am 06.09.2026 um
16:10 bzw. 16:12 Uhr auf den echten Geräten installiert. Automatisches Öffnen
wegen Gerätesperre verweigert; die Installation selbst war erfolgreich. Der
Watch-Build wurde nach einem Zielgerät-Timeout mit generischem watchOS-Ziel gebaut.
