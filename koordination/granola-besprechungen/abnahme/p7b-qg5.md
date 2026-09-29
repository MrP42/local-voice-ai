# P7b – QG5 Datenschutz / F27 (und F30 Kosten)

**Gate (GOAL.md):** Besprechungspfad ohne Netzverkehr (außer Modell-Download und bewusst gewähltem externem LLM), Nachweis per Offline-Lauf.
**Ergebnis: erfüllt.** Die Kette lief vollständig: Aufnahme-Simulation → Enddurchlauf → Sprecher → KI-Notizen → Index und Vektoren → Chat → PDF-Export. Dabei waren **0 Verbindungen außerhalb von Loopback** (94 beobachtete Sockets, alle 127.0.0.1) und **0 Anfragen am Proxy**, obwohl die Positivkontrolle protokolliert wurde.
Stand 30.09.2026, Release-Build `feat/granola-p7b` mit `--features gpu-vulkan`.

## Befehl → Ergebnis

`pwsh -File koordination/granola-besprechungen/tools/p7b-qg5.ps1 -AppExe <exe> -OutDir abnahme`

Beobachtung, beides nur lesend und ohne Adminrechte:
- `p7b-netwatch.ps1` erfasst alle 400 ms jede TCP-Verbindung und jeden UDP-Endpunkt der EXE **und aller Nachfahren** (llama-server, msedgewebview2) per `Get-NetTCPConnection`/`Get-NetUDPEndpoint`.
- `p7b_proxy_log.py` ist eine Sackgasse als Proxy. `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` zeigen auf `127.0.0.1:18089`, `NO_PROXY` auf Loopback. Jede über den Proxy geleitete Anfrage wird protokolliert und mit 403 beantwortet. So fallen auch Verbindungen auf, die kürzer als ein Abfragetakt leben, und nichts verlässt den Rechner über `reqwest` oder `hf-hub`. Positivkontrolle vor dem Lauf: Eine Anfrage an `http://p7b-control.invalid/` erscheint im Protokoll.

| Schritt | Befehl | Exit | Dauer | Ergebnis |
|---|---|---|---|---|
| 1 | `--simulate-meeting --scene scene1 --scene scene2 --final-model auto --notes` (10,7 min, Mikrofon + Systemton, AEC an) | 0 | 98,6 s | Enddurchlauf Whisper large-v3 (Vulkan), Sprecher `done`, KI-Notizen ok (31,7 s, lokal `llm-gemma4-e4b-q4`), Status `ready` |
| 2 | `--reindex-meetings` | 0 | 3,8 s | 16 Chunks, Vektoren über BGE-M3 (zweiter llama-server, 3,8 s), Server am Ende gestoppt |
| 3 | `--eval-chat tests/fixtures/chat` | 0 | 26,7 s | 24 Fragen, Treffergenauigkeit 1,0 (beide llama-server) |
| 4 | `--export-meeting <id> --format pdf` | 0 | 1,1 s | `%PDF-`, 75.583 Byte (verstecktes WebView2-Fenster) |

| Beobachtung | Wert |
|---|---|
| Abfragetakte gesamt / mit laufender App | 192 / 163 |
| gesehene Prozesse im Baum | local-voice-ai.exe, llama-server.exe (Notizen, Embeddings, Chat), msedgewebview2.exe (PDF), conhost.exe, taskkill.exe (28 PIDs) |
| Sockets gesamt | 94: 27 + 27 Established 127.0.0.1↔127.0.0.1 (App ↔ llama-server), 5 Listen **127.0.0.1** (llama-server), 31 Bound + 4 SynSent (Client-Sockets vor dem Loopback-Connect) |
| Sockets mit Adresse außerhalb von Loopback | **0** |
| UDP-Endpunkte | 0 |
| Anfragen am Proxy (ohne Kontrolle) | **0** |

Rohdaten: `abnahme/p7b-qg5.json` (Zusammenfassung mit Schritten, Prozessen, Sockets), `p7b-qg5-net.csv` (jeder Socket mit erstem/letztem Takt), `p7b-qg5-net.json`, `p7b-qg5-proxy.jsonl` (nur die Kontrollzeile), `p7b-qg5-sim.json`, `p7b-qg5-reindex.json`. Logs: `%LOCALAPPDATA%\lva-bench\results\p7b\p7b-qg5-*.log`.

Der llama-server bindet nur an 127.0.0.1 (`--host 127.0.0.1`, `managers/llm/server.rs:126`, Test `:896`). Er ist also auch aus dem LAN nicht erreichbar.

## Netzfunktionen der App außerhalb des Besprechungspfads (abgegrenzt)

