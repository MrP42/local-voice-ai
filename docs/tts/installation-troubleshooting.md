# TTS-Anbieter: Installation und Fehlersuche

Stand 26.10.2026 · Goal `tts-provider` (Paket D1). Meldungen und Verhalten stammen aus dem Code unter
`apps/local-voice/src-tauri/src/managers/tts/providers/` (**P/**) und `managers/tts/availability.rs`; Quellen und
Abrufdaten zu den Anbietern: [provider-research.md](provider-research.md). Zahlen ohne Fundstelle sind als „laut Auftrag“
gekennzeichnet. Einbindung in Einstellungen, UI, CLI und Agentenbrücke: [adapter-contract.md](adapter-contract.md), §11, und
[cli-and-automation.md](cli-and-automation.md). **Die App lädt nie selbst Programme oder Modelle für
die Experimental-Anbieter** (phonon.rs:679-694, ollama_speech.rs:505-510).

## 1. Fish Speech (`fish`, unterstützt)

* **Installation**: Ordner mit Python-Umgebung, `tools/api_server.py` und nicht leerem `checkpoints/`
  (availability.rs:124-146). Fehlt etwas, nennt die Meldung die Dateien (availability.rs:157-170).
* **Gerät**: GPU (`Device::Cuda`, builtin.rs:121-130); die App meldet kein CPU-Fallback.
* **Health** prüft nur das Dateisystem, nicht ob der Server läuft; den Start übernimmt der Manager beim ersten
  Vorlesen (builtin.rs:150-156).
* Gewichtslizenz ungeprüft, siehe [licenses.md](licenses.md).

## 2. Piper (`piper`, unterstützt)

* **Installation**: Programm und Stimmen kommen aus dem App-Katalog (`catalog.json`, Menü „Modelle > Vorlesestimmen“,
  availability.rs:157-162); Größe und SHA je Eintrag stehen im Katalog.
* **Gerät**: CPU (`Device::Cpu`, builtin.rs:309-312). Plattformen laut Deskriptor: `windows-x64`, `macos-x64`,
  `macos-aarch64` (builtin.rs:311).
* Abtastraten: x_low/low 16 kHz, medium/high 22,05 kHz (builtin.rs:303).
* Stimmenlizenz je Stimme, siehe [licenses.md](licenses.md).

## 3. Phonon (`phonon`, experimentell, opt-in)

**Programm**: das offizielle CLI `ptts` aus `github.com/gradium-ai/xn-ptts/releases` (Version 0.4.2 vom 25.10.2026;
Windows-Archiv `ptts-0.4.2-x86_64-pc-windows-msvc.zip`, 7 766 316 Byte (GitHub-API, Release v0.4.2, abgerufen 26.10.2026), mit `SHA256SUMS` (SHA-256 `699f3452…8b8a07`, lokal nachgerechnet); Prüfsumme vor dem Entpacken
vergleichen). Pfad in `exe_path` eintragen oder ein Name im `PATH` (phonon.rs:315-334). Voraussetzung unter Windows:
Windows 10 oder neuer, CPU mit x86-64-v3 (provider-research.md, A).

**Modell-Download (immer ausdrücklich, nie still)**

| Modell | Größe | Lizenz | Zugang |
|---|---|---|---|
| `hf://gradium/phonon-31m-en` (nur Englisch), q8 | `model.q8.gguf` 64 512 352 Byte (HF-API, Commit `92741c5b`, abgerufen 26.10.2026) | Apache-2.0 | öffentlich |
| `hf://gradium/phonon-40m-de` (nur Deutsch), q8 | 74,6 MB (provider-research.md, A) | CC BY-NC 4.0, nicht kommerziell | gated |
| `hf://gradium/phonon-40m-multilingual` | nicht geprüft | CC BY-NC 4.0 | gated |

* **Gated** (40m): auf der Hugging-Face-Seite des Modells die Bedingungen annehmen (Freigabe automatisch), dann
  `HF_TOKEN` setzen (phonon.rs:521-523, README laut provider-research.md). Ohne Token endet der Lauf mit
  `LicenseBlocked` (phonon.rs:894-902).
* **Cache-Pfade**: `HF_HUB_CACHE`, sonst `<HF_HOME>/hub`, sonst `~/.cache/huggingface/hub` (phonon.rs:344-374). Die
  Einstellung `hf_home` setzt `HF_HOME` für den Kindprozess (phonon.rs:758-774).
* **Offline-Verhalten des Adapters**: Standard `allow_network = false`. Der Kindprozess bekommt `HF_HUB_OFFLINE=1`; bei
  einem `hf://`-Modell prüft der Adapter VOR dem Start, ob der Snapshot im Cache liegt, und meldet sonst `Unavailable`,
  ohne `ptts` zu starten (phonon.rs:52-67, 698-754). Einmal laden: `allow_network` aktivieren, vorlesen lassen, wieder
  ausschalten. Die Cache-Layout-Annahme (`models--OWNER--NAME/snapshots/<commit>`) ist für das Rust-Tool nicht belegt;
  sie kann nur zu „Modell fehlt“ führen, nie zu einem stillen Download (phonon.rs:60-66).
* **Stimmen offline**: ohne `allow_network` sind nur `default`, Stimmen aus `<Modellordner>/voices/` und der Pfad einer
  vorhandenen `.safetensors`-Datei erlaubt; ein Katalogname (`Zoey`) würde `ptts` von Hugging Face holen lassen und endet
  deshalb vor dem Start mit „nicht verfügbar“ und Hinweis. Die Gesundheit meldet es für die konfigurierte Stimme früh.
* **Lokaler Modellordner**: statt `hf://` ein Ordner mit `config.json`, `tokenizer.json` und Gewichten (phonon.rs:702-720).
* **Gerät**: Release-Builds laufen nur auf der CPU, `--quant q8` immer; CUDA nur im Eigenbau (`--features cuda`), unter
  Windows **nicht verifiziert** (provider-research.md, A). Der Adapter meldet nur `Device::Cpu` (phonon.rs:1049-1052).
* **Betrieb**: ein Prozess je Satz (`Sentence`), Job-Objekt-Deckel und RAM-Start-Gate (1 GiB Schätzung + Systemreserve,
  phonon.rs:98-100, 1187). Text höchstens 8000 Zeichen.

## 4. Ollama-Speech (`ollama-speech`, experimentell, opt-in)

* **Voraussetzung**: Ollama **≥ 0.40.11** (`MIN_OLLAMA_VERSION`, ollama_speech.rs:75). Die Doku nennt widersprüchlich
  0.40.8 und 0.40.9; belegt sind nur die Release-Notes 0.40.11 (provider-research.md, B).
* **Plattform**: laut Release-Notes nur **macOS** („Ollama can now generate speech on macOS“). Auf anderen Systemen ist der
  Anbieter „nicht verfügbar“ mit Grund (ollama_speech.rs:337-347). Ob es unter Windows geht, ist **offen**; mit
  `probe_speech = true` entscheidet eine Mini-Probe (nur echtes Audio zählt). Die Probe lädt das Sprachmodell in Ollama und
  kann ein geladenes LLM verdrängen; sie ist standardmäßig aus und startet **nur, wenn der Anbieter in der App freigegeben
  ist** (Opt-in, bei nichtkommerziellem oder ungeprüftem Modell die Lizenz-Bestätigung); ohne Freigabe meldet die Gesundheit
  „Probe nicht gestartet“ und Ollama bekommt keine Sprachanfrage. Sie läuft in eigener Task (Frist 10 s), ihr Ergebnis
  (auch ein Fehlschlag) wird 5 Minuten zwischengespeichert; die Gesundheit wartet höchstens 3 s und meldet sonst „Probe läuft“.
* **Modell**: Vorgabe `phonon` (31M englisch); für Deutsch `phonon:de` (CC BY-NC, 141 MB laut Bibliothek). Laden macht der
  Nutzer selbst: `ollama pull <modell>` (die App ändert nichts an Ollama).
* **Diagnosestufen** des Health (ollama_speech.rs:20-33, 385-530): 1 Dienst erreichbar (`/api/version`, 2 s) → 2 Version
  ≥ 0.40.11 → 3 Modell vorhanden (`/api/tags`, Name exakt oder `name:latest`) → 4 Plattform bzw. Probe. Jede Stufe hat
  einen eigenen Grund und Hinweis.

## 5. OpenAI-kompatible Server (`openai-speech`, experimentell, opt-in)

* Vorgabe-Basis `http://127.0.0.1:8880/v1`; `model` ist Pflicht (openai_speech.rs:76,219). Angefordert wird `pcm` oder
  `wav`, nie `speed`.
* **ptts-openai-server** (Phonon): Port 8880, offiziell nur als Docker-Image `linux/amd64`/`linux/arm64`; nativ unter
  Windows **nicht verifiziert**. Modell über `PTTS_MODEL`, Sprache über `PTTS_LANG`, `--lang` ist Pflicht.
* **Kokoro-FastAPI** (Port 8880, Apache-2.0): **kein Deutsch** (provider-research.md, C). Nativ unter Windows per
  `start-gpu.ps1`, nicht getestet.
* Server ohne Loopback brauchen `allow_remote` (§6, Zeile „PrivacyBlocked“).

## 6. Fehlersuche

| Symptom (Meldung bzw. Grund) | Ursache | Abhilfe |
|---|---|---|
| „ptts wurde nicht gefunden: …“ (phonon.rs:687) | `exe_path` falsch, Datei fehlt | CLI herunterladen, Pfad eintragen |
| „`ptts --version` antwortet nicht innerhalb von 5 s“ (phonon.rs:1090) | falsche Datei oder CPU ohne x86-64-v3 | Datei und CPU prüfen |
| ptts-Versionsabfrage scheitert (phonon.rs:1104) | Datei beschädigt | neu laden, `SHA256SUMS` prüfen |
| „Modellordner fehlt“ / „fehlt config.json“ (phonon.rs:705-713) | lokaler Ordner unvollständig | vollständigen Checkpoint laden |
| „Das Modell hf://… liegt nicht im Hugging-Face-Cache … Downloads sind aus“ (phonon.rs:744-751) | Modell nie geladen | `allow_network` einmal an, oder lokalen Ordner angeben |
| „Hugging-Face-Cache ist nicht bestimmbar“ (phonon.rs:732) | `HOME`/`HF_HOME` fehlen | `hf_home` setzen |
| „Phonon-Modell gesperrt (gated)“ (`LicenseBlocked`, phonon.rs:894-902) | 40m-Bedingungen nicht angenommen oder `HF_TOKEN` fehlt | Bedingungen auf HF annehmen, `HF_TOKEN` setzen |
| „ptts endete mit Code …“ (`Backend`, phonon.rs:909) | Programmfehler | stderr-Auszug in der Meldung lesen |
| `Timeout` nach Auftragsfrist | CPU zu langsam oder Hänger | Frist erhöhen, Threads (`threads` 1 bis 256) prüfen; der Kindprozess wird beendet |
| „Ollama ist nicht erreichbar“ (ollama_speech.rs:409) | Dienst aus, falsche Adresse | Ollama starten, `base_url` prüfen |
| „Ollama … ist für Sprachausgabe zu alt“ (ollama_speech.rs:438) | Version < 0.40.11 | Ollama aktualisieren |
| „Das Modell '…' ist in Ollama nicht vorhanden“ (ollama_speech.rs:505) | nicht gezogen | `ollama pull <modell>` |
| „Ollama erzeugt Sprache laut Upstream nur auf macOS …“ (ollama_speech.rs:341) | Plattformsperre | `probe_speech` testen oder Phonon/Server nutzen |
| Ollama 404 auf `/v1/audio/speech` (ollama_speech.rs:255-262) | Modell fehlt oder Ollama zu alt | wie oben |
| „Der Sprachserver … lehnt die Anmeldung ab (HTTP 401/403)“ (openai_speech.rs:961) | Schlüssel fehlt | `api_key_env` setzen und Umgebungsvariable füllen |
| „Läuft der Sprachserver? Basis-URL und Port prüfen“ (openai_speech.rs:977-989) | Server aus, Port falsch | Server starten, URL prüfen |
| „… kennt weder /audio/voices noch /models (HTTP 404)“ (openai_speech.rs:993) | `/v1` fehlt in der Basis-URL | Basis auf `…/v1` enden lassen |
| `PrivacyBlocked`: „liegt nicht auf diesem Rechner“ (openai_speech.rs:213) | Host nicht Loopback | `allow_remote` nur bewusst setzen: Text verlässt das Gerät |
| `BadRequest` HTTP 400/404/422 | Format, Modell oder Stimme dem Server unbekannt | Stimme/Modell prüfen; Auszug ≤ 300 Zeichen in der Meldung |
| HTTP 429/5xx (`Backend`, wiederholbar) | Server überlastet | die Registry wiederholt bis 2 Versuche, nur vor dem ersten Chunk |
| „Fish Speech ist nicht eingerichtet (… fehlt: …)“ (availability.rs:165) | Python, `api_server.py` oder `checkpoints` fehlen | Fish einrichten oder Piper wählen |
| „Zeitüberschreitung: keine Antwort nach … ms“ (registry.rs:202) | Health-Prüfung hängt | Anbieter prüfen/neu starten |
| „Ungültige Anbieterkennung“ (registry.rs:241-247) | Kennung nicht `[a-z0-9][a-z0-9_-]*` | Kennung ändern |

## 7. Bekannte Grenzen

* **B2 Piper-Abbruch**: `run_piper_blocking` läuft auf einem Blocking-Thread. Ein Abbruch beendet das Warten sofort, den
  Piper-Prozess aber erst nach seiner eigenen Frist (Auftragsfrist + 2 s, `PIPER_KILL_GRACE`, builtin.rs:57,394,
  mod.rs:97-100). Kein Ressourcenleck, da die Frist begrenzt ist; ein echtes Kill bräuchte eine Änderung an `piper.rs`
  (außerhalb des Pakets). Status laut BEFUNDE.md: offen.
* **R1 Phonon-CLI nur satzweise**: das CLI schreibt nur Dateien, daher `StreamingMode::Sentence` und ein Prozess je Satz
  (phonon.rs:3-7). Echtes Chunk-Streaming gibt es bei Phonon nur über die Server (ptts-openai-server, WebSocket) oder Ollama.
* Phonon-Tempo: der OpenAI-Server kennt nur `speed = 1.0`; ob `duration_delta` als Ersatz taugt, ist offen
  (provider-research.md, Offene Punkte).
* Latenz, RTF und VRAM der Experimental-Anbieter sind bis zum Messlauf (P2L/P6) **nicht verifiziert**; P2L steht auf
  „blockiert“ (PAKETE.md).
* Anbieter-Lizenzen: siehe [licenses.md](licenses.md), Abschnitt „Widersprüche/offen“.
