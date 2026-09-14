# Local Voice AI — Weiterentwicklungsplan (Stand 2026-08-24, ab v0.13.0)

Analyse und Etappenplan für Features, Design und die Rückkopplung ins
WAI-Design-System. Jede Etappe endet mit einem anfassbaren Artefakt und
EINEM gebündelten Release (Tag `app-v*`).

## 1. Standortbestimmung

**Was die App heute kann** (Tauri 2, Rust + React, vollständig lokal):

| Bereich | Stand |
|---|---|
| Diktat | Global-Shortcut, VAD, Whisper (ggml) + ONNX-Modelle (Parakeet u. a.), Einfügen per Clipboard/Paste, Post-Processing per LLM |
| Meetings (M8) | Mikrofon + System-Audio (Loopback), Streaming-WAV, Protokolle, Re-Transkription, Export/Import, Aufbewahrung |
| Vorlesen (TTS) | Quellen-Menü, Reiter-Sitzungen, Player |
| Modelle | Katalog, Download, HF-Cache-Scan, Modell-Optionen je Karte |
| Plattformen | Windows (Vulkan, NSIS, portable), macOS NEU (Metal, DMG aarch64 + x86_64), Linux teilweise |
| Sonstiges | Updater (latest.json), 24 Sprachen, Tray, Overlay, Onboarding, CLI-Flags |

**Plattform-Lücken macOS** (frisch portiert, v0.12.1–v0.13.0):
- System-Audio-Mitschnitt fehlt (Loopback ist WASAPI/Windows-only; Meetings laufen mic-only).
- DMG ist nur ad-hoc signiert: erster Start per Rechtsklick → Öffnen, Gatekeeper warnt.
- Intel-Build (x86_64) ist Cross-Compile und noch auf keinem echten Intel-Mac verifiziert.
- Kein CoreML/ANE-Pfad für die ONNX-Modelle; Whisper nutzt Metal (ggml), das ist gut,
  aber die ONNX-Familie läuft CPU.

**Technische Schulden:**
- Upstream-Subtree (Handy) driftet; UPSTREAM.md pflegt die Grenze, aber jede
  Portierung (siehe cpal-!Send-Fix in `mic_capture.rs`) ist Handarbeit.
- `docs/KNOWN-LIMITATIONS.md` und die m*-Evidence-Ordner wachsen schneller als
  sie konsolidiert werden.
- App-`theme.css` spiegelt WAI-Tokens von Hand — keine Drift-Prüfung gegen den
  DS-Master (Abschnitt 4 behebt das).

## 2. Leitplanken

- Einstellungen wandern in bestehende Tabs, nie in neue Oberflächen (AGENTS.md).
- Releases bündeln: erst Etappe fertig, dann EIN Tag.
- Aufwand je Etappe vorab in kTok schätzen; bei 150 % harter Stopp und Lagebericht.
- MLX & Co.: neue Inferenz-Backends nur mit messbarem Nutzen gegen die Baseline
  (Metal-ggml). Liegt die Baseline gleichauf → abbrechen, dokumentieren, weiter.

## 3. Etappen

### E1 — macOS erstklassig (v0.14.x) · Schätzung ~150 kTok
1. **System-Audio für Meetings auf macOS** über ScreenCaptureKit
   (`SCStream` Audio-Tap, ab macOS 13): `LoopbackCapture`-Gegenstück hinter dem
   bestehenden `#[cfg]`-Schnitt in `audio_toolkit/audio/loopback.rs`.
2. **Signierung + Notarisierung**: Apple-Developer-Zertifikat einrichten
   (Secrets `APPLE_CERTIFICATE`, `APPLE_ID`, … — die Schritte stehen fertig im
   geerbten `build.yml`), dann entfällt der Rechtsklick-Start. *Entscheidung
   Patrick: Developer-Account (99 $/Jahr) ja/nein.*
3. **Intel-Verifikation**: x86_64-DMG auf dem MacBook Pro testen (Diktat,
   Meeting mic-only, TTS); Ergebnis in KNOWN-LIMITATIONS.md.
4. **Prüfauftrag CoreML** (klein): ONNX-Runtime CoreML-Execution-Provider für
   Parakeet messen. Nutzenkriterium: ≥ 1,5× Realtime-Faktor vs. CPU, sonst verwerfen.
   MLX bleibt außen vor, solange Metal-ggml nicht der Engpass ist.

