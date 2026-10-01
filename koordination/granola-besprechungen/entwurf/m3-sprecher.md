# M3 Sprecher – Spike-Ergebnisse und Entwurf (Paket P3)

Stand 29.09.2026, lv-architect. Gemessen auf dieser Maschine (Windows 11, i9-13900K, RTX 4090, 64 GB; parallel liefen
Cargo-Builds und die installierte App). Spike-Code, Daten und Rohergebnisse: `C:\Users\wolff\lva-spikes\m3\` (außerhalb
des Repos). Pfade: RS = `apps/local-voice/src-tauri/src`, MM = `RS/managers/meetings`, FE = `apps/local-voice/src`.

**Kernaussagen**
1. **Sortformer 4spk-v2.1 ist nicht in der App-Version von transcribe-cpp.** 0.1.3 (Cargo.toml:112) hat keine
   Sortformer-Architektur; erst **0.2.4 (25.09.2026)** bringt `arch/sortformer` samt Rust-API (`Diarize::On`,
   `RunExtension::Sortformer`, `Transcript::speaker_segments`). Die Recherche-Angabe „heute schon lauffähig" stimmt nur
   nach einem Versionssprung. Die öffentliche Rust-API ändert sich dabei kaum (u. a. entfällt `gpu_device`, genutzt in
   `transcription.rs:650ff`).
2. **Beide NVIDIA-Modelle erfüllen AK7 auf dem deutschen Testsatz sehr deutlich** (DER 0,7–0,9 % gewichtet, max. 1,6 %
   je Datei). Auf der AMI-Stichprobe liegen sie mit Nachbearbeitung bei **14,0 % (Nemotron) bzw. 14,8 % (Sortformer)**,
   also knapp unter der Grenze. Ohne Nachbearbeitung sind es ~35 %. Der Grund ist eine Konvention: Die AMI-Referenz
   zählt Wortpausen als Sprache (siehe 2.3). Der Unterschied Nemotron/Sortformer liegt innerhalb der Messunsicherheit.
3. **Klassisches Clustering (sherpa-onnx) fällt durch:** Mischkanal 18–19 %, AMI 46–81 %. Eine Schwelle, die für
   alle Dateien passt, gibt es nicht (0,5: 57 Sprecher statt 4; 0,9: 1 statt 3). Das bestätigt den Einzelbericht aus der Recherche.
4. **Nemotron-3-Diarization existiert** (23.09.2026, OpenMDW-1.1, ungated, 8 Sprecher). ONNX-Export und PyTorch
   (transformers main) laufen. Das GGUF von NVIDIA (107 MB) lädt transcribe-cpp 0.2.4 aber **nicht** (anderes Schlüsselschema
   `sortformer.*` statt `stt.sortformer.*`). Eine Rust-Anbindung heißt heute: Mel-Merkmale und den AOSC-Zustand selbst portieren.
5. **Sprecher-Embeddings trennen gut:** 3D-Speaker ERes2Net: echt ≥ 0,92, fremd ≤ 0,51 (Cosinus); WeSpeaker: ≥ 0,93 bzw.
   ≤ 0,65. Die Wiedererkennung über Besprechungen ist damit technisch machbar. Der Test über mehrere Besprechungen hinweg
   nutzte allerdings nur TTS-Stimmen und ist deshalb zu optimistisch (2.4).

## 1. Verifikation (Verfügbarkeit, Lizenz, Format)

| Kandidat | Befund (selbst geprüft) | Lizenz | Format / Größe | Rust-Weg |
|---|---|---|---|---|
| Sortformer 4spk-v2.1 | HF `handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf` (F32/F16/Q8); **nicht** im App-Katalog, nicht in transcribe-cpp 0.1.3 | NVIDIA Open Model License (HF-Tag „other"; nicht gegated) | GGUF Q8 139 MB | transcribe-cpp **0.2.4** (ggml, CPU/CUDA/Vulkan/Metal) |
| Nemotron-3-Diarization | `nvidia/Nemotron-3-Diarization` (safetensors, .nemo, eigenes GGUF q8_0 107 MB); `onnx-community/…-ONNX` (fp32 398 MB, int8 121 MB, q4 83 MB) | OpenMDW-1.1, nicht gegated | ONNX/PyTorch; bis 8 Sprecher | transcribe-cpp 0.2.4 lehnt das GGUF ab („missing KV stt.sortformer.max_speakers"); ONNX über `ort` + eigene Mel/AOSC-Logik (Referenz: transformers `nemotron3_diarization`) |
| pyannote community-1 | HF gated („auto": Konto + Zustimmung), Gewichte CC-BY-4.0 | CC-BY-4.0 + Gating | PyTorch + PLDA | `speakrs` 0.5.0 (Apache-2.0) auf `ort`; **nicht gemessen** (Gating, Python-Stack) |
| sherpa-onnx | Python 1.13.8, offizielles Rust-Crate `sherpa-onnx` 1.13.8 (11.09.2026); pyannote-segmentation-3.0 int8 1,5 MB | Apache-2.0 / MIT | ONNX | eigenes onnxruntime; Konfliktgefahr mit dem `ort`-onnxruntime von transcribe-rs [?] |
| Embeddings | WeSpeaker ResNet34-LM (26 MB), 3D-Speaker ERes2Net-base (40 MB), beide aus den sherpa-onnx-Releases | Apache-2.0 [Modellkarte nicht einzeln geprüft] | ONNX, 16 kHz, Fbank-Eingang | `ort` (schon im Baum, 2.0.0-rc.12 über transcribe-rs) + Kaldi-Fbank |

## 2. Messungen

**Testsatz.** (a) Die drei synthetischen deutschen Szenen aus `%LOCALAPPDATA%\lva-bench\synth` (SAPI-Stimmen Hedda/Stefan/Katja,
4,3/6,4/8,5 min). Je Szene gibt es die Systemspur (Gegenseite, 1/2/2 Sprecher) und die Mischung mic+system (2/3/3 Sprecher,
wie Import/Präsenz). Die Referenz stammt aus `reference.json`; die Äußerungsgrenzen sind an der Energie der Quellspur beschnitten
(`prep.py`). (b) AMI (CC-BY-4.0): EN2002b und TS3003a, jeweils die ersten 10 min des Headset-Mixes (`diarizers-community/ami`
ihm), Referenz `only_words` aus pyannote/AMI-diarization-setup (identisch mit der Datensatz-Referenz). Der Edinburgh-Spiegel
lieferte < 10 kB/s und wurde abgebrochen.
**DER** (`der.py`): 10-ms-Raster, Kragen ±0,25 s um jede Referenzgrenze (md-eval-Semantik wie im NVIDIA-Scorer), Überlappung zählt mit,
Hungarian-Zuordnung. Die Werte sind nach Referenzsprechzeit gewichtet. **Nachbearbeitung „N"** = je Sprecher Lücken ≤ 1,0 s schließen und
0,1 s anhängen (`postproc.py`).

| Datei | Sortformer Q8 roh | Sortformer + N | Nemotron roh | Nemotron + N | sherpa-onnx (WeSpeaker, thr 0,5) |
|---|---|---|---|---|---|
| Szene 1 System (1 Spr.) | 3,04 | 0,26 | 4,07 | 0,26 | 0,26 |
| Szene 1 Mix (2) | 2,19 | 1,58 | 0,54 | 1,48 | 0,38 |
| Szene 2 System (2) | 0,15 | 0,63 | 6,13 | 0,48 | 1,92 |
| Szene 2 Mix (3) | 4,55 | 0,84 | 0,35 | 0,76 | 19,33 (5 Spr.) |
| Szene 3 System (2) | 0,11 | 0,63 | 5,09 | 0,43 | 1,94 (4 Spr.) |
| Szene 3 Mix (3) | 4,34 | 1,26 | 0,30 | 0,64 | 17,78 (4 Spr.) |
| **Deutsch gewichtet** | 2,59 | **0,94** | 2,36 | **0,68** | 9,43 |
| AMI EN2002b (4, viel Überlappung) | 36,46 | 18,71 | 35,47 | 17,42 | 80,70 (57 Spr.) |
| AMI TS3003a (4) | 33,41 | 10,69 | 32,84 | 10,42 | 45,80 (24 Spr.) |
| **AMI gewichtet** | 34,98 | **14,83** | 34,20 | **14,03** | 63,81 |

Verwechslungsanteil (conf) bei Sortformer/Nemotron in allen Dateien ≤ 1,1 %; die Sprecherzahl stimmt überall.
### 2.1 Laufzeit und Speicher (je Prozess, BelowNormal, ein Modell zur Zeit, `measure.py`)

| Kandidat | Gerät | Geschwindigkeit (× Echtzeit) | 60 min Audio | Spitzen-RAM | VRAM |
|---|---|---|---|---|---|
| Sortformer Q8, transcribe-cpp 0.2.4, Preset „very high latency" (30,4 s) | CUDA 4090 | 121–133× | ≈ 29 s | 0,7 GB | ≈ +0,8 GB (Grundlast schwankt) |
| dito | CPU (ggml) | 24–29× | ≈ 2,3 min | 0,6 GB | – |
| Nemotron-3, PyTorch 2.9 CPU (transformers main), 8 Threads | CPU | 47–55× | ≈ 1,2 min | 1,6 GB (inkl. Python/Torch) | – |
| sherpa-onnx (Segmentierung int8 + WeSpeaker), 4 Threads | CPU | 6–12× | ≈ 7 min | 0,45 GB | – |
| Embedding je Stück ≤ 6 s (WeSpeaker / ERes2Net) | CPU | 47 / 54 ms | – | 0,24 / 0,33 GB | – |

Befehle (Arbeitsverzeichnis `lva-spikes\m3`): `venv\Scripts\python eval_sortformer.py cuda 1 models\sortformer-4spk-v2.1-Q8_0.gguf`
(bzw. `cpu`); `venv\Scripts\python run_nemotron.py out\nemotron_cpu data\*.wav`; `venv\Scripts\python run_sherpa.py out\sherpa_wespeaker
models\wespeaker_en_voxceleb_resnet34_LM.onnx 0.5 data\*.wav`; Wertung `python postproc.py out\<lauf> 1.0 0.1`; Embeddings
`python embed_eval.py models\<emb>.onnx`.

### 2.2 Folgerungen
- **Für den Einbau: Sortformer über transcribe-cpp 0.2.4.** Gleiche Bibliothek wie die STT, GGUF-Katalog, CPU-tauglich
  (2,3 min je Stunde), keine zweite Laufzeitumgebung. Güte gleichauf mit Nemotron. Grenze: **max. 4 Sprecher je Kanal**.
- **Nemotron später** (8 Sprecher, OpenMDW), sobald transcribe.cpp das Modell lädt. Eine eigene ONNX-Portierung (Mel +
  AOSC-Zustandsautomat nach dem transformers-Vorbild) ist möglich, rechnet sich aber erst, wenn Besprechungen mit mehr als 4 Personen
  je Kanal häufig sind (Owner-Entscheidung E1).
- **Die Nachbearbeitung ist Pflicht** und Teil des Moduls (Parameter in `DiarizeParams`). Die Parameter wurden auf denselben
  20 AMI-Minuten gewählt, auf denen auch gemessen wurde. Die Abnahme (P3a) misst deshalb auf **weiteren** AMI-Besprechungen.

### 2.3 Grenzen der Messung
- Die deutschen Szenen sind TTS-Stimmen mit sehr unterschiedlichem Klang und ohne Raum. Sie belegen die Kette (Kanal, Mischung,
  Zuordnung), sind aber keine Aussage über echte deutsche Besprechungen. Eine echte, freigegebene Aufnahme mit 3–5 Personen
  fehlt weiterhin (Recherche 8.2).
- AMI roh ~35 %: Die fehlenden Rahmen liegen im Median bei −69 bis −73 dBFS (Referenz-Stille −76), es sind also Wortpausen, die die
  `only_words`-Referenz als Sprache zählt. Mit Lücken ≤ 2 s sinkt Miss auf 0,7–10 %, dafür steigt FA. 1,0 s ist der Kompromiss.
- Zwei AMI-Besprechungen zu je 10 min sind eine kleine Stichprobe (EN2002b hat 37 % Überlappung).

### 2.4 Wiedererkennen (Embeddings, `embed_eval.py`)
Profil = Mittel über ≤ 8 überlappungsfreie Stücke ≥ 1,5 s (erste Hälfte), Probe = zweite Hälfte bzw. eine andere Datei.
18 Profile, 37 echte Paare (28 über Dateien hinweg, das sind nur TTS-Stimmen), 102 fremde Paare.

| Modell | echt Mittel / Min | fremd Mittel / Max | EER | Schwelle bei FAR 1 % |
|---|---|---|---|---|
| WeSpeaker ResNet34-LM | 0,979 / 0,933 | 0,454 / 0,645 | 0 % | 0,637 |
| 3D-Speaker ERes2Net-base | 0,975 / 0,916 | 0,266 / 0,505 | 0 % | 0,504 |

ERes2Net hat den größeren Abstand; es wird der Kandidat. Echte Stimmen über verschiedene Tage, Mikrofone und Codecs sind nicht
gemessen. Die Schwelle muss deshalb konservativ sein, und die App schlägt einen Namen **nur vor**, statt ihn stumm zu setzen.

## 3. Entwurf

### 3.1 Pipeline (Enddurchlauf, vor `TranscriptFinal`)
```
stop() -> Live-Worker leer -> [P2d] Enddurchlauf-Job (Status processing)
  1. diarize: je Kanal laut Plan (3.2) Sortformer laden (RAM-Gate process_guard) -> Turns -> Nachbearbeitung -> entladen
  2. [P2d] End-STT (words mit Wortzeiten) bzw. Live-Segmente bei final_model=off/CPU
  3. assign: Wort -> Sprecher, Segmente an Sprecherwechseln teilen (3.3)
  4. [nur Opt-in] recognize: Embedding je Sprecher -> Namensvorschlag (3.6)
  5. store: replace_segments (Epoche+1, EINE Transaktion) + speaker_hints_json + speakers-Zeilen upserten
  6. TranscriptFinal -> KI-Notizen (M1-P1f) sehen bereits Sprecherlabels