| Funktion | Ziel | Wann | Fundstelle |
|---|---|---|---|
| Update-Prüfung | `github.com/MrP42/local-voice-ai/releases/.../latest.json` | nur GUI, aus dem Frontend; Einstellung `update_checks_enabled` (abschaltbar) | `src-tauri/tauri.conf.json` (plugins.updater), `src/components/update-checker/UpdateChecker.tsx:127,196` |
| Modell-Downloads (STT, LLM, Embedding, Sortformer, llama-Laufzeit) | Hugging Face / GitHub-Releases | nur auf Klick „Herunterladen“; im Gate ausdrücklich ausgenommen | `managers/model/download.rs`, `managers/llm/runtime.rs`, `catalog/catalog.json` |
| Externes LLM (OpenAI, Anthropic, Groq, OpenRouter, …) für Notizen/Chat | Anbieter-API | nur bei bewusster Wahl von Anbieter **und** Modell; Standard ist `openai` **ohne Modell**, der Auto-Lauf bricht dann mit `no_model` ab, ohne Anfrage | `settings.rs:1204` (Standard-Anbieter), `:1339-1344` (Standardmodell leer), `managers/meetings/llm_call.rs:226-238` |
| Kalender (ICS-Adresse, später Microsoft Graph P5f) | vom Nutzer eingetragene Adresse | nur mit eingerichtetem Kalender; liest Termine, sendet keine Besprechungsinhalte | `managers/calendar/fetch.rs`, `service.rs` |
| Voice-Sync (Portal) | vom Nutzer angemeldetes Portal | nur nach Anmeldung; synchronisiert `projects/*` (Vorlese-Seiten) und TTS-Einstellungen, **keine Besprechungsdaten** (`grep meeting src-tauri/src/sync` = 0 Treffer) | `src-tauri/src/sync/collect.rs:75`, `sync/client.rs:122` |
| Beitritts-Link, Follow-up-Mail | Browser bzw. Mailprogramm | nur auf Klick; die App selbst sendet nichts (`mailto:`/.eml) | `managers/meetings/mail.rs` |

## F30 Kosten: 0 € laufend

**Erfüllt.** Alle Standardmodelle des Besprechungspfads laufen lokal. Kein Schritt braucht einen API-Schlüssel.

| Aufgabe | Modell (Katalog) | Lizenz | läuft |
|---|---|---|---|
| Live-STT | Parakeet TDT 0.6B v3 GGUF Q8 | CC-BY-4.0 | lokal (transcribe-cpp) |
| Enddurchlauf | Qwen3-ASR 1.7B Q5_K_M / Whisper large-v3 Q5_K_M | Apache-2.0 / Apache-2.0 | lokal, Vulkan oder CPU |
| Sprecher | Sortformer 4spk v2.1 Q8 | NVIDIA Open Model License | lokal |
| KI-Notizen, Chat, Brief | Gemma 4 E4B / Qwen3.5-9B (Anbieter `local`) | Modellkarten der Hersteller (Gemma-Lizenz / Apache-2.0) | lokal (llama-server) |
| Suche (Vektoren) | BGE-M3 Q8 | MIT | lokal (llama-server) |

Beleg aus den Läufen: Die KI-Notizen liefen mit Anbieter `local:llm-gemma4-e4b-q4`, in `p7b-qg3-*.json` auch mit `local:llm-qwen3.5-9b-q4`. Der Eintrag `post_process_api_keys.local` ist leer. Der Proxy zählte in der ganzen QG5-Kette **0** Anfragen, es gab also auch keinen kostenpflichtigen API-Aufruf. Kosten entstehen nur, wenn jemand bewusst einen externen Anbieter wählt (F27-Ausnahme).

## Grenzen

- Die Kette lief als Folge von **Headless-Aufrufen** derselben Release-EXE, nicht als Klickfolge in der GUI. Die Oberfläche selbst (WebView2 des Hauptfensters, Update-Prüfung) wurde nicht beobachtet und gehört laut Tabelle nicht zum Besprechungspfad.
- Die Socket-Abfrage alle 400 ms kann eine Verbindung übersehen, die zwischen zwei Takten auf- und abgebaut wird. Für HTTP-Clients, die die Proxy-Variablen beachten (`reqwest`, `hf-hub`), schließt der Proxy diese Lücke. Ein nativer Client, der Proxy-Variablen ignoriert, wäre nur über die Abfrage sichtbar. WebView2 (Chromium) nutzt die System-Proxy-Einstellung statt der Variablen, zeigte in den Abfragen aber keinen einzigen Socket.
- Keine Firewall-Sperre: Eine ausgehende Regel für die EXE braucht Adminrechte und ist deshalb entfallen. pktmon/netsh trace ebenso.
- `eval-chat` nutzt eine eigene Sandbox mit Fixtures (5 Besprechungen). Der Chat lief also nicht auf der simulierten Besprechung aus Schritt 1. Er nutzt aber denselben Pfad (Index, Embedding-Server, LLM).
