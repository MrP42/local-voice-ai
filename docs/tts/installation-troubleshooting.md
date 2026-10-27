# TTS-Anbieter: Installation und Fehlersuche

Stand 27.10.2026 (Abschnitt „Eigene Stimmen“ ergänzt) · Goal `tts-provider` (Paket D1). Meldungen und Verhalten stammen aus dem Code unter
`apps/local-voice/src-tauri/src/managers/tts/providers/` (**P/**) und `managers/tts/availability.rs`; Quellen und
Abrufdaten zu den Anbietern: [provider-research.md](provider-research.md). Zahlen ohne Fundstelle sind als „laut Auftrag“
gekennzeichnet. Einbindung in Einstellungen, UI, CLI und Agentenbrücke: [adapter-contract.md](adapter-contract.md), §11, und
[cli-and-automation.md](cli-and-automation.md). **Die App lädt nie von selbst Programme oder Modelle für
die Experimental-Anbieter** (phonon.rs, ollama_speech.rs:505-510). Phonon lässt sich auf ausdrücklichen Klick über die
Modellseite installieren (Abschnitt 3); die Adapter selbst laden nichts nach.

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

**Installation über die Modellseite (empfohlen)**: Modelle > Vorlesen > „Phonon (experimentell)“ (Code:
`managers/tts/phonon_models.rs`, Katalog `catalog.json`). Wie bei Piper mit Größe, Lizenz, „Nur nicht-kommerziell“,
Fortschritt, Abbruch und Löschen:

| Eintrag | Inhalt | Größe | Lizenz |
|---|---|---|---|
| Phonon-Programm (`phonon-runtime`) | `ptts` 0.4.2 für die eigene Plattform (Windows x64, macOS Intel/Apple Silicon, Linux x64/ARM64); entpackt werden nur das Programm und `LICENSE-MIT`, `LICENSE-APACHE`, `README.md` | Windows 7 766 316 Byte, SHA-256 `699f3452…8b8a07` | MIT OR Apache-2.0 |
| `phonon-40m-de` (Deutsch, q8) | sieben Dateien, Commit `79cfc7b4`; **gesperrt**; dazu die **13 deutschen Stimmen** | 75 158 617 Byte + 13 × 256 136 Byte (zusammen 78 488 385 Byte) | CC BY-NC 4.0, nur nicht-kommerziell (Stimmen: Apache-2.0) |
| `phonon-31m-en` (Englisch, q8) | sieben Dateien, Commit `92741c5b`; öffentlich; dazu die **37 englischen Stimmen** (25 Sprecher, 12 Figurenstimmen) | 65 116 929 Byte + 37 × 256 136 Byte (zusammen 74 593 961 Byte) | Apache-2.0 |

**Stimmen gehören zum Modell.** Sie stehen auf der Modellseite nicht als eigene Zeilen: „Modell installieren“ lädt die
Stimmen der Sprache mit (je eine `.safetensors` aus `gradium/phonon-voices`, Commit `0ee17205`, je 256 136 Byte, Katalog
`purpose: phonon-voice` mit `parent` = Modell; immer ohne Token, auch beim gesperrten Modell). Die Größenanzeige des
Modells ist Modell plus Stimmen, „Löschen“ entfernt beides. Die Modellzeile nennt „13 Stimmen enthalten“ bzw. „37 …“.

* **Ablage**: `<Datenordner>/tts/phonon/<plattform>/ptts(.exe)`, `…/models/<id>/` (die sieben Dateien plus Marke
  `.lva-install.json`), `…/models/<modell>/voices/<Name>.safetensors` mit `voices/SOURCE.json`
  (`{"source", "license", "voices": [Zeilen aus voices.jsonl]}`; bei Figurenstimmen ohne Geschlecht und Alter steht
  `null`). Der Dateiname ist der Stimmenname, Leerzeichen als `-` (`Captain-Silas-Blackwater.safetensors`); die
  Stimmenkennung im Adapter ist dieser Dateiname. Ohne die Marke gilt ein Ordner nicht als über die Modellseite
  installiert.
* **Prüfung**: Größe und SHA-256 je Datei (Dateien ohne LFS, bei denen die Hugging-Face-API kein SHA-256 nennt:
  Git-Blob-Kennung). Jede Datei kommt als `.partial` (fortsetzbar), ein Modell entsteht erst in `<id>.installing/` und
  wird nach der Prüfung aller Dateien als Ganzes eingesetzt; Abbruch und Fehler lassen eine vorhandene Installation
  unberührt. Ein Modell braucht das Programm (die Oberfläche bietet es an). Beim ersten Installieren liegen die Stimmen
  im Zwischenordner `<id>.installing/voices/`, `SOURCE.json` wird dort mitgeschrieben, die Marke kommt zuletzt.
* **Stimmen nachladen (Bestand ohne Stimmen)**: Wer das Modell schon vor der Bündelung installiert hat (oder nur einzelne
  Stimmen aus 0.21.20), sieht in der Modellzeile „Stimmen fehlen (n von m)“ und den Knopf „Stimmen nachladen“. Er ruft
  dieselbe Installation auf: ist das Modell vollständig, werden **nur** die fehlenden Stimmen geladen (Datei in richtiger
  Größe = vorhanden, nicht neu geladen; fehlt nur der Eintrag in `SOURCE.json`, wird er ohne Download ergänzt). Das Modell
  und seine Marke bleiben unberührt. Einzeln installierte Stimmen aus 0.21.20 haben denselben Zielpfad und bleiben gültig.
* **Gesperrtes Modell (40m-de)**: Bedingungen auf `huggingface.co/gradium/phonon-40m-de` annehmen und anmelden
  (`hf auth login`) oder `HF_TOKEN` setzen. Die App liest das Token aus `HF_TOKEN`, sonst aus `$HF_HOME/token`,
  `$XDG_CACHE_HOME/huggingface/token` bzw. `~/.cache/huggingface/token`. Es geht **nur an `https://huggingface.co`**;
  Weiterleitungen (Hugging Face leitet große Dateien auf ein CDN um) folgt der Downloader selbst und nimmt das Token nie
  zu einem anderen Host mit. Es steht in keiner Meldung und keinem Log. Fehlt es oder lehnt der Server mit 401/403 ab,
  nennt die Meldung den Weg: „Bedingungen auf huggingface.co/gradium/phonon-40m-de annehmen und anmelden (hf auth login)“.
  Öffentliche Quellen bekommen nie ein Token.
* **Automatische Pfade**: Ist Phonon so installiert und sind `exe_path`/`model` in den Einstellungen leer, nimmt der
  Adapter die installierten Pfade (Modell: das deutsche, sonst das englische). Vorrang: Argument > Umgebung >
  Einstellungen > **installiert** > Vorgabe; `ConfigSource::Installed` meldet die Herkunft. Die Lizenz-Zustimmung hängt
  weiter am Fingerabdruck (Modell): wer vorher ein `hf://`-Modell bestätigt hatte, bestätigt das installierte Modell neu.
  Die Lizenz installierter Katalog-Modelle kommt aus dem Katalog (verifiziert, Quelle und Datum 27.10.2026), aber nur für
  Ordner mit passender Marke; andere Ordner bleiben „unbekannt“ und vorsorglich nichtkommerziell.
* Ist nichts installiert, nennt die Gesundheit „Phonon ist nicht installiert“ und als Hinweis die Modellseite.
* Die Stimmen sind laut Karte für die Modellgrößen 31m, 40m, 56m und 98m gedacht; mit dem englischen Modell geprüft
  (Stimme Everly: anderes Audio als die Standardstimme, echter Download am 27.10.2026).
* **Welche Stimmen im Vorlesen erscheinen**: Einstellungen > Ausgabe > Sprachausgabe-Anbieter > Phonon, Abschnitt „Stimme“,
  „Im Vorlesen anzeigen“: eine Liste aller Stimmen des Anbieters (Name, weiblich/männlich, Kurzbeschreibung) mit Haken,
  „Alle“ und „Keine“. Gespeichert in `tts_providers.visible_voices` (je Anbieterkennung eine Liste von Stimmenkennungen;
  fehlt der Eintrag oder ist er leer, sind alle sichtbar; „Keine“ steht als `["default"]`). Das Vorlesen-Dropdown
  (Vorlesen-Seite und Wiedergabeleiste der Aufnahmen) bietet die sichtbaren Stimmen freigegebener Anbieter als eigene
  Einträge „Phonon (ptts): Nele · weiblich“ an (Wert `provider:<anbieter>:<stimme>`, Wahl wie `tts_provider_select`).
  Die Standardstimme des Anbieters und die gerade gewählte Stimme bleiben immer sichtbar (der Haken der gewählten Stimme
  ist gesetzt und gesperrt). Das gilt für jeden Anbieter mit Stimmenliste außer Fish und Piper, die ihre eigenen Einträge
  im Dropdown haben; Anbieter ohne Freigabe oder mit Sperre erscheinen dort nicht.
* **Mehrere Sprecher beim Vorlesen** (wie bei Fish): Im Vorlesen-Dropdown steht „Phonon · Skript mit Sprechern“; dort lesen die
  Marker im Text `<Nele>`, `<Anton>` … die Zeilen des jeweiligen Sprechers, Text ohne Marker liest die Standardstimme des Modells.
  Sprecher sind die sichtbaren Stimmen aus „Im Vorlesen anzeigen“ (Name = Teil vor „ – “; Rechtsklick im Text > „Sprecher
  einfügen“ zeigt sie mit weiblich/männlich, die Skript-Prüfung kennt sie, „Sprecher tauschen“ bietet sie an). Ein unbekannter
  Marker (`<Bob>`) wird nie vorgelesen, sondern gestrichen und liest die Standardstimme. Wählt man im Dropdown eine einzelne Stimme
  („Phonon · Nele“), liest sie den ganzen Text und die Marker werden übergangen. Jede Stimme wird für sich zwischengespeichert,
  ein Wechsel mitten im Vorlesen gilt ab dem nächsten Satz. Stil-Tags `[…]` setzt Phonon nicht um: sie werden vor der Synthese
  entfernt (nie vorgelesen), die Palette zeigt nur Pausen. Dasselbe gilt für jeden Anbieter mit Stimmenliste und Stimmwechsel
  (`caps.voice_switching`), z. B. einen OpenAI-kompatiblen Server mit `/audio/voices`.

### Eigene Stimmen (importieren, mit Gradium erstellen)

Einstellungen > Ausgabe > Sprachausgabe-Anbieter > Phonon > „Eigene Stimmen“ (Code: `managers/tts/phonon_voices.rs`,
`phonon_clip.rs`, `phonon_clone.rs`, `gradium.rs`; Befehle in `commands/phonon_voices.rs`). Eigene Stimmen liegen danach
lokal im Phonon-Modell und werden **ohne Netz** benutzt; nur „Stimme erstellen“ geht ins Netz, und nur auf Klick.

* **Warum nicht lokal klonen**: die veröffentlichten Phonon-Modelle haben keinen Sprecher-Encoder. Eine Stimme ist eine
  fertige `.safetensors`-Datei (Tensor `speaker_wavs`, F32 [1, 512, 125], 256 136 Byte); eine neue erzeugt nur der Dienst
  von Gradium. Den Weg beschreibt `scripts/clone-voice.py` in `github.com/gradium-ai/xn-ptts`; die App baut ihn in Rust
  nach (ohne Python).
* **Importieren** („Eigene Stimme importieren …“): `.safetensors` wählen, Namen vergeben. Geprüft wird nur der Aufbau (Kopf
  lesbar, jeder Tensor mit gültigen Offsets und passender Größe, Daten lückenlos und ohne Anhang, `speaker_wavs` in der Form
  [1, 512, T], Größe 16 KiB bis 4 MiB); die Datei wird nie ausgeführt und nicht ganz gelesen, wenn sie schon zu groß ist.
* **Ablage**: erst im eigenen Speicher `<Datenordner>/tts/phonon/eigene-stimmen/` (`<Name>.safetensors`, `stimmen.json`),
  von dort als Kopie nach `…/models/<modell>/voices/<Name>.safetensors` **jedes** installierten Phonon-Modells, mit einer Zeile in
  `voices/SOURCE.json`: `origin` (`import` oder `gradium-clone`), `license: "eigene/unbekannt"`, `verified: false`, bei
  erstellten Stimmen `voice_id` (Gradium), `language`, `consent_at` (Zeitpunkt der bestätigten Einwilligung) und `source_note`
  (Audiodatei, Mikrofonaufnahme oder „Fish-Stimme: <Kennung>“; nie ein Pfad). Der eigene Speicher ist die Quelle der
  Wahrheit: „Modell löschen“ auf der Modellseite entfernt den ganzen Modellordner samt `voices/`, **nicht** den eigenen
  Speicher. Beim nächsten Öffnen der Liste „Eigene Stimmen“ (oder beim Import) gleicht die App ab und kopiert fehlende
  Dateien und Zeilen in jedes installierte Modell; Zeilen, deren Zusatzfelder die Modellseite beim Umschreiben von
  `SOURCE.json` verliert, werden wiederhergestellt. Eine fremde Datei gleichen Namens im Modell wird nie überschrieben
  (nur im Protokoll vermerkt). Ohne installiertes Modell bleibt die Stimme im eigenen Speicher und steht bereit, sobald ein
  Modell installiert ist. Von Hand eingetragene Modellordner (`model` in den Einstellungen) bekommen keine Kopie: dort die
  Datei selbst nach `voices/` legen.
* **Name**: frei (Buchstaben, Ziffern, Leerzeichen, `- _ . ' ( )`, höchstens 40 Zeichen); der Dateiname ist daraus gefaltet
  (`Müller Größe` → `Mueller-Groesse.safetensors`) und zugleich die Stimmenkennung im Adapter. Pfadzeichen, Steuerzeichen,
  Windows-Gerätenamen (`CON`, `NUL`, …) und `default` sind abgelehnt. Ein vorhandener eigener Name wird nur nach Rückfrage
  ersetzt; Namen **mitgelieferter** Stimmen sind tabu (auch mit Zustimmung). Höchstens 200 eigene Stimmen.
* **Entfernen**: „Entfernen“ löscht Datei und Zeile in jedem Modell und im eigenen Speicher; mitgelieferte Stimmen lassen
  sich hier nie entfernen. Eine bei Gradium gespeicherte Kopie bleibt in Ihrem Gradium-Konto und ist dort zu löschen.
* **Erstellen über Gradium** („Stimme erstellen …“): Quelle ist eine Audiodatei (wav, mp3, m4a, aac, flac, ogg, opus, wma; mp3
  und Co. brauchen ffmpeg, WAV nicht), eine Mikrofonaufnahme oder die Referenzaufnahme einer vorhandenen Fish-Stimme (Liste
  „Fish-Stimmen nach Phonon übernehmen“). Dazu Startsekunde (ab dort werden etwa zehn Sekunden genutzt), Sprache (`en`, `fr`,
  `de`, `es`, `pt`) und Name. Ablauf wie `clone-voice.py`: `POST https://api.gradium.ai/api/voices/` (multipart `name`,
  `language`, `start_s`, `audio_file`; Kopf `x-api-key`) → `uid`; `GET …/voices/embeddings/<uid>/default` bis `found` (bis zu
  60 Mal im Abstand von 2 s, je Anfrage 60 s Frist) → `GET …/blob?as_safetensors=1` → die Stimmdatei, die geprüft und wie
  beim Import abgelegt wird. Fehlerbilder und Meldungen: Abschnitt 6.
  * **Was den Rechner verlässt**: genau das ausgeschnittene Stück (≈ 10 s, Mono-WAV mit der Abtastrate der Quelle, die
    Startsekunde ist danach 0), der Name der Stimme und die Sprache — nicht die ganze Datei, nicht ihr Name, nicht ihr Pfad.
    Gradium legt die Stimme in Ihrem Konto ab; die App kann sie dort nicht löschen.
  * **Hinweis und Einwilligung (Pflicht)**: der Dialog zeigt immer „Die Aufnahme (ca. 10 s) wird an Gradium (api.gradium.ai)
    gesendet und dort in Ihrem Konto gespeichert.“ und verlangt das Häkchen „Die sprechende Person hat eingewilligt.“ — je
    Stimme neu (jede neue Quelle, jede andere Fish-Stimme, jede neue Aufnahme setzt es zurück). Ohne Häkchen ist der Knopf
    gesperrt, und das Backend lehnt den Befehl ohne `consent` ab, **bevor** es den Schlüssel liest, eine Datei öffnet oder
    eine Verbindung aufbaut. Nichts geht beim Start, in einer Gesundheitsprüfung oder von allein ins Netz.
  * **API-Schlüssel** („Gradium-Zugang“): ein Konto bei Gradium und ein Schlüssel sind nötig. Er wird mit DPAPI verschlüsselt
    im Geheimnisspeicher abgelegt (`<Datenordner>/secrets/int-tts-gradium-apikey.bin`), **nie** in `settings_store.json`; die
    Oberfläche bekommt ihn nie zurück, nur „gespeichert / nicht gespeichert“. Beim Speichern geht nichts ins Netz. Er geht nur
    an den Ursprung `https://api.gradium.ai`: Weiterleitungen folgt die App selbst, ein anderer Host bekommt den Schlüssel nie,
    ein `POST` wird nie weitergeleitet. Er steht in keiner Meldung, keinem Log und keinem `Debug` (Fremdtext aus Antworten wird
    um ihn bereinigt). Ohne DPAPI (nicht Windows) wird er nicht gespeichert; es gibt keinen Klartext-Ersatz.
  * **Nur über die Oberfläche**: `phonon_voice_clone` und die Schlüsselbefehle sind weder in der CLI noch in der Agentenbrücke
    noch im MCP-Server erreichbar (ein Test prüft die Verwendungsstellen). Ein Agent kann also nicht für den Nutzer einwilligen.
  * **Ein Auftrag zugleich**; „Abbrechen“ beendet laufende Anfragen binnen etwa 50 ms, es wird nichts gespeichert. Hat Gradium
    die Stimme schon angelegt, bleibt sie in Ihrem Konto (Meldung nennt die Kennung); die App hat noch keinen Weg, eine
    vorhandene Kennung nachträglich zu laden.
  * Mikrofon: dieselbe Aufnahmefunktion wie die Fish-Referenzaufnahme (`AudioRecordingManager`, VAD aus), höchstens 30 s, mindestens 3 s;
    Stille und zu kurze Stücke scheitern vor dem Senden.

**Von Hand (Alternative)**: die folgenden Angaben gelten für ein selbst geladenes Programm und Modell.

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
| „Phonon ist nicht installiert: …“ (phonon.rs, `validate`) | `exe_path` leer und nichts über die Modellseite installiert | Modelle > Vorlesen > Phonon: Programm und Modell installieren |
| „… ist gesperrt … Bedingungen auf huggingface.co/… annehmen und anmelden (hf auth login)“ (phonon_models.rs) | Modellseite: kein Token gefunden oder HTTP 401/403 | Bedingungen annehmen, `hf auth login` oder `HF_TOKEN` setzen, erneut installieren |
| „Das Archiv enthält einen unzulässigen Pfad“ / „… Verknüpfung statt einer Datei“ (phonon_models.rs) | Programm-Archiv mit `..`, absolutem Pfad oder Symlink (kommt trotz SHA-256 nur bei geänderter Quelle vor) | nichts wird installiert; Quelle prüfen, Meldung melden |
| „… ist beschädigt (Prüfsumme weicht ab)“ / „file is corrupt“ | Datei stimmt nicht mit Katalog überein | die Datei wurde gelöscht; erneut installieren (bei einer Stimme des Bündels bleibt das Modell, wie es war, bzw. wird nicht eingesetzt) |
| „Stimmen fehlen (n von m)“ in der Modellzeile | Modell wurde vor der Bündelung installiert, Stimmen einzeln oder nie geladen, Datei gelöscht | „Stimmen nachladen“: lädt nur die fehlenden Stimmen, nicht das Modell |
| „ptts wurde nicht gefunden: …“ (phonon.rs:687) | `exe_path` falsch, Datei fehlt | Programm auf der Modellseite installieren oder Pfad eintragen |
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
| „Gradium lehnt den API-Schlüssel ab (HTTP 401)“ (gradium.rs) | Schlüssel fehlt, falsch oder widerrufen | unter „Gradium-Zugang“ prüfen oder neu eintragen |
| „Gradium hat die Aufnahme abgelehnt (HTTP 4xx): …“ (gradium.rs) | Gradium nimmt das Audio nicht an (zu kurz, zu leise, falsche Sprache) | klare Aufnahme einer Person ohne Musik, Hinweis des Servers lesen |
| „Gradium: Kontingent erschöpft oder zu viele Anfragen (HTTP 402/429)“ (gradium.rs) | Konto bei Gradium | später erneut oder Konto prüfen |
| „Der Gradium-Server meldet einen Fehler (HTTP 5xx)“ (gradium.rs) | Störung bei Gradium | später erneut |
| „Gradium hat nicht innerhalb von … s geantwortet“ / „api.gradium.ai ist nicht erreichbar“ (gradium.rs) | Netz, Proxy, Firewall, Gradium | Verbindung prüfen; lokal wurde nichts verändert |
| „Die Stimme „…“ ist bei Gradium nach … s noch nicht fertig“ (gradium.rs) | Gradium rechnet noch (mehr als 60 Abfragen) | die Stimme liegt in Ihrem Gradium-Konto; später neu erstellen |
| „Gradium hat keine gültige Phonon-Stimmdatei geliefert“ (phonon_clone.rs) | Antwort entspricht nicht dem erwarteten Format | wird nicht gespeichert; Kennung der Stimme im Gradium-Konto in der Meldung |
| „Ohne die Einwilligung der sprechenden Person wird nichts gesendet“ (phonon_clone.rs) | Häkchen fehlt | Häkchen setzen, wenn die Person eingewilligt hat |
| „Es läuft schon eine Stimmenerstellung“ (phonon_clone.rs) | zweiter Auftrag gleichzeitig | warten oder abbrechen |
| „Es ist noch kein Gradium-API-Schlüssel eingetragen“ (phonon_clone.rs) | kein Schlüssel gespeichert | unter „Gradium-Zugang“ eintragen |
| „Für diese Audiodatei wird ffmpeg gebraucht“ (phonon_clone.rs) | mp3/m4a/flac … ohne ffmpeg | `winget install ffmpeg` oder die Datei als WAV wählen |
| „Im gewählten Ausschnitt ist keine Sprache zu hören“ / „Ab der Startsekunde bleiben nur … s“ (phonon_clip.rs) | Stille, zu leise oder zu kurz | Startsekunde ändern oder neu aufnehmen |
| „Das ist keine safetensors-Datei …“ / „Der Tensor „speaker_wavs“ fehlt …“ (phonon_voices.rs) | Import einer Datei, die keine Phonon-Stimme ist | eine Stimmdatei von Gradium (`phonon-voices`) wählen |
| „Der Name „…“ gehört zu einer mitgelieferten Stimme des Modells“ (phonon_voices.rs) | Name wie eine Modellstimme | anderen Namen wählen |
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
