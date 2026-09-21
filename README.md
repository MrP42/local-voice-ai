# Local Voice AI

**Sprache zu Text und Text zu Sprache — vollständig auf dem eigenen Rechner.**
Diktieren in jedes Programm, Besprechungen mitschneiden und protokollieren,
Dokumente vorlesen lassen. Kein Cloud-Dienst, kein Konto, kein Upload: die
Modelle laufen lokal, die Aufnahmen bleiben auf der Platte.

Windows-Desktop-Anwendung (Tauri 2 · Rust · React), entwickelt von
**Ingenieurbüro Wolff / Wolff Applied AI**.

---

## Plattformen

- [`apps/local-voice/`](apps/local-voice/README.md): die Desktop-App für Windows
  (und macOS), Tauri 2 · Rust · React. Das ist das Hauptprodukt.
- [`apps/apple/`](apps/apple/README.md): native iPhone-/Apple-Watch-App (SwiftUI)
  mit lokalen Modellen, Sprachnotizen, Gesprächsmodus und Verlauf. Noch ein
  Entwicklungsbuild, kein TestFlight/App Store — siehe
  [RELEASING.md](apps/apple/RELEASING.md).
- [`packages/voice-protocol/`](packages/voice-protocol/README.md): das
  Paketformat, über das Watch, iPhone und Desktop Aufnahmen austauschen.

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

Wer selbst baut, kann der App einen **lokalen Installer-Ordner** nennen
(Einstellungen → App → **Lokaler Update-Ordner**). Liegt dort ein neuerer
`Local Voice AI_<version>_x64-setup.exe`, bietet die Fußleiste ihn an; die App
beendet sich, der Installer läuft still durch, die App startet neu.

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
| **Besprechungen** | Mikrofon **und** System-Audio (Gegenseite) mitschneiden, Live-Transkript, Import vorhandener Audio-/Video-/Untertiteldateien, Neu-Transkription mit anderem Modell, Protokoll per LLM. Die Transkription läuft immer bis zum Ende: ein Block, der nicht transkribiert werden kann, wird als Lücke mit Zeitraum markiert und lässt sich von Hand ergänzen. Zeitstempel im Transkript springen in der Aufnahme an die Stelle. |
| **Modelle** | Rund 70 Transkriptionsmodelle zum Herunterladen, nach Sprache filterbar; eigene GGUF-Dateien werden erkannt. |
| **Sprachmodelle** | Lokale LLMs (Qwen, Gemma, …) über einen eingebauten `llama-server` — für Protokolle, Zusammenfassungen und Nachbearbeitung ohne Cloud. Mit Verbrauchs-Ledger und Speicherprognose je Modell. |
| **Vorlesen** | Ganze Dokumente (TXT, MD, PDF, DOCX) vorlesen: schnell und ohne GPU mit **Piper** (Sprache je Satz automatisch erkannt), oder mit geklonter eigener Stimme über einen lokalen **Fish-Speech**-Server. Übersetzen, zusammenfassen, Text aufbereiten. Skript-Editor mit Sprecherwechseln (`<Name>`) und Vortrags-Tags (`[ruhig]`, wahlweise deutsch oder englisch), Live-Skriptprüfung mit Korrekturvorschlägen, Historie (Rückgängig/Wiederherstellen), „Überall ersetzen", Vorlesen ab Satz, Vorab-Erzeugung geänderter Sätze im Hintergrund, Auto-Tagging mit Vorlagen. |
| **Skript-Werkstatt** | Geschichten in Teilen: ein **Buch** bündelt Seiten, Figuren haben feste Stimmen, das **Gedächtnis** (Welt, Figuren, Verlauf, Stil) liegt als Markdown im Anwendungsordner und wird nach jedem erzeugten Teil fortgeschrieben. Skripte entstehen KI-gestützt aus editierbaren Vorlagen; Bücher und Seiten lassen sich mit Dateien und Stimmen als Paket exportieren und einspielen (Rechtebestätigung für Stimmen). |
| **Nachbearbeitung** | Diktate und Protokolle per LLM aufräumen — lokal (eingebaut, **Ollama**, **vLLM**) oder über einen API-Anbieter. |
| **Multi-Device-Sync** | Optional: Ende-zu-Ende verschlüsselter Abgleich zwischen Geräten über ein eigenes Portal-Konto. |
| **Systemschutz** | Jeder Kindprozess (Modell-Server, TTS) läuft mit RAM- und CPU-Deckel; ein Start wird verweigert, wenn der Speicher nicht reicht. Der Rechner friert nicht mehr ein, wenn ein Modell zu groß ist. |

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

npx tauri dev                          # Entwicklung
pwsh -File scripts/dev.ps1 bundle      # Installer unter src-tauri/target/release/bundle/
pwsh -File scripts/dev.ps1 build       # nur die .exe
pwsh -File scripts/dev.ps1 test        # Rust-Tests
```

`scripts/dev.ps1` setzt Cargo-Pfad und Umgebung richtig; Details in
[apps/local-voice/README.md](apps/local-voice/README.md). Der Build-Ordner
`src-tauri/target/` wächst mit jedem Debug-Build — bei Platzmangel
`cargo clean` im `src-tauri`-Verzeichnis.

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
node scripts/set-version.mjs 0.19.0  # package.json + Cargo.toml + tauri.conf.json
git commit -am "chore: v0.19.0"
git tag app-v0.19.0 && git push --follow-tags
```

Der Präfix ist **`app-v`**, nicht `v`: im Repo stecken 63 Tags aus dem
Handy-Subtree, deren Nummern bis v0.9.4 laufen. Ein blankes `v0.19.0` wäre dort
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
apps/local-voice/          die Desktop-App (Tauri-Projekt) — README dort erklärt den Code
apps/apple/                iPhone-/Watch-App (SwiftUI, Xcode-Projekt)
packages/voice-protocol/   Paketformat zwischen den Plattformen, mit Fixtures
docs/                      Anleitung, Entscheidungen, Roadmap, Status, Nachweise — Index in docs/README.md
tooling/                   projektfremde Werkzeuge (Skill-Entwicklung), nicht Teil der App
.github/workflows/         Release-Workflows (Windows, macOS, Apple-Buildprüfung)
AGENTS.md                  Regeln für alle, die am Repo arbeiten — Mensch wie KI-Werkzeug
```

## Mitarbeiten

Mehrere Entwickler und KI-Zugänge arbeiten parallel an diesem Repo. Die Regeln
dafür stehen in [AGENTS.md](AGENTS.md): eigener Zweig, PR nach `main`, kein
Force-Push, keine Formatierläufe über fremde Dateien. Was auf `main` bereits rot
ist (Prettier, Clippy, Übersetzungsprüfung), ist dort benannt — wer eine Änderung
einreicht, prüft die eigenen Dateien und nicht den ganzen Baum.

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
