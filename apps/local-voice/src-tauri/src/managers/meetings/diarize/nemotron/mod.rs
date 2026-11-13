//! Paket N1: Sprechertrennung mit NVIDIA Nemotron-3-Diarization (bis acht
//! Sprecher je Kanal) ueber ONNX Runtime (`ort`, CPU), als zweite Engine hinter
//! [`super::DiarizeEngine`].
//!
//! Das ONNX-Modell (`onnx-community/Nemotron-3-Diarization-ONNX`, int8,
//! `onnx/model_quantized.onnx` + `model_quantized.onnx_data`) rechnet nur EINEN
//! Block. Alles drumherum ist hier nach Rust portiert:
//!
//! * [`frontend`]: Mel-Vorstufe (16 kHz, 128 Baender, FFT 512, Fenster 400,
//!   Schritt 160, Preemphase 0,97, Slaney-Mel),
//! * [`blocks`]: Blockschnitt (Offline-Arbeitspunkt fuer ganze Dateien), Block-
//!   schleife mit Abbruchpruefung, Schwelle und Turns,
//! * [`cache`]: der Sprecher-Zwischenspeicher "Arrival-Order Speaker Cache"
//!   (AOSC) mit FIFO aus Streaming Sortformer,
//! * [`engine`]: `NemotronOnnxEngine` (ONNX-Sitzung, `DiarizeEngine`).
//!
//! Herkunft und Lizenz des Portes: Vorlage sind die Klassen
//! `Nemotron3DiarizationSpeakerCache`, `Nemotron3DiarizationProcessor`,
//! `Nemotron3DiarizationForAudioFrameClassification` und
//! `NemotronAsrStreamingFeatureExtractor` aus HuggingFace `transformers`
//! (`models/nemotron3_diarization`, `models/nemotron_asr_streaming`),
//! Copyright 2026 The HuggingFace Inc. team, Apache License 2.0. Die Algorithmen
//! (Mel-Vorstufe, Blockschleife, Bewertung und Verdichtung des Zwischenspeichers)
//! sind hier neu in Rust geschrieben, nicht Zeile fuer Zeile uebersetzt; die
//! Konstanten stammen aus der `config.json` des Modells. Die Gewichte
//! (nvidia/Nemotron-3-Diarization, ONNX-Export von onnx-community) stehen unter
//! OpenMDW-1.1 und werden geladen, nicht mitgeliefert (Katalog).
//!
//! Fehlerfaelle und ihre Absicherung (Tests in den Untermodulen):
//! * zwei Laeufe gleichzeitig: `Diarizer` belegt weiter den einen Platz
//!   (`engine::ExclusiveSlot`), nie zwei Modelle;
//! * wenig RAM: Start-Gate vor dem Laden wie bei Sortformer;
//! * Modell oder `.onnx_data` fehlt, abgeschnittene oder fremde ONNX-Datei:
//!   `ModelMissing` / `Load` / `WrongModel`, nie eine Panik;
//! * Abbruch: zwischen den Bloecken geprueft (ein Block sind 27,2 s Audio, gemessen
//!   1,5 s Rechenzeit mit einem Thread); ein abgebrochener Lauf liefert KEINE
//!   Teil-Turns (`Cancelled`) und macht die Engine nicht unbrauchbar;
//! * zu kurzes Audio (< 2 s): leeres Ergebnis, das Modell wird nicht geladen
//!   (`diarize_with`);
//! * NaN/Inf: im Eingang vor dem Modell auf 0 gesetzt (`finite_pcm`), im
//!   Ausgang als Fehler gemeldet, bevor sie den Zwischenspeicher vergiften;
//! * Panik im FFI-Aufruf: abgefangen (`engine::guarded_logits`), die Engine ist
//!   danach unbrauchbar; ein nativer Absturz (kein Kindprozess, wie bei
//!   Sortformer) risse die App mit, dafuer gilt die Absturzmarke der Aufrufer;
//! * Gleichstand im Zwischenspeicher: `torch.topk` laesst offen, welche von mehreren
//!   exakt gleichen Kopien eines Rahmens bleibt; hier gilt "kleinerer Index zuerst"
//!   (`cache::top_indices`), die Goldwerte nutzen dieselbe Regel. Gegen die
//!   Standardauswahl von torch bleiben 100 % der Rahmen gleich bei zwei Sprechern und
//!   97,6 % bei ueberlagerter Sprache von vier Sprechern (`pipeline_ref.json`);
//! * Speicher: Mel-Merkmale werden je Block berechnet (nie die ganze Datei),
//!   das Ergebnis ist nur `Rahmen x 8` Logits.

pub mod blocks;
pub mod cache;
pub mod engine;
pub mod frontend;

pub use engine::NemotronOnnxEngine;

/// Mel-Baender je Rahmen.
pub const MEL_BINS: usize = 128;
/// Mel-Rahmen je Encoder-Rahmen (Merkmalsstapelung): 80 ms je Encoder-Rahmen.
pub const SUBSAMPLING: usize = 8;
/// Breite der Encoder-Rahmen.
pub const HIDDEN: usize = 512;
/// Sprecher-Kanaele des Modells.
pub const NUM_SPEAKERS: usize = 8;
/// Ein Mel-Rahmen dauert 10 ms.
pub const FRAME_MS: u64 = 10;
/// Katalog-ID des Modells.
pub const MODEL_ID: &str = "diar-nemotron3-8spk-onnx-q8";
/// Dateiendung, an der die Engine-Wahl erkennt, dass es dieses Modell ist.
pub const MODEL_EXTENSION: &str = "onnx";
