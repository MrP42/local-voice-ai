# Sprachausgabe-Anbieter (TTS): Einstieg

Stand 26.10.2026 · Goal `tts-provider`. Diese Seite ist der Einstieg; Details stehen in den verlinkten Dateien.

## Überblick

Jeder Anbieter ist ein `Arc<dyn TtsProvider>` in einer `ProviderRegistry`; Fish und Piper sind dünne Hüllen um den
vorhandenen Code, `phonon`, `openai-speech` und `ollama-speech` sind Adapter. Die Wahl steht im Einstellungsfeld
`tts_engine` als `provider:<id>`; Satz-Pipeline, Cache und Player des Vorlesens bleiben unverändert. Vor **jeder** Nutzung
entscheidet `check_use` über Opt-in, Lizenz-Bestätigung (mit Fingerabdruck) und Fernzugriff; ohne Freigabe verhält sich die
App wie bisher. Freigaben setzt nur der Nutzer in der Oberfläche, nie Kommandozeile oder Agentenbrücke. Gemessen wird mit
`--tts-bench` und dem Python-Harness unter `scripts/bench/tts`.

## Anbieter

| Anbieter (`id`) | Stufe | Weg | Streaming | Freigabe nötig |
|---|---|---|---|---|
| `fish` | supported | Hülle um den lokalen Fish-Speech-Server | Sentence | nein |
| `piper` | supported | Hülle um den vorhandenen Piper-Prozess | Sentence | nein |
| `phonon` | experimental | `ptts`-Programm je Satz, offline | Sentence | Opt-in; 40m-Modelle zusätzlich Lizenz |
| `openai-speech` | experimental | Server mit `POST /audio/speech`, nur Loopback ohne Fernzugriff | Chunk | Opt-in und Lizenz (nie verifiziert) |
| `ollama-speech` | experimental | Ollama `/v1/audio/speech`, Versions- und Plattformprüfung | Chunk | Opt-in und Lizenz (CC BY-NC) |
| `mock` | benchmark-only | Testanbieter (nie registriert) | Chunk | n. a. |
| `kokoro`, `xtts-v2`, `f5-tts` | not available | nur Statuszeile, kein Adapter | n. a. | n. a. |

Stufen: `contract.rs:24`; Streaming-Werte laut adapter-contract.md §4. Lizenzen: [licenses.md](licenses.md).

## Dokumente

| Datei | Inhalt |
|---|---|
| [adapter-contract.md](adapter-contract.md) | Vertrag, Fehlerklassen, Abbruch, Cache-Tag, neuen Anbieter hinzufügen, Einbindung (§11) |
| [cli-and-automation.md](cli-and-automation.md) | CLI-Schalter, Exit-Codes, Agentenbrücke, was bewusst fehlt |
| [installation-troubleshooting.md](installation-troubleshooting.md) | Einrichtung und Fehlersuche je Anbieter, bekannte Grenzen |
| [licenses.md](licenses.md) | Lizenzübersicht, Widersprüche, Durchsetzung in der App |
| [provider-research.md](provider-research.md) | Primärquellen-Recherche |
| [implementation-plan.md](implementation-plan.md) | Zielarchitektur, Pakete, Teststrategie |
| [CHANGELOG.md](CHANGELOG.md) | Änderungen des Zweigs, Grenzen, offene Entscheidungen |
| [../../scripts/bench/tts/README.md](../../scripts/bench/tts/README.md) | Benchmark-Harness, Korpus `de-v1`, Hörtest |

## Schnellstart

* In der App: Einstellungen > Ausgabe > „Sprachausgabe-Anbieter“: aktivieren, Lizenz bestätigen, Stimme wählen, Probestimme.
* Per Kommandozeile: `local-voice-ai.exe --tts-providers` zeigt Zustand und Sperrgrund; Exit 3 heißt „in der App freigeben“.
