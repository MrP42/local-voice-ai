# Status — Stand 2026-08-17

Nur real ausgeführte und verifizierte Dinge stehen unter „verifiziert".

## Phase 2 — Skill `research-first-rebuilder`

| Punkt | Status |
|---|---|
| Skill-Quelle (`tooling/research-first-rebuilder/`) | **fertig** — SKILL.md, 4 Referenzen, 12 Templates, 3 Skripte |
| Hilfsskripte real getestet | **verifiziert** — `repo_health.py` gegen echtes und nicht existierendes Repo; `license_scan.py` gegen 5 konstruierte Lizenzfälle (Stock-Apache, Apache+Commons-Clause, Stub, Brand-Carve-out, keine Lizenz) |
| Dabei gefundene und behobene Fehler | 2 Windows-Encoding-Bugs (cp1252) + ein Apache-2.0-Falschalarm |
| Installation | **verifiziert** — 23 Dateien unter `~/.claude/skills/research-first-rebuilder/`, Dev-Evals korrekt ausgeschlossen |
| Eval-Läufe | **abgeschlossen** — 4 Testfälle x (mit/ohne Skill), alle 8 Läufe real ausgeführt |
| Grading | **abgeschlossen** — programmatischer Grader (`grade.py`), Assertions teils per Skript ausgeführt (z. B. Bugfix wird importiert und mit 4 Payloads aufgerufen) |
| A/B gegen Baseline | **durchgeführt** — Ergebnis unten |
| Skill-Überarbeitung nach Eval | **durchgeführt** — v1.0.0 → v1.1.0, installiert |
| Benchmark-Aggregation, Eval-Viewer, Description-Optimierung (`run_loop.py`) | **offen** |

### Eval-Ergebnis Iteration 1 (real ausgeführt)

| Testfall | mit Skill | ohne Skill |
|---|---|---|
| `local-dictation-alternative` | **9/9** | 8/9 |
| `dont-reinvent-find-fork` | **8/8** | 8/8 |
| `refuse-asset-theft-but-help` | **6/6** | 6/6 |
| `not-a-rebuild-simple-bugfix` (Nicht-Trigger) | **5/5** | 5/5 |
| **Summe** | **28/28** | **27/28** |

**Ehrliche Einordnung.** Der Abstand ist dünn und darf nicht als starker Beleg gelesen werden:
die Assertion-Menge ist nahe der Sättigung und damit wenig trennscharf — ein starkes Basismodell
erfüllt die meisten Kriterien ohnehin. Belastbar sind zwei Befunde:

1. **Nicht-Trigger funktioniert.** Beim simplen Bugfix löst der Skill den 10-Gate-Workflow
   korrekt **nicht** aus und produziert keine Rechercheartefakte.
2. **Beim Ablehnungsfall gibt es keinen Vorteil** gegenüber der Baseline (6/6 zu 6/6).

Der qualitativ sichtbare Unterschied liegt in der **Struktur und Nachvollziehbarkeit** der
Artefakte: der Skill-Lauf lieferte ein Quellenverzeichnis mit Repository, Commit- **und**
Blob-SHA je Lizenzbeleg und bemerkte dabei, dass zwei Projekte byte-identische Lizenzdateien
führen. Solche Qualität erfassen binäre Assertions schlecht.

**Zwei zunächst gemessene Skill-Vorteile waren Fehler in meinem eigenen Grader** (zu enge
Regex für Ablehnungsformulierungen; Messung des `https://`-Präfixes statt der tatsächlichen
Nachvollziehbarkeit). Beide wurden korrigiert, statt das schmeichelhaftere Ergebnis stehen zu
lassen.

## Phase 3 — Anwendung „Local Voice AI" (bis 18.08.2026 „Sprechstift")

