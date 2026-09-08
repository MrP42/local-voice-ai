# Local Voice AI

**Sprache zu Text und Text zu Sprache — vollständig auf dem eigenen Rechner.**
Diktieren in jedes Programm, Besprechungen mitschneiden und protokollieren,
Dokumente vorlesen lassen. Kein Cloud-Dienst, kein Konto, kein Upload: die
Modelle laufen lokal, die Aufnahmen bleiben auf der Platte.

Windows-Desktop-Anwendung (Tauri 2 · Rust · React), entwickelt von
**Ingenieurbüro Wolff / Wolff Applied AI**.

---

## Plattformen im Hauptprojekt

- `apps/local-voice/`: bestehende Windows-/macOS-Desktop-App (Tauri).
- [`apps/apple/`](apps/apple/README.md): native iPhone-/Apple-Watch-App (SwiftUI),
  inklusive lokaler Modelle, Sprachnotizen, Gesprächsmodus und Verlauf.
- [Apple-Build und Release-Vorbereitung](apps/apple/RELEASING.md): automatische
  Buildprüfung; öffentliche Verteilung über TestFlight/App Store ist noch nicht
  eingerichtet. Die bisherige Geräteinstallation ist ein Entwicklungsbuild.

## Installieren

1. Auf der [**Releases-Seite**](https://github.com/MrP42/local-voice-ai/releases/latest)
   die Datei `Local Voice AI_<version>_x64-setup.exe` herunterladen.
2. Ausführen. Windows SmartScreen meldet einen unbekannten Herausgeber (das
   Paket ist nicht kommerziell code-signiert) → *Weitere Informationen* →
   *Trotzdem ausführen*.
3. Beim ersten Start führt ein kurzer Einrichtungsassistent durch die
   Mikrofon-Berechtigung und lädt ein Transkriptionsmodell herunter.

Wer lieber ohne Installer arbeitet, nimmt die `.msi` aus demselben Release.

**Systemvoraussetzungen:** Windows 10/11 x64. Eine GPU ist nicht nötig,
beschleunigt die Transkription aber deutlich (Vulkan). Für das Vorlesen mit
eigener Stimme wird zusätzlich eine NVIDIA-GPU und eine lokale
Fish-Speech-Installation gebraucht — siehe [docs/SO-STARTEN-SIE.md](docs/SO-STARTEN-SIE.md).

## Aktualisieren

Die App prüft beim Start selbst, ob ein neueres Release vorliegt, und bietet es
im Fenster unten rechts an („Nach Updates suchen"). Der Download wird gegen einen
Signaturschlüssel geprüft und dann installiert — es ist kein manueller Download
nötig.

Automatische Prüfung abschalten: **Einstellungen → App → Updates**.

> Technisch: die App liest
> `https://github.com/MrP42/local-voice-ai/releases/latest/download/latest.json`.
> Diese Datei entsteht im Release-Workflow und trägt die Minisign-Signatur des
> Installers; ohne gültige Signatur verweigert die App die Installation.

---

## Was die App kann

| Bereich | Kurz |
|---|---|
| **Diktat** | Globales Tastenkürzel, Text landet direkt im aktiven Fenster. Streaming-Modelle schreiben schon während des Sprechens mit. |
| **Verlauf** | Die letzten Diktate mit Audio, nachträglich kopierbar. |
| **Besprechungen** | Mikrofon **und** System-Audio (Gegenseite) mitschneiden, laufendes Live-Transkript, Import vorhandener Audio-/Video-/Untertiteldateien, Protokollerzeugung per LLM, konfigurierbare Aufbewahrung der Audiodateien. |
| **Modelle** | Rund 70 Transkriptionsmodelle zum Herunterladen, nach Sprache filterbar; eigene GGUF-Dateien werden erkannt. |
| **Vorlesen** | Ganze Dokumente (TXT, MD, PDF, DOCX) vorlesen, Stimmen klonen, übersetzen, Stimmwechsler — über einen lokalen Fish-Speech-Server. |
| **Nachbearbeitung** | Diktate und Protokolle per LLM aufräumen — wahlweise über einen lokalen **Ollama-** oder **vLLM-**Server oder einen API-Anbieter. |

### Empfohlene Transkriptionsmodelle

| Modell | Wofür |
|---|---|
| **Parakeet TDT 0.6B primeLine** | Deutsch. Auf Deutsch nachtrainiert, WER 6,0 auf FLEURS-DE, 25 europäische Sprachen. Kein Streaming. |
| **Nemotron 3.5 ASR Streaming** | Mehrsprachiges Live-Diktat, 28 Sprachen. Schreibt beim Sprechen mit. |
| **Parakeet Unified EN 0.6B** | Schnelles englisches Live-Diktat. |
| **Whisper Large v3 Turbo** | Größte Sprachabdeckung (100 Sprachen), dafür langsamer. |

Besprechungen dürfen ein **eigenes** Modell benutzen (Einstellung
*Transkriptionsmodell* unter Besprechungen): Streaming-Modelle sind fürs Diktat
gebaut, Besprechungen werden in Blöcken transkribiert und profitieren von
Batch-Modellen. Ein bereits transkribiertes Meeting lässt sich jederzeit mit
einem anderen Modell **neu transkribieren**.

---

## Datenschutz

- Transkription, Sprachsynthese und Modell-Inferenz laufen ausschließlich lokal.
- Nach außen geht die App nur für: Modell-Downloads (Hugging Face), die
  Update-Prüfung (GitHub) und — **nur wenn ausdrücklich so eingestellt** — einen
  LLM-Anbieter für die Nachbearbeitung. Mit Ollama oder vLLM bleibt auch das lokal.
- Besprechungen verlangen vor jeder Aufnahme und vor jedem Import eine
  Einwilligungsbestätigung (§ 201 StGB). Der Zeitpunkt wird protokolliert.
- Aufnahmedateien werden nach einer einstellbaren Frist gelöscht; Vorgabe ist
  „bis das Protokoll erstellt ist".

Grenzen und bekannte Schwächen: [docs/KNOWN-LIMITATIONS.md](docs/KNOWN-LIMITATIONS.md).

---

## Aus dem Quelltext bauen

Voraussetzungen: Rust (stable), Node 22+, pnpm, Visual Studio 2022 Build Tools,
Vulkan SDK.

```bash
git clone git@github.com:MrP42/local-voice-ai.git
cd local-voice/apps/local-voice
pnpm install

# Silero-VAD-Modell (nicht im Repo, wird zum Kompilieren gebraucht)
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx \
     https://blob.handy.computer/silero_vad_v4.onnx

npx tauri build          # Installer unter src-tauri/target/release/bundle/
npx tauri build --no-bundle   # nur die .exe
npx tauri dev            # Entwicklung
```

> **`cargo build --release` ist kein gültiger Build-Weg.** Er endet mit Exit 0,
> erzeugt eine startende EXE — und die lädt ihr Frontend trotzdem von
> `localhost:1420`, weil das `dev`-Flag aus `build.rs` von cargo gecacht wird.
> Nur die Tauri-CLI setzt die Umgebung richtig. Hintergrund und weitere
> Windows-Fallstricke: [docs/BUILD-WINDOWS.md](docs/BUILD-WINDOWS.md).

### Tests

```bash
cd apps/local-voice/src-tauri && cargo test --lib   # Rust
cd apps/local-voice && pnpm run build               # TypeScript + Vite
```

---

## Ein Release veröffentlichen

Jede ausgelieferte Änderung bekommt eine neue Versionsnummer — sonst kann der
Updater sie weder anbieten noch als installiert erkennen.

```bash
cd apps/local-voice
node scripts/set-version.mjs 0.3.0   # package.json + Cargo.toml + tauri.conf.json
git commit -am "chore: v0.3.0"
git tag app-v0.3.0 && git push --follow-tags
```

Der Präfix ist **`app-v`**, nicht `v`: im Repo stecken 63 Tags aus dem
Handy-Subtree, deren Nummern bis v0.9.4 laufen. Ein blankes `v0.3.1` ist dort
die Veröffentlichung eines anderen Programms.

Der Tag startet [`.github/workflows/release-windows.yml`](.github/workflows/release-windows.yml):
Windows-Build, Signatur des Update-Artefakts mit dem Repository-Secret
`TAURI_SIGNING_PRIVATE_KEY`, veröffentlichtes Release samt `latest.json`. Stimmen
Tag und `tauri.conf.json` nicht überein, bricht der Workflow ab.

> Die Workflows unter `apps/local-voice/.github/` stammen aus dem Handy-Subtree
> und **laufen nie** — GitHub liest ausschließlich das Repo-Wurzelverzeichnis.

---

## Aufbau des Repos

```
apps/local-voice/          die Anwendung (Tauri-Projekt)
  src/                     React-Frontend
  src-tauri/src/           Rust-Backend
    managers/              Audio, Modelle, Transkription, Besprechungen, TTS
    catalog/catalog.json   der einkompilierte Modellkatalog
  scripts/                 gen_catalog.py, set-version.mjs, Prüfskripte
docs/                      Statusberichte, Entscheidungen, Build- und Testnachweise
tooling/                   projektfremde Werkzeuge (Skill-Entwicklung)
.github/workflows/         der aktive Release-Workflow
```

## Herkunft und Lizenz

Fork von **[Handy](https://github.com/cjpais/Handy)** (CJ Pais), Commit
`ea3c20a3`, MIT-Lizenz — die vollständige Upstream-Historie steckt über
`git subtree` in diesem Repo. Code MIT, siehe
[apps/local-voice/LICENSE](apps/local-voice/LICENSE).

Handys **Name, Logo und Markenzeichen sind ausdrücklich nicht Teil der
Open-Source-Lizenz.** Diese Anwendung führt deshalb eigenen Namen, eigenes
Erscheinungsbild und eigenen Bundle-Identifier und behauptet keinerlei
Verbindung zu oder Billigung durch CJ Pais oder das Handy-Projekt. Einzelheiten:
[apps/local-voice/UPSTREAM.md](apps/local-voice/UPSTREAM.md).

Modelle stehen unter ihren eigenen Lizenzen (im Katalog je Modell vermerkt).
