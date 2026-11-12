//! Blockschnitt und Blockschleife der Nemotron-Engine (Paket N1), Offline-
//! Arbeitspunkt fuer ganze Dateien (`chunk_length` 340, `chunk_right_context`
//! 40 Encoder-Rahmen a 80 ms, also Bloecke von 27,2 s mit 3,2 s Vorausschau).
//!
//! Die Schleife folgt `Nemotron3DiarizationForAudioFrameClassification.forward`
//! (transformers, Apache-2.0) im Offline-Modus: das Modell rechnet je Block den
//! Encoder ueber `[Zwischenspeicher-Rahmen | Block | Vorausschau]` und liefert
//! Logits fuer alle diese Rahmen; behalten werden nur die des Blocks. Danach
//! fuettert der Block den Zwischenspeicher ([`SpeakerCache::update`]).
//!
//! Das Modell selbst steckt hinter [`BlockModel`]: die echte Umsetzung ist die
//! ONNX-Sitzung in `engine.rs`, die Tests nutzen Attrappen, damit Blockschnitt,
//! Abbruch und Fehlerwege ohne Modelldatei laufen.

use super::cache::{CacheConfig, SpeakerCache};
use super::frontend::{num_frames, MelFrontend};
use super::{FRAME_MS, HIDDEN, NUM_SPEAKERS, SUBSAMPLING};
use crate::managers::meetings::diarize::postproc::RawTurn;
use crate::managers::meetings::diarize::DiarizeError;

/// Blockgroessen in Encoder-Rahmen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockPlan {
    pub chunk_length: usize,
    pub right_context: usize,
}

impl BlockPlan {
    /// Offline-Arbeitspunkt aus der `config.json` des Modells.
    pub const OFFLINE: Self = Self {
        chunk_length: 340,
        right_context: 40,
    };
}

/// Ein Block: `chunk_frames` eigene Rahmen ab `start`, dahinter Vorausschau bis
/// `context_end` (ausschliesslich), alles in Encoder-Rahmen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Block {
    pub start: usize,
    pub chunk_frames: usize,
    pub context_end: usize,
}

/// Encoder-Rahmen fuer `valid_mel_frames` gueltige Mel-Rahmen: die Referenz hat
/// einen Mel-Rahmen mehr (der letzte, von der Maske auf 0 gesetzt), gestapelt
/// zu Achtergruppen und aufgefuellt.
pub fn num_embed_frames(valid_mel_frames: usize) -> usize {
    (valid_mel_frames + 1).div_ceil(SUBSAMPLING)
}

/// Schneidet `embed_frames` Encoder-Rahmen in Bloecke.
pub fn plan_blocks(embed_frames: usize, plan: BlockPlan) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut start = 0;
    while start < embed_frames {
        let end = (start + plan.chunk_length).min(embed_frames);
        blocks.push(Block {
            start,
            chunk_frames: end - start,
            context_end: (end + plan.right_context).min(embed_frames),
        });
        start = end;
    }
    blocks
}

/// Eingang eines Blocks. Alles zeilenweise, Stapelgroesse 1.
pub struct BlockInput<'a> {
    /// `mel_rows * 128`: Mel-Rahmen des Blocks samt Vorausschau (nicht auf ein
    /// Vielfaches von 8 aufgefuellt, das macht das Modell).
    pub mel: &'a [f32],
    pub mel_rows: usize,
    /// `cached_frames * hidden`: Zwischenspeicher, dann FIFO.
    pub cached: &'a [f32],
    pub cached_frames: usize,
    /// Ein Eintrag je Encoder-Rahmen von `cached ++ Block`: 1 gueltig, 0 Auffuellung.
    pub mask: &'a [i64],
}

pub struct BlockOutput {
    /// `(cached_frames + Block-Rahmen) * 8` Zeilen zu 8 Logits.
    pub logits: Vec<f32>,
    /// `Block-Rahmen * hidden`: die gestapelten Mel-Rahmen des Blocks, vor dem Encoder.
    pub chunk_embeds: Vec<f32>,
    /// `hidden`: gelernte Stille-Einbettung.
    pub silence: Vec<f32>,
}