| Meilenstein | Status |
|---|---|
| M-1 Rust-Toolchain | **verifiziert** — rustc/cargo 1.97.1, stable-x86_64-pc-windows-msvc |
| M0.1 Lizenzen | **verifiziert** — `cargo deny check licenses` ok; kein GPL/LGPL/AGPL, keine unlizenzierte Crate auf dem Windows-Zielgraphen |
| M0.2 Advisories | **verifiziert** — 8 Vulnerabilities gefunden, 6 behoben, 2 dokumentiert ignoriert |
| M0.3 Namensprüfung | **verifiziert** — Sprechstift frei auf npm, crates.io, PyPI, GitHub |
| M0.5 Baseline-Build | **verifiziert** — nach Behebung des Vulkan-SDK-Blockers |
| M0.6 Modell-Integrität | **verifiziert** — Parakeet V3 SHA-256 stimmt mit dem Katalogwert überein |
| M1 Fork + Rebranding | **verifiziert** — Subtree-Import mit voller Historie, Updater und SignCommand entfernt, App startet unter eigenem Namen |
| **Lokale deutsche Transkription** | **verifiziert** — siehe Messung unten |
| M2 vertikaler Pfad (Hotkey bis Einfügen) | **verifiziert 2026-07-29** — Harness-Lauf 6/8, siehe `docs/m2-evidence/` |
| **M3 Stabilisierung des Basispfads** | **teilweise verifiziert 2026-08-17** — siehe unten |
| **M4 / TP1 Vorlesen (Fish-Speech-TTS)** | **verifiziert (headless) 2026-08-18** — siehe `docs/m4-evidence/`; Hotkey-Hörtest manuell offen |
| **M5 / TP2 Stimmen klonen (zero-shot)** | **verifiziert (headless+live) 2026-08-18** — siehe `docs/m5-evidence/`; In-App-Aufnahme manuell offen |
| **M6 / TP5 Performance (compile + Satz-Pipeline)** | **verifiziert 2026-08-18** — RTF 0,63–0,72 statt ~6; erster Satz in ~1,8 s; siehe `docs/m6-evidence/` |
| **M7 / TP3+TP4 Übersetzung + Stimmwechsler** | **verifiziert (Bausteine + Live-LLM) 2026-08-18** — siehe `docs/m7-evidence/`; Mikro-Flows manuell offen |
| M8 bis M10 | **offen** |

### Verifizierte Messung (2026-07-28, i9-13900K, CPU-Backend)

```
Modell     : Parakeet TDT 0.6B v3, int8, CC-BY-4.0 (Attribution: NVIDIA)
Audio      : 9,15 s deutsch
Modellladen: 1035 ms
Inferenz   : 392 ms  (etwa 23x Echtzeit, ohne GPU)
Ausgabe    : "Guten Tag, dies ist ein Test der lokalen Spracherkennung.
              Der Termin ist am 3. Februar um 14.30 Uhr."
```

Korrekte deutsche Groß- und Kleinschreibung sowie Interpunktion; gesprochene Zahlen normalisiert
(dritten Februar wird zu 3. Februar, vierzehn Uhr dreißig wird zu 14.30 Uhr).

## M3 — Stabilisierung (2026-08-17)

Ziel: der Basispfad Hotkey → Aufnahme → Parakeet V3 → **genau ein** kontrollierter
Einfügeversuch → bei Unsicherheit Zwischenablage plus sichtbare Meldung.

### Was geändert wurde

| Bereich | Änderung |
|---|---|
| Live-Injektion (Streaming) | Zunächst hinter `experimental_enabled` gesperrt, **noch am selben Tag durch Messung freigegeben**: Die Defektmeldung war älter als ihre eigenen Fixes (`6b9143e`, `d223fa8`). Verifiziert, wieder einfacher Opt-in-Schalter. Siehe D8. |
| Transkripte in Logs | Klartext nur noch in **Debug-Builds**. Im Release-Build loggen STREAMDIAG, der Segmenter, der Abschlusslog und die Refinement-Ablehnung ausschließlich Längen und Gate-Namen — unabhängig von `debug_mode`. |
| Einfügepfad | Neu `paste_guard.rs` + `paste_transcript_guarded()`: Zielfenster beim Stop erfassen, Fokus und Rechtelage vor dem Einfügen prüfen, **genau ein** Versuch, Verifikation der Zwischenablage durch Rücklesen, danach erneute Fokusprüfung. |
| Sichtbare Meldung | Neuer Overlay-Zustand `notice` — erscheint **auch bei `overlay_style: none`** und blendet nach 9 s aus. Zusätzlich ein Toast im Hauptfenster, falls es offen ist. |
| Einstellungen | Store auf sichere Baseline gesetzt: Parakeet V3, `stream_injection: false`, `debug_mode: false`, `log_level: info`. Sicherung unter `settings_store.json.vor-stabilisierung-2026-08-17.bak`. |