```
- Die Diarisierung läuft **vor** dem End-STT: Sie ist klein, und so liegt nie mehr als ein großes Modell gleichzeitig im Speicher.
  Auf CPU-Release-Builds verlängert sie die Zeit bis `TranscriptFinal` um ≈ 2,3 min je Stunde Audio. Mit GPU (P2f) sind es ≈ 30 s.
- **Import** (`import.rs`, Kanal 2) und **Neu-Transkription** (`retranscribe.rs`) rufen denselben Job-Schritt auf.
  Neu-Transkription nutzt vorhandene Turns weiter (3.5) und diarisiert nur, wenn keine da sind.
- Einstellung `meeting_diarization: auto | off` (Default `auto`) in der Gruppe „Besprechungen". Kein neuer Reiter, kein Menüpunkt.

### 3.2 Welcher Kanal wird diarisiert
| Aufnahmeart | Kanal 0 (mic / mic_aec) | Kanal 1 (system) | Kanal 2 (Import) |
|---|---|---|---|
| Online mit Systemton | „Ich" (Nutzer), **nicht** diarisiert | diarisiert → „Gegenseite 1..n" | – |
| … Häkchen „Mehrere Personen am Mikrofon" (hybrid) | diarisiert → „Raum 1..n" | diarisiert | – |
| Präsenz (nur Mikrofon) | diarisiert → „Person 1..n" | – | – |
| Import Audio/Video | – | – | diarisiert → „Person 1..n" |
| Import VTT/SRT | keine Audiodaten → keine Diarisierung (Sprecher aus Untertiteln sind nicht Scope) | | |

Das Häkchen ist eine Option je Besprechung in der RecorderCard (neben „Systemton"), gespeichert in `meetings.metadata_json`
(`{"diarize_mic":true}`). Eingang für Kanal 0 ist `mic_aec.wav` (P2c), falls vorhanden, sonst `mic.wav`.

### 3.3 Zuordnung Wort → Sprecher (`MM/diarize/assign.rs`, rein, ohne I/O)
- Turns je Kanal: `Turn { start_ms, end_ms, speaker: u32 }` (1-basiert in Ankunftsreihenfolge, Überlappung erlaubt).
- Wort mit Zeit (P2d `words`): Sprecher = größte Zeitüberlappung mit Turns desselben Kanals. Bei Gleichstand gewinnt der Turn,
  der das Wortzentrum enthält. Ohne Überlappung gilt der nächste Turn ≤ 500 ms entfernt, sonst der Sprecher des Vorgängerworts.
- Glättung: Ein Sprecherlauf mit weniger als 2 Wörtern **und** unter 600 ms geht an den Nachbarn (verhindert Flackern an Grenzen).
- Teilen: An jedem verbleibenden Wechsel entsteht ein neues `StoredSegment` (Text = Wörter, Zeiten aus Wortgrenzen, `speaker_index`
  gesetzt, `words` aufgeteilt). Neu transkribiert wird nicht; Parakeet-Wörter reichen.
- Ohne `words` (Whisper-Endmodell, Altdaten): Sprecher = größte Überlappung des ganzen Segments, **kein** Teilen.
- `speaker_index: None` bleibt für Kanal 0 ohne Diarisierung („Ich") und für Segmente ohne jede Überlappung.
- Quellen der KI-Notizen: Das Teilen verschiebt `segment_index`. Das läuft über dieselbe Epoche wie P2d und `remap_sources()`
  (größte Zeitüberlappung). Es gibt keinen zweiten Epochensprung.

### 3.4 Datenmodell – für P3a–P3c **keine Migration**
Alles Nötige steht im M8-Schema (`store.rs:36-47`):
- `transcripts.speaker_hints_json` (ungenutzt) enthält die Diarisierung, unabhängig vom Transkript:
  `{"v":1,"model":"sortformer-4spk-v2.1-q8","params":{"gap_ms":1000,"pad_ms":100},"channels":{"1":[[start_ms,end_ms,spk],…]}}`
- `speakers` (ungenutzt): eine Zeile je (meeting_id, channel, speaker_index), `display_name` = Name des Nutzers, `human_id`
  = Verweis auf `humans` (gefüllt ab P5d bzw. P3d), `consent_state` = `NULL | 'voiceprint_consented' | 'voiceprint_declined'`.
  Das Upsert läuft in der Store-Transaktion per `SELECT … WHERE deleted_at IS NULL`, nicht über einen Unique-Index.
- `StoredSegment.speaker_index` (schon vorhanden) wird gesetzt. Frontend und bindings brauchen keine neuen Segmentfelder.
- **Migration nur für P3d** (Fingerabdrücke), auf dem **nächsten freien Index beim Merge**: M4 hat Index 3. Mergt P5a vorher,
  bekommt P3d Index 5, sonst 4 und P5a 5 (die Tests laufen über `MIGRATIONS.len()`). Nur `CREATE`:
  ```sql
  CREATE TABLE voiceprints (id TEXT PRIMARY KEY, human_id TEXT NOT NULL, model TEXT NOT NULL, dim INTEGER NOT NULL,
    vector BLOB NOT NULL /*f32 LE, L2-normiert*/, n_samples INTEGER NOT NULL, consent_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
  CREATE INDEX idx_voiceprints_human ON voiceprints(human_id);
  ```
  Harte Löschung statt Soft-Delete (biometrische Daten). `humans` bleibt die Personentabelle aus M5-F17 (P5a ergänzt
  `is_self`, `merged_into`). Beim Zusammenführen von Personen (P5d) hängt `voiceprints.human_id` mit um.

### 3.5 Anzeige, Benennen, Stabilität
- Neu `MM/speakers.rs`: `SpeakerDirectory::load(store, meeting_id)` → `label(&StoredSegment) -> String` (Name, sonst „Gegenseite 2"
  / „Person 1" / „Ich"). Es **ersetzt `stats::label_for_channel`** an allen Aufrufstellen: `minutes.rs:94`, `notes/enhance.rs:484`,
  `chat/context.rs:250`, `search/chunking.rs:221`. `speaking_shares` rechnet je (Kanal, Sprecher), und der Protokollkopf zeigt Anteile je Person.
- **Namen überstehen Neu-Transkription:** `clear_segments` (store.rs:527) lässt `speaker_hints_json` und `speakers` unberührt.
  Die neuen Segmente bekommen ihre Sprecher aus den gespeicherten Turns, also bleiben Index und Name gleich. Wird neu diarisiert
  (anderes Modell, fehlende Turns), ordnet `remap_speakers(old_turns, new_turns) -> HashMap<u32,u32>` alte und neue Sprecher per
  Hungarian über die Zeitüberlappung zu (derselbe Code wie DER). Namen wandern mit, Überzählige bekommen neue Nummern.
- **Benennen im Transkript** (`MeetingDetail.tsx`, Segmentkopf `:608`): Ein Klick auf das Sprecherlabel öffnet ein Popover mit
  (a) Namen eingeben (Vorschläge: Kalender-Teilnehmende ab P5, `humans`), (b) „Mit … zusammenführen" (zwei Cluster sind dieselbe Person),
  (c) „Nur dieses Segment → Sprecher …". Die Änderung gilt sofort in allen Segmenten des Sprechers. Kein neuer Menüpunkt.
- Tauri-Commands (neu `RS/commands/meeting_speakers.rs`):
  `meeting_speakers_list(meeting_id) -> Vec<MeetingSpeaker>`, `meeting_speaker_rename(meeting_id, channel: u8, speaker_index: u32,
  name: Option<String>) -> MeetingSpeaker`, `meeting_speaker_merge(meeting_id, channel, from: u32, into: u32)`,
  `meeting_segment_set_speaker(meeting_id, segment_index: u32, epoch: u32, speaker_index: Option<u32>)` (Stale-Fehler bei
  falscher Epoche, Muster wie `update_segment_text`). Nach dem Zusammenführen werden Turns und Segmente umgeschrieben (eine
  Transaktion, `content_revision + 1`), die Epoche bleibt. Event: vorhandenes `Segments`/`Reset`-Muster, neu `SpeakersChanged{meeting_id}`.
- TS-Typ (specta): `MeetingSpeaker { channel: number; speaker_index: number; label: string; display_name: string | null;
  human_id: string | null; share_pct: number; suggestion: { human_id: string; name: string; score: number } | null }`.
- Export (docx/txt/md, `MeetingDetail.tsx:220/252`) und M4-Suche nutzen das Label. Der Suchindex wird bei `SpeakersChanged`
  neu gebaut (M4-Indexer-Gate „Epoche/Revision geändert").

### 3.6 Wiedererkennen (Opt-in, P3d)
- Global `meeting_voiceprints_enabled: bool` (Default **aus**) unter „Besprechungen → Datenschutz". Ist sie aus, wird **kein** Embedding
  berechnet, also nichts Biometrisches verarbeitet (Test mit einem Fake-Extraktor, der Aufrufe zählt).
- Anlernen nur per Einzelaktion im Sprecher-Popover: „Stimme von *Anna Berg* merken" mit Pflicht-Häkchen „Anna Berg hat
  eingewilligt" (Art. 9 Abs. 2 lit. a DSGVO: Einwilligung der **betroffenen Person**, nicht des Nutzers). Für das eigene
  Profil („Ich", `humans.is_self`) willigt der Nutzer selbst ein.
- Embedding: 3D-Speaker ERes2Net-base (ONNX, 40 MB, Katalog-Download auf Knopfdruck) über `ort` mit eigener Kaldi-Fbank (80 Mel, 25/10 ms,
  CMN). Profil = Mittel über ≤ 8 überlappungsfreie Turns ≥ 1,5 s (max. 6 s). Laufendes Mittel über Besprechungen (`n_samples`).
- Abgleich nach der Diarisierung: Cosinus gegen alle Profile. Liegt der beste Wert ≥ `RECOGNIZE_THRESHOLD` (Start 0,60, konservativ
  über der gemessenen Fremd-Spitze von 0,505) und mit Abstand ≥ 0,05 vor dem zweitbesten, entsteht ein **Vorschlag** im Label
  („Anna Berg?"). Ein Klick bestätigt ihn. Automatisch übernommen wird nie.
- Löschen: je Person („Stimmprofil löschen") und „Alle Stimmprofile löschen" (hart, `VACUUM` danach). Profile gehen **nie** in den
  E2EE-Sync (`RS/sync/collect.rs` nimmt nur Seiten/Einstellungen; Test sichert das ab). Beim Löschen einer Person werden ihre Profile mitgelöscht.

### 3.7 Ressourcen
- Die Reihenfolge ersetzt den Koordinator: Live-Worker → Diarisierer (0,6–0,7 GB RAM, GPU nur mit GPU-Build) → End-STT → LLM
  erst nach `TranscriptFinal`. Vor dem Laden läuft das RAM-Gate `process_guard`. Der Job läuft mit niedriger Thread-Priorität
  und begrenzten Threads (`n_threads = max(1, cores/2)`); Diktat bleibt möglich (Diarisierer 139 MB neben dem Diktatmodell).

## 4. Fehlerfälle
| Fall | Verhalten |
|---|---|
| Diarisierungsmodell nicht installiert / `off` | Schritt entfällt, Segmente behalten Kanal-Labels, `TranscriptFinal` wie bisher; einmaliger Hinweis mit Download-Knopf |
| RAM-Gate verweigert / Ladefehler / Laufzeitfehler (`Result`) | Warnung im Log, Schritt übersprungen, Besprechung wird `ready`; „Sprecher erkennen" später per Knopf im Detail |
| Absturz/Beenden während des Jobs | Recovery `processing` (P2d) startet den Job neu; Turns werden erst am Ende geschrieben (idempotent) |
| Kanal < 2 s Sprache / Loopback tot / stiller Kanal | kein Aufruf, 0 Sprecher; Kanal-Label bleibt |
| > 4 Sprecher im Kanal (Sortformer-Grenze) | Das Modell fasst Sprecher zusammen. Das UI bietet „Segment → anderer Sprecher" und „neuer Sprecher"; E1 entscheidet über Nemotron |
| Echo ohne AEC (Gegenseite im Mikro) | Kanal 0 wird standardmäßig nicht diarisiert; mit „Mehrere Personen am Mikrofon" und ohne `mic_aec.wav` Hinweis „Kopfhörer oder AEC" |
| Überlappung zweier Sprecher | Das Wort geht an den Sprecher mit größerer Überlappung; ein Segment hat immer genau einen Sprecher |
| Nutzer benennt, dann Neu-Transkription / anderes Modell | Namen bleiben (3.5), Test `retranscribe_keeps_speaker_names` |
| Profil-Treffer bei fremder Person | nur Vorschlag; „Nicht Anna" merkt die Ablehnung für diese Besprechung in `speaker_hints_json.rejected` (kein erneuter Vorschlag) |
| Altdaten ohne `speaker_hints_json` | wie heute; „Sprecher erkennen" im Detail, sofern Audio vorhanden (bei Aufbewahrung `AfterMinutes` ggf. gelöscht → Knopf aus) |

## 5. Testplan
- **Rust-Einheit** (`--lib meetings::diarize`, `meetings::speakers`): DER (perfekt = 0, vertauschte Labels = 0, alles fehlt = 100,
  Kragen, Überlappung, leere Hypothese); Nachbearbeitung (Lücke schließen, Pad, Kanten); `assign` (Überlappung, Gleichstand,
  kein Turn, Glättung, Teilen mit Wortaufteilung, ohne `words` kein Teilen); `remap_speakers` (Permutation, zusätzlicher und fehlender Sprecher);
  `SpeakerDirectory::label`; Store-Upsert/Rename/Merge in einer Transaktion; `clear_segments` lässt Turns und Namen stehen;
  Stale-Epoche bei `segment_set_speaker`; P3d: Migration mit Altzeilen (DB mit `MIGRATIONS[..n-1]`), Opt-in-Gate
  (Fake-Extraktor: 0 Aufrufe bei „aus"), Löschen hart, Sync-Ausschluss, Fbank gegen sherpa-onnx-Referenz (Cosinus ≥ 0,99 auf 3 Stücken).
- **Integration mit Fake-Diarisierer** (Turns fest vorgegeben): Enddurchlauf → Segmente mit Sprechern → `TranscriptFinal`;
  Umbenennen → Neu-Transkription → gleiche Namen.
- **DER-Werkzeug (Akzeptanz AK7):** `local-voice-ai.exe --eval-diarization <dir> [--model <id>] [--collar 0.25] --json [--out f]`
  liest Paare `<name>.wav` + `<name>.rttm` und gibt je Datei `der, miss, fa, conf, ref_speakers, hyp_speakers, rtf` aus, dazu
  `weighted_der` getrennt nach Präfix (`ami_` / sonst). Korpus: `scripts/bench/make_diar_corpus.py` (nur ASCII) nach
  `%LOCALAPPDATA%\lva-bench\diar\`: die 3 Szenen × {system, mix} + AMI (4 Besprechungen × 10 min: EN2002b, TS3003a als Entwicklungsstücke;
  **ES2004a, IS1009a als unberührter Prüfteil**). Nie ins Repo, nie in den Installer; die Namensnennung ergänzt `docs/m2-evidence/ATTRIBUTION.md`.
  Gegenprobe: `lva-spikes\m3\der.py` auf denselben RTTM-Ausgaben, Abweichung ≤ 0,1 Prozentpunkte.
- **Frontend:** Playwright mit Attrappe: Segmentkopf zeigt Namen; Umbenennen ändert alle Segmente des Sprechers; Zusammenführen;
  Vorschlag „Anna Berg?" bestätigen.

## 6. Paketschnitt

**P3a – Diarisierungs-Kern + DER-Werkzeug** · lv-coder-xhigh (Abhängigkeitssprung trifft alle STT-Modelle) · L · Abh.: –
(parallel möglich, **Merge nach P2d**, weil beide `transcription.rs` berühren)
Scope: `Cargo.toml` transcribe-cpp `0.1.3 → =0.2.4` (alle Plattform-Zeilen 112/211–225), Anpassung `RS/managers/transcription.rs`
(`gpu_device`), Katalog-Eintrag Sortformer Q8 (`handy-computer/diar_streaming_sortformer_4spk-v2.1-gguf`, eigene Kategorie „Sprechertrennung",
Lizenzhinweis), neu `MM/diarize/{mod,engine,postproc,der}.rs` (`fn diarize(pcm: &[f32], p: &DiarizeParams) -> Result<Vec<Turn>>`,
`DiarizeParams{gap_ms:1000,pad_ms:100,preset:VeryHighLatency,threads}`), `RS/cli.rs` + `RS/lib.rs` (`--eval-diarization`, Block `// M3-P3a`),
`scripts/bench/make_diar_corpus.py`.
Akzeptanz: `cargo test --manifest-path apps/local-voice/src-tauri/Cargo.toml --lib meetings::diarize` → ≥ 12 Tests grün;
`cargo test --lib` gesamt grün; `cargo test --test german_transcription -- --nocapture` unverändert (Regression 0.2.4);
M2-Satzbench Parakeet ONNX/Qwen3-1.7B auf 40 FLEURS-Sätzen: WER-Abweichung ≤ 0,3 Pp zu `bench.md`;
`target/release/local-voice-ai.exe --eval-diarization %LOCALAPPDATA%\lva-bench\diar --json` → Exit 0, `weighted_der` deutsch ≤ 5 %,
AMI-Prüfteil (ES2004a+IS1009a) ≤ 15 % (**AK7**; liegt er darüber: Bericht, nicht nachtunen, Owner entscheidet E4).

**P3b – Pipeline-Einbau + Zuordnung + Labels** · lv-coder-xhigh (Enddurchlauf, Transaktion, Epoche) · L · Abh.: P3a, **P2d**
Scope: `MM/diarize/assign.rs` (3.3), neu `MM/speakers.rs` (`SpeakerDirectory`, `remap_speakers`, Store-Upsert), `MM/store.rs`
(Funktionen für `speaker_hints_json`/`speakers`, keine Migration), `MM/final_pass.rs` (P2d) Schritt diarize/assign, `import.rs`,
`retranscribe.rs`, Ersatz von `label_for_channel` an den vier Stellen + `stats.rs`, Einstellung `meeting_diarization` (`settings.rs`), `metadata_json.diarize_mic`.
Akzeptanz: `cargo test … --lib meetings::` grün inkl. `speakers::tests::retranscribe_keeps_speaker_names`,
`diarize::assign::tests::split_on_speaker_change_with_words`, `…::no_words_no_split`; `local-voice-ai.exe --simulate-meeting
--mic %LOCALAPPDATA%\lva-bench\synth\scene3_sprint\mic.wav --system …\system.wav --json` → Segmente von Kanal 1 tragen genau 2
`speaker_index`-Werte, Zuordnungsfehler auf Wortebene gegen `reference.json` ≤ 3 %; `--import-meeting …\scene2_mix.wav` → 3 Sprecher.

**P3c – Benennen-UI + Commands** · lv-coder · M · Abh.: P3b
Scope: neu `RS/commands/meeting_speakers.rs` (4 Commands, Event `SpeakersChanged`), `lib.rs` (Registrierung, Block `// M3-P3c`),
`bindings.ts`, `MeetingDetail.tsx` (Label → Popover), neu `FE/components/settings/meetings/SpeakerPopover.tsx`, `RecorderCard.tsx`
(Häkchen „Mehrere Personen am Mikrofon"), Einstellungszeile `meeting_diarization` unter „Besprechungen", i18n de/en `meetings.speakers.*`.
Akzeptanz: `cargo test … --lib commands::meeting_speakers` grün; `npx tsc --noEmit` und `pnpm lint` für die berührten Dateien ohne neue Fehler;
`pnpm test:playwright -g speakers` → ≥ 4 Tests grün (Umbenennen, alle Segmente, Zusammenführen, Segment umhängen); Screenshot
`abnahme/p3c-sprecher.png` mit benannten Sprechern.

**P3d – Wiedererkennen (Opt-in)** · lv-coder-xhigh (Migration, biometrische Daten) · M–L · Abh.: P3c, **P5a gemergt** (Index, `humans.is_self`)
Scope: Migration nächster freier Index (3.4), neu `MM/voiceprint/{mod,fbank,embed}.rs` (`ort` direkt, Version wie transcribe-rs),
Katalog-Eintrag ERes2Net-base, Einstellungen `meeting_voiceprints_enabled`, Popover-Aktionen „Stimme merken"/„Vorschlag bestätigen"/„Löschen",
Datenschutz-Zeile „Alle Stimmprofile löschen", Sync-Ausschluss-Test.
Akzeptanz: `cargo test … --lib meetings::voiceprint` ≥ 10 Tests grün inkl. `store::tests::migration_keeps_rows_voiceprints`,
`voiceprint::tests::disabled_computes_no_embedding`, `…::fbank_matches_reference`; `local-voice-ai.exe --eval-voiceprints
%LOCALAPPDATA%\lva-bench\diar --json` → `eer_pct` ≤ 5 %, `false_suggestions` = 0 bei der Schwelle 0,60 (AMI-Sprecher gegen fremde Profile).

**Reihenfolge:** P3a ∥ (P2d) → P3b → P3c → P3d (nach P5a). Konflikte: `lib.rs`/`cli.rs` mit eigenen `// M3-P3x`-Blöcken; `bindings.ts`
über `tools/reapply_diff.py`; `MeetingDetail.tsx` nach P1c/P1d seriell; `store.rs` MIGRATIONS nur in P3d.

## 7. Owner-Entscheidungen (mit Vorschlag)
- **E1 Modell:** Sortformer 4spk (4 Sprecher je Kanal) jetzt; Nemotron-3 (8) erst, wenn transcribe.cpp es lädt; keine eigene ONNX-Portierung
  vorab. *Vorschlag: ja.* Nemotron war auf AMI 0,8 Pp besser, das liegt in der Unsicherheit.
- **E2 transcribe-cpp 0.2.4 für die ganze App** (Voraussetzung für E1, bringt evtl. auch Korrekturen für Qwen3-ASR). *Vorschlag: ja, mit
  WER-Regressionstor in P3a.*
- **E3 Standard:** Diarisierung nach der Besprechung standardmäßig **an** (lokal, keine Speicherung biometrischer Merkmale).
  Wiedererkennen standardmäßig **aus**, Einwilligung je Person. *Vorschlag: ja.*
- **E4 AK7 auf AMI knapp:** Liegt der Prüfteil über 15 %, dann nicht nachtunen, sondern AK7 für AMI mit Nemotron (E1) neu bewerten
  oder die Referenz-Konvention (Pausen) offen ausweisen. *Vorschlag: Bericht abwarten.*
- **E5 Lizenz:** NVIDIA Open Model License (Sortformer) und OpenMDW (Nemotron) erlauben die kommerzielle Nutzung. Die Namensnennung kommt
  in „Über". Das Modell wird per Katalog geladen, nicht gebündelt. *Vorschlag: ja; Lizenztext beim Katalogeintrag verlinken.*
- **E6 Echte deutsche Testaufnahme** (3–5 Personen, freigegeben) für die Abnahme M3: *Vorschlag: Patrick stellt eine 10-min-Aufnahme bereit;
  ohne sie bleibt der deutsche Teil von AK7 synthetisch.*

## 8. Offen / Risiken
- **R1 Versionssprung transcribe-cpp:** geändertes GGUF-Laden oder Verhalten bei bestehenden Modellen → Regressionstor P3a. Fällt es,
  bleibt nur Nemotron-ONNX (Portierungsaufwand grob 3–5 Tage [S]).
- **R2 Wortpausen/Referenz:** Die Nachbearbeitungs-Parameter sind auf den Entwicklungs-AMI-Stücken gewählt; der Prüfteil entscheidet.
- **R3 > 4 Sprecher je Kanal** (Präsenz/Import großer Runden): Sortformer fasst zusammen, die Korrektur ist manuell (E1).
- **R4 Embeddings real:** Kanal-/Codec-Wechsel (Teams-Loopback vs. Präsenzmikro) senkt die Cosinus-Werte; ob die Schwelle 0,60 hält, ist
  nur an Echtdaten prüfbar → nur Vorschläge, nie Automatik.
- **R5 `ort`/onnxruntime:** Das sherpa-onnx-Rust-Crate bringt eine eigene onnxruntime mit. Deshalb nutzt P3d `ort` direkt, in der Version, die transcribe-rs festlegt.
