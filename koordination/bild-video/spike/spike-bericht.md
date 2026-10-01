# Spike M7 – Bild- und Videoinhalte (#69): Folienerkennung, OCR, Bildanalyse

Stand 01.10.2026 · Goal issues-abschluss (#70) · Messrechner: Windows 11, 32 Threads, 64 GB RAM,
RTX 4090 (24 GB; Patricks App-llama-server lief parallel) · alle Zahlen selbst gemessen, Skripte und Rohdaten
liegen neben diesem Bericht (`m7/*.py`, `m7/out/*.json`).

## Kurzfassung und Empfehlung

| Frage | Ergebnis | Empfehlung |
|---|---|---|
| (1) Folien aus Video | Abtastung mit 1 fps (160×90, grau) + **dHash 64 bit, Schwelle 4**, Stabilität 2 Abtastungen: 15/16 Wechsel, 0 Fehlalarme, 14/14 Folien, auch mit Webcam-Einblendung. **4–11 s je 10 min 1080p** (ffmpeg-Dekodierung, das Hashen dauert < 0,2 s). | dHash in Rust (kein neues Crate nötig), ffmpeg wie bisher als Kindprozess. ffmpeg-`scene` nicht als Hauptverfahren nutzen (verfehlt Aufbaustufen, im echten Video 82 Fehlalarme). |
| (2) OCR lokal | **Windows.Media.Ocr**: 47–70 ms/Bild, 0 MB, keine Lizenzfrage, aber Zahlentreue nur 47/60 (verliert Tabellenzellen, „§“→„5“, „27%“→„270/0“). **RapidOCR (PP-OCRv5 latin)**: 60/60 Zahlen, 38 MB, 1,8–2,3 s/Bild CPU. **Gemma 4 E4B als OCR**: 60/60 Zahlen, Recall 0,996, 0,7 s/Bild auf GPU. Tesseract: Dezimalkommas gehen verloren. | Stufe 1 immer: **Windows-OCR** (Dubletten- und Kameraerkennung, Suche). Stufe 2 bei Bildanalyse an + GPU: **Gemma-OCR ersetzt den Text** (Zahlen korrekt). RapidOCR erst, wenn macOS oder Rechner ohne GPU Qualitäts-OCR brauchen. |
| (3) Bildanalyse | **Gemma 4 E4B + mmproj** (bereits im Katalog, nur der Projektor fehlt: 990 MB): 4,7 GB VRAM gesamt (Projektor allein ≈ 0,8 GB), **2,0 s je Beschreibung**, 17/17 Kernfakten, 1 Ablesefehler in 14 Folien. Gemma 4 12B: kein Qualitätsgewinn, 9,1 GB VRAM, 16 GB Prozess-RAM, 1,6× langsamer. Nur CPU: 98 s je Beschreibung (unbrauchbar). | E4B-mmproj in den Katalog; Bildanalyse **optional, nur mit GPU**, Standard aus. **Kein JSON-Schema-Zwang** (Fehlerbild unten), `-b/-ub 2048` beim Start mit mmproj. |
| (4) Integration | Eigene Tabelle `meeting_slides` (Anhang, keine Transkript-Fassung), Folien zeitlich ins Protokoll-Prompt eingewoben, Suchindex-Quelle `slide`. | 8 Pakete D1–D8, Summe ≈ 1,85 MTok (Rahmen M7: 2,0 MTok). |

Gesamtzeit einer 10-Minuten-Folienaufzeichnung mit 15 Folien: Erkennung 4–11 s + Vollbilder 15 × 0,31 s
+ Windows-OCR 15 × 0,07 s ≈ **10–17 s ohne Bildanalyse**; mit Bildanalyse zusätzlich Modellstart 19–23 s +
15 × (0,7 + 2,0) s ≈ **+60 s** (GPU).

## Testmaterial

- 16 Folien 1920×1080 per PIL (`make_slides.py`), Deutsch und Englisch: Titel, Agenda, Balken-, Kreis- und
  Gantt-Diagramm, Tabelle, Flussschema, Code, Foto mit Bildunterschrift, Fließtext 30 px, dunkles Layout,
  Sonderzeichen (§ „ “ – − € ß ÄÖÜ), eine Folie in drei Aufbaustufen; Ground Truth je Folie in `slides/truth.json`.
- Video **A** (reine Bildschirmfreigabe) und **B** (dazu ein dauernd bewegtes 384×216-„Webcam“-Feld), je 600 s,
  1080p25, H.264 CRF 23, 17 Abschnitte inkl. Aufbaustufen und Rücksprung auf eine frühere Folie.
- Realtest **C**: Wikimedia Commons, „Lightening talk ‚8 FLOSS tools and 4 hardware …‘“, CC BY-SA 4.0, 6 min,
  1080×606 VP8, halbtransparente Folien über Kamerabild mit Überblendungen; Folien-PDF (14 Seiten) als Referenz.

## (1) Folienerkennung aus Video

Bewertung: Treffer = erkannter Wechsel innerhalb ±1 s (Abtastung: ±1/fps + 0,5 s) um einen der 16 echten Wechsel;
„Folien“ = alle 14 verschiedenen Folien mit einem Repräsentanten abgedeckt; Repräsentant = letztes stabiles Bild
vor dem nächsten Wechsel (bei Aufbaustufen also der vollständige Endstand).

| Verfahren | Video | Zeit je 10 min | Wechsel erkannt | Fehlalarme | Folien | Endstand Aufbau |
|---|---|---|---|---|---|---|
| ffmpeg `scene>0.05`, volle Auflösung | A / B | 9,1 / 6,2 s | 14/16 / 14/16 | 0 / 0 | – | nein (Szene = erstes Bild) |
| ffmpeg `scene>0.3` (übliche Voreinstellung) | A / B | 6,2 / 5,2 s | 8/16 / 7/16 | 0 | – | – |
| ffmpeg `fps=2,scale=480,scene>0.05` | A / B | 4,6 / 4,0 s | 14/16 | 0 | – | – |
| ffmpeg nur Keyframes (`-skip_frame nokey`) | A / B | 0,8 / 0,6 s | 14/16 / 13/16 | 0 / 1 | – | – |
| **1 fps, dHash Schwelle 4** | A / B | **4,0 / 4,1 s** | **15/16** | **0** | **14/14** | **ja** |
| 1 fps, pHash Schwelle 4 | A / B | 4,0 / 4,1 s | 16/16 / **11/16** | 0 | 14/14 / 9/14 | A ja / B nein |
| 0,5 fps, dHash Schwelle 4 | A / B | 3,2 / 4,1 s | 15/16 | 0 | 14/14 | ja |
| Pixel-Differenz + adaptive Maske dauerbewegter Kacheln | B | 6,2 s | 15/16 | **15** | 13/14 | ja |

- Der eine verfehlte Wechsel ist Aufbaustufe 1→2 (eine Zeile mehr); unschädlich, weil der Endstand behalten wird.
- pHash bricht mit Webcam ein: das dunkle Webcam-Feld dominiert die niederfrequenten DCT-Anteile. dHash
  (Gradienten) bleibt robust. Die adaptive Maske lernt das Feld zu langsam (Fehlalarme in den ersten 60 s).
- Nur-Keyframes ist 6× schneller, trifft aber nur, weil x264 an Schnitten Keyframes setzt; Teams/OBS-Aufnahmen mit
  festem GOP würden Wechsel bis zu 10 s verspätet oder gar nicht liefern. Nicht verlässlich.
- Die Zeiten schwanken mit der Last (gleiche Messung 3,9–11,3 s, wenn parallel ein Modell lädt); 4 statt aller
  Threads für ffmpeg kostet 1–4 s mehr und ist für den Hintergrundbetrieb die richtige Wahl.
- Dublettenabgleich über den ganzen Vortrag (Hamming ≤ 4 zu einer schon behaltenen Folie): der Rücksprung auf
  Folie 3 wird korrekt als zweites Vorkommen derselben Folie erkannt (16 erkannte Abschnitte → 15 Folien: 14 verschiedene + Aufbau-Endstand getrennt von Stufe 1/2).

**Realtest C (halbtransparente Folien, Überblendungen, Kamerabild):**

| Stufe | Ergebnis | Zeit |
|---|---|---|
| dHash 1 fps, Schwelle 4 | 35 Abschnitte, 24 nach Hash-Dubletten (PDF: 14 Folien) | 0,8 s Dekodierung + 0,4 s Hash (6 min, 606p) |
| ffmpeg `scene>0.05` | 82 „Wechsel“ (jede Überblendung) | 1,3 s |
| + Windows-OCR auf den 35 Repräsentanten: < 3 Wörter = keine Textfolie; Wort-Jaccard ≥ 0,5 = Dublette | 14 Textfolien, 15 ohne Text; davon 2 verbliebene Dubletten (Überblendstufe mit verstümmeltem Text), dafür 5 reine Bildfolien (Fotos ohne Text) zusammen mit Kamera- und Schwarzbildern abgetrennt | 0,68 s für 35 Bilder |
| + Gemma E4B klassifiziert 18 Bilder (die 15 ohne Text + 3 Folien, an denen der JSON-Weg scheiterte; Klartext, ohne Grammatik) | Schwarzbilder 2/2, Textfolie 1/1, Bildfolien 6/9, Sprecheraufnahmen 4/6 → **13/18 richtig**; Fehler: das Klassenwort „kamera“ wird mit abgebildeten Kameras/Rekordern verwechselt | 0,47 s/Bild |

Folgerung: Bei echten Aufzeichnungen erzeugt die Hash-Stufe zu viele Kandidaten; der Text-Abgleich halbiert sie
zuverlässig und billig. Bilder ohne Text werden **nicht automatisch verworfen**, sondern als „ohne Text“ markiert
und eingeklappt (Nutzer blendet aus/ein); Schwarzbilder (mittlere Helligkeit < 8) werden verworfen. Klassenwort
für Kameraaufnahmen künftig „sprecher_raum“ statt „kamera“.

## (2) OCR lokal

Satz „Video“ = 16 Einzelbilder aus Video B (H.264, mit Webcam-Feld), „sauber“ = Original-PNG. Recall/Präzision =
Wort-Multimengen (reihenfolgeunabhängig), CER = Zeichenfehlerrate in Lesereihenfolge (bestraft auch abweichende
Reihenfolge bei Tabellen), Zahlentreue = exakt getroffene Zahl-Token (12,4 · 43,0 % · 01.01.2027 …) auf „Video“.

| Verfahren | Recall (Video / sauber) | Präzision | CER | Umlaut-Wörter | Sonderzeichen | **Zahlentreue** | Zeit/Bild | Größe | Lizenz | Aus Rust |
|---|---|---|---|---|---|---|---|---|---|---|
| Windows.Media.Ocr (de-DE) | 0,945 / 0,947 | 0,97 | 0,12 | 69/70 | 13/23 | **47/60** | **70 / 47 ms** | 0 (im OS; de-DE, en-US installiert) | Windows-Bestandteil, nichts weiterzugeben | `windows` 0.61 ist schon da; Features `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`, `Globalization` ergänzen |
| Tesseract 5.4 (deu+eng, psm 3) | 0,981 / 0,961 | 0,97 | 0,07 | 68/70 | 20/23 | 54/60 | 700 / 337 ms | ≈ 50 MB Programm + Sprachdaten | Apache-2.0 (Leptonica BSD-2) | nur Kindprozess oder `leptess` (C-Build); separate Installation nötig |
| RapidOCR 3.9 · PP-OCRv6-det + **PP-OCRv5 latin rec** (4 Threads) | **0,991** / 0,980 | **0,997** | 0,05 | 69/70 | **22/23** | **60/60** | 2 256 / 1 751 ms | 38 MB ONNX | Apache-2.0 (Modelle PaddleOCR, Apache-2.0) | `oar-ocr` (Apache-2.0) – **nur `=0.9.0`**, ab 0.9.1 pinnt es `ort =2.0.0-rc.13`, die App (transcribe-rs) `=rc.12` |
| **Gemma 4 E4B als OCR** (llama-server + mmproj, GPU) | **0,996** | – | **0,03** | 58/59 | (alle sichtbar korrekt) | **60/60** | 720 ms | 5,0 GB Modell (vorhanden) + 0,99 GB mmproj | Apache-2.0 (laut Katalog) | vorhandener llama-server, Bild als `image_url` |
| Gemma 4 12B als OCR | 0,996 | – | 0,03 | 58/59 | – | 60/60 | 1 610 ms | 7,1 GB + 0,18 GB | Apache-2.0 | wie oben |

Typische Fehler (aus `out/ocr_texts.json`):
- Windows-OCR liest Tabellen spaltenweise und **lässt Zellen fallen** (Q2-Spalte fast ganz weg), „§ 3“ → „5 3“,
  „27%“ → „270/0“, isolierte Diagrammzahlen fehlen. Für Fließtext und Titel sehr gut.
- Tesseract verschluckt Dezimalkommas: „43,0 %“ → „430%“, „7,9“ → „79“ – für Protokolle gefährlich, weil
  plausibel aussehend.
- RapidOCR verlor auf einer sauberen Folie eine Aufzählungszeile (Erkennung, nicht Lesung).
- Florence-2 (MIT) **nicht gemessen**: braucht torch oder eine eigene ONNX-Kette mit 4 Sitzungen und Tokenizer,
  OCR ist englischlastig, und Gemma E4B erreicht auf denselben Bildern schon 0,996 Recall bei 0,7 s.

## (3) Bildanalyse mit der gebündelten llama.cpp-Laufzeit

Laufzeit: `llm-runtime-windows-x64-cuda` der App (llama.cpp build 10938), eigener Server auf Port 18089,
`-c 8192 -ngl 99 --parallel 1 -t 8`, RAM-Start-Gate 10 GB, RAM-Wächter, Ende per PID (alle Prozesse geprüft beendet).
Prompt: Art (Aufzählung), Beschreibung (≤ 2 Sätze), Kernaussage mit Zahlen, Deutsch, „Erfinde nichts“.
14 Folien aus Video B (Aufbau nur Endstand).

| Modell | VRAM nach Laden | Prozess-RAM | Laden | Beschreibung/Bild | OCR/Bild | Kernfakten | Art richtig* |
|---|---|---|---|---|---|---|---|
| **Gemma 4 E4B Q4_K_M + mmproj F16** | **4,7 GB** (5,2 nach Lauf) | 5,1 GB | 19–23 s | **2,0 s** (Bild-Encode 0,3–0,6 s) | 0,72 s | **17/17** | 9/14 |
| Gemma 4 E4B ohne mmproj (Vergleich) | 3,9 GB | – | 19 s | – | – | – | – |
| Gemma 4 12B Q4_K_M + mmproj F16 | 9,1 GB | **16,4 GB** | 9 s (Cache warm) | 3,1 s | 1,61 s | 17/17 | 7/14 |
| Gemma 4 E4B nur CPU (`-ngl 0`) | 0,76 GB | 5,7 GB | 11 s | **97,6 s** | 39,5 s | – | – |

\* „Art“ ist unscharf definiert (Aufzählung vs. Fließtext vs. Titelfolie); alle Diagramme, Tabellen, Codes und das
Foto wurden bei E4B richtig erkannt. Die Abweichungen sind fast nur Aufzählung↔Fließtext.

Qualität E4B (von Hand gelesen, `out/vision_gemma4-e4b.json`): Beschreibungen sachlich, Zahlen aus Tabelle,
Balken-, Kreisdiagramm und Code korrekt wiedergegeben (z. B. „Region Süd 6,8 Mio. € höchster, Ost 3,1 Mio. €
niedrigster Umsatz“). **Fehler:** Gantt-Diagramm „Pilotphase beginnt im Februar“ (richtig: Januar) – Ablesen von
Positionen ist die Schwachstelle; beim Foto wird die Bildunterschrift als Bildinhalt übernommen; „je nach
Überschreitung“ wird zu „pro Überschreitung“. Für das Protokoll heißt das: Folien**text** (OCR) ist belastbar,
Bild**beschreibung** nur als gekennzeichneter Hinweis verwenden.

Befunde für die Einbindung:
1. **`-b 2048 -ub 2048` ist Pflicht** beim Start mit mmproj: Gemma 4 12B brach mit Standard-`ubatch` 512 ab
   (`GGML_ASSERT … non-causal attention requires n_ubatch >= n_tokens`, ~1 000 Bild-Token). E4B lief zufällig durch.
2. **Kein JSON-Schema-Zwang (`response_format: json_schema`) für Bildanfragen.** Auf synthetischen Folien ging es
   gut, auf den echten Bildern lief E4B bis `max_tokens` ins Leere (Leerzeichen-Schleife): mit langen Enum-Werten 35/35,
   mit kurzen 18/35 (alle Bilder ohne Text und 3 Folien). Ohne Grammatik, mit Zeilenformat
   („Zeile 1: Klasse, Zeile 2: Satz“) 18/18 auswertbare Antworten in 0,47 s. Also: Klartext, `max_tokens` ≤ 200,
   Parser mit Rückfall, Antwort verwerfen statt speichern, wenn die erste Zeile keine bekannte Klasse ist.
3. Der Projektor kostet ~0,8 GB VRAM dauerhaft, solange der Server mit `--mmproj` läuft. Vorschlag: den Chat-Server
   **nicht** dauerhaft mit mmproj starten, sondern für einen Folienauftrag mit mmproj neu starten (19–23 s) und danach
   wieder ohne – oder mmproj nur laden, wenn „Bildanalyse“ eingeschaltet ist. Entscheidung in D3.
4. Qwen3.5-9B (bereits geladen, mmproj 918 MB) und Qwen2.5-VL-3B nicht gemessen: E4B erfüllt die Anforderungen,
   und der zusätzliche Download hätte die 2-GB-Grenze des Spikes überschritten. Ersatzkandidat, falls E4B im
   Praxistest nicht reicht.

## Lizenzen (kommerzielle Nutzung, Weitergabe im Installer)

| Komponente | Lizenz | Kommerziell | Im Installer? |
|---|---|---|---|
| Windows.Media.Ocr | Teil von Windows | ja | nichts mitzuliefern (Sprachpakete via Windows; fehlt de-DE → Rückfall en-US/Hinweis) |
| Gemma 4 E4B + mmproj (unsloth GGUF) | Apache-2.0 (Katalogeintrag `license_url` gemma_4_license) | ja | nein, Katalog-Download auf Wunsch wie bisher (990 MB, SHA-256 in den Katalog) |
| RapidOCR / PaddleOCR-Modelle (optional, später) | Apache-2.0 | ja | möglich (38 MB), Notices ergänzen |
| `oar-ocr` 0.9.0 (optional) | Apache-2.0 | ja | ja (Rust-Abhängigkeit) |
| `image_hasher` (falls statt Eigenbau) | MIT OR Apache-2.0 | ja | ja – Eigenbau des dHash (≈ 30 Zeilen) reicht aber |
| Tesseract | Apache-2.0 | ja | nicht empfohlen (Größe, kein Mehrwert gegenüber RapidOCR) |
| ffmpeg | wird wie bisher vom Nutzer installiert (PATH), nicht mitgeliefert | – | unverändert (keine neue GPL/LGPL-Frage) |
| Florence-2 | MIT | ja | nicht empfohlen (s. o.) |
| Testvideo C | CC BY-SA 4.0 | nur Testmaterial, nicht ins Repo | nein |

## (4) Integrationsvorschlag

### Ablauf (neue Job-Phase `Slides`, pausierbar je Bild, hinter Warteschlange U7)

1. **Abtasten** (Rust, `ffmpeg -threads 4 -i <video> -an -vf fps=1,scale=160:90:flags=area,format=gray -f rawvideo -`
   über `media::run_child_cancellable`): Bilder werden gestreamt gehasht (konstanter RAM, 14 400 B je Bild).
2. **Segmentieren** (reine Funktion): dHash-Hamming > 4 und 2 Abtastungen stabil = Wechsel; Repräsentant = letztes
   Bild vor dem nächsten Wechsel; Schwarzbilder (Mittelwert < 8) verwerfen.
3. **Vollbilder** je Repräsentant: `ffmpeg -ss <t> -frames:v 1 -q:v 3` → `<Besprechungsordner>/slides/0007.jpg`
   (0,31 s, ~100 KB je Bild) plus Vorschaubild 320 px.
4. **Windows-OCR** je Vollbild (70 ms) → Text; Dubletten über den ganzen Vortrag zusammenführen (Hash ≤ 4 **oder**
   Wort-Jaccard ≥ 0,5) → Vorkommen als Zeitbereiche an EINER Folie; < 3 Wörter → `kind = ohne_text` (eingeklappt).
5. **Optional Bildanalyse** (Einstellung, nur mit CUDA/Vulkan-GPU, VRAM-Gate ≥ 6 GB frei): Gemma E4B mit mmproj,
   zwei Klartext-Anfragen je Folie: Folientext (ersetzt den Windows-OCR-Text, `ocr_engine = gemma-4-e4b`) und
   Klasse + Beschreibung.
6. **Index und Protokoll**: Folientext und Beschreibung als Chunks (`source = 'slide'`, `start_ms`) in den
   Suchindex; Protokoll/KI-Notizen/Chat bekommen die Folien zeitlich eingewoben.

### Datenmodell (neue Migration, idempotent, im Muster von `variants.rs`)

Folien sind **kein** Transkript-Fassungsobjekt (Fassungen schließen sich gegenseitig aus, Folien kommen hinzu),
sondern ein Anhang der Besprechung:

```sql
CREATE TABLE meeting_slides (
  id TEXT PRIMARY KEY,                 -- ULID
  meeting_id TEXT NOT NULL,
  number INTEGER NOT NULL,             -- 1..n nach erstem Auftreten
  origin TEXT NOT NULL CHECK (origin IN ('video','image')),
  image_path TEXT NOT NULL,            -- relativ zum Besprechungsordner: slides/0007.jpg
  thumb_path TEXT,
  dhash INTEGER NOT NULL,              -- 64 bit
  occurrences_json TEXT NOT NULL DEFAULT '[]',   -- [{"start_ms":..,"end_ms":..}], Ruecksprung = mehrere
  ocr_text TEXT, ocr_engine TEXT,      -- 'windows-ocr' | 'gemma-4-e4b' | ...
  kind TEXT,                           -- 'text' | 'ohne_text' | Modellklasse (nur Hinweis)
  description TEXT, description_model TEXT,
  hidden INTEGER NOT NULL DEFAULT 0,   -- Nutzer blendet aus (Sprecherbild, Dublette)
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, deleted_at INTEGER);
CREATE INDEX idx_slides_meeting ON meeting_slides(meeting_id, number);
```

`ChunkSource` bekommt `Slide` (`"slide"`); `SourceRef { kind: "slide", ref: <slide_id>, title: "Folie 7 · 04:12" }`
in der Herkunft von Protokoll und Notizen (#66-Muster, keine neue `SubjectKind` nötig, Modell steht in
`ocr_engine`/`description_model`). Löschen der Besprechung löscht `slides/` mit (Retention wie Audio).

### Schnittstellen

```rust
// managers/meetings/slides/mod.rs
pub struct SlideDetectConfig { pub sample_fps: f32 /*1.0*/, pub hash_threshold: u32 /*4*/,
                               pub stable_samples: u32 /*2*/, pub text_jaccard: f32 /*0.5*/ }
pub struct SlideSegment { pub start_ms: u64, pub end_ms: u64, pub rep_ms: u64, pub hash: u64, pub black: bool }
pub fn dhash64(gray: &[u8], w: usize, h: usize) -> u64;
pub fn segment(hashes: &[(u64, f32 /*mean luma*/)], fps: f32, cfg: &SlideDetectConfig) -> Vec<SlideSegment>;
pub fn group_duplicates(segs: &[SlideSegment], texts: &[String], cfg: &SlideDetectConfig) -> Vec<SlideGroup>;
pub fn sample_video(video: &Path, cfg: &SlideDetectConfig, cancel: &AtomicBool) -> Result<Vec<(u64, f32)>, SlideError>;
pub fn extract_frame(video: &Path, at_ms: u64, out: &Path, cancel: &AtomicBool) -> Result<(), SlideError>;
// managers/meetings/slides/ocr.rs
pub trait OcrBackend: Send + Sync { fn recognize(&self, image: &Path) -> Result<OcrText, OcrError>; fn id(&self) -> &'static str; }
#[cfg(windows)] pub struct WindowsOcr { lang: String } // de-DE, Rueckfall en-US; macOS: Unsupported
// managers/meetings/slides/vision.rs
pub async fn read_slide(llm: &LlmClient, image: &Path) -> Result<String, VisionError>;          // Klartext-OCR
pub async fn describe_slide(llm: &LlmClient, image: &Path) -> Result<SlideVision, VisionError>; // Klasse + Satz
```

Tauri-Commands (bindings.ts von Hand nachziehen, siehe Toolchain-Fallen):
`detect_meeting_slides(meeting_id, options: SlideOptions) -> Result<(), String>` (reiht Auftrag ein),
`list_meeting_slides(meeting_id) -> Result<Vec<MeetingSlide>, String>`,
`set_meeting_slide_hidden(slide_id, hidden) -> Result<(), String>`,
`add_meeting_images(meeting_id, paths: Vec<String>) -> Result<Vec<MeetingSlide>, String>` (D6).

```ts
type MeetingSlide = { id: string; meetingId: string; number: number; origin: "video" | "image";
  imagePath: string; thumbPath: string | null; occurrences: { startMs: number; endMs: number }[];
  ocrText: string | null; ocrEngine: string | null; kind: string | null;
  description: string | null; descriptionModel: string | null; hidden: boolean };
```

Einstellungen in der **vorhandenen** Gruppe Besprechungen/Import (kein neuer Reiter): „Folien aus Videos
erkennen“ (an), „Abtastabstand“ (1 s), „Bildanalyse mit lokalem Modell“ (aus; nur wählbar mit GPU und
geladenem mmproj).

`llama-server`: `server_args` bekommt bei Bildanalyse `--mmproj <datei> -b 2048 -ub 2048`; der Katalog einen
Eintrag für `mmproj-F16.gguf` (unsloth/gemma-4-E4B-it-GGUF, 990 372 672 Byte, SHA-256 beim Paket ermitteln).

### Fehlerfälle

| Fall | Verhalten |
|---|---|
| ffmpeg fehlt / Datei ohne Videospur (Audio, YouTube-Audio) | Phase übersprungen, Hinweis „keine Bildspur“, kein Fehler der Besprechung |
| Abbruch/Pause | Kindprozess per PID beendet (`run_child_cancellable`), bereits geschriebene Folien bleiben, Lauf idempotent wiederholbar (Zuordnung über dHash + Zeit) |
| 3-h-Video, 4K | Abtastung streamt, RAM konstant; 4K wird von ffmpeg verkleinert; Laufzeit linear (≈ 1 min je Stunde) |
| Webcam/Video im Video, Überblendungen | dHash + Stabilität + Text-Abgleich (gemessen), Rest per „Ausblenden“ |
| Windows ohne de-DE-OCR | en-US-Engine (Umlaute dann schwächer), Hinweis in Einstellungen |
| Kein GPU / VRAM-Gate verfehlt | Bildanalyse nicht anbieten bzw. Auftrag ohne Analyse fertigstellen (CPU: 98 s je Bild) |
| Modell liefert Unsinn (Leerzeichen-Schleife) | `max_tokens` 200, Zeilenparser, Antwort verwerfen, Folie behält Windows-OCR-Text |
| Datenschutz | Bilder bleiben im Besprechungsordner; externe API nur bei bewusst gewähltem externem LLM, wie bei #66 gekennzeichnet |

### Nutzung im Protokoll

Folien als eigene Zeilen in die zeitlich sortierte Transkriptfolge einweben, Format analog `transcript_line`:
`[Folie 7 · 04:12] <Folientext, ≤ 600 Zeichen> {Bild: <Beschreibung>}` – Beschreibung nur, wenn vorhanden, als
„Bild:“ gekennzeichnet. Budget: ≈ 50–150 Token je Folie (15 Folien ≈ 2 k Token) in `notes/budget.rs` einrechnen;
bei Platzmangel Beschreibungen zuerst kürzen, Folientext zuletzt. BASE_RULES ergänzen: „Zahlen aus Folien nur aus
dem Folientext übernehmen, nicht aus der Bildbeschreibung.“ Belege `[F7]` neben `[S17]` (Chat und KI-Notizen),
Klick springt zur Folie und zur Zeitmarke.

### Pakete

| ID | Scope | Akzeptanztest (Befehl → Ergebnis) | Abh. | Aufwand |
|---|---|---|---|---|
| **D1** | `slides/mod.rs`: Abtastung über ffmpeg-Pipe, `dhash64`, `segment`, Schwarzbild, Vollbild-/Vorschau-Extraktion, Migration `meeting_slides`, Store-Funktionen; Job-Phase `Slides` | `cargo test --lib slides` → grün, darunter: (a) reine Hashfolgen inkl. Aufbaustufen/Rücksprung, (b) per ffmpeg-lavfi erzeugtes 30-s-Video mit 3 Folien + bewegtem Feld → 3 Folien ±1 s, (c) Migration idempotent, (d) Abbruch lässt keinen ffmpeg-Prozess zurück | – | 300 kTok |
| **D2** | `slides/ocr.rs`: `WindowsOcr` über `windows`-Crate (+4 Features), Text-Dubletten (Jaccard), `ohne_text`-Markierung; macOS-Stub | `cargo test --lib slides::ocr` → eingecheckte PNG-Fixture (Folie mit „Größe/Maße“, „Bußgeld: 70 €“) wird erkannt, Recall ≥ 0,9; Realfall: 35 Repräsentanten-Fixture-Hashes/Texte → ≤ 16 Folien | D1 | 200 kTok |
| **D3** | Bildanalyse: Katalogeintrag mmproj, `server_args` mit `--mmproj -b 2048 -ub 2048`, VRAM-Gate, Bild-Nachricht im `llm_client`, `read_slide`/`describe_slide` ohne Grammatik mit Zeilenparser, Einstellung „Bildanalyse“ | `cargo test --lib slides::vision llm::server` → Argument- und Request-JSON-Tests, Parser-Tests inkl. Leerzeichen-Antwort; `#[ignore]`-Test mit echtem E4B: Tabellenfolie → „13,1“ im Text, < 5 s | D1, D2 | 350 kTok |
| **D4** | Oberfläche: Folienleiste/-reiter in der Besprechungsmitte neben Transkript (Layout G4), Klick → Audio-Sprung, Abspielzeit markiert aktuelle Folie, Folienmarken im Transkript, Ausblenden, Großansicht; Import-Option „Folien erkennen“; i18n de+en | `pnpm exec playwright test meeting-slides` → grün (Mock-Commands); tsc 0; `check_i18n_meetings.py` grün | D1 | 350 kTok |
| **D5** | Protokoll, KI-Notizen, Chat, Suche: Einweben `[Folie n · mm:ss]`, Budget, BASE_RULES, `ChunkSource::Slide`, Belege `[F7]`, `SourceRef kind=slide` | `cargo test --lib minutes notes search` → Prompt-Render-Test mit Folien, Budget-Test, Index-Test (Folientext per FTS findbar); Eval mit Testvideo A + gesprochenem Text: Protokoll nennt „13,1 Mio. €“ mit Beleg `[F4]` | D1, D2 | 300 kTok |
| **D6** | Einzelbilder als Quelle (PNG/JPG) in eine Besprechung/ein Projekt laden → `origin='image'`, OCR/Analyse wie Folien; PDF-Seiten zunächst über die vorhandene Textextraktion | `cargo test --lib slides::images` + Playwright „Bild hinzufügen“ → Folie erscheint mit Text | D2, D4 | 150 kTok |
| **D7** | YouTube mit Bild: bei „Folien erkennen“ Videospur ≤ 720p mit laden (yt-dlp-Format), sonst wie bisher nur Audio | Rust-Test der yt-dlp-Argumente; manueller Lauf mit einem Folienvortrag → Folien sichtbar | D1 | 100 kTok |
| **D8** | Abnahme: Installer, Gesamttest, Hilfe/Doku, Testvideo-Generator (`make_slides.py`) als Fixture-Skript ins Repo | Installer 0.20.x baut; volle Suite grün; AK7: Folien des Testvideos in der Besprechung sichtbar und im Protokoll belegt | D1–D5 | 100 kTok |

Summe ≈ **1,85 MTok** (inkl. ~15 % Review), innerhalb M7-Rahmen 2,0 MTok. Reihenfolge: D1 → (D2 ∥ D4) → D3 → D5 →
D6/D7 → D8. Für AK7 genügen D1, D2, D4, D5, D8 (≈ 1,25 MTok); D3 (Bildanalyse), D6, D7 sind abtrennbar.

## Offene Punkte / Owner-Entscheidungen

- **O1** mmproj-Download 990 MB zusätzlich zum E4B-Modell (Katalog, auf Wunsch). Alternative 12B-mmproj nur 175 MB,
  aber 12B selbst braucht 9 GB VRAM und 16 GB RAM – nicht empfohlen.
- **O2** mmproj dauerhaft im Chat-Server (+0,8 GB VRAM, kein Neustart) oder nur für Folienaufträge (Neustart ~20 s)?
  Vorschlag: nur für Folienaufträge.
- **O3** macOS: Windows-OCR gibt es dort nicht; Vorschlag später Apple Vision-Framework oder RapidOCR (`oar-ocr =0.9.0`).
  Für 0.21.0 genügt: macOS ohne OCR, mit Bildanalyse falls GPU.
- **R1** Echte Aufzeichnungen (Teams/Zoom mit Kacheln, Mauszeiger, eingebettete Videos in Folien) nur an einem
  CC-Video geprüft; dort ≈ 2 Dubletten und abgetrennte Bildfolien je 14 Folien. Ausblenden-Funktion ist deshalb
  Pflichtbestandteil von D4.
- **R2** Bildbeschreibungen enthalten Ablesefehler bei Positionen (Gantt); nur gekennzeichnet verwenden (D5-Regel).
