# Lokale Medienauswertung und WAI-Oberfläche – 06.09.2026

Quellstand `3f313ec` auf `codex/apple-p0-p1`, Intel MacBook Pro 16 mit iPhone-15-Pro-Max-
und Watch-Simulator. Alle Inhalte der Messaufzeichnungen sind synthetisch.

## Verhalten

Audio-/Videoimport bewahrt das Original nach dauerhaftem Speichern auf. Lokales
Whisper transkribiert in 30-Sekunden-Abschnitten; Qwen 2.5 1.5B Q4_K_M erstellt
abschnittsweise ein strukturiertes Protokoll. Gesicherte Fortschritte überleben
App-Beendigung. Bei inaktiver App wird Verarbeitung aufgeschoben. Modellgewichte
bleiben außerhalb Git; das zusätzliche Auswertungsmodell wird ausdrücklich geladen.

Ergebnisse haben kompakte WAI-Karten, getrennte Auswertung/Transkript-Ansichten und
Kopieren sowie TXT/HTML/SRT/JSON/Original-Teilen. Der Systemmodus bestimmt Hell/Dunkel.
Hinweistexte verwenden den WAI-Texttoken mit 72 % Deckkraft: rechnerisch 5,74:1 auf
hellem und 8,81:1 auf dunklem WAI-Hintergrund. Das ist ein Tokenvergleich, keine
vollständige Barrierefreiheitszertifizierung. Doppelte Kontextblöcke entfallen.

## Messung: 45 Sekunden Video, Neustart nach Sekunde 30

| Erfolgreich gesicherte Arbeit | Dauer |
| --- | ---: |
| Originalimport | 1,14 s |
| Audiospur vorbereiten | 0,14 s |
| Lokale Transkription | 11,89 s |
| Lokale Auswertung | 39,21 s |

`video-resume.json` enthält Messwerte, Original-Hash und tatsächliches Protokoll.
Original unverändert, sechs eindeutige Segmente, kein verlorener bestätigter
Fortschritt. Abgebrochene Arbeit, Wartezeit und App-Start sind in den Stufensummen
nicht enthalten. Dies ist kein Echtzeit- oder Gerätefunktest. TTS gehört zum bereits
separat geprüften Watch-Kurzantwortpfad, nicht zum neuen Medienimport.

## Nachweise und Grenzen

- 70 Swift-Kerntests: Archiv, Fortschritt, Grenzen, Exporte, belegte Zuordnungen,
  strenge Auswertungsschemata und Schutz vor erfundenen Namen/Terminen.
- Erfolgreicher iPhone-Simulator-Build und native Oberflächentests; einzelne
  Modusprotokolle und Screenshots liegen daneben. Vollständige iPhone-Serie:
  jeweils 6/6 in Hell und Dunkel. Nach Kontrastkorrektur zusätzlich 3/3 dunkel,
  2/2 hell sowie Watch 1/1. Nach dem letzten Hinweistext-Abgleich auch die
  einzelne Modellansicht in Hell und Dunkel jeweils 1/1 erfolgreich geprüft.
- Tatsächlich erzeugte TXT/HTML/SRT/JSON-Dateien auf Inhalt geprüft (`export-check.json`).
  System-Teilen-Menü geöffnet; Versand an externe Apps wurde nicht ausgelöst.
- Importpipeline per Debug-Testdatei geprüft; vollständige Dateiauswahl aus fremden
  Dateianbietern ist damit nicht nachgewiesen.
- Sichtbare Transkriptfehler: „Webseite“ → „Wettseite“, „Ben“ → „wenn“. Das lokale
  Modell lässt Aufgaben teils aus oder ordnet Aussagen falsch zu. Quellenbindung
  verhindert neue Faktentexte, garantiert aber keine korrekte Kategorisierung.
- Für gemischte Audioaufnahmen keine Diarisierung und keine erfundenen Redeanteile.
- Keine physische Akkumessung im Simulator möglich; Akkuauswirkung bleibt ungemessen.
- Keine allgemeine Live-Aufnahme fremder Telefon-/Teams-/WhatsApp-Audioströme.
- Synchronisierung, entfernte Mikrofone und aktivitätsabhängige Zielwahl sind noch
  nicht implementiert; der Ausbauplan kennzeichnet diese Punkte ausdrücklich offen.

Desktop-Oberflächenprüfung ist separat unter `docs/desktop-evidence/2026-09-06-workspace`
belegt: sechs Browser-Tests einschließlich Hell/Dunkel und Windows-UI-Pfad; kein
frischer nativer Windows-Pakettest.