**Artefakt:** notarisierte DMGs (arm + Intel) + Vergleichstabelle CoreML/CPU.

### E2 — Meetings vertiefen (v0.15.x) · ~200 kTok
1. **Sprecher-Trennung (Diarisierung)**: ONNX-Diarisierungsmodell (z. B.
   pyannote-Community-ONNX) als optionaler Nachbearbeitungsschritt; Sprecher-Labels
   in Protokoll und Untertitel-Export.
2. **Live-Transkript** während der Aufnahme (Chunker existiert) als Seitenpanel.
3. **Protokoll-Qualität**: Abschnitts-Zusammenfassungen + Aufgabenliste über den
   vorhandenen Post-Processing-LLM-Pfad, Prompt-Vorlagen je Meeting-Typ.

**Artefakt:** ein echtes Meeting-Protokoll mit Sprecherlabels als Screenshot/Export.

### E3 — Vorlesen ausbauen · GRÖSSTENTEILS VORGEZOGEN (v0.14.0, 2026-08-24)

Der Fish-Speech-Vollausbau (Plan `alle-features-von-fish-speech`) hat E3 vorgezogen
und erweitert. **Geliefert in v0.14.0:** Emotion-Tag-System (Chip-Editor mit
Mirror-Overlay, Palette mit ~95 kuratierten Tags, Autocomplete, Kontextmenü,
Pointer-Drag), KI-Auto-Tagging mit Nur-Einfügen-Validierung (Ollama/Claude Haiku),
Sprecher-Registry-Backend (meta.json je Stimme: Anzeigename, Farbe, Avatar, Stile,
Default-Tags; security-gehärtet), TTS-Engine-Abstraktion mit cache-stabiler Naht,
Piper-CPU-Engine + Katalog-Downloads mit SHA-Pins („Vorlesestimmen" auf der
Modelle-Seite), s1-mini-Probeskript.

**Nachgeliefert (unreleased, kommt mit v0.15.0):** Sprecher-Chips im Editor
(S3) — `<Name>`/`<Name:Stil>` und das alte `Name:` am Zeilenanfang werden als
farbiger Chip gezeigt, der Text bis zum nächsten Wechsel blass in der Farbe der
Stimme hinterlegt; Chip-Klick öffnet die Sprecherauswahl, das Kontextmenü fügt
Sprecher ein. Zugleich der Bruch dahinter geschlossen: die Vorlese-Pipeline gab
dem Parser nur die nackten voice_ids, weshalb ein Marker mit dem ANZEIGENAMEN
nicht schaltete und sogar mitgelesen wurde (`utterances` → `split_speaker_segments`
über `known_speakers`).

**Offener Rest (nächste Etappe, Briefs liegen im SDD-Workspace des Plans):**
1. Sprecher NUR mit Fish-Speech: `known_speakers` listet die geklonten Stimmen,
   und die Piper-Engine ignoriert die Stimme je Satz (sie ist beim Auflösen an
   EIN Modell gebunden). Mit Piper werden Marker also korrekt entfernt, aber
   alles in einer Stimme gelesen. Mehrsprecher-Piper hieße: je Marker eine
   eigene `PiperEngine` auflösen — eigenes Paket.
2. Stimmen-UI v2 (VoicesCard: Meta/Stile/Analyse) — S2; Baukasten (Seed→Probe→
   Speichern) — S4; Stil-Auflösung in der Pipeline — S5.
3. Blitz-Vorschau-UX + Telemetrie + „Text speichern" — E4-Preview; Export-Pfad
   „Dialog-Qualität" (`<|speaker:N|>`, msgpack) — S7.
4. Diktat-Auto-Tag-Toggle; SenseVoice-Rich-Fork (akustische Emotionen); s1-mini-
   Integration (nach Probe); Dubbing (Autofit-Konzept, UI in Besprechungen).
5. Aus E3 alt weiter offen: Satz-Synchronisation im Text (Klick springt zum Satz),
   Kapitelmarken beim Export.

**Artefakt:** v0.14.0-Release mit Tag-Editor (dieser Abschnitt), Rest-Artefakte je Folge-Etappe.