### Aktueller Verifikationsstand

| Prüfung | Ergebnis |
|---|---|
| `cargo test --lib` frisch ausgeführt | **verifiziert** — 198 passed, 0 failed (vorher 190; 8 neue) |
| `cargo build --release` frisch | Exit 0 — **aber als Funktionsbeleg wertlos**, siehe Falle 3 unten |
| `npx tauri build --no-bundle` | siehe unten |
| Frontend `npm run build` | **verifiziert** — Exit 0 |
| Kein Transkript-Klartext im Release-Binary | **verifiziert am Artefakt** — siehe unten |
| Native Windows-Abnahme (`scripts/m3-verify.ps1`) | **verifiziert** — 11/11 Szenarien, siehe Matrix unten |
| 100 aufeinanderfolgende Diktate | siehe Matrix unten |

### Windows-Testmatrix (2026-08-17, echtes Mikrofon, echter Hotkey, echtes Notepad)

| Fall | Ergebnis | Beleg |
|---|---|---|
| Normaltext | PASS | 100 Zeichen in 1,9 s |
| Umlaute und ß | PASS | 103 Zeichen — „Der ältere Herr aus der Straße … Fußballschuhen … Köln." (im M2-Lauf noch FAIL) |
| Satzzeichen | PASS | 77 Zeichen |
| Mehrzeilig | PASS | 101 Zeichen |
| Zahlen | PASS | 56 Zeichen |
| Abbruch | PASS | Notepad bleibt leer |
| Stille | PASS | kein Text, kein Hänger |
| Schnelles mehrfaches Umschalten | PASS | App lebt nach 4 Zyklen, kein Text |
| Fokuswechsel während der Aufnahme | PASS | Ausgangsfenster bleibt **leer**, Text landet im Zielfenster |
| Fenster ohne Eingabefeld (Explorer) | PASS | App lebt; Text im Verlauf (siehe Einschränkung) |
| Erhöhtes Ziel (Task-Manager) | PASS | kein Einfügeversuch, vollständiges Transkript in der Zwischenablage |
| Datenschutz des Logs | PASS | keines der diktierten Wörter steht im Log |
| **Live-Einfügung während des Sprechens** | PASS | Text steht **vor** dem Stopp im Dokument; Pausen und Umlaute korrekt, keine Duplikate |

Transkription war in **allen** Fällen erfolgreich; keine Duplikate, kein stiller Verlust,
Einfügedauer rund 317 ms.

**Nicht abgenommen** und daher nicht behauptet: Browser-Textfeld, Microsoft Word, VS Code.

#### Beleg für die Log-Bereinigung

**Der erste Beleg war unzureichend.** Geprüft wurde, ob die *bekannten*
Formatzeichenketten im Binary fehlen — das war grün, und die Sache galt als
erledigt. Der Dauerlauf legte danach 47 vollständige Diktate im Log offen:
`info!("Transcription result: {}", …)` hatte das Suchmuster des Quelltext-Audits
nicht getroffen. Behoben, und seither prüft das Szenario `log-privacy` **das
echte Log nach einem echten Diktat** auf die gesprochenen Wörter.

Der Binary-Nachweis bleibt als zusätzliche Prüfung bestehen:

| Formatzeichenkette | in `local-voice-ai.exe` |
|---|---|
| `STREAMDIAG committed(len={})={:?}` | **nein** |
| `STREAMDIAG delta(len={})={:?}` | **nein** |
| `Transcription completed in {:?}: '{}'` | **nein** |
| `gate={:?} original={:?} candidate={:?}` | **nein** |
| `Transcription result: {}` (Klartext) | **nein** — erst nach der zweiten Runde |
| `STREAMDIAG committed_len=` | ja |
| `Transcription completed in {:?} ({} chars)` | ja |
| `Transcription result: {} chars` | ja |
| `Text refinement rejected: stage=` | ja |

### Dauerlauf: 100 Diktate

Zweimal gefahren. Der erste Lauf lief noch gegen das Binary mit dem Log-Leck; der hier
berichtete zweite gegen das **ausgelieferte** Artefakt.

