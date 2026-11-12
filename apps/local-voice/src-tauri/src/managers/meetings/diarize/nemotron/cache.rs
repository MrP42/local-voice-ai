//! Sprecher-Zwischenspeicher "Arrival-Order Speaker Cache" (AOSC) mit
//! FIFO-Warteschlange (Paket N1), nach Streaming Sortformer und der Klasse
//! `Nemotron3DiarizationSpeakerCache` aus transformers (Apache-2.0).
//!
//! Jeder Block sieht neben seinen eigenen Rahmen die Rahmen des
//! Zwischenspeichers (die wichtigsten Rahmen frueherer Bloecke, nach Sprecher
//! gruppiert) und der FIFO (die letzten Rahmen). Nach jedem Block rutschen die
//! Rahmen des Blocks in die FIFO; laeuft sie ueber, wandern die aeltesten in den
//! Zwischenspeicher, und ist der voll, verdichtet [`SpeakerCache::update`] ihn
//! auf die `cache_length` wichtigsten Rahmen.
//!
//! Wichtigkeit eines Rahmens je Sprecher (`frame_scores`): Log-Quotenverhaeltnis
//! `ln p - ln(1 - p)` plus die Summe der Gegenwahrscheinlichkeiten aller
//! Sprecher (Rahmen mit Ueberlappung zaehlen weniger), nur fuer Rahmen mit
//! `p > 0,5`. Die besten Rahmen je Sprecher bekommen einen Bonus, damit jeder
//! Sprecher Platz behaelt (stark) und keiner den Speicher ueberschwemmt (schwach).
//! Pro Sprecher bleiben `silence_frames` Plaetze fuer die gelernte Stille
//! reserviert.
//!
//! Gleichstand: bei exakt gleichen Werten gewinnt der kleinere Index. Die
//! Referenz (`torch.topk`) legt das nicht fest; bei Gleitkommawerten aus einem
//! Modell tritt er nur bei Saettigung auf und hat dann denselben Wert.

use super::NUM_SPEAKERS;

/// Feste Zahlen des Zwischenspeichers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CacheConfig {
    pub num_speakers: usize,
    /// Mel-Rahmen je Encoder-Rahmen (Logits kommen mit dieser Rate je Encoder-Rahmen).
    pub subsampling: usize,
    /// Fassungsvermoegen der FIFO in Encoder-Rahmen.
    pub fifo_length: usize,
    /// So viele Rahmen wandern mindestens aus der ueberlaufenden FIFO in den Speicher.
    pub update_period: usize,
    pub cache_length: usize,
    /// Plaetze je Sprecher fuer die Stille-Einbettung im verdichteten Speicher.
    pub silence_frames: usize,
    /// Untere Klemme der Wahrscheinlichkeiten vor dem Logarithmus.
    pub score_threshold: f32,
    /// Bonus fuer die zuletzt hinzugekommenen Rahmen beim Verdichten.
    pub latest_boost: f32,
    pub strong_boost_rate: f64,
    pub weak_boost_rate: f64,
    pub min_positive_rate: f64,
}

impl CacheConfig {
    /// Offline-Arbeitspunkt des Modells: `fifo_length` und
    /// `speaker_cache_update_period` aus der obersten Ebene der `config.json`,
    /// der Rest aus `streaming_config`.
    pub const fn offline() -> Self {
        Self {
            num_speakers: NUM_SPEAKERS,
            subsampling: super::SUBSAMPLING,
            fifo_length: 40,
            update_period: 300,
            cache_length: 264,
            silence_frames: 1,
            score_threshold: 0.25,
            latest_boost: 0.05,
            strong_boost_rate: 0.75,
            weak_boost_rate: 1.5,
            min_positive_rate: 0.5,
        }
    }

    /// Anteil des Speichers je Sprecher ohne seine Stille-Plaetze.
    fn budget(&self) -> usize {
        (self.cache_length / self.num_speakers).saturating_sub(self.silence_frames)
    }

    fn frames_for(&self, rate: f64) -> usize {
        (self.budget() as f64 * rate).floor() as usize
    }

    /// Pruefung wie `validate_architecture` der Referenz.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.num_speakers == 0 || self.subsampling == 0 || self.cache_length == 0 {
            return Err("Zwischenspeicher: leere Groesse");
        }
        if self.cache_length < (1 + self.silence_frames) * self.num_speakers {
            return Err("Zwischenspeicher zu klein fuer die Stille-Plaetze");
        }
        Ok(())
    }
}