### E4 — Design & UX nach WAI-Sprache (v0.17.x) · ~180 kTok
Grundlage existiert (`src/styles/theme.css`, Gelb auf Ink, Logo-Regeln). Vertiefen:
1. **Akzent-Inseln** konsequent: CTAs, Featured-Icons und Kennzahlen-Kacheln als
   dunkle Inseln mit gelbem Inhalt auf neutralem Grund; kein Gelb-auf-Hell ohne Ink.
2. **Media-Controls**: der Vorlese-Player übernimmt das `.mediabar`/`.mbtn`-Muster
   des DS (ein Primär-Schalter, Glyph-Wechsel Play/Pause) statt generischer Buttons.
3. **Icon-Konsolidierung**: durchgängig Lucide (`lucide-react` ist schon da),
   Eigenbau-Icons (`components/icons/*`) nur wo semantisch nötig (Logo, Aufnahme).
4. **A11y-Pass** nach ui-ux-pro-max-Checkliste: Fokus-Ringe, 4,5:1-Kontraste in
   beiden Themes, `prefers-reduced-motion`, Touch-Ziele ≥ 44 px (Overlay!),
   `cursor-pointer` auf allen klickbaren Karten.
5. **Onboarding** als geführte drei Schritte (Mikrofon → Modell → Probediktat)
   im Akzent-Insel-Stil.

**Artefakt:** Vorher/Nachher-Screenshots aller Hauptansichten in Light + Dark.

### E5 — Rückkopplung ins Design-System (parallel zu E4) · ~80 kTok
Das DS ist Quelle der Wahrheit; die App ist sein erster Desktop-Konsument.
1. **App als Downstream-Kopie registrieren**: `apps/local-voice/src/styles/theme.css`
   in `check_token_drift.py` aufnehmen — Token-Abweichungen fallen dann mechanisch auf.
2. **Neuer DS-Layer „Desktop-App (Tauri)"** in `references/`: Tray-/Overlay-Muster,
   Fensterchrome, Einstellungs-Listen (SettingsGroup/SettingContainer), Verhalten
   bei Systemthemen — als `references/desktop-app.md` + Katalog-Sektion
   (`wai-portal/config/designsystem.php` + Partial + Testzahl, wie im Skill beschrieben).
3. **Neue Bausteine zurückspielen statt forken**: Wellenform-/Pegel-Anzeige und
   Aufnahme-Status-Badge entstehen zuerst als DS-Komponente (portal.css + Katalog),
   die App übernimmt Markup/Klassen.
4. Nach jeder DS-Änderung: `check_token_drift.py`, `check_catalog_coverage.py`,
   Katalog-Sichtprüfung — erst dann „fertig".

**Artefakt:** Katalog-Sektion „Desktop-App" live unter `/admin/designsystem`.

## 4. Reihenfolge und nächste Schritte

Empfohlene Reihenfolge: **E1 → E4+E5 → E2 → E3.** Begründung: E1 macht die neue
Plattform belastbar, solange der Portierungskontext frisch ist; E4/E5 sind
sichtbar und tragen jede Demo; E2/E3 bauen auf stabilem Fundament.

**Sofort (vor E1):**
1. v0.13.0-Release abnehmen: DMG auf M-Mac (Icon!) und Intel-Mac testen,
   Windows-Update von 0.12.x prüfen.
2. Entscheidung Apple-Developer-Account (blockiert E1.2).
3. `theme.css` in den Token-Drift-Check aufnehmen (E5.1, 30 Minuten, sofort möglich).


## Apple Mobile/Watch – P0/P1-Ergebnis 05.09.2026

Der native iPhone-/Watch-Machbarkeitsprototyp ist im vom Benutzer angeforderten
Simulatorumfang abgeschlossen, isoliert in `codex/apple-p0-p1`. Der bestehende
Desktop-Plan bleibt unverändert. P2 beginnt mit Qualität, Laufzeit und sicherer
Job-/Speicherwartung; physische Abnahme wird gesondert beauftragt.
[Messwerte und Lifecycle-Grenzen](apple-evidence/2026-09-05-simulator-report.md),
[aktualisierter P2-Plan](apple-evidence/P2-follow-up.md).

## Apple Mobile/Watch – P2 abgeschlossen im Simulatorumfang, 06.09.2026

