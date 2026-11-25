# #806 P1b Teil 2: Queue-/Import-Anbindung

## Entscheidungen

- Worker ist dauerhaft eingebaut; `audiobook-worker` bleibt als leerer Alias
  fuer bestehende Build-Befehle. Einstieg weiterhin vor Clap/Tauri/Single-Instance,
  `current_exe()` ist die installierte App-EXE. Keine neue Bundle-Datei.
- Ein Worker je laufendem Hoerbuch-Import, nach dem Dekodieren und vor der
  Sprachprobe. Modell aus `audiobook_runs`; pro Inferenz Sprache aus dem
  Laufkontext, Custom Words aus den bestehenden Einstellungen. ORT- und
  transcribe-cpp-Beschleuniger werden ins Kind uebergeben.
- Die Queue behaelt ihre logischen Primary-/Extra-Plaetze und damit ihre
  bestehenden Kapazitaetsvertraege. Bei Hoerbuechern laden BEIDE Plaetze keine
  App-Engine. Auch der Import startet/restauriert kein gemeinsames Modell.
- RAM-/VRAM-Reservierung vor jedem Workerstart unter dem Engine-Ledger.
  Geladene Kinder stecken im gemessenen freien System-RAM/VRAM; sie werden
  nicht erneut abgezogen. Das Ledger zaehlt Kinder separat per RAII-Lease.
- Buchaktionen verteilen dieselbe absolute Frist (Aktion + 2 s) an alle
  registrierten Teile. Diktat/Aufnahme/TTS-Start teilen eine Vordergrundfrist.
  Lade- und Inferenzschleifen pollen alle 20 ms. Nach erzwungenem Ende wird
  die Generation per Compare-and-update entwertet; Cursor bleibt unveraendert.
  Ein vorher fertig gewordener Block darf noch durch die bestehende Commitstelle.
- Parent behaelt Freigaben, Nachpruefung, Cursor, `append_audiobook_block` und
  Ereignisse. Jede Wiederholung braucht ein neues Freigabetor. Pro Block
  hoechstens zwei Neustarts; danach `audiobook_worker_crashed`, kein Commit.
- Start-/Ladefehler beim ersten Worker fuehren zur eigenen App-Engine hinter
  demselben Speichertor. `worker_fallback_code` steht am Auftrag; endgueltige
  Workerfehler in `worker_failure_code`. `AudiobookRunJobInfo.worker_code` und
  die bestehende Buchkarte zeigen diese Codes. Bindings von Hand in Erzeugerform.
  Pause/Abbruch und Speichermangel fuehren nicht zum Rueckfall.

## Geaenderte Dateien

Backend: `audiobook_worker/{mod,process,protocol,parent,tests}.rs`,
`audiobook_worker/parent/tests.rs`, `managers/engine_ledger.rs`,
`managers/meetings/{audiobook,audiobook_run,import,language_run,queue}.rs`,
`managers/transcription.rs`, `lib.rs`, `main.rs`, `Cargo.toml`.

Frontend: `src/bindings.ts`, `src/components/library/KurzbuchCard.tsx`.
Pruefskripte: `test_audiobook_worker_contract.py`, `test_audiobook_worker_queue.py`.
Alle angefassten Dateien mit LF; keine neuen Abhaengigkeiten oder Migrationen.

## Pruefung / Uebergabe

Ausgefuehrt: Worker-Strukturvertrag mit sechs Gegenproben, Queue-Strukturvertrag
mit acht Gegenproben, echte SQLite-Generationsentwertung mit zwei Mutationen,
vier bestehende SQL-Kontexttests, 32 P1a-Node-Tests und 26 Import-Node-Tests.
Die Node-Tests brauchen in dieser Sandbox `--test-isolation=none`; der
Import-Test zusaetzlich `--experimental-vm-modules`. Ohne diese Flags scheiterten
die Aufrufe an `spawn EPERM` bzw. `vm.SourceTextModule is not a constructor`.
Diese Ergebnisse ersetzen keine Rust-Kompilierung.

Neue Rust-Tests (noch NICHT ausgefuehrt): vier haengende Trait-Worker mit einer
Frist unter fuenf Sekunden, Stoppsignale fuer vier registrierte Teile, begrenzte
Neustarts und neue Freigaben, sichtbarer Startrueckfall mit Gegenproben,
Worker-Ledger, konfigurierte Inferenz, DB-Cursor/Generation und Hinweis-Codes.
Bestehende Tests wurden nicht umgeschrieben (ausser dem Strukturvertrag fuer
den nun dauerhaft eingebauten Worker).

Build-Thread, in `apps/local-voice/src-tauri`:

```text
cargo check --locked
cargo test --locked --lib audiobook_worker::
cargo test --locked --lib managers::meetings::audiobook
cargo test --locked --lib managers::meetings::queue::tests
cargo test --locked --lib managers::meetings::import::
cargo test --locked --lib managers::meetings::language_run::
```

Frontend-Build/Typpruefung ebenfalls noch offen. Es wurden keine Modelle geladen,
keine laufende App/OBS/WSL-Instanz beruehrt und kein Installer gebaut.

## Offene Punkte / Grenzen

- Punkt 6 (ONNX-Threadbegrenzung ohne Vendorpatch) und Punkt 7
  (Webview-Gespraechslease, Akku-Polling) sind wie erlaubt ausgespart.
  Es wird weder eine unbewiesene ORT-Umgebungsvariable gesetzt noch die
  Affinitaet der App veraendert. ONNX erhaelt noch keine garantierte Threadgrenze.
- Tempo/Threadbudget und Prozessprioritaet werden zwischen Bloecken verglichen;
  Aenderungen ersetzen den dann untätigen Worker vor dem naechsten Block.
- Der ausdrueckliche App-Prozess-Rueckfall behaelt das bisherige native
  Block-/Pauseverhalten. Die harte Kindprozessfrist gilt fuer den Workerpfad;
  eine haengende Inferenz im Rueckfall kann nicht per Kind-Kill beendet werden.
- Das 5-s-Ziel ist noch nicht an der echten Queue/Installations-EXE abgenommen.
  Die vorhandenen echten Kindprozess-Killtests und neuen Trait-Tests muessen
  vom Build-Thread laufen. Windows/macOS-Prioritaet, native stdout-Ausgaben
  und GPU-/Modellabnahme bleiben ungeprueft.
