# Nativer Windows-Build und -Test

Reproduzierbarer Ablauf ohne Docker. Alles läuft als natives Windows-Programm.

## Voraussetzungen

| Werkzeug | Version hier | Fundort |
|---|---|---|
| Rust | rustc/cargo 1.97.1 | `%USERPROFILE%\.cargo\bin` |
| Node | v25.8.0 | im PATH |
| PowerShell 7 | 7.6.4 | für `scripts/m3-verify.ps1` (nicht 5.1) |
| Visual Studio 2022 Build Tools | — | für den CMake-Generator |

Bun ist **nicht** installiert; das Frontend wird mit npm gebaut (`node_modules`
liegt im pnpm-Layout vor und ist vollständig).

## Abkürzung: `scripts/dev.ps1`

Die beiden folgenden Stolpersteine sind in einem Wrapper gekapselt. Er setzt
den PATH selbst, prüft jeden Exit-Code einzeln (kein grünes Ergebnis aus einer
Pipeline) und räumt den CMake-Cache auf beiden Seiten der Junction:

```powershell
pwsh -File apps\local-voice\scripts\dev.ps1 test          # cargo test --lib
pwsh -File apps\local-voice\scripts\dev.ps1 check         # fmt + clippy + test
pwsh -File apps\local-voice\scripts\dev.ps1 build         # tauri build --no-bundle
pwsh -File apps\local-voice\scripts\dev.ps1 clean-cmake   # Stolperstein 2
pwsh -File apps\local-voice\scripts\dev.ps1 harness       # scripts/m8-verify.ps1
pwsh -File apps\local-voice\scripts\dev.ps1 notices       # THIRD-PARTY-NOTICES + SBOM
pwsh -File apps\local-voice\scripts\test-dev-guards.ps1   # prueft die Waechter selbst
```

`test-dev-guards.ps1` belegt in wenigen Sekunden ohne Build: fehlt cargo, endet
`dev.ps1` mit Meldung `FEHLT: cargo nicht gefunden` und Exit 2; ein Schritt mit
Exit != 0 beendet das Skript mit genau diesem Exit-Code; ein fremder
CMake-Cache wird erkannt.

Die Handarbeit darunter bleibt gültig und erklärt, was der Wrapper tut.

## Stolperstein 1 — cargo ist nicht im PATH

Weder Git Bash noch PowerShell finden `cargo`. Vor jedem Rust-Befehl:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
```

Beobachtet 2026-08-17: In Git Bash meldet `cargo test` dann
`bash: cargo: command not found`, und wenn die Ausgabe durch `tail` läuft,
ist der Exit-Code trotzdem **0**. Ein grüner Exit-Code aus einer Pipeline ist
hier also kein Beleg dafür, dass Tests gelaufen sind.

## Stolperstein 2 — CMake-Generator-Konflikt

```
CMake Error: Error: generator : Visual Studio 17 2022
Does not match the generator used previously: Ninja
```

Ursache: `transcribe-cpp-sys` baut über eine kurze NTFS-Junction
`%LOCALAPPDATA%\tcs\<hash>` (MAX_PATH-Umgehung) und legt dort einen
`CMakeCache.txt` an. Ein früherer Lauf hatte diesen Cache mit dem
Ninja-Generator erzeugt.

Der `CMakeCache.txt` liegt **hinter** der Junction, also im echten
`target/<profile>/build/transcribe-cpp-sys-*`-Verzeichnis. Nur den
`tcs`-Ordner zu löschen genügt nicht — er wird neu verlinkt und der alte
Cache ist sofort wieder sichtbar. Beide Seiten müssen weg:

```powershell
Get-ChildItem "apps\local-voice\src-tauri\target\*\build" -Directory `
  -Filter "transcribe-cpp-sys-*" | Remove-Item -Recurse -Force
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\tcs"
```

Danach konfiguriert der Build neu (dauert einmalig ~7 Minuten).

