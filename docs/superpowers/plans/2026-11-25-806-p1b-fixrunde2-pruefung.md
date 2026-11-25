# #806 P1b Fixrunde 2: VRAM im DXGI-Ersatzpfad

## Entscheidungen

- Ausschliesslich Befund 2 im Code: `Loaded` uebertraegt die Lade-Reservierung
  unter der Ledger-Sperre in den geladenen Workerbedarf. RAM ist bereits
  systemweit gemessen und wird freigegeben; VRAM bleibt separat bilanziert.
- Nur der DXGI-Ersatzpfad zieht diesen Bedarf vom konstanten Parent-Wert ab.
  Backend-Werte werden nicht korrigiert. Der Bedarf folgt weiterhin der
  vorhandenen Modell-Footprint-Schaetzung; keine neue Messschnittstelle.
- Die Bilanz folgt dem Kind bis zum bestaetigten Ende im asynchronen Reaper,
  auch bei Fehler, Pause, Neustart und Tempoaenderung. Scheitern Reaperstart
  oder Warten, bleibt der Bedarf konservativ bis zum App-Ende stehen.
- Keine Abhaengigkeiten, Logs oder Protokollaenderungen.
- Befund 1, angepasste Anforderung: **nicht schlechter als der bisherige
  App-Prozess**, der dieselben Inhalte haelt und denselben administrativen
  LocalDumps unterliegt. Administrative LocalDumps koennen Inhalte enthalten;
  absolute Inhaltsfreiheit ist dafuer keine Anforderung. Kein weiterer Code.

## Rust-Pruefung fuer Thread-Claude (hier nicht ausgefuehrt)

In `apps/local-voice/src-tauri`:

```text
cargo test --locked --lib managers::engine_ledger::tests::
cargo test --locked --lib audiobook_worker::
cargo check --locked
```

- `loaded_workers_count_against_a_constant_parent_dxgi_value_until_drop`:
  derselbe synthetische Parent-Wert laesst zwei Worker zu, nach wachsender
  Belegung keinen dritten. Drop macht Platz wieder nutzbar. RAM bleibt frei
  von geladenen Reservierungen. Eingebaute Gegenprobe: derselbe unveraenderte
  Wert ohne Workerabzug laesst den dritten Worker zu.
- `worker_vram_preserves_unknowns_and_does_not_charge_cpu_models`: unbekannter
  GPU-Bedarf bleibt unbekannt, CPU-Modelle ziehen keinen VRAM ab, Abzug
  saettigt bei null, fehlende Messung bleibt fehlend, Drop gibt frei.
- `reaping_a_worker_releases_its_loaded_vram`: synthetisches haengendes Kind
  haelt Bedarf, asynchrones Beenden gibt ihn wieder frei. Gegenprobe in der
  Testkopie: auch bei erfolgreichem `wait` den Memory-Guard per `forget`
  behalten; die abschliessende Freigabe-Assertion muss scheitern.
- Rot-Nachweis in isolierter Testkopie: in `worker_vram_free` den geladenen
  Bedarf nicht abziehen (`free_mb` zurueckgeben). Beide Tests muessen scheitern.
  Zusaetzlich den Korrekturaufruf im DXGI-Zweig bzw. die Uebertragung nach
  `Loaded` einzeln entfernen und die Plattformabnahme wiederholen.

Rust-Rot/Gruen und Mutationslaeufe bleiben beim Build-Thread; Cargo/pnpm wurden
hier auftragsgemaess nicht gestartet. Die Python-Strukturtests ersetzen diese
Verhaltenstests nicht. Praktische DXGI-/GPU-Abnahme, Reaper-Fehlerpfade und die
bisher offenen Plattformpruefungen bleiben offen.

Hier ausgefuehrt: bestehende Python-Worker-Vertragstests (1 Test samt sechs
Mutationen) und Queue-Pruefskript (2 Tests samt Struktur-/SQL-Gegenproben),
LF-Pruefung der geaenderten Dateien sowie `git diff --check`.