Persistente Jobs, Speicherwiederherstellung, Modellverwaltung und überprüfbare
Sprach-/Lifecycle-Proben sind implementiert und nach Claude-Review-Korrekturen
abgenommen. Messwerte und die verbleibenden Qualitäts-/Hardwaregrenzen stehen im [P2-Bericht](apple-evidence/p2/README.md).
Die [Folgeprioritäten](apple-evidence/p2/next-priorities.md) behandeln Antwortqualität,
Zielgerät-Latenz und gesonderte Hardware-/Energieabnahme. Sie autorisieren weder
weitere Produktfunktionen noch eine Veröffentlichung.

### Apple: Hintergrundverarbeitung, Nachtrag 06.09.2026

Kurze Watch-Sprachnotizen werden jetzt auch ohne sichtbare iPhone-App verarbeitet (auf echter Hardware nachgewiesen). Auch Verarbeitung bei gesperrtem iPhone bestätigt; Zustellung dabei verzögert. Weiter offen: natürlich ausgelöste BGProcessingTask-Läufe, Akku/Langzeitmessung und interaktive Watch-Komplikationen. Force-Quit bleibt eine iOS-Ausführungsgrenze; bestätigte Aufnahmen bleiben erhalten. Siehe `docs/apple-evidence/2026-09-06-background/README.md`.

### Lokaler Gesprächsmodus, 06.09.2026

Auf `codex/watch-conversation-background`: getrenntes automatisches Vorlesen und Freisprechen, persistenter Gesprächskontext mit bis zu sechs Wortwechseln, sicheres Stoppen/Pausieren. 84 Kerntests und vier Bedienungsläufe bestanden; Gedächtnis mit Apple Foundation Models auf echtem iPhone bestätigt. Geräuschqualität und Akku bei längeren Gesprächen bleiben zu messen. Nachweis: `docs/apple-evidence/2026-09-06-conversation/README.md`. Parallele Arbeit erfolgt in eigenem Worktree; Integrationsregeln: `docs/apple-evidence/2026-09-06-background/parallel-development.md`.

### Modelle und Stimmen, 06.09.2026

Persistente Hintergrunddownloads mit kompaktem Symbolknopf, Fortschritt und Status direkt am Modell; lokale Bereitschaftsbenachrichtigung erst nach Integritätsprüfung. Echter iPhone-Download von Qwen 0.5B im Hintergrund bestätigt. Geräte-UI-Test beim Wiederöffnen durch Sperre blockiert. Stimmenauswahl mit Hörprobe auf iPhone und Watch; 84 Core-, fünf Integritäts- und fünf gezielte Simulator-Bedienungstests bestanden. iOS-Force-Quit erfordert erneutes Öffnen; Banner-Sichtprüfung und Energiebedarf noch offen. Nachweis: `docs/apple-evidence/2026-09-06-model-downloads/README.md`.

### Gespräch nach Geräuschen, 06.09.2026

Leere Transkriptionen beenden das Freisprechen nicht mehr. Nicht-KI-Systemergebnis mit dauerhafter Watch-Zustellung, Originalerhalt und automatischer Fortsetzung; Apple-Leerresultat korrekt als noSpeech eingeordnet. Drei Mikrofonempfindlichkeiten und automatische Umgebungspegelschwelle. 87 Core-Tests und zwei gezielte iPhone-UI-Tests bestanden. Reale akustische Abnahme bleibt offen. Nachweis: `docs/apple-evidence/2026-09-06-conversation-noise/README.md`.

### Sprachnotizen löschen, 06.09.2026

Bestätigtes lokales Löschen über Detailansicht und Verlauf-Wischaktion auf iPhone/Watch. Abbrechen erhält die Notiz. Leere Löschmarkierung verhindert Wiederauftauchen durch alte Transfers; unterbrochene Bereinigung wird fortgesetzt. 89 Core-Tests und vier Bedienungsläufe bestanden. Kopien auf anderen Geräten bleiben erhalten. Nachweis: `docs/apple-evidence/2026-09-06-delete-notes/README.md`.

---

# Nachtrag 2026-09-14 — Reife, Vorlesen-Neuschnitt, Multi-Device