**Warum der Generator ueberhaupt wechselt:** Die `cmake`-Crate, ueber die
`transcribe-cpp-sys` baut, nimmt den Generator aus der **Umgebung**
(`CMAKE_GENERATOR`, auch `CMAKE_GENERATOR_<target>` und
`TARGET_CMAKE_GENERATOR`). Ohne Variable waehlt CMake Visual Studio. Ein Lauf in
einer Shell mit `CMAKE_GENERATOR=Ninja` (Entwickler-Eingabeaufforderung, ein
anderes Werkzeug) legt einen Ninja-Cache an, der naechste Lauf ohne die Variable
scheitert. `dev.ps1` entfernt diese Variablen fuer seine Laeufe (immer derselbe
Generator wie in der CI; `LVA_KEEP_CMAKE_GENERATOR=1` behaelt sie) und raeumt
einen vorhandenen Cache mit fremdem Generator vor `test`/`build`/`bundle`/`clippy`
selbst weg. Ein direktes `cargo` in einer Shell mit gesetzter Variable ist davon
nicht geschuetzt - dafuer bleibt die Handarbeit oben.

## Lizenzen, Third-Party-Notices und SBOM

`dev.ps1 notices` (bzw. `node apps/local-voice/scripts/gen-notices.mjs`) erzeugt
aus `Cargo.lock` (cargo-about, `src-tauri/about.toml`), `pnpm licenses`, dem
Modellkatalog und `docs/m2-evidence/ATTRIBUTION.md`:

- `src-tauri/resources/THIRD-PARTY-NOTICES.md` - im Repository und (ueber
  `resources`) im Installer; nach jeder Aenderung an Abhaengigkeiten oder
  ATTRIBUTION.md neu erzeugen und mit committen. `--check` prueft, ob die Datei
  aktuell ist.
- `src-tauri/target/sbom/local-voice-ai-<version>.cdx.json` - CycloneDX 1.5,
  Build-Artefakt (nicht im Repository). Enthaelt Rust-Crates (cargo-cyclonedx),
  npm-Pakete (Entwicklungswerkzeuge mit `scope: excluded`), Modelle und Datensaetze.

Einmalig noetig (reine Entwicklerwerkzeuge, keine Laufzeitabhaengigkeit):
`cargo install cargo-about --locked --features cli` (MIT OR Apache-2.0) und
`cargo install cargo-cyclonedx --locked` (Apache-2.0); dazu `pnpm install`.

`cargo deny --no-default-features check` ist das Gate fuer Lizenzen und
Sicherheitshinweise; `about.toml` fuehrt dagegen alles auf, was im Graphen liegt,
damit das Verzeichnis nichts verschweigt.

## Stolperstein 3 — `cargo build --release` erzeugt kein lauffähiges Produkt

**Das ist der teuerste Fallstrick des Projekts.** Er sieht wie ein Erfolg aus:
`cargo build --release` endet mit Exit 0, die EXE entsteht, sie startet, das
Tray-Symbol erscheint — und die Anwendung ist trotzdem funktionsunfähig.

Beobachtet 2026-08-17: Das Fenster zeigte nur
`localhost – Netzwerkfehler`. Die Webview lud `http://localhost:1420`,
also den **Entwicklungsserver**, statt des eingebetteten Frontends. Ohne
Frontend ruft niemand `initialize_shortcuts` auf (siehe `lib.rs`: Shortcuts
werden bewusst vom Frontend registriert, nicht beim Backend-Start) — der
globale Hotkey ist damit tot, und die gesamte Diktatstrecke reagiert auf
nichts.

Nachweis am Artefakt:

```powershell
$t = [System.Text.Encoding]::ASCII.GetString(
       [System.IO.File]::ReadAllBytes("src-tauri\target\release\local-voice-ai.exe"))
$t.Contains("localhost:1420")     # darf NICHT True sein
$t.Contains("index-<hash>.js")    # ein Asset aus dist\assets\ - muss True sein
```

Ursache: Das `dev`-Flag setzt `tauri-build` in `build.rs` — und dessen
Ergebnis wird von cargo gecacht. Ein Cache aus einer früheren
`tauri dev`-Sitzung überlebt beliebig viele `cargo build --release`-Läufe,
weil `build.rs` nicht neu ausgeführt wird. Hier stammte er vom 28./29.07.

**Deshalb ist die Tauri-CLI der einzige gültige Build-Weg**; sie setzt die
Umgebungsvariablen korrekt und baut das Frontend vorher mit. Bei Verdacht
zusätzlich den Cache löschen:

