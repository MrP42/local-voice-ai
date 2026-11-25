# #806 P1b Fixrunde 1: Pruefuebergabe

Ergaenzung Fixrunde 2: Die Windows-LocalDumps-Anforderung ist ausdruecklich
angepasst auf **nicht schlechter als der bisherige App-Prozess**, der dieselben
Inhalte haelt und denselben administrativen LocalDumps unterliegt. Solche Dumps
koennen weiterhin Inhalte enthalten. Absolute Inhaltsfreiheit wird fuer diesen
Pfad nicht verlangt; zu Befund 1 folgt keine weitere Codeaenderung.
Die unten genannten WER-Flagtests beweisen keine LocalDumps-Sperre.

Implementierung im Arbeitsbaum auf Basis d1c57d34; keine Builds, Installationen,
Netzzugriffe oder Eingriffe in laufende Programme durch diesen Auftrag.

## Entscheidungen

- Die beiden roten Kontexttests scheiterten an `metadata_json`: dieser Zugang
  liefert bei geloeschten Meetings einen Fehler. `audiobook_job_info` liest den
  optionalen Hinweis jetzt mit einem LEFT JOIN im selben Snapshot. Der Lauf
  bleibt auch ohne Meeting erhalten; geloeschte Meetings liefern keinen Hinweis.
- Beim Hinweis wird jeder JSON-Wert zuerst in `Option<&str>` umgewandelt.
  Ein `null` beim Fehlercode verdeckt den Rueckfallcode damit nicht mehr.
- Unix: Parent-PID vor Spawn mitgeben, unter Linux bereits vor exec
  `PR_SET_PDEATHSIG(SIGKILL)` samt nachfolgender PID-Pruefung. Im Worker nochmals
  absichern und einen unabhaengigen Thread fuer Parent-PID und stdin-HUP starten.
  Er liest keine Protokollbytes und beendet ohne Destruktoren; Pollintervall 50 ms.
- Vor dem ersten Frame: Unix setzt Soft- und Hardlimit RLIMIT_CORE auf null,
  Linux zusaetzlich PR_SET_DUMPABLE auf null. Fehler verhindern Nutzdatenempfang.
- Windows: vorhandenes `windows` 0.61.3, zusaetzliches Feature
  `Win32_System_ErrorReporting`; SetErrorMode mit SEM_NOGPFAULTERRORBOX und
  WerSetFlags(NOHEAP), ausschliesslich im Worker. Keine globale Ausnahme und
  kein WerAddExcludedApplication. Das bestehende Job-Objekt bindet das Kind.
  Grenze: NOHEAP allein schliesst Stack-Inhalte nicht aus. Prozesslokale WER-
  Einstellungen garantieren keine Sperre gegen administrativ konfigurierte
  LocalDumps, externe Dumper/Debugger oder eigene native Crash-Handler. Auch
  macOS-Diagnoseberichte sind keine durch RLIMIT_CORE abgeschalteten Core-Dateien.
  Eine absolute Inhaltsfreiheit solcher externen Berichte ist nicht abgenommen.
- Der Queue-Test nutzt gemeinsame Produktionshelfer fuer Registrierung, Polling
  und Generationsentwertung; die Queue und SQLite bleiben echt. Die Registry-
  Signale sind sein einziger Pauseeingang, damit ein fehlender Steueraufruf
  nicht durch ein DB-Ersatzsignal verdeckt wird.

## Rust-Gates fuer Thread-Claude (hier nicht ausgefuehrt)

In `apps/local-voice/src-tauri`:

```text
cargo test --lib managers::meetings::audiobook_run::tests::
cargo test --lib audiobook_worker::
cargo test --lib managers::meetings::queue::tests::control_audiobooks_stops_four_hung_workers_and_persists_invalidation
```

Danach das bisherige Windows-Tor inklusive Bindings und Clippy wiederholen.
Unter Linux und macOS die `audiobook_worker::safety::tests::` separat ausfuehren:
echte synthetische Kind-/Enkelprozesse, stdin-EOF waehrend blockiertem Hauptthread,
Parent-Abschuss bei offengehaltener Pipe, unter Linux auch ohne Kernel-Signal,
nativer SIGABRT nach synthetischem Marker. Alle Wartepfade sind begrenzt.
Windows testet WER-Flags im eigenen Testkind und unveraenderte Parent-Flags.

## Gegenproben / Rot-Nachweis

Die folgenden Mutationen nur einzeln in einer isolierten Testkopie anwenden;
keine Aenderungen im gemeinsam genutzten Baum zuruecksetzen:

| Test | Ohne Behebung / eingebaute Gegenprobe |
| --- | --- |
| `deleted_jobs_keep_their_context_even_without_siblings_after_restart`, `legacy_resume_shares_metadata_context_before_replacing_original_jobs` | Alter `metadata_json(id)?`-Zugang scheitert nach Loeschung; erster Test prueft vor Loeschung einen sichtbaren Fehlerhinweis. |
| `worker_notice_is_per_job_and_terminal_code_takes_precedence` | Altes `get(...).or_else(...).as_str()` liefert bei explizitem Fehlercode-null keinen Rueckfallhinweis. Terminaler String behaelt Vorrang. |
| `parent_death_ends_a_hung_worker_even_with_an_open_stdin_writer`, `stdin_eof_ends_a_hung_worker_while_its_parent_lives` | Eingebauter ungeschuetzter Prozess bleibt am Leben und wird aufgeraeumt; geschuetzter Prozess muss enden. Linux testet den Waechter auch nach Abschalten von PDEATHSIG. |
| `native_crash_after_synthetic_payload_cannot_dump_core` | Ohne Dump-Policy scheitern die Policy-Assertions vor Marker/SIGABRT. Linux setzt zuvor dumpable=1; Unix aktiviert das geerbte zulaessige Core-Limit. Bei bereits hartem Core-Limit null auf macOS ist fuer den RLIMIT-Rot-Nachweis ein anderer Testhost erforderlich. |
| `dump_policy_is_confined_to_the_worker_process` (Windows) | Eingebautes Kind ohne Schutz scheitert nach explizitem Zuruecksetzen der Flags; geschuetztes Kind besteht, Parent bleibt gleich. |
| `control_audiobooks_stops_four_hung_workers_and_persists_invalidation` | Entferntes `stop_books` in `control_audiobooks` fuehrt zum Test-Timeout statt Yield; entfernte Entwertung in `poll_worker_block` laesst die persistierte Generation unveraendert. Vor Pause bestehen Commit und Replay; nach Pause werden spaeter Block und Replay abgewiesen, Cursor bleibt, Folgeauftrag nutzt freien Platz. |

Rust-Rot/Gruen und Mutationslaeufe sind noch offen, ebenso praktische Windows-
Crashdump-Abnahme und Plattformlaeufe unter Linux/macOS. Lokal ausgefuehrt:
Python/SQLite mit dem Original-SQL (lebend/geloescht/fehlend und Gegenprobe alter
Metadatenzugang), TOML-/Feature- und LF-Pruefung sowie `git diff --check`.