Brief von Patrick (mündlich, 14.09.2026), zerlegt in Etappen. Reihenfolge ist
Vorschlag, offene Fragen stehen am Ende. Nichts davon ist begonnen.

## 0. Standort

- `feat/sprachmodelle-l1-anbieter` (0.16.7, L1–L4 + Ledger) ist unreleased.
  **Erst mergen und als v0.17.0 taggen**, bevor der Vorlesen-Neuschnitt beginnt —
  beide fassen `TtsSettings.tsx` und die Fußleiste an, parallel gäbe das Konflikte.
- `TtsSettings.tsx` = 1802 Zeilen, dazu `VoicesCard`, `VoiceChangerCard`,
  `ReadingCard`, `WorkspaceSidebars`. Der Neuschnitt ist auch eine Zerlegung
  dieser Datei.
- Login/Sync: kein Bestand, keine Vorarbeit im Repo. Vergleichbares Muster
  existiert im Bewerbungs-Dashboard (Sync-Hub auf IONOS, OwnerGuard, 2FA).

## E5 — Vorlesen als eine Seite (Reife der Kernfunktion) · Priorität 1

**Ziel:** Die Vorlesen-Seite ist EINE Vollbild-Arbeitsfläche: Inhalt links
(Text/Übersetzung/Zusammenfassung), Player unten, Verlauf links, Dateien und
Kontexthilfe rechts. Nichts mehr „unten drunter".

1. **Feature-Audit** (Tabelle: Feature · Use-Case · bleibt/raus/verschieben):
   - Stimmwechsler → **raus** (Entscheidung Patrick). `VoiceChangerCard.tsx`,
     Rust-Pfad aus M7 und i18n-Schlüssel entfernen; Übersetzung bleibt.
   - Bücher & Dokumente (`ReadingCard`) → **raus von der Seite**; Import wird
     eine Quelle im „+“-Menü des Editors (Dokument/URL/Projektdatei gibt es
     dort schon). Offene Frage, ob ein eigener Ort für Bibliothek bleibt.
   - Stimmen anhören/verwalten (`VoicesCard`) → **raus von der Seite**; Ort
     zu entscheiden (siehe Fragen).
   - KI-Funktionen im Editor (Auto-Tagging, Übersetzen, Zusammenfassen):
     bleiben, aber jede einzeln gegen den Installer abnehmen — Verdacht
     „hat nicht richtig funktioniert" ist offen.
2. **Seitenleisten mit Funktion füllen:**
   - Links „Verlauf": zuletzt vorgelesene Texte/Seiten mit Stimme, Dauer,
     Datum; Klick lädt Text und Stimme zurück in den Editor.
   - Rechts „Dateien": erzeugte Audios der aktuellen Seite (existiert), plus
     **Kontexthilfe** als zweiter Reiter (siehe E7).
3. **Einheitlicher Seitenaufbau** für alle Inhaltsseiten (Diktat, Besprechungen,
   Modelle, Vorlesen, Einstellungen): ein Layout-Baustein
   `PageShell` (Titelzeile · optionale Seitenleisten · Inhalt · Fußleiste),
   damit Positionierung und Stil nicht mehr je Seite abweichen. Bestehende
   Menü- und Fußleiste bleiben, die sind abgenommen.
4. **Zerlegung** von `TtsSettings.tsx` in Editor, Quellen-Menü, Player,
   Server-Status, KI-Aktionen — je Datei eine Aufgabe, keine Verhaltensänderung
   außer den Punkten oben.