```powershell
Get-ChildItem "src-tauri\target\release\build" -Directory -Filter "sprechstift-*" |
  Remove-Item -Recurse -Force
```

`cargo build --release` bleibt für einen reinen **Kompilierbarkeitstest**
brauchbar. Als Beleg dafür, dass die Anwendung funktioniert, ist er wertlos.

## Ablauf

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cd apps\local-voice

# 1) Rust-Tests (Frontend nicht nötig)
cd src-tauri
cargo test --lib          # erwartet: 198 passed, 0 failed
cargo fmt --check
cd ..

# 2) Lauffähiges Release-Binary - NICHT cargo build, siehe Stolperstein 3.
#    Baut das Frontend selbst (beforeBuildCommand) und bettet es ein.
npx tauri build --no-bundle   # -> src-tauri\target\release\local-voice-ai.exe
```

`--no-bundle` überspringt den Installer; für ein Auslieferungspaket entfällt
der Schalter (siehe Issue #7).

## Selbsttest ohne Mikrofon (für Automatisierung)

Der Weg über `m3-verify.ps1` spielt Audio über die Lautsprecher ab und nimmt es
mit dem Mikrofon wieder auf. Das prüft die ganze Kette einschließlich Akustik,
ist dafür aber weder schnell noch reproduzierbar — und es belegt den Rechner.

Für Messungen gibt es deshalb den headless-Pfad. Er speist die WAV **direkt**
in die Transkription ein, ohne Mikrofon, ohne Fenster, ohne irgendwo Text
einzufügen:

```powershell
$exe = "apps\local-voice\src-tauri\target\release\local-voice-ai.exe"
$fx  = "apps\local-voice\src-tauri\tests\fixtures"

# Batch-Pfad, mit Bewertung gegen den Sollsatz
& $exe --transcribe-file "$fx\de_test_01.wav" `
       --reference "Guten Tag. Dies ist ein Test der lokalen Spracherkennung." `
       --json

# Streaming-Pfad: Audio wird in Echtzeit eingespeist und gemessen,
# wann Text tatsächlich erschienen ist
& $exe --transcribe-file "$fx\de_test_01.wav" --stream `
       --reference "Guten Tag. Dies ist ein Test der lokalen Spracherkennung." `
       --json
