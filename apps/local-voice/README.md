# Local Voice AI — Desktop-App

Die Windows-/macOS-Anwendung des Projekts: Tauri 2, Rust-Backend, React-Frontend.
Was die App kann, wie man sie installiert und woher sie stammt, steht in der
[README des Repos](../../README.md). Diese Datei ist der Einstieg für alle, die
am Code arbeiten wollen.

## Schnellstart

```powershell
cd apps/local-voice
pnpm install

# Silero-VAD-Modell (nicht im Repo, wird zum Kompilieren gebraucht)
mkdir src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx

npx tauri dev                                  # Entwicklung mit Hot-Reload
pwsh -File scripts/dev.ps1 bundle              # Installer bauen (NSIS + MSI)
pwsh -File scripts/dev.ps1 test                # Rust-Tests
pwsh -File scripts/dev.ps1 check               # cargo check + TypeScript
```

`scripts/dev.ps1` kapselt die Toolchain-Fallen (Cargo-Pfad, Tauri-CLI statt
`cargo build`, CMake-Reste). Details und Windows-Stolpersteine:
[BUILD.md](BUILD.md) und [docs/BUILD-WINDOWS.md](../../docs/BUILD-WINDOWS.md).

> **`cargo build --release` ist kein gültiger Build-Weg.** Nur die Tauri-CLI
> setzt die Umgebung richtig; ein reiner Cargo-Build liefert eine EXE, die ihr
> Frontend von `localhost:1420` laden will.

## Aufbau

```
src/                        React-Frontend (Vite, Tailwind, zustand)
  components/settings/      Seiten und Einstellungsbausteine (history, meetings, models, tts, …)
  stores/                   Zustand je Domäne (Modelle, Sprachmodelle, TTS, …)
  bindings.ts               von tauri-specta erzeugte Befehls- und Ereignistypen
  i18n/locales/             24 Sprachen aus Upstream; de und en werden hier gepflegt
src-tauri/src/
  commands/                 Tauri-Befehle, je Domäne eine Datei
  managers/                 die Fachlogik:
    audio.rs                Aufnahme, Geräte, Pegel
    transcription.rs        Modelle laden, transkribieren, Streaming
    meetings/               Besprechungen: Aufnahme, Import, Blöcke, Protokoll
    tts/                    Vorlesen: Piper und Fish-Speech, Stimmen, Stile
    llm/                    lokale Sprachmodelle (llama-server), Katalog, Downloads
  sync/                     Multi-Device-Sync (Ende-zu-Ende verschlüsselt, Portal-Login)
  local_update.rs           Updates aus einem lokalen Installer-Ordner
  process_guard.rs          RAM-/CPU-Deckel und Start-Gate für Kindprozesse
  catalog/                  der einkompilierte Modellkatalog
scripts/                    dev.ps1, set-version.mjs, gen_catalog.py, Prüfskripte
tests/                      Playwright-Tests der Oberfläche
```

## Konventionen

- Zweig anlegen, PR öffnen, nie direkt auf `main` — Regeln für alle Beteiligten
  in [AGENTS.md](../../AGENTS.md) (Repo-Wurzel) und [AGENTS.md](AGENTS.md) (App).
- Jeder abnahmefähige Stand bekommt eine neue Patch-Version
  (`node scripts/set-version.mjs 0.x.y`), damit der Updater ihn erkennt.
- Prettier und Clippy sind auf `main` an einigen unberührten Stellen rot;
  eigene Änderungen gezielt prüfen, nicht den ganzen Baum formatieren.
- `bindings.ts` wird nach Änderungen an Befehlen oder Ereignissen neu erzeugt
  (`npx tauri dev` schreibt sie beim Start).

## Herkunft

Fork von [Handy](https://github.com/cjpais/Handy) (CJ Pais, MIT). Die
ursprüngliche Handy-README liegt unverändert unter
[README.upstream.md](README.upstream.md); was übernommen, umbenannt und
ersetzt wurde, steht in [UPSTREAM.md](UPSTREAM.md). Lizenz: [LICENSE](LICENSE).