| Kennzahl | Wert |
|---|---|
| Läufe | 100 |
| Fehlschläge | **0** |
| Leere Ergebnisse | **0** |
| Median Diktatdauer | 1,9 s |
| Maximum | 1,9 s |
| Klartext-Diktate im Log danach | **0** (geprüft auf „Termin", „Februar", „dritten") |
| Speicher der App danach | 759 MB — das geladene Parakeet-Modell, wird nach 5 min Leerlauf entladen |

Fixture `de_short_01.wav` (2,8 s deutsch), Ziel Notepad, Rücklesung per UI Automation.
Der einzige `ERROR` im Log über beide Läufe ist der erwartete `TargetElevated`-Fallback
aus dem Szenario mit dem erhöhten Task-Manager.

### Drei Toolchain-Fallen, die erst der native Lauf sichtbar gemacht hat

1. **`cargo` fehlte in beiden Shell-PATHs** (liegt unter `~\.cargo\bin`). In Git Bash meldet
   `cargo test` dann `command not found` — und liefert durch eine Pipe trotzdem **Exit 0**.
2. **CMake-Generator-Konflikt** bei `transcribe-cpp-sys`: Der Cache stand auf Ninja, der Build
   forderte Visual Studio. Der Cache liegt hinter einer NTFS-Junction; nur `%LOCALAPPDATA%\tcs`
   zu löschen genügt nicht.
3. **`cargo build --release` erzeugt kein lauffähiges Produkt.** Das ist der teuerste Fund:
   Der Build endet mit Exit 0, die EXE startet, das Tray-Symbol erscheint — und die Anwendung
   ist funktionsunfähig, weil die Webview `http://localhost:1420` lädt statt des eingebetteten
   Frontends. Ohne Frontend registriert niemand die Shortcuts (das tut bewusst das Frontend,
   nicht das Backend), der globale Hotkey ist tot.

Alle drei sind in `docs/BUILD-WINDOWS.md` mit Nachweis und Abhilfe beschrieben.

### Der Testfall „erhöhtes Ziel" war zuerst falsch konstruiert

Er scheiterte im ersten vollständigen Lauf — nicht am Einfügepfad, sondern am Testaufbau:
Er löste die Aufnahme per Hotkey aus, und **der Hotkey ist gegenüber erhöhten Fenstern
blind** (gemessen: 0 Bytes Log-Zuwachs mit erhöhtem Task-Manager im Vordergrund gegenüber
8223 Bytes ohne). Eine Aufnahme, die dort nie startet, kann auch keinen Fallback auslösen.

Umgebaut auf `--toggle-transcription`, das die laufende Instanz über
`tauri_plugin_single_instance` erreicht statt über den Tastatur-Hook. Damit ist der Zweig
erreichbar — und er greift:

```
paste guard: target=…pid: 160960 foreground=… self_elevated=false target_elevated=Some(true)
Automatic paste not performed: TargetElevated
Zwischenablage danach: "Der Termin ist am dritten Februar."
```

Zusätzlich fehlte dem Guard jede Protokollierung seiner eigenen Entscheidung — ohne die
war der Fehlschlag im Log unsichtbar. Die Zeile oben ist die Nachrüstung.

### Erster nativer Abnahmelauf: fehlgeschlagen, Ursache geklärt

Der erste Lauf von `m3-verify.ps1` meldete fünfmal „0 chars". **Nicht der Einfügepfad war
schuld** — das Log zeigte nach dem Start keine einzige Zeile mehr, die Hotkeys kamen also nie
an. Ursache war Falle 3. Nachweis am Artefakt:

```
local-voice-ai.exe enthält "localhost:1420"        -> ja   (falsch)
local-voice-ai.exe enthält "index-<hash>.js"       -> nein (falsch)
Fenstertitel laut UI Automation                 -> "localhost – Netzwerkfehler"
```

Zwei Folgeänderungen am Harness, damit dieser Fehler nie wieder als Fehler des Diktatpfads
erscheint: ein Preflight bricht mit Build-Checkliste ab, wenn der Hotkey die App nicht
erreicht, und das Skript stirbt nicht mehr auf seinem eigenen Fehlerpfad (`$txt` war null),
sondern schreibt den Bericht auch bei Fehlschlägen.

## Nächste Schritte

Als Issues erfasst unter https://github.com/MrP42/local-voice-ai/issues

1. **#1** Abnahme gegen Browser, Word und VS Code nachziehen — Notepad, Explorer und
   erhöhter Task-Manager sind erledigt
2. **#6** Alte Logdatei mit den 47 Klartext-Diktaten löschen
3. **#4** Ursache der defekten Live-Injektion messen, statt weiter zu raten
4. **#5** Refinement-Stufe erst danach optional aktivieren
5. **#7** Installer, SBOM, Third-Party-Notices

## Repository

| | |
|---|---|
| `origin` | `git@github.com:MrP42/local-voice-ai.git` — **privat** |
| `upstream` | `https://github.com/cjpais/Handy.git` — fremdes Fork-Original, **niemals dorthin pushen** |
| Arbeitsbranch | `feat/m3-stabilize-paste-path` |

## Native Apple P0/P1 – abgeschlossener Simulatorumfang, 05.09.2026

Isolierter Branch `codex/apple-p0-p1`: native SwiftUI-Apps für iPhone und Watch,
dauerhafte Audioübergabe, lokale CPU-STT und kurze lokale Modellantwort, System-TTS
auf der Watch und Verlauf auf dem iPhone. Der Benutzer hat virtuelle Xcode-Geräte
anstelle physischer Geräte beauftragt. Finaler Programmstand `aad2e0f`:
31 Kerntests, je ein frischer nativer iPhone-/Watch-Bedienungstest sowie Simulator-Build
bestanden; 100/100 lokale Sprachaufträge mit Hintergrund, Neustarts und
Verbindungsunterbrechung vollständig beantwortet und quittiert (896,018 s).
Lock, Wrist Down, benutzerseitiges App-Ende, Duplikate, Mikrofonverweigerung und
expliziter Wiedergabestop sind mit ihren Simulatorgrenzen dokumentiert.
Zwei Claude-Reviews abgeschlossen, bestätigte Befunde bearbeitet.

P0/P1 als Machbarkeitsprototyp abgeschlossen. Vorläufige Latenzziele nicht erreicht,
Akkuauswirkung unbekannt; keine physische Audio-/Funk-/Data-Protection-Abnahme.
Bei inaktivem, gesperrtem oder beendetem iPhone bleibt die Aufnahme gespeichert;
Verarbeitung wird nach Aktivierung fortgesetzt. Desktop-Bestand und lokale Änderungen
im ursprünglichen Checkout erhalten; kein Push und keine Veröffentlichung.
Details: [Simulatorbericht](apple-evidence/2026-09-05-simulator-report.md),
[P2-Folgeplan](apple-evidence/P2-follow-up.md). Der anschließende P2-Stand ist unten separat dokumentiert.

## Native Apple P2 – Simulatorabnahme abgeschlossen, 06.09.2026

Programmstand `a9fbf65`: persistente Aufträge, begrenzte Retries/Abbruch, getrennte
Aufnahme-/Transport-/Jobsteuerung, sichere Wiederherstellung und Modellverwaltung.
Claude-Review nach ausdrücklicher Freigabe abgeschlossen, Befunde bewertet und
korrigiert. 59 Kerntests, 14 Prozessabbrüche, vier iPhone-UI-Tests, ein Watch-UI-Test
und gezielter letzter Setup-Retry bestanden. Beide finalen Apps installiert.

Frische 20 Warm-/Kaltturns, 100/100 beantwortete Aufträge und Audit aller 200
Originaldateien bestanden. Alle vereinbarten Simulator-Lifecyclefälle erneut geprüft.
96 TTS-Starts im absichtlich unterbrochenen 100er-Lauf; keine Behauptung 100 vollständig
abgespielter Antworten. Einzelturn median 20,718 s warm / 24,216 s kalt bis TTS-Start.
Echtzeitziel verfehlt, Modell-Rechenfehler bekannt, echte Akkuauswirkung unbekannt.
Hintergrund/Sperre/App-Ende verschieben Verarbeitung bis zur Aktivierung.

Keine ausstehende Freigabe. Desktop unverändert, kein Push/Release.
[P2-Bericht](apple-evidence/p2/README.md), [Lifecycle-Matrix](apple-evidence/p2/lifecycle-matrix.md),
[Folgeprioritäten](apple-evidence/p2/next-priorities.md).