/// Zustand des Zwischenspeichers (ein Strom, Stapelgroesse 1).
pub struct SpeakerCache {
    cfg: CacheConfig,
    hidden: usize,
    min_positive: usize,
    strong_frames: usize,
    weak_frames: usize,
    /// `num_cache * hidden`
    embeds: Vec<f32>,
    /// `num_cache * num_speakers`
    probs: Vec<f32>,
    /// `num_fifo * hidden`
    fifo: Vec<f32>,
    num_cache: usize,
    num_fifo: usize,
    compressed: bool,
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Indizes der `k` groessten Werte; bei Gleichstand der kleinere Index zuerst.
///
/// Gleichstand ist hier der Normalfall an der Auswahlgrenze: nach dem ersten
/// Verdichten liegt derselbe Rahmen je Sprecher einmal im Speicher, im naechsten
/// Block sind das exakte Kopien mit gleichen Werten. `torch.topk` (Referenz)
/// legt nicht fest, welche Kopie bleibt; sie unterscheiden sich nur durch ihre
/// Stelle im Speicher, die Logits des naechsten Blocks aber spuerbar (gemessen
/// bis 0,8 Logit, 97,6 % gleiche Rahmen bei ueberlagerter Sprache von vier
/// Sprechern, 100 % bei zwei Sprechern). Die Regel hier ist fest und die der
/// Goldwerte (`make_reference.py::deterministic_topk`).
fn top_indices(values: &[f32], k: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|&a, &b| values[b].total_cmp(&values[a]).then(a.cmp(&b)));
    idx.truncate(k);
    idx
}

impl SpeakerCache {
    pub fn new(cfg: CacheConfig, hidden: usize) -> Self {
        Self {
            hidden,
            min_positive: cfg.frames_for(cfg.min_positive_rate),
            strong_frames: cfg.frames_for(cfg.strong_boost_rate),
            weak_frames: cfg.frames_for(cfg.weak_boost_rate),
            cfg,
            embeds: Vec::new(),
            probs: Vec::new(),
            fifo: Vec::new(),
            num_cache: 0,
            num_fifo: 0,
            compressed: false,
        }
    }

    /// Rahmen, die der naechste Block vorangestellt bekommt (Speicher + FIFO).
    pub fn cached_frames(&self) -> usize {
        self.num_cache + self.num_fifo
    }

    /// Haengt Speicher- und FIFO-Rahmen (in dieser Reihenfolge) an `out`.
    pub fn fill_cached(&self, out: &mut Vec<f32>) {
        out.extend_from_slice(&self.embeds[..self.num_cache * self.hidden]);
        out.extend_from_slice(&self.fifo[..self.num_fifo * self.hidden]);
    }

    pub fn num_cache(&self) -> usize {
        self.num_cache
    }

    pub fn num_fifo(&self) -> usize {
        self.num_fifo
    }

    pub fn is_compressed(&self) -> bool {
        self.compressed
    }

    pub fn cache_embeds(&self) -> &[f32] {
        &self.embeds[..self.num_cache * self.hidden]
    }

    pub fn cache_probs(&self) -> &[f32] {
        &self.probs[..self.num_cache * self.cfg.num_speakers]
    }

    pub fn fifo_embeds(&self) -> &[f32] {
        &self.fifo[..self.num_fifo * self.hidden]
    }

    /// Wahrscheinlichkeiten je Encoder-Rahmen: Mittel der Sigmoid-Werte ueber
    /// `subsampling` Logit-Zeilen, Auffuellrahmen (`mask` falsch) auf 0.
    fn pool_probs(&self, logits: &[f32], mask: &[bool]) -> Vec<f32> {
        let (s, sub) = (self.cfg.num_speakers, self.cfg.subsampling);
        let mut probs = Vec::with_capacity(mask.len() * s);
        for (frame, &valid) in mask.iter().enumerate() {
            for spk in 0..s {
                if !valid {
                    probs.push(0.0);
                    continue;
                }
                let sum: f32 = (0..sub)
                    .map(|j| sigmoid(logits[(frame * sub + j) * s + spk]))
                    .sum();
                probs.push(sum / sub as f32);
            }
        }
        probs
    }

    fn num_popped(&self, fifo_frames: usize) -> usize {
        if fifo_frames <= self.cfg.fifo_length {
            return 0;
        }
        let wanted = self
            .cfg
            .update_period
            .max(fifo_frames - self.cfg.fifo_length);
        wanted.min(fifo_frames)
    }

