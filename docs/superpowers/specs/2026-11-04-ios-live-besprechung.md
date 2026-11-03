# iOS: Aufnahmen übernehmen und Besprechungen live mitschreiben

Stand 04.11.2026 · Auftrag Patrick 03.11.2026 · Epic siehe Abschnitt „Pakete“.
Gilt für `apps/apple` (iPhone, Apple Watch). Bibliothek, Hörbücher, EPUB und
Vorlesen auf dem iPhone sind **nicht** Teil dieses Plans; sie laufen unter Epic
lva#210 (Thread „Bibliothek und Vorlesen aufs iPhone bringen“). Hier stehen sie
nur in der Paritätsmatrix.

## Ziel

1. Aufnahmen vom iPhone und von der Apple Watch mit minimalem Aufwand in die
   App übernehmen: Teilen-Menü, „Öffnen in“, Watch-App, Aufnahme direkt in der App.
2. Danach ohne weiteren Tipp automatisch lokal transkribieren und auswerten.
3. Während einer Besprechung live: Transkript, laufendes Protokoll, KI-Notizen,
   To-dos, aktueller Stand, offene Punkte und Fragen, die man noch ansprechen kann.
4. Alles offline auf dem Gerät. Keine Cloud, kein Server-STT, kein Fremd-LLM.
5. Bei jedem großen Release (Minor-Sprung, z. B. 0.23.0) ziehen Mac- und iOS-App
   funktional nach (Paritätsmatrix, Abschnitt „Release-Parität“).

## Ausgangslage (main, ff8fc5e)

| Baustein | Stand |
|---|---|
| Medienimport iPhone | `MeetingArchive.importMedia` (bis 2 h, 2 GiB, Original mit SHA-256, atomar) über `fileImporter` |
| STT | Apple `SpeechTranscriber` (de-DE, iOS 26) bevorzugt; Fallback whisper.cpp 1.7.5 (`ggml-base`, optional `small`), 30-s-Abschnitte, fortsetzbar |
| Auswertung | `MeetingMinutes` (Desktop-Schema) mit Belegprüfung `validateEvidence`; Qwen 2.5 1.5B Q4 per llama.cpp; `SystemLanguageModel` nur als Verfügbarkeitsabfrage |
| Watch | Push-to-talk bis 30 s, Pakete ≤ 1 MiB, `sendMessageData`/`transferFile`, dauerhafter Ausgang mit Quittung |
| Hintergrund | UIKit-Hintergrundfrist + `BGProcessingTask`, kooperativer Abbruch |
| Live | **fehlt** auf iOS. Desktop hat Live-Transkript (`dsp.rs::live_segments`), Endlauf (`final_pass.rs`) und Einwilligung (`consent_confirmed_at`) |
| Teilen-Menü, „Öffnen in“, Kurzbefehle | **fehlen** |

## Architektur

```
 Apple Watch                     iPhone (Local Voice)
 ───────────                     ───────────────────────────────────────────────
 Aufnahme (AAC 16 kHz mono)      Aufnahme in der App ──┐
  └ 30-s-Stücke ─transferFile──► WatchEmpfang ─────────┤
                                                       ▼
 Sprachmemos / Dateien /         Eingang (App-Group-Ordner „Eingang“)
 Teilen-Menü ─► Share-Extension ─┘  │ nur kopieren + Einwilligungsvermerk
                                    ▼
                         MeetingArchive (Original, Hash, atomar)
                                    │
             ┌──────────────────────┴───────────────────────┐
             ▼ live (Vordergrund)                           ▼ danach (auch Hintergrund)
   SpeechAnalyzer-Strom → Segmente               Endlauf whisper (optional large-v3-turbo)
             │                                              │
             ▼ alle ~90 s / ~250 Wörter                     ▼
   LiveStand-Delta (lokales LLM, belegt)          MeetingMinutes (bestehend, belegt)
             │                                              │
             └────────► Besprechungsansicht ◄───────────────┘
                        Transkript · Stand · To-dos · Entscheidungen · Offene Fragen · Notizen
```

### 1. Übernahme mit minimalem Aufwand