5. **Robustheit:** Abnahme-Checkliste je Funktion gegen den Installer
   (Regel „Installer je Abnahmestand"), nicht gegen Vite.

**Artefakt:** Installer v0.18.x mit der neuen Vorlesen-Seite + Screenshot-Reihe
(alle fünf Inhaltsseiten im gleichen Aufbau). Schätzung ~250 kTok inkl. Audit.

## E6 — Multi-Device mit Login und Sync · Priorität 1 (architektonisch)

Braucht ein Design-Dokument vor dem ersten Commit. Drei Ansätze, mit Empfehlung:

| Ansatz | Was | Für | Gegen |
|---|---|---|---|
| **A. Eigener Hub, Ende-zu-Ende verschlüsselt** (Empfehlung) | Kleiner Sync-Dienst auf IONOS (Muster Bewerbungs-Hub): Konto = E-Mail + Passwort, daraus per Argon2 ein Schlüssel; Client verschlüsselt jedes Objekt (XChaCha20-Poly1305) vor dem Upload; Server sieht nur Blobs + Versionen | volle Kontrolle, Server kann nichts lesen, Muster ist im Haus schon gebaut | Server betreiben, Konto-Recovery ist bei E2E ein Design-Thema (Recovery-Code) |
| B. Gerätekopplung ohne Konto | Geräte tauschen per QR-Code Schlüssel, Sync über den Hub nur als Relais | kein Passwort, keine Konten | „Login" fällt weg, was Patrick ausdrücklich will; Neugerät braucht ein altes Gerät |
| C. Fremdspeicher als Transport (OneDrive/Drive) | verschlüsselte Blobs im Nutzer-Cloudspeicher | kein eigener Server | Konflikte/Versionen schlecht beherrschbar, OAuth je Anbieter, Datei-Sync ist kein Objekt-Sync |

**Was synchronisiert wird (Vorschlag, klein anfangen):** Vorlese-Seiten (Texte,
Tags, Sprecherzuordnung), Verlauf, Einstellungen (ohne Pfade/Hotkeys), Sprecher-
Registry (`meta.json`). **Nicht** in Stufe 1: Klon-Referenzaudios, Modelle,
Meeting-Aufnahmen (groß, teils sensibel) — als Stufe 2 mit eigener Freigabe.

**Konfliktregel:** letzte Änderung gewinnt je Objekt, mit Versionsverlauf im Hub,
sodass nichts still verloren geht.

**Artefakt Stufe 1:** Text auf Gerät A anlegen, auf Gerät B (Mac) vorlesen.
Schätzung ~300 kTok (Hub ~100, Client Rust+UI ~150, Design/Review ~50).
Externer Review (Codex) hier ja: Schlüssel-, Konto- und Konfliktlogik.

## E7 — Kontexthilfe an Ort und Stelle · Priorität 2, parallel per Subagent

Rechter Seitenbereich bekommt einen Reiter „Hilfe", der je aktivem Bereich
ohne Klick eine kompakte Einführung zeigt: welche Funktionen es gibt, wie Tags
und Sprecher im Text funktionieren, Stimm-Einstellungen, Hardware-Voraussetzung
(VRAM-Hinweis, Fish-Speech vs. Piper). Inhalte als Markdown je Bereich und
Sprache (`content/help/<bereich>.<lang>.md`), die `(i)`-Symbole springen dorthin.
Kann nach E5 Punkt 3 (PageShell) unabhängig laufen. Schätzung ~80 kTok.

## Außerhalb dieses Repos (nur notiert)

- **Lesefuchs** (Vorleser-App für die Kinder) fertigstellen — eigenes Projekt,
  eigener Brief.
- **Bewerbungsplattform** veröffentlichbar machen: persönliche Daten trennen,
  Beispieldaten, Lizenz. Eigenes Projekt, niedrige Priorität.

## Reihenfolge

1. v0.17.0 aus dem Sprachmodell-Zweig (PR, Merge, Tag).
2. E5 Feature-Audit → Entscheidungstabelle abnehmen → Neuschnitt.
3. E6 Design-Dokument (Spec) → Review → Hub → Client.
4. E7 parallel zu E6, sobald PageShell steht.

## Offene Fragen an Patrick

1. Wohin mit **Stimmen anhören/verwalten**: eigener Menüpunkt „Stimmen",
   oder Bereich auf der Modelle-Seite neben „Vorlesestimmen"? (Regel „kein
   neuer Menüpunkt für eine Einstellung" — Stimmen sind aber Inhalte.)
2. Bleibt eine **Bibliothek** für Bücher/Dokumente irgendwo, oder ist Import
   ins Editor-Menü genug?
3. **Sync-Ansatz A** bestätigt? Und welcher Kontoserver: IONOS neben dem
   Bewerbungs-Hub, oder getrennt?
4. **Sync-Umfang Stufe 1** wie oben, oder sollen Klonstimmen von Anfang an mit?
5. Darf der **Stimmwechsler auch im Rust-Backend** entfernt werden, oder nur
   aus der Oberfläche (Wiederbelebung später einfacher)?
