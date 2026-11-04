# Portable Windows-Version (ohne Installation)

Stand: 04.11.2026, ab Release 0.23.0.

## Varianten je Release

| Datei | Inhalt | Größe (ca.) |
|---|---|---|
| `Local Voice AI_<V>_x64-setup.exe` | Installer, aktualisiert sich selbst | ~150 MB |
| `LocalVoiceAI_<V>_x64-portable.zip` | Portabel, nur die App. Modelle werden bei Bedarf geladen. | ~150 MB |
| `LocalVoiceAI_<V>_x64-portable-offline.teil<N>von<M>.zip` | Portabel und vollständig offline, siehe unten. Alle Teile in **denselben** Ordner entpacken. | ~4 GB gesamt |

GitHub nimmt je Release-Datei höchstens 2 GiB an. Die Gesamtgröße eines Releases ist nicht begrenzt. Das Offline-Paket wird deshalb auf mehrere ZIPs unter 1,9 GiB verteilt. Dateien über diesem Budget teilt das Bauskript in Stücke zu 1,5 GiB. Das betrifft derzeit nur das Sprachmodell mit 2,7 GB. Die App setzt die Stücke beim ersten Start wieder zusammen und prüft dabei Größe und SHA-256 (`src-tauri/src/portable_parts.rs`). Fehlt ein Teil, bleibt alles liegen und die App startet trotzdem. Das Modell gilt dann bis zum nächsten Start als nicht installiert.

## Inhalt des Offline-Pakets

Zielgerät: Laptop ohne eigene Grafikkarte mit 16 GB RAM, Referenz ist ein Lenovo ThinkPad mit Intel Core Ultra 5 225U.

| Teil | Katalogkennung | Lizenz | Weitergabe im Paket |
|---|---|---|---|
| Spracherkennung Nemotron 3.5 ASR Streaming 0.6B (Q8_0, 751 MB) | `handy-computer/nemotron-3.5-asr-streaming-0.6b-gguf` | OpenMDW-1.1 | erlaubt (permissiv) |
| Lokale KI-Laufzeit llama.cpp, Vulkan (Intel-iGPU) und CPU | `llm-runtime-windows-x64-vulkan`, `-cpu` | MIT, Apache-2.0 WITH LLVM-exception | erlaubt |
| Sprachmodell Qwen 3.5 4B (Q4_K_M, 2,7 GB) für Zusammenfassung und Protokoll | `llm-qwen3.5-4b-q4` | Apache-2.0 | erlaubt |
| Phonon-Programm `ptts` 0.4.2 | `phonon-runtime-windows-x64` | MIT OR Apache-2.0 | erlaubt |
| Phonon 31m Englisch und alle englischen Stimmen | `phonon-31m-en`, `phonon-voice-en-*` | Apache-2.0 | erlaubt |
| Phonon 40m Deutsch | `phonon-40m-de` | **CC BY-NC 4.0** | erlaubt, nur nicht-kommerziell, siehe unten |
| Deutsche Phonon-Stimmen | `phonon-voice-de-*` | Apache-2.0 | erlaubt |
| Windows-Stimmen (SAPI/OneCore) | – | Teil von Windows | nicht im Paket, kommen vom Betriebssystem |
| Silero VAD | bereits im Programm | MIT | erlaubt |

Nicht enthalten:
- **Fish Audio.** Ausdrücklich ausgeschlossen (Patrick, 04.11.2026).
- **Piper.** Es wird nicht gebraucht. Seine espeak-ng-Komponente steht unter GPL-3.0.
- **Sprecher-Erkennung (Sortformer) und Such-Embeddings (bge-m3).** Es gibt dafür noch keinen Ladeweg über die Modellseite. Sie lassen sich bei Bedarf über `-Provision` ergänzen, sobald einer vorhanden ist.

Voreingestellt sind Nemotron als Spracherkennung und Qwen 3.5 4B als lokales Sprachmodell (`provision.rs`, `preselect`). **Phonon wird bewusst nicht eingeschaltet.** Opt-in und Lizenzhinweis bestätigt die Person selbst unter Einstellungen → Vorlesen. Das geht offline mit einem Klick.

Die Wahl von Qwen 3.5 4B: Es ist der Nachfolger des Katalog-Standards Qwen3 4B und schneidet bei Zusammenfassung und Übersetzung besser ab. Es braucht etwa 3 GB Arbeitsspeicher und passt damit neben Nemotron in 16 GB. Auf der iGPU läuft es per Vulkan, sonst auf der CPU. Größere Modelle (8B/9B) wären auf diesem Gerät deutlich langsamer und knapp im Speicher.

## Lizenzbefund Phonon (Prüfung 04.11.2026)