| Weg | Umsetzung | Aufwand für Patrick |
|---|---|---|
| Sprachmemos (iPhone/Watch, per iCloud auf dem iPhone) | Share-Extension `VoiceShare` für `public.audio`, `public.movie`, `com.apple.m4a-audio`, `.qta` | Teilen → Local Voice |
| Dateien-App, Mail, Messenger | dieselbe Extension + `CFBundleDocumentTypes` „Öffnen in“ | 1 Tipp |
| Watch-App | neue Besprechungsaufnahme (unbegrenzt bis 2 h) mit Stücken, siehe Abschnitt 4 | Start/Stopp an der Uhr |
| In der App | Knopf „Besprechung aufnehmen“, Live-Ansicht | 1 Tipp + Einwilligung |
| Kurzbefehle/Aktionstaste | App Intents: „Besprechung aufnehmen“ (`AudioRecordingIntent`), „Datei an Local Voice übergeben“; Steuerelement im Kontrollzentrum | Aktionstaste |

Die Share-Extension transkribiert **nicht** (Speichergrenze der Extension ca.
120 MB). Sie kopiert die Datei atomar in den App-Group-Ordner `Eingang/<uuid>/`
mit `eingang.json` (Quelle, Name, Zeitpunkt, Einwilligungsvermerk) und meldet
„Übernommen – wird transkribiert“. Die App holt den Eingang beim Start, bei
Rückkehr in den Vordergrund und per `BGContinuedProcessingTask` (iOS 26,
nutzergestartet, mit Fortschrittsanzeige) ab. Damit läuft die Transkription
nach dem Teilen ohne weiteren Tipp an; iOS kann Hintergrundzeit verweigern,
dann beginnt sie beim nächsten Öffnen.

### 2. Lokale Spracherkennung: Modellgrößen und Zuschnitt

| Modell | Datei | RAM im Lauf | Eignung live auf A17 Pro/A18 | Deutsch |
|---|---|---|---|---|
| Apple SpeechTranscriber | Systemasset, nicht in der App | außerhalb des App-Budgets | ja, Strom mit Zwischenergebnissen, Neural Engine | gut |
| whisper base (q5_1) | ca. 60 MB | ca. 0,3 GB | ja | mäßig |
| whisper small (q5_1) | ca. 190 MB | ca. 0,6 GB | knapp (30-s-Stücke) | ordentlich |
| whisper large-v3-turbo (q5_0) | ca. 550 MB (+ Core-ML-Encoder) | ca. 1,0–1,5 GB | nein, Endlauf | sehr gut |

Entscheidung (Vorschlag):
- **Live:** SpeechTranscriber per `SpeechAnalyzer` mit `volatileResults`. Belastet
  das Speicherbudget der App nicht und läuft energiesparend auf der Neural Engine.
  Fallback ohne Asset: whisper base in 15-s-Stücken.
- **Endlauf nach Ende:** wie Desktop optional whisper large-v3-turbo über das
  gesamte Original (Ladeknopf, nie stillschweigend). Live-Transkript bleibt als
  Kopie erhalten, wird nur bei erfolgreichem Endlauf atomar ersetzt (Regel aus
  `final_pass.rs` übernehmen).
- Neues Modell nur per expliziter Auswahl laden, SHA-256 gebunden, Lizenz MIT.

### 3. Lokales LLM für den Live-Stand

| Modell | Größe | Lizenz | Kontext | Bemerkung |
|---|---|---|---|---|
| Apple Foundation Model (`SystemLanguageModel`) | System, ca. 3B | Apple-Nutzungsbedingungen, keine Weitergabe | 4096 Token | nur Geräte mit Apple Intelligence (ab iPhone 15 Pro); Deutsch; geführte Ausgabe `@Generable` |
| Qwen 2.5 1.5B Instruct Q4_K_M (vorhanden) | 1,12 GB | Apache-2.0 | 32k | schwach bei Zuordnung |
| Qwen3 4B Instruct Q4_K_M | ca. 2,5 GB | Apache-2.0 | 32k | bestes freies Modell im Budget, nur Geräte mit 8 GB RAM |
| ~~Qwen 2.5 3B~~ | 1,9 GB | Qwen Research (nicht kommerziell) | – | ausgeschlossen |

Entscheidung (Vorschlag): Foundation Model als Standard, Qwen3 4B als lokale
Alternative (explizit laden), Qwen 2.5 1.5B als Mindestlösung für ältere Geräte.

**Rollierendes Verfahren statt Gesamtprompt** (passt in 4096 Token):
- Zustand `LiveStand` (kompakt, höchstens ca. 900 Token): `stand` (3–5 Sätze),
  `themen[]`, `entscheidungen[]`, `todos[]` (Text, wer, bis wann), `offen[]`
  (offene Punkte/Fragen), `vorschlaege[]` (Fragen, die man jetzt ansprechen
  könnte). Jeder Eintrag hat eine stabile ID und Belegsegmente (`belege: [Int]`).
