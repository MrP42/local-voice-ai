# #806 P1b Teil 1: Worker-Infrastruktur (ausgeschalteter Teilschnitt)

Der Umsetzungsauftrag erlaubt bei zu grossem Umfang einen konsistenten
Teilschnitt hinter einem ausgeschalteten Schalter. Dieser Stand aktiviert
**keine** Hoerbuch-Queue und erfuellt noch **nicht** das 5-s-Ziel der App.
Keine neuen Abhaengigkeiten, keine Migration, keine Bindings-Aenderung.

## Entscheidungen und Schnitt

- Cargo-Feature `audiobook-worker` ist nicht standardmaessig aktiv.
  `--lva-audiobook-worker` wird vor Clap/Tauri/Logger abgefangen; ohne Feature
  oder mit weiteren Argumenten endet der Aufruf still mit Exitcode 64.
- `transcription.rs`: lokaler Modellaufbau und Segmentinferenz sind gemeinsame
  Funktionen. Die bisherigen Aufrufer behalten Freigaben, Engine-Bindung,
  Panic-Behandlung, Laufzeit-Capabilities und Nachbearbeitung. Der Worker ruft
  dieselben Funktionen und dieselbe Schleifenbereinigung auf. Diktat und
  Accurate-Inferenz sind beim Herausziehen unveraendert geblieben.
- Protokoll v1: vier Byte Little-Endian-Payloadlaenge, dann UTF-8-JSON mit
  `version` und `message`. Maximal 8 MiB pro Frame, 320000 endliche f32-Samples
  pro Infer (20 Sekunden bei 16 kHz). Nachrichten: Load, Loaded, Infer, Result,
  Shutdown, Error. Load bekommt einen absoluten Modellpfad, Modellart und
  Sprachfaehigkeiten vom Parent; keine eigene Modellsuche und kein Download.
- Worker installiert keinen Logger; Parent verwirft stderr. Fehler sind feste
  Enum-Codes, nie native Fehlertexte oder Panic-Payloads. PCM-/Textnachrichten
  implementieren absichtlich kein Debug.
- ProcessWorker: ein begrenzter Pipe-I/O-Thread, keine blockierenden Pipe-Zugriffe
  im Steuerthread. Windows: Creation-Flags IDLE (gentle), sonst BELOW_NORMAL,
  kein Konsolenfenster, eigenes Kill-on-close-Job ohne Prioritaetsueberschreibung.
  Fehlende Job-Zuordnung gilt als Startfehler. Unix: nice +10 vor exec.
- InferenceWorker/InferenceSession: ein Block gleichzeitig, Generation pruefen,
  eine vom Aufrufer gesetzte absolute Stopfrist, danach Ergebnis sperren und
  Kind beenden. Stopfrist darf bei weiteren Signalen nur kuerzer werden.
  Prozess-Reaping wartet hoechstens 100 ms pro Kind im Steuerthread; Drop
  toetet das Kind und lagert verbleibendes Reaping aus. Keine globale Registry.

## Pruefung durch den Build-Thread (noch auszufuehren)

Im Verzeichnis `apps/local-voice/src-tauri`:

```text
cargo check --locked
cargo test --locked --lib --features audiobook-worker audiobook_worker::
cargo test --locked --lib transcribe_cpp_run_plan
```

Rust-Tests sind synthetisch und laden kein Modell. Sie pruefen Frame-Grenzen,
Version, Muell, Teilframes, ungueltige Samples, feste Fehlercodes, Shutdown,
Load-Reihenfolge, fremde Generationen, Ergebnisse vor/nach der Frist,
Crash vor Ergebnisuebergabe, doppelte Antworten, gemeinsame Frist fuer vier
Fakes sowie Kill/Reaping von vier echten haengenden Test-Kindprozessen.
Gegenproben stehen jeweils daneben. Der ignorierte `hanging_child`-Test wird
nur vom Kill-Test als Kind gestartet; nicht manuell mit `--include-ignored`.

Bereits ausgefuehrt: Python-Strukturpruefung
`python apps/local-voice/scripts/test_audiobook_worker_contract.py` mit sechs
gezielten Quelltext-Mutationen als Gegenproben, LF-Pruefung und `git diff --check`.
Das beweist keine Rust-Kompilierbarkeit oder Plattformfunktion. Rust-Tests und
Builds bleiben gemaess Auftrag beim Thread-Claude.

## Noch erforderlich vor Aktivierung

1. Queue/Import und Sprachproben auf Worker routen, primaere Engine dabei
   auslassen; RAM-/VRAM-Gate und Engine-Ledger um Workerbelegung erweitern.
   App-seitige ONNX-Initialisierung und Custom-Words-Uebergabe abgleichen.
2. Freigabe pro Inferenz im Parent einloesen und danach erneut pruefen;
   `append_audiobook_block` unveraendert zur einzigen Commitstelle machen.
3. Buchsteuerung muss dieselbe Frist ab der Nutzeraktion an alle Sessions
   verteilen, diese weiter pollen, Generationen dauerhaft entwerten und erst
   nach bestaetigtem Prozessende `paused` publizieren. Die Session allein
   aendert keinen DB-Zustand und ist kein autonomer Watchdog.
4. Sichtbarer P1a-Rueckfall bei Start-/Ladefehlern, begrenzte Block-Neustarts
   (maximal zwei), danach Failed-Code; keine stille Fehlerluecke.
5. DB-/Queue-Integrationstests: Crash vor/nach Commit, Pause zwischen Claim und
   Start, keine spaeten/doppelten Commits, vier echte Worker inklusive Pausefrist
   unter fuenf Sekunden. Modell-/GPU-/Prioritaetsabnahme auf Windows/macOS/Linux,
   einschliesslich des Verhaltens nativer stdout-Ausgaben im Pipe-Protokoll.

ONNX-Threadbegrenzung, Gespraech/Akku und P2-Fortschritt bleiben ausserhalb
dieses Teilschnitts. Ohne Queue-Anbindung gilt weiterhin das P1a-Pauseverhalten.