    /// Schiebt einen verarbeiteten Block in die FIFO; laeuft sie ueber, wandern
    /// die aeltesten Rahmen in den Speicher, und der wird bei Bedarf verdichtet.
    ///
    /// * `input_embeds`: Eingang des Encoders (`Rahmen * hidden`): zwischengespeicherte
    ///   Rahmen, Rueckblick, Block, Vorausschau;
    /// * `logits`: Logits des Blocks (`Rahmen * subsampling` Zeilen zu je `num_speakers`);
    /// * `silence`: gelernte Stille-Einbettung (`hidden`);
    /// * `num_chunk_frames`: Rahmen des Blocks hinter Speicher und Rueckblick; nur
    ///   die kommen in die FIFO (die Vorausschau wird im naechsten Block erneut gespeist);
    /// * `mask`: gueltige Rahmen von `input_embeds`;
    /// * `lookback`: Rueckblick-Rahmen zwischen Speicher und Block.
    pub fn update(
        &mut self,
        input_embeds: &[f32],
        logits: &[f32],
        silence: &[f32],
        num_chunk_frames: usize,
        mask: &[bool],
        lookback: usize,
    ) -> Result<(), &'static str> {
        let (h, s) = (self.hidden, self.cfg.num_speakers);
        let frames = mask.len();
        if input_embeds.len() != frames * h
            || logits.len() != frames * self.cfg.subsampling * s
            || silence.len() != h
        {
            return Err("Zwischenspeicher: Groessen passen nicht");
        }
        let (nc, nf) = (self.num_cache, self.num_fifo);
        let chunk_start = nc + nf + lookback;
        if chunk_start + num_chunk_frames > frames {
            return Err("Zwischenspeicher: Block ragt ueber den Eingang");
        }
        let probs = self.pool_probs(logits, mask);
        let chunk_embeds = &input_embeds[chunk_start * h..(chunk_start + num_chunk_frames) * h];