- Takt: alle 90 s oder ab ca. 250 neuen Wörtern, nie parallel zu sich selbst.
  Eingabe = bisheriger Zustand + neue Segmente (mit Index). Ausgabe = **Delta**
  (`neu`, `aendern`, `erledigt`), kein Neuschreiben.
- Belegpflicht wie `MeetingMinutes.validateEvidence`: Namen, Termine und To-dos
  nur mit wörtlichem Beleg in den genannten Segmenten; sonst wird das Delta
  verworfen und das Ereignis gezählt (keine Halluzination im Protokoll).
- „Offen“ wird automatisch „erledigt“, wenn ein späteres Segment die Frage
  beantwortet (vom Modell vorgeschlagen, mit Beleg).
- Thermik und Akku: bei `ProcessInfo.thermalState >= .serious` oder Stromsparmodus
  Takt auf 5 min, bei `.critical` nur noch Transkript. Anzeige „KI pausiert (Wärme)“.
- Hintergrund: GPU/Metal ist im Hintergrund nicht nutzbar. Gesperrtes Gerät =
  Aufnahme und STT laufen weiter (Audio-Hintergrundmodus), LLM-Takt pausiert und
  holt nach Entsperren in einem Lauf auf. Live-Aktivität auf dem Sperrbildschirm
  zeigt Dauer, Aufnahmestatus und Zahl der To-dos.
- Nach Ende: bestehende `MeetingMinutes`-Pipeline über das ganze Transkript;
  der Live-Stand dient als Vorlage, nicht als Ersatz.

### 4. Apple Watch: Grenzen und Weg

| Thema | Grenze | Lösung |
|---|---|---|
| Spracherkennung auf der Uhr | kein SpeechTranscriber, whisper zu groß für RAM/Akku | Uhr nimmt nur auf, iPhone erkennt |
| Lange Aufnahme | heutiger Pfad: 30 s, 1 MiB | neuer Pfad „Besprechung“: AAC 16 kHz mono 32 kbit/s (ca. 14 MB/h), Stücke à 30 s |
| Hintergrund | Aufnahme läuft nur mit aktiver Audiositzung weiter (Hintergrundmodus `audio`) | Start im Vordergrund, dann Handgelenk senken erlaubt; Gerätetest nötig |
| Übertragung | `sendMessageData` nur bei Erreichbarkeit, Grenzen der Paketgröße | jedes Stück per `transferFile` mit Metadaten (Sitzung, Stück-Nr., Hash); Quittung je Stück; Original bleibt auf der Uhr bis zur Quittung |
| „Live“ von der Uhr | Bluetooth-Latenz, iPhone muss in Reichweite sein | halbes Live: Transkript mit ca. 30–90 s Verzug; ohne iPhone in Reichweite puffert die Uhr und überträgt später |
| Speicher Uhr | wenige GB, mit anderen Apps geteilt | Obergrenze 2 h, Warnung bei < 500 MB frei |

Das iPhone setzt Stücke in Reihenfolge zusammen (Lücken bleiben sichtbar), speist
sie in denselben Live-Weg ein und schreibt am Ende ein Original in `MeetingArchive`.

### 5. Datenschutz und Einwilligung (DSGVO, § 201 StGB)

Heimliches Aufnehmen des nichtöffentlich gesprochenen Wortes ist strafbar
(§ 201 StGB); für die Verarbeitung gilt DSGVO bzw. LDSG BW.
- **Vor jeder Aufnahme** (iPhone, Watch, Kurzbefehl) erscheint der Hinweis:
  „Alle Anwesenden müssen der Aufnahme zugestimmt haben.“ mit Bestätigung
  „Alle haben zugestimmt“. Ohne Bestätigung startet keine Aufnahme. Auf der Uhr
  eine kurze Ein-Tipp-Bestätigung. Kurzbefehl/Aktionstaste öffnen den Hinweis,
  sie überspringen ihn nicht.
- **Beim Import** (Teilen-Menü): Bestätigung „Die Aufnahme erfolgte mit
  Zustimmung aller Beteiligten“ in der Extension, sonst kein Import.
- Vermerk `consentConfirmedAt` + Weg (`app`, `watch`, `import`, `kurzbefehl`) am
  Dokument, wie Desktop (`consent_confirmed_at`). Kein Name, keine Ortung.