```

Das JSON enthält unter `score`:

| Feld | Bedeutung |
|---|---|
| `accuracy` | 1 minus Wortfehlerrate |
| `correct` / `substitutions` / `deletions` / `insertions` | Wortbilanz |
| `diff` | Wort-für-Wort-Abgleich (`same`, `missing`, `extra`, `different`) |
| `commit_times_ms` | Zeitpunkt **jeder** Textzunahme seit Start |
| `first_text_ms` | wann zum ersten Mal Text da war |
| `median_gap_ms` | typischer Abstand zwischen Aktualisierungen |

Bewertet wird mit derselben Logik wie in der Oberfläche (`src-tauri/src/selftest.rs`).
Interpunktion, Groß- und Kleinschreibung sowie ß/ae-Schreibweisen zählen **nicht**
als Fehler; Zahlwort gegen Ziffer dagegen schon — das ist ein echter Unterschied
zwischen den Modellen.

`--stream` braucht ein streaming-fähiges Modell (Nemotron); Parakeet V3 meldet
`supports_streaming: false` und liefert dann einen Fehler statt stiller Stille.

## Native Abnahme

### Sprachfixtures erzeugen

`*.wav` ist per `.gitignore` ausgeschlossen — die Fixtures liegen **nicht** im
Repository und müssen nach einem frischen Checkout erzeugt werden. Der
Generator nutzt die OneCore-Stimme Katja (die alten SAPI-Desktop-Stimmen
sprechen deutsche Umlaute falsch aus, siehe `docs/m2-evidence/VOICE-SETUP.md`):

```powershell
$gen = "apps\local-voice\scripts\bin\TtsGen.exe"
$out = "apps\local-voice\src-tauri\tests\fixtures"
& $gen Katja "$out\de_test_01.wav"   "Guten Tag, dies ist ein Test der lokalen Spracherkennung. Der Termin ist am dritten Februar um vierzehn Uhr dreissig."
& $gen Katja "$out\de_umlaute.wav"   "Ältere Menschen gehen über die Straße in den großen Städten wie Köln."
& $gen Katja "$out\de_punkt.wav"     "Kommst du morgen mit? Das wäre wirklich großartig! Ich warte, bis du da bist."
& $gen Katja "$out\de_multiline.wav" "Erste Zeile des Textes. Neuer Absatz. Zweite Zeile des Textes. Neuer Absatz. Dritte Zeile des Textes."
& $gen Katja "$out\de_short_01.wav"  "Der Termin ist am dritten Februar."
& $gen Katja "$out\de_zahlen.wav"    "Die Rechnung lautet 1.234,50 Euro bei 19 Prozent Mehrwertsteuer."
```

`de_short_01.wav` ist absichtlich kurz (rund 2,8 s) — der Dauerlauf spielt sie
100-mal ab.

### Harness starten

Die App muss dafür **laufen**; das Skript steuert sie über den echten Hotkey.

```powershell
# aus dem Repository-Wurzelverzeichnis, mit pwsh (nicht 5.1)
.\apps\local-voice\src-tauri\target\release\local-voice-ai.exe
pwsh -File apps\local-voice\scripts\m3-verify.ps1
pwsh -File apps\local-voice\scripts\m3-verify.ps1 -Scenario endurance -Runs 100
```

Das Skript liest den Hotkey aus `settings_store.json` — es setzt nicht mehr
Strg+Leertaste voraus. Ergebnisse landen unter `docs/m3-evidence/`.

## GPU per Vulkan (Release und lokaler Installer)

Das Release und der lokale Installer rechnen STT (transcribe-cpp: Whisper,
Parakeet-GGUF, Qwen3-ASR) per Vulkan auf der GPU. Das Cargo-Feature
`gpu-vulkan` bleibt aber **nicht** Standard: ein frischer Checkout ohne SDK
baut weiter mit den CPU-Backends.

- **Bauen braucht das LunarG Vulkan SDK** (Header, Import-Bibliothek, `glslc`).
  Der SDK-Installer setzt `VULKAN_SDK` maschinenweit; eine vorher geöffnete
  Shell sieht die Variable erst nach einem Neustart.
- **Zur Laufzeit** genügt der Grafiktreiber (`vulkan-1.dll`). `ggml-vulkan.dll`
  (≈ 71 MB, im Installer LZMA-gepackt ≈ 6 MB) landet über `build.rs` in
  `transcribe-libs/` und wird wie `ggml-cpu-*` neben die EXE gebündelt
  (`tauri.windows.conf.json`), keine eigene Ressource.
- **Rückfall:** Fehlt der Treiber oder ein Vulkan-Gerät, lädt transcribe-cpp
  das Modul nicht bzw. registriert kein Vulkan-Gerät; die Einstellung „Auto“
  bindet dann die CPU. Kein Absturz.
- **CI:** `release-windows.yml` installiert SDK und SPIRV-Headers und baut mit
  `--features gpu-vulkan`. Lokal: `dev.ps1 bundle` hängt das Feature an,
  sobald `VULKAN_SDK` gesetzt ist, und warnt sonst.

```powershell
# Git Bash: export VULKAN_SDK=/c/VulkanSDK/<version>; PATH="$VULKAN_SDK/Bin:$PATH"
cargo build --release --features gpu-vulkan          # nur Rust, erster Lauf ~10 min länger
pwsh -File apps\local-voice\scripts\dev.ps1 bundle   # Installer, Feature automatisch
.\local-voice-ai.exe --list-devices                   # Vulkan-Geräte mit Index
.\local-voice-ai.exe -f audio.wav --model <id> --device-index 0 --json
```

Gemessen am 29.09.2026 (RTX 4090, i9-13900K, `m8_short_de.wav` 60 s,
beste von 3 Läufen):

| Modell | Vulkan 4090 | CPU |
|---|---|---|
| Whisper large-v3-turbo Q8 | 536 ms, RTF 112 | 16,8 s, RTF 3,6 |
| Parakeet TDT 0.6B v3 Q8 (GGUF) | 429 ms, RTF 140 | 3,4 s, RTF 17,7 |

Der erste Lauf nach dem Laden kann spürbar länger dauern (Parakeet 5,4 s),
weil Vulkan dann seine Pipelines anlegt.