/// Das Modell fuer einen Block.
pub trait BlockModel {
    fn run(&mut self, input: &BlockInput<'_>) -> Result<BlockOutput, DiarizeError>;

    fn hidden(&self) -> usize {
        HIDDEN
    }
}

fn run_error(msg: &str) -> DiarizeError {
    DiarizeError::Run(msg.to_string())
}

/// Rechnet eine ganze Datei Block fuer Block und liefert die Logits aller
/// gueltigen Mel-Rahmen (`num_frames(pcm.len()) * 8`, rahmenweise).
///
/// `cancelled` wird vor jedem Block und noch einmal nach dem letzten gefragt; ein
/// Abbruch liefert nie Teil-Ergebnisse, auch nicht, wenn er erst waehrend des
/// letzten Blocks kommt. Kein Block wird gerechnet, wenn schon vorher abgebrochen ist.
pub fn run_offline(
    model: &mut dyn BlockModel,
    front: &MelFrontend,
    pcm: &[f32],
    cache_cfg: CacheConfig,
    plan: BlockPlan,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<f32>, DiarizeError> {
    cache_cfg.validate().map_err(run_error)?;
    let hidden = model.hidden();
    let valid = num_frames(pcm.len());
    if valid == 0 {
        // Kein gueltiger Rahmen: kein Modelllauf (ein Block aus nur maskierten
        // Rahmen ist fuer das Modell keine sinnvolle Eingabe).
        return if cancelled() {
            Err(DiarizeError::Cancelled)
        } else {
            Ok(Vec::new())
        };
    }
    let total_rows = valid + 1;
    let embed_frames = num_embed_frames(valid);
    let mut cache = SpeakerCache::new(cache_cfg, hidden);
    let mut logits = Vec::with_capacity(valid * NUM_SPEAKERS);
    let (mut mel, mut cached, mut input_embeds) = (Vec::new(), Vec::new(), Vec::new());

    for block in plan_blocks(embed_frames, plan) {
        if cancelled() {
            return Err(DiarizeError::Cancelled);
        }
        let first_row = block.start * SUBSAMPLING;
        let end_row = (block.context_end * SUBSAMPLING).min(total_rows);
        mel.clear();
        front.rows(pcm, first_row, end_row - first_row, &mut mel);

        cached.clear();
        cache.fill_cached(&mut cached);
        let cached_frames = cache.cached_frames();
        let block_frames = block.context_end - block.start;
        let mut mask = vec![1i64; cached_frames];
        mask.extend((block.start..block.context_end).map(|i| i64::from(i * SUBSAMPLING < valid)));

        let out = model.run(&BlockInput {
            mel: &mel,
            mel_rows: end_row - first_row,
            cached: &cached,
            cached_frames,
            mask: &mask,
        })?;
        let rows = (cached_frames + block_frames) * SUBSAMPLING;
        if out.logits.len() != rows * NUM_SPEAKERS
            || out.chunk_embeds.len() != block_frames * hidden
            || out.silence.len() != hidden
        {
            return Err(run_error("Modell lieferte unerwartete Groessen"));
        }
        if out.logits.iter().any(|v| !v.is_finite())
            || out.chunk_embeds.iter().any(|v| !v.is_finite())
            || out.silence.iter().any(|v| !v.is_finite())
        {
            return Err(run_error("Modell lieferte ungueltige Werte (NaN/Inf)"));
        }

        input_embeds.clear();
        input_embeds.extend_from_slice(&cached);
        input_embeds.extend_from_slice(&out.chunk_embeds);
        let mask_bool: Vec<bool> = mask.iter().map(|&m| m != 0).collect();
        cache
            .update(
                &input_embeds,
                &out.logits,
                &out.silence,
                block.chunk_frames,
                &mask_bool,
                0,
            )
            .map_err(run_error)?;

        let from = cached_frames * SUBSAMPLING * NUM_SPEAKERS;
        let to = from + block.chunk_frames * SUBSAMPLING * NUM_SPEAKERS;
        logits.extend_from_slice(&out.logits[from..to]);
    }
    if cancelled() {
        return Err(DiarizeError::Cancelled);
    }
    logits.truncate(valid * NUM_SPEAKERS);
    Ok(logits)
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Schwelle 0,5 auf der Sigmoid-Wahrscheinlichkeit, dann je Sprecher
/// zusammenhaengende Rahmen als Turn (Rahmen zu 10 ms). Sprecher 1-basiert in
/// der Reihenfolge der Modellkanaele (das Modell ordnet nach dem ersten
/// Auftreten); sortiert nach Beginn, dann Sprecher. Ueberlappung bleibt.
pub fn turns_from_logits(logits: &[f32]) -> Vec<RawTurn> {
    let frames = logits.len() / NUM_SPEAKERS;
    let mut turns = Vec::new();
    for spk in 0..NUM_SPEAKERS {
        let mut start: Option<usize> = None;
        for f in 0..=frames {
            let active = f < frames && sigmoid(logits[f * NUM_SPEAKERS + spk]) > 0.5;
            match (active, start) {
                (true, None) => start = Some(f),
                (false, Some(s)) => {
                    turns.push(RawTurn {
                        t0_ms: (s as u64 * FRAME_MS) as i64,
                        t1_ms: (f as u64 * FRAME_MS) as i64,
                        speaker_id: spk as i32 + 1,
                    });
                    start = None;
                }
                _ => {}
            }
        }
    }
    turns.sort_by_key(|t| (t.t0_ms, t.speaker_id));
    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::diarize::nemotron::MEL_BINS;

    const SMALL: BlockPlan = BlockPlan {
        chunk_length: 5,
        right_context: 2,
    };

    fn small_cache() -> CacheConfig {
        CacheConfig {
            fifo_length: 6,
            update_period: 4,
            cache_length: 16,
            ..CacheConfig::offline()
        }
    }

    fn secs(s: f32) -> Vec<f32> {
        (0..(s * 16_000.0) as usize)
            .map(|i| 0.1 * ((i as f32) * 0.05).sin())
            .collect()
    }

    /// Attrappe: Logit `[Zeile, 0, ...]` verraet, welche Zeile des Encoder-
    /// Eingangs gemeint ist; Einbettung `[Rahmennummer im Block, 0]`.
    struct Probe {
        calls: Vec<(usize, usize, Vec<i64>)>,
        fail_at: Option<usize>,
        /// Welche der drei Ausgaben NaN/Inf enthaelt: `logits`, `embeds`, `silence`.
        poison: Option<&'static str>,
    }

    impl Probe {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                fail_at: None,
                poison: None,
            }
        }
    }

    impl BlockModel for Probe {
        fn hidden(&self) -> usize {
            2
        }

        fn run(&mut self, input: &BlockInput<'_>) -> Result<BlockOutput, DiarizeError> {
            self.calls
                .push((input.mel_rows, input.cached_frames, input.mask.to_vec()));
            if self.fail_at == Some(self.calls.len()) {
                return Err(DiarizeError::Run("kaputt".into()));
            }
            assert_eq!(input.mel.len(), input.mel_rows * MEL_BINS);
            assert_eq!(input.cached.len(), input.cached_frames * 2);
            let block_frames = input.mel_rows.div_ceil(SUBSAMPLING);
            assert_eq!(input.mask.len(), input.cached_frames + block_frames);
            let rows = (input.cached_frames + block_frames) * SUBSAMPLING;
            let mut logits = vec![0.0; rows * NUM_SPEAKERS];
            for r in 0..rows {
                logits[r * NUM_SPEAKERS] = r as f32;
            }
            let mut chunk_embeds: Vec<f32> =
                (0..block_frames).flat_map(|i| [i as f32, 0.0]).collect();
            let mut silence = vec![-1.0, -1.0];
            match self.poison {
                Some("logits") => logits[3] = f32::NAN,
                Some("embeds") => chunk_embeds[0] = f32::INFINITY,
                Some("silence") => silence[1] = f32::NAN,
                _ => {}
            }
            Ok(BlockOutput {
                logits,
                chunk_embeds,
                silence,
            })
        }
    }

    fn run(pcm: &[f32], model: &mut Probe, plan: BlockPlan) -> Result<Vec<f32>, DiarizeError> {
        run_offline(
            model,
            &MelFrontend::new(),
            pcm,
            small_cache(),
            plan,
            &|| false,
        )
    }

    #[test]
    fn embed_frames_follow_the_reference_including_its_extra_row() {
        assert_eq!(num_embed_frames(0), 1);
        assert_eq!(num_embed_frames(7), 1);
        // 8 gueltige Rahmen + der Maskenrahmen = 9 -> ein zweiter, ganz maskierter Rahmen.
        assert_eq!(num_embed_frames(8), 2);
        assert_eq!(num_embed_frames(200), 26);
        assert_eq!(num_embed_frames(2_719), 340);
        assert_eq!(num_embed_frames(2_720), 341);
    }

    #[test]
    fn audio_shorter_than_one_block_is_a_single_block_without_lookahead() {
        let blocks = plan_blocks(26, BlockPlan::OFFLINE);
        assert_eq!(
            blocks,
            vec![Block {
                start: 0,
                chunk_frames: 26,
                context_end: 26
            }]
        );
    }

    #[test]
    fn exactly_one_block_has_no_lookahead_to_take() {
        let blocks = plan_blocks(340, BlockPlan::OFFLINE);
        assert_eq!(blocks.len(), 1);
        assert_eq!((blocks[0].chunk_frames, blocks[0].context_end), (340, 340));
    }

    #[test]
    fn one_frame_over_a_block_makes_a_second_tiny_block() {
        let blocks = plan_blocks(341, BlockPlan::OFFLINE);
        assert_eq!(
            blocks,
            vec![
                Block {
                    start: 0,
                    chunk_frames: 340,
                    context_end: 341
                },
                Block {
                    start: 340,
                    chunk_frames: 1,
                    context_end: 341
                },
            ]
        );
    }

    #[test]
    fn many_blocks_with_a_remainder_cover_every_frame_once() {
        let blocks = plan_blocks(1_000, BlockPlan::OFFLINE);
        assert_eq!(
            blocks.iter().map(|b| b.chunk_frames).collect::<Vec<_>>(),
            vec![340, 340, 320]
        );
        assert_eq!(
            blocks
                .iter()
                .map(|b| (b.start, b.context_end))
                .collect::<Vec<_>>(),
            vec![(0, 380), (340, 720), (680, 1_000)]
        );
        let exact = plan_blocks(1_020, BlockPlan::OFFLINE);
        assert_eq!(exact.len(), 3);
        assert_eq!(exact.last().unwrap().context_end, 1_020);
        assert!(plan_blocks(0, BlockPlan::OFFLINE).is_empty());
    }

    #[test]
    fn two_seconds_make_one_block_with_the_extra_masked_row() {
        let mut probe = Probe::new();
        let logits = run(&secs(2.0), &mut probe, BlockPlan::OFFLINE).unwrap();
        assert_eq!(logits.len(), 200 * NUM_SPEAKERS);
        assert_eq!(probe.calls.len(), 1);
        let (mel_rows, cached, mask) = &probe.calls[0];
        assert_eq!((*mel_rows, *cached), (201, 0));
        // 26 Encoder-Rahmen: 25 gueltige, der letzte (Zeile 200 = Maskenrahmen) nicht.
        assert_eq!(mask.len(), 26);
        assert_eq!(mask.iter().sum::<i64>(), 25);
        assert_eq!(*mask.last().unwrap(), 0);
        // Ohne Zwischenspeicher sind Zeile und Logit dasselbe.
        assert_eq!(logits[NUM_SPEAKERS * 123], 123.0);
    }

    #[test]
    fn logits_of_every_block_start_after_its_cached_frames() {
        // 18 s: 1800 Rahmen -> 226 Encoder-Rahmen -> Bloecke zu 5 -> 46 Bloecke.
        let pcm = secs(18.0);
        let mut probe = Probe::new();
        let logits = run(&pcm, &mut probe, SMALL).unwrap();
        let frames = 1_800;
        assert_eq!(logits.len(), frames * NUM_SPEAKERS);
        let blocks = plan_blocks(num_embed_frames(frames), SMALL);
        assert_eq!(probe.calls.len(), blocks.len());
        assert_eq!(blocks.len(), 46);
        // Jeder Block liefert seine Zeilen ab (Speicher-Rahmen * 8); die Attrappe
        // nummeriert die Zeilen, also muss das Logit dort anfangen.
        let mut out_row = 0;
        for (block, (_, cached, _)) in blocks.iter().zip(&probe.calls) {
            for k in 0..block.chunk_frames * SUBSAMPLING {
                let global = out_row + k;
                if global >= frames {
                    break;
                }
                let want = (cached * SUBSAMPLING + k) as f32;
                assert_eq!(
                    logits[global * NUM_SPEAKERS],
                    want,
                    "Block ab {}",
                    block.start
                );
            }
            out_row += block.chunk_frames * SUBSAMPLING;
        }
        // Der Zwischenspeicher wurde gefuellt und ueberlief (Speicher 16 + FIFO 6).
        assert!(probe.calls.iter().any(|(_, cached, _)| *cached >= 16));
        assert!(probe.calls.iter().all(|(_, cached, _)| *cached <= 16 + 6));
    }

    #[test]
    fn the_last_block_is_cut_at_the_audio_not_padded_by_the_loop() {
        let mut probe = Probe::new();
        run(&secs(18.0), &mut probe, SMALL).unwrap();
        let (mel_rows, _, _) = probe.calls.last().unwrap();
        // 1801 Zeilen insgesamt; der letzte Block endet dort, das Modell fuellt auf.
        let blocks = plan_blocks(226, SMALL);
        let last = blocks.last().unwrap();
        assert_eq!(*mel_rows, 1_801 - last.start * SUBSAMPLING);
        // Jeder Block, dessen Vorausschau nicht ueber das Ende reicht, hat Block +
        // Vorausschau in ganzen Achtergruppen; nur Bloecke am Ende sind gekuerzt.
        let mut full = 0;
        for (block, (rows, _, _)) in blocks.iter().zip(&probe.calls) {
            if block.context_end < 226 {
                assert_eq!(rows % SUBSAMPLING, 0, "Block ab {}", block.start);
                full += 1;
            }
        }
        assert!(full >= 1, "kein Block ausserhalb des Endes");
    }

    #[test]
    fn audio_without_a_full_frame_runs_no_model_and_gives_no_logits() {
        for samples in [0usize, 1, 159] {
            let mut probe = Probe::new();
            let out = run_offline(
                &mut probe,
                &MelFrontend::new(),
                &vec![0.1; samples],
                small_cache(),
                SMALL,
                &|| false,
            )
            .unwrap();
            assert!(out.is_empty(), "{samples} Samples");
            assert!(probe.calls.is_empty(), "{samples} Samples: Modell gerufen");
        }
        // Ein Abbruch gilt auch hier.
        let mut probe = Probe::new();
        let err = run_offline(
            &mut probe,
            &MelFrontend::new(),
            &[],
            small_cache(),
            SMALL,
            &|| true,
        )
        .unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled));
        // Genau ein Rahmen (160 Samples) laeuft dagegen als Block.
        let mut probe = Probe::new();
        let out = run_offline(
            &mut probe,
            &MelFrontend::new(),
            &[0.1; 160],
            small_cache(),
            SMALL,
            &|| false,
        )
        .unwrap();
        assert_eq!(out.len(), NUM_SPEAKERS);
        assert_eq!(probe.calls.len(), 1);
    }

    #[test]
    fn cancel_before_the_first_block_runs_nothing() {
        let mut probe = Probe::new();
        let err = run_offline(
            &mut probe,
            &MelFrontend::new(),
            &secs(5.0),
            small_cache(),
            SMALL,
            &|| true,
        )
        .unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled));
        assert!(probe.calls.is_empty());
    }

    #[test]
    fn cancel_between_blocks_stops_without_partial_turns() {
        use std::cell::Cell;
        let mut probe = Probe::new();
        let asked = Cell::new(0);
        let err = run_offline(
            &mut probe,
            &MelFrontend::new(),
            &secs(18.0),
            small_cache(),
            SMALL,
            &|| {
                asked.set(asked.get() + 1);
                asked.get() > 3
            },
        )
        .unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled));
        assert_eq!(probe.calls.len(), 3, "genau die Bloecke vor dem Abbruch");
    }

    #[test]
    fn a_model_error_passes_through_and_stops_the_loop() {
        let mut probe = Probe::new();
        probe.fail_at = Some(2);
        let err = run(&secs(18.0), &mut probe, SMALL).unwrap_err();
        assert!(err.to_string().contains("kaputt"), "{err}");
        assert_eq!(probe.calls.len(), 2);
    }

    /// Jede der drei Ausgaben (Logits, Rahmen-Einbettungen, Stille-Einbettung)
    /// wird auf NaN/Inf geprueft: die beiden Einbettungen gehen in den
    /// Zwischenspeicher und von dort in jeden spaeteren Block.
    #[test]
    fn non_finite_model_output_is_an_error_before_it_poisons_the_cache() {
        for which in ["logits", "embeds", "silence"] {
            let mut probe = Probe::new();
            probe.poison = Some(which);
            let err = run(&secs(3.0), &mut probe, SMALL).unwrap_err();
            assert!(matches!(err, DiarizeError::Run(_)), "{which}: {err}");
            assert!(err.to_string().contains("NaN"), "{which}: {err}");
            assert_eq!(probe.calls.len(), 1, "{which}");
        }
    }

    /// Ein Abbruch, der erst waehrend des letzten Blocks kommt, wird nicht
    /// verschluckt: kein Ergebnis, sondern `Cancelled` (zuvor lieferte das
    /// direkte Logit-API hier noch alle Logits).
    #[test]
    fn a_cancel_during_the_last_block_still_cancels() {
        use std::cell::Cell;
        let pcm = secs(18.0);
        let mut counting = Probe::new();
        run(&pcm, &mut counting, SMALL).unwrap();
        let blocks = counting.calls.len();
        assert!(blocks > 1);

        let mut probe = Probe::new();
        let asked = Cell::new(0);
        let err = run_offline(
            &mut probe,
            &MelFrontend::new(),
            &pcm,
            small_cache(),
            SMALL,
            // Je Block eine Frage davor; die Frage nach dem letzten ist die (blocks + 1)-te.
            &|| {
                asked.set(asked.get() + 1);
                asked.get() > blocks
            },
        )
        .unwrap_err();
        assert!(matches!(err, DiarizeError::Cancelled), "{err}");
        assert_eq!(
            probe.calls.len(),
            blocks,
            "alle Bloecke liefen, das Ergebnis wird verworfen"
        );
        assert_eq!(asked.get(), blocks + 1);
    }

    /// Gegenprobe: wer bis zum Schluss nicht abbricht, bekommt alle Logits.
    #[test]
    fn without_a_cancel_the_last_question_changes_nothing() {
        let mut probe = Probe::new();
        let out = run(&secs(18.0), &mut probe, SMALL).unwrap();
        assert_eq!(out.len(), num_frames(secs(18.0).len()) * NUM_SPEAKERS);
    }

    #[test]
    fn wrong_output_sizes_are_an_error() {
        struct Short;
        impl BlockModel for Short {
            fn run(&mut self, _: &BlockInput<'_>) -> Result<BlockOutput, DiarizeError> {
                Ok(BlockOutput {
                    logits: vec![0.0; 8],
                    chunk_embeds: vec![0.0; HIDDEN],
                    silence: vec![0.0; HIDDEN],
                })
            }
        }
        let err = run_offline(
            &mut Short,
            &MelFrontend::new(),
            &secs(3.0),
            CacheConfig::offline(),
            BlockPlan::OFFLINE,
            &|| false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Groessen"), "{err}");
    }

    #[test]
    fn turns_are_runs_of_frames_above_one_half_numbered_by_channel() {
        // 6 Rahmen: Sprecher 1 aktiv 1..3, Sprecher 3 aktiv 2..6 (bis zum Ende).
        let mut logits = vec![-5.0f32; 6 * NUM_SPEAKERS];
        for f in 1..3 {
            logits[f * NUM_SPEAKERS] = 2.0;
        }
        for f in 2..6 {
            logits[f * NUM_SPEAKERS + 2] = 0.1;
        }
        let turns = turns_from_logits(&logits);
        assert_eq!(
            turns,
            vec![
                RawTurn {
                    t0_ms: 10,
                    t1_ms: 30,
                    speaker_id: 1
                },
                RawTurn {
                    t0_ms: 20,
                    t1_ms: 60,
                    speaker_id: 3
                },
            ]
        );
    }

    #[test]
    fn a_logit_of_exactly_zero_is_not_active_and_nan_never_is() {
        let mut logits = vec![0.0f32; 3 * NUM_SPEAKERS];
        logits[NUM_SPEAKERS] = f32::NAN;
        assert!(turns_from_logits(&logits).is_empty());
        assert!(turns_from_logits(&[]).is_empty());
    }

    #[test]
    fn overlapping_speakers_give_overlapping_turns_sorted_by_start() {
        let mut logits = vec![-9.0f32; 4 * NUM_SPEAKERS];
        for f in 0..4 {
            logits[f * NUM_SPEAKERS + 1] = 3.0;
        }
        for f in 1..3 {
            logits[f * NUM_SPEAKERS] = 3.0;
        }
        let turns = turns_from_logits(&logits);
        assert_eq!(
            turns
                .iter()
                .map(|t| (t.t0_ms, t.speaker_id))
                .collect::<Vec<_>>(),
            vec![(0, 2), (10, 1)]
        );
    }
}