- Sichtbare Aufnahmeanzeige (iOS-Mikrofonpunkt, Live-Aktivität, Watch-Symbol).
- Daten bleiben lokal im App-Container mit Dateischutz
  `completeUntilFirstUserAuthentication` (Aufnahme bei gesperrtem Gerät muss
  weiterlaufen). Löschen eines Dokuments entfernt Original, Stücke auf Uhr
  und Eingang.
- Hinweistext in den Einstellungen, Formulierung mit Desktop
  (`meetings.consent.*`) abgleichen.

## Release-Parität Windows → Mac → iOS

- Mac = derselbe Tauri-Build für macOS. Parität entsteht beim Bau; der Thread
  „Local Voice Mac-App bauen und veröffentlichen“ baut ihn zu jedem großen
  Release, sobald der Mac (Intel) erreichbar ist.
- iOS ist nativ (SwiftUI) und muss portiert werden. Neue Datei
  `docs/apple-paritaet.md`: Matrix Funktion × Windows × Mac × iOS mit Status
  (gleich / angepasst / bewusst nicht / offen) und Issue-Verweis.
- Versionsregel: Apple-Marketingversion folgt dem großen Desktop-Release
  (Desktop 0.23.0 → Apple 0.23.0, Build fortlaufend). Lokale Zwischenstände
  ändern nur die Buildnummer. `RELEASING.md` wird entsprechend angepasst.
- Release-Checkliste: vor `app-v0.X.0` Paritätsmatrix aktualisieren; offene
  iOS-Punkte entweder umgesetzt oder als bewusst offen markiert.

## Pakete (Issues, je ein PR)

| Nr. | Paket | Abhängig | Testbar ohne Mac |
|---|---|---|---|
| P1 | VoiceCore `LiveStand`: Datenmodell, Delta-Anwendung, Belegprüfung, Taktsteuerung, Prompt-Bau; Einwilligungsvermerk am Dokument | – | ja (SwiftPM unter Linux) |
| P2 | VoiceCore `Eingang`: App-Group-Eingang (atomar, idempotent), Abholung ins Archiv; Watch-Stückprotokoll (Reihenfolge, Lücken, Quittung) | – | ja |
| P3 | iPhone: Besprechungsaufnahme mit Live-Ansicht (SpeechAnalyzer-Strom, Live-Stand mit Foundation Model, Einwilligung, Live-Aktivität) | P1 | nein (Simulator) |
| P4 | Share-Extension + „Öffnen in“ + `BGContinuedProcessingTask` | P2 | nein |
| P5 | Watch: Besprechungsaufnahme in Stücken, Übertragung, Einwilligung | P2 | nein |
| P6 | App Intents: Kurzbefehle, Aktionstaste, Kontrollzentrum | P3 | nein |
| P7 | Endlauf whisper large-v3-turbo + Qwen3 4B als Alternative (explizit laden) | P3 | teilweise |
| P8 | `docs/apple-paritaet.md`, Versionsregel in `RELEASING.md` | – | ja |
| P9 | Vorschau (HTML) der Live-Ansicht iPhone/Watch zur Codex-Abnahme, vor P3-Merge | – | ja |

Reihenfolge: P8, P1, P2 und P9 sofort in der Cloud. P3–P7 brauchen Xcode auf dem
Mac (Intel, derzeit offline) und Gerätetests; sie gehen an den Mac-Thread, sobald
der Mac wieder erreichbar ist.

## Risiken und offene Prüfungen am Gerät

1. SpeechAnalyzer-Strom bei gesperrtem Gerät über 60 min stabil? (Gerätetest)
2. Watch-Hintergrundaufnahme ≥ 60 min ohne Abbruch; Akkuverbrauch Uhr pro Stunde.
3. Foundation Model: Deutschqualität der Deltas, Zeit je Takt (Ziel < 15 s),
   Ablehnungen durch Schutzmechanismen bei Besprechungsinhalten.
4. Speicher: Qwen3 4B parallel zu laufender Aufnahme; nur mit
   `increased-memory-limit`-Berechtigung und nur auf 8-GB-Geräten anbieten.
5. Thermik bei 60 min Live auf dem iPhone (Ziel: kein Drosseln unter `.serious`).
6. `.qta`-Sprachmemos (mit Apple-Transkript) – Audio-Spur lesbar per AVFoundation?