- **Gewichte `gradium/phonon-40m-de`: CC BY-NC 4.0.** Quellen: Hugging-Face-Karte und API (`cardData.license = cc-by-nc-4.0`). Das Repo ist `gated: auto`. Der Download verlangt nur Anmeldung und das Teilen der Kontaktdaten, es gibt keine zusätzlichen Vertragsbedingungen (`extra_gated_prompt` fehlt). Maßgeblich ist damit allein CC BY-NC 4.0.
- CC BY-NC 4.0 erlaubt das **Vervielfältigen und Weitergeben**, auch als Teil eines Pakets (Abschnitt 2(a)(1)), unter drei Bedingungen:
  1. **Namensnennung.** Urheber gradium, Lizenzname mit Link und der Hinweis, ob etwas verändert wurde, hier unverändert. Erfüllt über `THIRD-PARTY-NOTICES.md` und die Datei `voices/SOURCE.json`, die der Installer im Modellordner mit Quelle und Lizenz anlegt.
  2. **Nicht-kommerziell.** Nutzung und Weitergabe nur für Zwecke, die nicht vorrangig auf einen geschäftlichen Vorteil oder eine Vergütung zielen. Die kostenlose Weitergabe im öffentlichen GitHub-Release ist dadurch gedeckt.
  3. Keine zusätzlichen Beschränkungen für Empfänger (Abschnitt 2(a)(5)). Erfüllt: Die Lizenz wird unverändert weitergegeben.
- **Risiko:** Wird das Paket in einem kommerziellen Rahmen eingesetzt oder weitergegeben, etwa als bezahlte Leistung von Wolff.ai, deckt die Lizenz das deutsche Modell nicht. Für die Nutzung in der öffentlichen Verwaltung ist „nicht-kommerziell“ nicht abschließend geklärt. Hier vor dem Produktivbetrieb die Rechtsabteilung bzw. den Lizenzgeber fragen. In diesen Fällen das Paket ohne `phonon-40m-de` bauen (`-Provision` ohne die Kennung): Englisch und die Windows-Stimmen bleiben verfügbar.
- Programm `ptts`, das englische Modell und alle Stimmen-Dateien stehen unter MIT bzw. Apache-2.0 und sind frei weitergebbar, mit Lizenztext.

Hinweis für den Bau: Das deutsche Modell lässt sich nur mit einem Hugging-Face-Token laden, dessen Konto die Bedingungen angenommen hat. Repository-Secret `HF_TOKEN`. Fehlt es, baut der Workflow das Offline-Paket ohne deutsches Phonon-Modell und gibt eine Warnung aus.

## Datenablage und Verhalten

- Die Marke `portable` neben der EXE (Inhalt `Handy Portable Mode`) schaltet `portable.rs` ein. Alle Daten liegen dann in `Data\` neben der EXE: Einstellungen, Modelle, Verlauf, Besprechungen, Bibliothek, Logs, WebView-Cache und Geheimnisse. Ohne die Marke verhält sich dieselbe EXE wie eine Installation.
- Im Offline-Paket liegt die Voreinstellung als `Data\settings_store.seed.json`. Die App übernimmt sie nur, wenn es noch keine Einstellungen gibt. Entpacken über einen bestehenden Ordner überschreibt also nichts.
- **Updater:** Die App meldet neue Versionen wie gewohnt, installiert sie aber nicht selbst. Ein Klick öffnet die Release-Seite. Zum Aktualisieren reicht die leichte ZIP: über den Ordner entpacken, `Data\` bleibt erhalten. Ohne Internet schlägt die Prüfung still fehl.
- Voraussetzung ist die WebView2-Runtime. Auf Windows 11 und aktuellem Windows 10 ist sie vorhanden. Die ZIP kann sie nicht nachinstallieren.
- Es läuft nur eine Instanz der App gleichzeitig, auch zwischen installierter und portabler Version.

## Bauen

```powershell
# aus apps/local-voice, nach `npx tauri build ...`
pwsh scripts/build-portable-zip.ps1 -SmokeTest                       # leicht, lokaler Stand
pwsh scripts/build-portable-zip.ps1 -Release -SmokeTest              # leicht, alle Release-DLLs Pflicht
$env:HF_TOKEN = '…'; pwsh scripts/build-portable-zip.ps1 -Release -Offline -SmokeTest
```

`-Offline` startet die EXE mit `--provision <Kennungen>`. Das ist derselbe Ladeweg wie die Modellseite, mit Prüfsummen und Installationsmarken, nur ohne Oberfläche. Rechnerspezifisches wie Logs, WebView-Cache und Geräte-Probe entfernt das Skript anschließend. Danach teilt es große Dateien und verteilt alles auf die ZIPs. Der Rauchtest entpackt alle Teile in einen Ordner. Er startet `--help` als DLL-Probe und `--list-models`, um das Zusammensetzen und ein installiertes Spracherkennungsmodell zu prüfen.

Der Release-Workflow (`.github/workflows/release-windows.yml`) führt beide Varianten nach dem Installer aus und hängt ZIPs und `.sha256` an das Release.