        let mut fifo_embeds = self.fifo[..nf * h].to_vec();
        fifo_embeds.extend_from_slice(chunk_embeds);
        let fifo_frames = nf + num_chunk_frames;
        let popped = self.num_popped(fifo_frames);
        if popped > 0 {
            let mut fifo_probs = probs[nc * s..(nc + nf) * s].to_vec();
            fifo_probs
                .extend_from_slice(&probs[chunk_start * s..(chunk_start + num_chunk_frames) * s]);
            let stored: &[f32] = if self.compressed {
                &self.probs[..nc * s]
            } else {
                &probs[..nc * s]
            };
            let mut cache_probs = stored.to_vec();
            cache_probs.extend_from_slice(&fifo_probs[..popped * s]);
            let mut cache_embeds = self.embeds[..nc * h].to_vec();
            cache_embeds.extend_from_slice(&fifo_embeds[..popped * h]);
            fifo_embeds.drain(..popped * h);

            if cache_embeds.len() / h > self.cfg.cache_length {
                let (e, p) = self.compress(&cache_embeds, &cache_probs, silence);
                cache_embeds = e;
                cache_probs = p;
                self.compressed = true;
            }
            self.num_cache = cache_embeds.len() / h;
            self.embeds = cache_embeds;
            self.probs = cache_probs;
        }
        self.num_fifo = fifo_embeds.len() / h;
        self.fifo = fifo_embeds;
        Ok(())
    }

    /// Wichtigkeit je Rahmen und Sprecher (`frames * num_speakers`, rahmenweise);
    /// `-inf` fuer Rahmen ohne Sprache und fuer ueberlappende Rahmen eines
    /// Sprechers, der schon genug eindeutige hat.
    pub fn frame_scores(&self, probs: &[f32]) -> Vec<f32> {
        let s = self.cfg.num_speakers;
        let frames = probs.len() / s;
        let thr = self.cfg.score_threshold;
        let mut scores = vec![f32::NEG_INFINITY; probs.len()];
        for f in 0..frames {
            let row = &probs[f * s..(f + 1) * s];
            let log_comp: Vec<f32> = row.iter().map(|&p| (1.0 - p).max(thr).ln()).collect();
            let sum_comp: f32 = log_comp.iter().sum();
            for (spk, &p) in row.iter().enumerate() {
                if p > 0.5 {
                    let log_p = p.max(thr).ln();
                    scores[f * s + spk] = log_p - log_comp[spk] + sum_comp + std::f32::consts::LN_2;
                }
            }
        }
        for spk in 0..s {
            let positive = (0..frames).filter(|&f| scores[f * s + spk] > 0.0).count();
            if positive < self.min_positive {
                continue;
            }
            for f in 0..frames {
                let v = &mut scores[f * s + spk];
                if *v <= 0.0 {
                    *v = f32::NEG_INFINITY;
                }
            }
        }
        scores
    }

    /// Die besten `k` Rahmen je Sprecher bekommen `boost` dazu.
    fn boost_scores(&self, scores: &mut [f32], frames: usize, k: usize, boost: f32) {
        let s = self.cfg.num_speakers;
        for spk in 0..s {
            let column: Vec<f32> = (0..frames).map(|f| scores[f * s + spk]).collect();
            for f in top_indices(&column, k.min(frames)) {
                scores[f * s + spk] += boost;
            }
        }
    }

    /// Behaelt die `cache_length` wichtigsten Rahmen, nach Sprecher gruppiert und
    /// je Sprecher in der urspruenglichen Reihenfolge; freie Plaetze und die
    /// Stille-Plaetze fuellt `silence`.
    fn compress(&self, embeds: &[f32], probs: &[f32], silence: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let (h, s) = (self.hidden, self.cfg.num_speakers);
        let n = embeds.len() / h;
        let mut scores = self.frame_scores(probs);
        for v in scores[self.cfg.cache_length.min(n) * s..].iter_mut() {
            *v += self.cfg.latest_boost;
        }
        let strong = -2.0 * 0.5f32.ln();
        let weak = -0.5f32.ln();
        self.boost_scores(&mut scores, n, self.strong_frames, strong);
        self.boost_scores(&mut scores, n, self.weak_frames, weak);

        // Sprecherweise hintereinander, am Ende jedes Sprechers die Stille-Plaetze.
        let scored = n + self.cfg.silence_frames;
        let sentinel = scored * s;
        let mut flat = Vec::with_capacity(scored * s);
        for spk in 0..s {
            flat.extend((0..n).map(|f| scores[f * s + spk]));
            flat.extend(std::iter::repeat(f32::INFINITY).take(self.cfg.silence_frames));
        }
        let mut picked: Vec<usize> = top_indices(&flat, self.cfg.cache_length)
            .into_iter()
            .map(|i| {
                if flat[i] == f32::NEG_INFINITY {
                    sentinel
                } else {
                    i
                }
            })
            .collect();
        picked.sort_unstable();

        let mut out_embeds = Vec::with_capacity(picked.len() * h);
        let mut out_probs = Vec::with_capacity(picked.len() * s);
        for i in picked {
            let frame = if i == sentinel {
                n
            } else {
                (i % scored).min(n)
            };
            if frame < n {
                out_embeds.extend_from_slice(&embeds[frame * h..(frame + 1) * h]);
                out_probs.extend_from_slice(&probs[frame * s..(frame + 1) * s]);
            } else {
                out_embeds.extend_from_slice(silence);
                out_probs.extend(std::iter::repeat(0.0).take(s));
            }
        }
        (out_embeds, out_probs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const REF: &str = include_str!("../../../../../tests/fixtures/nemotron/cache_ref.json");

    /// Gleicher Zahlengenerator wie `make_reference.py`: 32-bit-LCG, 24 Bit je Zug.
    struct Lcg(u32);

    impl Lcg {
        fn draw(&mut self) -> u32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            self.0 >> 8
        }
    }

    fn config(v: &Value) -> CacheConfig {
        let u = |k: &str| v[k].as_u64().unwrap() as usize;
        let f = |k: &str| v[k].as_f64().unwrap();
        CacheConfig {
            num_speakers: u("num_speakers"),
            subsampling: u("subsampling"),
            fifo_length: u("fifo_length"),
            update_period: u("update_period"),
            cache_length: u("cache_length"),
            silence_frames: u("silence_frames"),
            score_threshold: f("score_threshold") as f32,
            latest_boost: f("latest_boost") as f32,
            strong_boost_rate: f("strong_boost_rate"),
            weak_boost_rate: f("weak_boost_rate"),
            min_positive_rate: f("min_positive_rate"),
        }
    }

    fn ids(embeds: &[f32], hidden: usize) -> Vec<i64> {
        embeds.chunks_exact(hidden).map(|e| e[0] as i64).collect()
    }

    fn json_ids(v: &Value) -> Vec<i64> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_i64().unwrap())
            .collect()
    }

    fn replay(scenario: &Value) {
        let name = scenario["name"].as_str().unwrap();
        let cfg = config(&scenario["config"]);
        cfg.validate().unwrap();
        let hidden = scenario["hidden"].as_u64().unwrap() as usize;
        let chunk = scenario["chunk"].as_u64().unwrap() as usize;
        let bias: Vec<i32> = scenario["bias"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_i64().unwrap() as i32)
            .collect();
        let mut rng = Lcg(scenario["seed"].as_u64().unwrap() as u32);
        let mut cache = SpeakerCache::new(cfg, hidden);
        let silence = vec![-1.0f32; hidden];
        let mut compressed_steps = 0;

        for (i, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
            let lookahead = step["lookahead"].as_u64().unwrap() as usize;
            let lookback = step["lookback"].as_u64().unwrap() as usize;
            let mask_tail = step["mask_tail"].as_u64().unwrap() as usize;
            let base = (i * chunk) as i64;

            let mut input = Vec::new();
            cache.fill_cached(&mut input);
            let cached = cache.cached_frames();
            let first = base - lookback as i64;
            for id in first..base + (chunk + lookahead) as i64 {
                input.push(id as f32);
                input.push((id % 7) as f32);
                input.extend(std::iter::repeat(0.0).take(hidden.saturating_sub(2)));
            }
            let frames = input.len() / hidden;
            let mut mask = vec![true; frames];
            for m in mask.iter_mut().rev().take(mask_tail) {
                *m = false;
            }
            let mut logits = Vec::with_capacity(frames * cfg.subsampling * cfg.num_speakers);
            for _ in 0..frames * cfg.subsampling {
                for b in &bias {
                    let li = (rng.draw() % 385) as i32 - 192 + 16 * b;
                    logits.push(li as f32 / 16.0);
                }
            }
            assert_eq!(cached + lookback + chunk + lookahead, frames, "{name} {i}");
            cache
                .update(&input, &logits, &silence, chunk, &mask, lookback)
                .unwrap();

            let at = format!("{name}, Block {i}");
            assert_eq!(
                cache.num_cache(),
                step["num_cache"].as_u64().unwrap() as usize,
                "{at}: Rahmen im Speicher"
            );
            assert_eq!(
                cache.num_fifo(),
                step["num_fifo"].as_u64().unwrap() as usize,
                "{at}: Rahmen in der FIFO"
            );
            assert_eq!(
                cache.is_compressed(),
                step["compressed"].as_bool().unwrap(),
                "{at}: verdichtet"
            );
            // Rahmen UND Reihenfolge, auch wo derselbe Rahmen mehrfach im Speicher
            // liegt: das Golden nutzt dieselbe Auswahl bei Gleichstand wie `top_indices`
            // (kleinerer Index zuerst, `make_reference.py::deterministic_topk`).
            assert_eq!(
                ids(cache.cache_embeds(), hidden),
                json_ids(&step["cache_ids"]),
                "{at}: Speicherrahmen"
            );
            assert_eq!(
                ids(cache.fifo_embeds(), hidden),
                json_ids(&step["fifo_ids"]),
                "{at}: FIFO-Rahmen"
            );
            if let Some(want) = step["cache_probs"].as_array() {
                let got = cache.cache_probs();
                assert_eq!(got.len(), want.len(), "{at}: Wahrscheinlichkeiten");
                for (j, (g, w)) in got.iter().zip(want).enumerate() {
                    let w = w.as_f64().unwrap() as f32;
                    assert!((g - w).abs() < 2e-5, "{at}: Wert {j}: {g} statt {w}");
                }
            }
            compressed_steps += usize::from(cache.is_compressed());
        }
        assert!(compressed_steps >= 2, "{name}: Ueberlauf nicht erreicht");
    }

    #[test]
    fn small_scenario_follows_the_reference_through_overflow() {
        let reference: Value = serde_json::from_str(REF).unwrap();
        let scenarios = reference["scenarios"].as_array().unwrap();
        replay(scenarios.iter().find(|s| s["name"] == "small").unwrap());
    }

    #[test]
    fn small_scenario_without_lookback_and_with_lookback_are_both_covered() {
        let reference: Value = serde_json::from_str(REF).unwrap();
        let small = reference["scenarios"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "small")
            .unwrap();
        let steps = small["steps"].as_array().unwrap();
        assert!(steps.iter().any(|s| s["lookback"].as_u64().unwrap() > 0));
        assert!(steps.iter().any(|s| s["lookback"].as_u64().unwrap() == 0));
        assert!(steps.iter().any(|s| s["mask_tail"].as_u64().unwrap() > 0));
        // Vor dem ersten Ueberlauf ist der Speicher unverdichtet.
        assert!(steps.iter().any(|s| !s["compressed"].as_bool().unwrap()));
    }

    #[test]
    fn full_size_offline_scenario_follows_the_reference() {
        let reference: Value = serde_json::from_str(REF).unwrap();
        let scenarios = reference["scenarios"].as_array().unwrap();
        let full = scenarios.iter().find(|s| s["name"] == "offline").unwrap();
        assert_eq!(config(&full["config"]), CacheConfig::offline());
        replay(full);
        // Der Fall, auf den die Auswahlregel bei Gleichstand ankommt: derselbe Rahmen
        // mehrfach im Speicher.
        let last = full["steps"].as_array().unwrap().last().unwrap();
        let ids = json_ids(&last["cache_ids"]);
        let distinct: std::collections::HashSet<_> = ids.iter().collect();
        assert!(
            distinct.len() < ids.len(),
            "keine doppelten Rahmen im Speicher"
        );
    }

    #[test]
    fn frame_scores_match_the_reference_including_minus_infinity() {
        let reference: Value = serde_json::from_str(REF).unwrap();
        let case = &reference["scores"];
        let cfg = config(&case["config"]);
        let cache = SpeakerCache::new(cfg, 2);
        let probs: Vec<f32> = case["probs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_f64().unwrap() as f32)
            .collect();
        let got = cache.frame_scores(&probs);
        let want = case["scores"].as_array().unwrap();
        assert_eq!(got.len(), want.len());
        for (i, (g, w)) in got.iter().zip(want).enumerate() {
            match w.as_f64() {
                None => assert_eq!(*g, f32::NEG_INFINITY, "Wert {i}"),
                Some(w) => assert!((g - w as f32).abs() < 1e-5, "Wert {i}: {g} statt {w}"),
            }
        }
        assert!(
            want.iter().any(Value::is_null),
            "ohne -inf prueft der Test zu wenig"
        );
        assert!(want.iter().any(|w| !w.is_null()));
    }

    #[test]
    fn a_fresh_cache_is_empty_and_gives_no_cached_frames() {
        let c = SpeakerCache::new(CacheConfig::offline(), 4);
        let mut out = Vec::new();
        c.fill_cached(&mut out);
        assert!(out.is_empty());
        assert_eq!(c.cached_frames(), 0);
        assert!(!c.is_compressed());
    }

    #[test]
    fn wrong_sizes_are_an_error_not_a_panic() {
        let mut c = SpeakerCache::new(CacheConfig::offline(), 4);
        let silence = [0.0f32; 4];
        // Eingang mit 2 Rahmen, aber Logits fuer 1.
        assert!(c
            .update(&[0.0; 8], &[0.0; 64], &silence, 2, &[true, true], 0)
            .is_err());
        // Block ragt ueber den Eingang.
        assert!(c
            .update(&[0.0; 8], &[0.0; 128], &silence, 3, &[true, true], 0)
            .is_err());
        // Nichts davon hat den Zustand veraendert.
        assert_eq!(c.cached_frames(), 0);
    }

    #[test]
    fn ties_prefer_the_lower_index() {
        assert_eq!(top_indices(&[1.0, 3.0, 3.0, 2.0], 2), vec![1, 2]);
        assert_eq!(
            top_indices(&[f32::NEG_INFINITY, 0.0, f32::NEG_INFINITY], 3),
            vec![1, 0, 2]
        );
    }

    #[test]
    fn the_offline_config_matches_the_published_budget() {
        let c = CacheConfig::offline();
        c.validate().unwrap();
        assert_eq!((c.budget(), c.frames_for(c.min_positive_rate)), (32, 16));
        assert_eq!(
            (
                c.frames_for(c.strong_boost_rate),
                c.frames_for(c.weak_boost_rate)
            ),
            (24, 48)
        );
        let too_small = CacheConfig {
            cache_length: 8,
            ..c
        };
        assert!(too_small.validate().is_err());
    }
}
