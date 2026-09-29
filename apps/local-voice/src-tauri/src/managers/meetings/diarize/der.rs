//! M3-P3a: Diarization Error Rate und das Werkzeug `--eval-diarization`.
//!
//! DER wie im Spike (`lva-spikes/m3/der.py`, md-eval-Semantik wie im
//! NVIDIA-Scorer): 10-ms-Raster, Kragen +-`collar` um jede Referenzgrenze
//! (dort wird nicht gewertet), Ueberlappung zaehlt mit, optimale 1:1-Zuordnung
//! der Sprecher (Hungarian). DER = (Miss + FA + Verwechslung) / Referenzsprechzeit.
//! Rundung der Rahmengrenzen wie Python `round` (Banker's Rounding), damit
//! die Gegenprobe mit `der.py` rahmengenau passt.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Value};

use super::postproc::{self, RawTurn};
use super::{audio_ms, DiarizeEngine, DiarizeError, DiarizeParams, Turn};

/// Rasterweite (s).
pub const STEP_S: f64 = 0.01;
/// Standard-Kragen (s) um jede Referenzgrenze.
pub const DEFAULT_COLLAR_S: f64 = 0.25;
/// Ziel AK7: gewichtete DER der deutschen Testdateien (Prozent).
pub const TARGET_DE_PCT: f64 = 5.0;
/// Ziel AK7: gewichtete DER des AMI-Pruefteils (Prozent).
pub const TARGET_AMI_TEST_PCT: f64 = 15.0;

/// Ein RTTM-Segment.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_s: f64,
    pub end_s: f64,
    pub speaker: String,
}

/// `SPEAKER <uri> <ch> <start> <dur> <NA> <NA> <spk> ...` -> Segmente.
/// Andere Zeilen und kaputte Zahlen werden uebersprungen.
pub fn parse_rttm(text: &str) -> Vec<Segment> {
    text.lines()
        .filter_map(|line| {
            let p: Vec<&str> = line.split_whitespace().collect();
            if p.len() < 8 || p[0] != "SPEAKER" {
                return None;
            }
            let start: f64 = p[3].parse().ok()?;
            let dur: f64 = p[4].parse().ok()?;
            (start.is_finite() && dur.is_finite()).then(|| Segment {
                start_s: start,
                end_s: start + dur,
                speaker: p[7].to_string(),
            })
        })
        .collect()
}

/// Turns als RTTM (Format wie `der.py::write_rttm`, Sprecher `S<n>`).
pub fn turns_to_rttm(uri: &str, turns: &[Turn]) -> String {
    let mut sorted = turns.to_vec();
    sorted.sort_by_key(|t| (t.start_ms, t.end_ms, t.speaker));
    sorted
        .iter()
        .filter(|t| t.end_ms > t.start_ms)
        .map(|t| {
            format!(
                "SPEAKER {uri} 1 {:.3} {:.3} <NA> <NA> S{} <NA> <NA>\n",
                t.start_ms as f64 / 1000.0,
                (t.end_ms - t.start_ms) as f64 / 1000.0,
                t.speaker
            )
        })
        .collect()
}

pub fn turns_to_segments(turns: &[Turn]) -> Vec<Segment> {
    turns
        .iter()
        .map(|t| Segment {
            start_s: t.start_ms as f64 / 1000.0,
            end_s: t.end_ms as f64 / 1000.0,
            speaker: format!("S{}", t.speaker),
        })
        .collect()
}

/// Rahmenzaehler einer Datei (gewertete Rahmen zu 10 ms).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DerCounts {
    /// Referenz-Sprecherrahmen (Nenner, mit Ueberlappung mehrfach).
    pub total: u64,
    pub miss: u64,
    pub fa: u64,
    pub conf: u64,
    pub ref_speakers: usize,
    pub hyp_speakers: usize,
}

impl DerCounts {
    fn pct(&self, x: u64) -> f64 {
        x as f64 / self.total.max(1) as f64 * 100.0
    }
    pub fn der_pct(&self) -> f64 {
        self.pct(self.miss + self.fa + self.conf)
    }
    pub fn miss_pct(&self) -> f64 {
        self.pct(self.miss)
    }
    pub fn fa_pct(&self) -> f64 {
        self.pct(self.fa)
    }
    pub fn conf_pct(&self) -> f64 {
        self.pct(self.conf)
    }
    pub fn ref_speech_s(&self) -> f64 {
        self.total as f64 * STEP_S
    }
    /// Summe fuer die nach Referenzsprechzeit gewichtete DER.
    pub fn add(&mut self, o: &DerCounts) {
        self.total += o.total;
        self.miss += o.miss;
        self.fa += o.fa;
        self.conf += o.conf;
    }
}

fn frame(t_s: f64) -> i64 {
    (t_s / STEP_S).round_ties_even() as i64
}

fn activity(segs: &[Segment], labels: &[String], n: usize) -> Vec<Vec<bool>> {
    let mut m = vec![vec![false; n]; labels.len()];
    for s in segs {
        let Some(row) = labels.iter().position(|l| *l == s.speaker) else {
            continue;
        };
        let a = frame(s.start_s).max(0) as usize;
        let b = (frame(s.end_s).max(0) as usize).min(n);
        if b > a {
            m[row][a..b].iter_mut().for_each(|x| *x = true);
        }
    }
    m
}

fn labels_of(segs: &[Segment]) -> Vec<String> {
    let mut l: Vec<String> = segs.iter().map(|s| s.speaker.clone()).collect();
    l.sort();
    l.dedup();
    l
}

/// DER-Zaehler einer Datei.
pub fn score(reference: &[Segment], hypothesis: &[Segment], collar_s: f64) -> DerCounts {
    let end = reference
        .iter()
        .chain(hypothesis)
        .map(|s| s.end_s)
        .fold(0.0_f64, f64::max);
    let n = (end / STEP_S).ceil().max(0.0) as usize + 1;
    let rl = labels_of(reference);
    let hl = labels_of(hypothesis);
    let r = activity(reference, &rl, n);
    let h = activity(hypothesis, &hl, n);

    let mut scored = vec![true; n];
    let c = frame(collar_s).max(0);
    for s in reference {
        for t in [s.start_s, s.end_s] {
            let f = frame(t);
            let a = (f - c).max(0) as usize;
            let b = ((f + c).max(0) as usize).min(n);
            if b > a {
                scored[a..b].iter_mut().for_each(|x| *x = false);
            }
        }
    }

    let mut out = DerCounts {
        ref_speakers: rl.len(),
        hyp_speakers: hl.len(),
        ..Default::default()
    };
    let mut overlap = vec![vec![0i64; hl.len()]; rl.len()];
    let mut min_sum = 0u64;
    for f in (0..n).filter(|&f| scored[f]) {
        let nr = r.iter().filter(|row| row[f]).count() as u64;
        let nh = h.iter().filter(|row| row[f]).count() as u64;
        out.total += nr;
        out.miss += nr.saturating_sub(nh);
        out.fa += nh.saturating_sub(nr);
        min_sum += nr.min(nh);
        if nr > 0 && nh > 0 {
            for (i, ri) in r.iter().enumerate() {
                if ri[f] {
                    for (j, hj) in h.iter().enumerate() {
                        if hj[f] {
                            overlap[i][j] += 1;
                        }
                    }
                }
            }
        }
    }
    let correct = if rl.is_empty() || hl.is_empty() {
        0
    } else {
        max_assignment(&overlap)
    };
    out.conf = min_sum.saturating_sub(correct as u64);
    out
}

/// Maximale Summe einer 1:1-Zuordnung Zeilen -> Spalten (rechteckig erlaubt).
/// Hungarian mit Potentialen, O(k^3) auf der quadratisch aufgefuellten Matrix.
pub fn max_assignment(w: &[Vec<i64>]) -> i64 {
    let rows = w.len();
    let cols = w.first().map_or(0, |r| r.len());
    let k = rows.max(cols);
    if k == 0 {
        return 0;
    }
    let maxv = w.iter().flatten().copied().max().unwrap_or(0).max(0);
    // Kosten = maxv - Gewicht (Auffuellung: Gewicht 0).
    let cost = |i: usize, j: usize| -> i64 {
        maxv - w.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0)
    };
    const INF: i64 = i64::MAX / 4;
    let mut u = vec![0i64; k + 1];
    let mut v = vec![0i64; k + 1];
    let mut p = vec![0usize; k + 1]; // p[j] = Zeile (1-basiert) an Spalte j
    let mut way = vec![0usize; k + 1];
    for i in 1..=k {
        p[0] = i;
        let mut j0 = 0usize;
        let mut minv = vec![INF; k + 1];
        let mut used = vec![false; k + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = INF;
            let mut j1 = 0usize;
            for j in 1..=k {
                if !used[j] {
                    let cur = cost(i0 - 1, j - 1) - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=k {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    (1..=k)
        .filter(|&j| p[j] >= 1 && p[j] <= rows && j <= cols)
        .map(|j| w[p[j] - 1][j - 1])
        .sum()
}

// --------------------------------------------------------------- Werkzeug

/// Gruppe einer Korpusdatei nach Namenspraefix: `ami_test_*` (Pruefteil,
/// AK7), andere `ami_*` (Entwicklung), sonst deutsch (`de`).
pub fn group_of(name: &str) -> &'static str {
    if name.starts_with("ami_test_") {
        "ami_test"
    } else if name.starts_with("ami_") {
        "ami_dev"
    } else {
        "de"
    }
}

/// Paare `<name>.wav` + `<name>.rttm` im Ordner (nach Namen sortiert) und
/// WAVs ohne Referenz.
pub fn find_pairs(dir: &Path) -> std::io::Result<(Vec<(String, PathBuf, PathBuf)>, Vec<String>)> {
    let mut pairs = Vec::new();
    let mut orphans = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("wav"))
        })
        .collect();
    entries.sort();
    for wav in entries {
        let Some(name) = wav.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        let rttm = wav.with_extension("rttm");
        if rttm.is_file() {
            pairs.push((name, wav, rttm));
        } else {
            orphans.push(name);
        }
    }
    Ok((pairs, orphans))
}

pub struct EvalOptions {
    pub collar_s: f64,
    /// Hypothesen-RTTMs hierhin schreiben (Gegenprobe mit `der.py`).
    pub rttm_out: Option<PathBuf>,
    pub model: String,
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

/// Wertet alle Paare im Ordner aus. Rueckgabe: (Exit-Code, JSON).
/// Exit 0 Ziele erfuellt, 3 verfehlt, 1 Fehler (mind. eine Datei nicht
/// auswertbar), 2 keine Paare.
pub fn run_eval<E: DiarizeEngine + ?Sized>(
    engine: &mut E,
    dir: &Path,
    params: &DiarizeParams,
    opts: &EvalOptions,
    load_wav: impl Fn(&Path) -> Result<Vec<f32>, String>,
) -> (i32, Value) {
    let (pairs, orphans) = match find_pairs(dir) {
        Ok(x) => x,
        Err(e) => {
            return (
                2,
                json!({"mode": "eval_diarization", "error": format!("{}: {e}", dir.display())}),
            )
        }
    };
    if pairs.is_empty() {
        return (
            2,
            json!({"mode": "eval_diarization", "error": format!("keine Paare <name>.wav + <name>.rttm in {}", dir.display())}),
        );
    }
    if let Some(out) = &opts.rttm_out {
        if let Err(e) = std::fs::create_dir_all(out) {
            return (
                1,
                json!({"mode": "eval_diarization", "error": format!("{}: {e}", out.display())}),
            );
        }
    }

    let mut files = Vec::new();
    let mut groups: BTreeMap<&'static str, (DerCounts, DerCounts, f64, f64)> = BTreeMap::new();
    let mut failed = 0usize;
    for (name, wav, rttm) in &pairs {
        let group = group_of(name);
        eprintln!("eval-diarization: {name}");
        match eval_one(engine, name, wav, rttm, params, opts, &load_wav) {
            Ok((row, post, raw, audio_s, secs)) => {
                let g = groups.entry(group).or_default();
                g.0.add(&post);
                g.1.add(&raw);
                g.2 += audio_s;
                g.3 += secs;
                let mut row = row;
                row["group"] = json!(group);
                files.push(row);
            }
            Err(e) => {
                failed += 1;
                files.push(json!({"file": name, "group": group, "error": e}));
            }
        }
    }

    let mut weighted = serde_json::Map::new();
    let mut weighted_raw = serde_json::Map::new();
    let mut rtf = serde_json::Map::new();
    let mut ami = (DerCounts::default(), DerCounts::default());
    for (g, (post, raw, audio_s, secs)) in &groups {
        weighted.insert(g.to_string(), json!(r2(post.der_pct())));
        weighted_raw.insert(g.to_string(), json!(r2(raw.der_pct())));
        rtf.insert(g.to_string(), json!(r2(audio_s / secs.max(1e-9))));
        if g.starts_with("ami") {
            ami.0.add(post);
            ami.1.add(raw);
        }
    }
    if groups.keys().any(|g| g.starts_with("ami")) {
        weighted.insert("ami".into(), json!(r2(ami.0.der_pct())));
        weighted_raw.insert("ami".into(), json!(r2(ami.1.der_pct())));
    }

    let de = groups.get("de").map(|g| g.0.der_pct());
    let ami_test = groups.get("ami_test").map(|g| g.0.der_pct());
    let de_ok = de.is_none_or(|x| x <= TARGET_DE_PCT);
    let ami_ok = ami_test.is_none_or(|x| x <= TARGET_AMI_TEST_PCT);
    let code = if failed > 0 {
        1
    } else if de_ok && ami_ok {
        0
    } else {
        3
    };
    let payload = json!({
        "mode": "eval_diarization",
        "model": opts.model,
        "backend": engine.describe(),
        "collar_s": opts.collar_s,
        "params": {"gap_ms": params.gap_ms, "pad_ms": params.pad_ms,
                   "preset": format!("{:?}", params.preset), "threads": params.threads},
        "files": files,
        "skipped_without_rttm": orphans,
        "failed": failed,
        "weighted_der": weighted,
        "weighted_der_raw": weighted_raw,
        "rtf": rtf,
        "targets": {
            "de_max_pct": TARGET_DE_PCT, "de_ok": de_ok,
            "ami_test_max_pct": TARGET_AMI_TEST_PCT, "ami_test_ok": ami_ok,
        },
    });
    (code, payload)
}

type OneResult = (Value, DerCounts, DerCounts, f64, f64);

fn eval_one<E: DiarizeEngine + ?Sized>(
    engine: &mut E,
    name: &str,
    wav: &Path,
    rttm: &Path,
    params: &DiarizeParams,
    opts: &EvalOptions,
    load_wav: &impl Fn(&Path) -> Result<Vec<f32>, String>,
) -> Result<OneResult, String> {
    let reference =
        parse_rttm(&std::fs::read_to_string(rttm).map_err(|e| format!("{}: {e}", rttm.display()))?);
    let pcm = load_wav(wav)?;
    let dur_ms = audio_ms(&pcm);
    let audio_s = dur_ms as f64 / 1000.0;
    let started = Instant::now();
    let raw: Vec<RawTurn> = engine
        .raw_turns(&pcm, params.preset, params.cancel.as_ref())
        .map_err(|e: DiarizeError| e.to_string())?;
    let secs = started.elapsed().as_secs_f64();
    drop(pcm);
    let post_turns = postproc::apply(&raw, params.gap_ms, params.pad_ms, dur_ms);
    let raw_turns = postproc::apply_raw(&raw, dur_ms);
    let post = score(&reference, &turns_to_segments(&post_turns), opts.collar_s);
    let rawc = score(&reference, &turns_to_segments(&raw_turns), opts.collar_s);
    if let Some(out) = &opts.rttm_out {
        write_atomic(
            &out.join(format!("{name}.rttm")),
            &turns_to_rttm(name, &post_turns),
        )?;
        write_atomic(
            &out.join(format!("{name}.raw.rttm")),
            &turns_to_rttm(name, &raw_turns),
        )?;
    }
    let row = json!({
        "file": name,
        "der": r2(post.der_pct()), "miss": r2(post.miss_pct()), "fa": r2(post.fa_pct()),
        "conf": r2(post.conf_pct()),
        "ref_speakers": post.ref_speakers, "hyp_speakers": post.hyp_speakers,
        "der_raw": r2(rawc.der_pct()), "hyp_speakers_raw": rawc.hyp_speakers,
        "ref_speech_s": r2(post.ref_speech_s()),
        "audio_s": r2(audio_s), "ms": (secs * 1000.0).round() as u64,
        "rtf": r2(audio_s / secs.max(1e-9)), "turns": post_turns.len(),
    });
    Ok((row, post, rawc, audio_s, secs))
}

/// Erst in eine Nachbardatei schreiben, dann umbenennen: ein Abbruch oder
/// volle Platte hinterlaesst nie eine halbe Datei unter dem Zielnamen.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|x| x.to_str()).unwrap_or("out")
    ));
    std::fs::write(&tmp, text).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", tmp.display())
    })?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

/// Kurzfassung fuer die Konsole (ohne `--json`).
pub fn summary_lines(payload: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(files) = payload["files"].as_array() {
        for f in files {
            if let Some(err) = f["error"].as_str() {
                lines.push(format!(
                    "{:28} FEHLER {err}",
                    f["file"].as_str().unwrap_or("?")
                ));
            } else {
                lines.push(format!(
                    "{:28} DER {:6.2} (roh {:6.2})  miss {:5.2} fa {:5.2} conf {:5.2}  spk {}/{}  {:.0}x",
                    f["file"].as_str().unwrap_or("?"),
                    f["der"].as_f64().unwrap_or(f64::NAN),
                    f["der_raw"].as_f64().unwrap_or(f64::NAN),
                    f["miss"].as_f64().unwrap_or(f64::NAN),
                    f["fa"].as_f64().unwrap_or(f64::NAN),
                    f["conf"].as_f64().unwrap_or(f64::NAN),
                    f["hyp_speakers"],
                    f["ref_speakers"],
                    f["rtf"].as_f64().unwrap_or(f64::NAN),
                ));
            }
        }
    }
    lines.push(format!("weighted_der {}", payload["weighted_der"]));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::meetings::diarize::Preset;
    use transcribe_cpp::CancelToken;

    fn seg(s: f64, e: f64, spk: &str) -> Segment {
        Segment {
            start_s: s,
            end_s: e,
            speaker: spk.to_string(),
        }
    }

    #[test]
    fn perfect_hypothesis_is_zero() {
        let r = vec![seg(0.0, 5.0, "A"), seg(6.0, 9.0, "B")];
        let c = score(&r, &r, 0.25);
        assert_eq!(c.der_pct(), 0.0);
        assert!(c.total > 0);
    }

    #[test]
    fn swapped_labels_are_still_zero() {
        let r = vec![seg(0.0, 5.0, "A"), seg(6.0, 9.0, "B")];
        let h = vec![seg(0.0, 5.0, "S2"), seg(6.0, 9.0, "S1")];
        assert_eq!(score(&r, &h, 0.25).der_pct(), 0.0);
    }

    #[test]
    fn empty_hypothesis_is_all_miss() {
        let r = vec![seg(0.0, 5.0, "A"), seg(6.0, 9.0, "B")];
        let c = score(&r, &[], 0.25);
        assert_eq!(c.der_pct(), 100.0);
        assert_eq!(c.miss, c.total);
        assert_eq!(c.hyp_speakers, 0);
    }

    #[test]
    fn the_collar_hides_boundary_errors() {
        // Hypothese 0,2 s zu spaet und zu frueh: ganz im Kragen von 0,25 s.
        let r = vec![seg(1.0, 5.0, "A")];
        let h = vec![seg(1.2, 4.8, "X")];
        assert_eq!(score(&r, &h, 0.25).der_pct(), 0.0);
        // Ohne Kragen: 0,4 s von 4 s fehlen = 10 %.
        let c = score(&r, &h, 0.0);
        assert!((c.der_pct() - 10.0).abs() < 1e-9, "{}", c.der_pct());
        assert!((c.miss_pct() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn overlap_counts_both_speakers() {
        // A 0-4, B 2-6 (Ueberlappung 2 s). Hypothese erkennt nur einen
        // Sprecher 0-6 -> in der Ueberlappung fehlt einer (2 s Miss von 8 s).
        let r = vec![seg(0.0, 4.0, "A"), seg(2.0, 6.0, "B")];
        let h = vec![seg(0.0, 6.0, "S1")];
        let c = score(&r, &h, 0.0);
        assert_eq!(c.total, 800);
        assert_eq!(c.miss, 200);
        assert_eq!(c.fa, 0);
        // S1 wird A zugeordnet (400 Rahmen); B-only 4-6 s ist Verwechslung.
        assert_eq!(c.conf, 200);
        assert!((c.der_pct() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn confusion_and_false_alarm_are_counted() {
        let r = vec![seg(0.0, 2.0, "A"), seg(2.0, 4.0, "B")];
        // Alles einem Sprecher, dazu 1 s Sprache in Stille.
        let h = vec![seg(0.0, 4.0, "S1"), seg(5.0, 6.0, "S2")];
        let c = score(&r, &h, 0.0);
        assert_eq!((c.total, c.miss, c.fa, c.conf), (400, 0, 100, 200));
    }

    #[test]
    fn hungarian_finds_the_best_rectangular_assignment() {
        assert_eq!(max_assignment(&[vec![5, 1], vec![4, 3]]), 8);
        assert_eq!(max_assignment(&[vec![1, 9, 2]]), 9);
        assert_eq!(max_assignment(&[vec![7], vec![8], vec![1]]), 8);
        assert_eq!(
            max_assignment(&[vec![3, 3, 0], vec![0, 3, 3], vec![3, 0, 3]]),
            9
        );
        assert_eq!(max_assignment(&[]), 0);
    }

    #[test]
    fn rttm_round_trip_and_rounding_like_python() {
        let text =
            "SPEAKER x 1 0.125 1.000 <NA> <NA> A <NA> <NA>\nJUNK\nSPEAKER x 1 bad 1 <NA> <NA> B\n";
        let segs = parse_rttm(text);
        assert_eq!(segs, vec![seg(0.125, 1.125, "A")]);
        // Python round(12.5) = 12 (Banker's), nicht 13.
        assert_eq!(frame(0.125), 12);
        let out = turns_to_rttm(
            "u",
            &[Turn {
                start_ms: 1500,
                end_ms: 2750,
                speaker: 2,
            }],
        );
        assert_eq!(out, "SPEAKER u 1 1.500 1.250 <NA> <NA> S2 <NA> <NA>\n");
    }

    #[test]
    fn groups_follow_the_file_prefix() {
        assert_eq!(group_of("ami_test_ES2004a"), "ami_test");
        assert_eq!(group_of("ami_dev_EN2002b"), "ami_dev");
        assert_eq!(group_of("ami_EN2002b"), "ami_dev");
        assert_eq!(group_of("scene1_status_mix"), "de");
    }

    /// Attrappe: gibt die Referenz einer Datei als Hypothese zurueck.
    struct Oracle(Vec<RawTurn>);
    impl DiarizeEngine for Oracle {
        fn raw_turns(
            &mut self,
            _: &[f32],
            _: Preset,
            _: Option<&CancelToken>,
        ) -> Result<Vec<RawTurn>, DiarizeError> {
            Ok(self.0.clone())
        }
        fn describe(&self) -> String {
            "orakel".into()
        }
    }

    #[test]
    fn eval_tool_scores_pairs_writes_rttm_and_sets_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("hyp");
        std::fs::write(dir.path().join("a_mix.wav"), b"x").unwrap();
        std::fs::write(
            dir.path().join("a_mix.rttm"),
            "SPEAKER a 1 0.000 4.000 <NA> <NA> A <NA> <NA>\nSPEAKER a 1 5.000 3.000 <NA> <NA> B <NA> <NA>\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("ohne_referenz.wav"), b"x").unwrap();
        let mut engine = Oracle(vec![
            RawTurn {
                t0_ms: 0,
                t1_ms: 4000,
                speaker_id: 3,
            },
            RawTurn {
                t0_ms: 5000,
                t1_ms: 8000,
                speaker_id: 1,
            },
        ]);
        let params = DiarizeParams::new("m.gguf");
        let opts = EvalOptions {
            collar_s: 0.25,
            rttm_out: Some(out.clone()),
            model: "test".into(),
        };
        let (code, payload) = run_eval(&mut engine, dir.path(), &params, &opts, |_| {
            Ok(vec![0.0; 10 * super::super::SAMPLE_RATE])
        });
        assert_eq!(code, 0, "{payload}");
        assert_eq!(payload["files"][0]["der"], json!(0.0));
        assert_eq!(payload["files"][0]["hyp_speakers"], json!(2));
        assert_eq!(payload["weighted_der"]["de"], json!(0.0));
        assert_eq!(payload["skipped_without_rttm"], json!(["ohne_referenz"]));
        assert_eq!(payload["backend"], json!("orakel"));
        let hyp = std::fs::read_to_string(out.join("a_mix.rttm")).unwrap();
        assert!(hyp.starts_with("SPEAKER a_mix 1 0.000 4.100"), "{hyp}");
        assert!(!out.join("a_mix.rttm.tmp").exists());

        // Eine unlesbare WAV: Datei als Fehler, Exit 1, Rest laeuft weiter.
        let (code, payload) = run_eval(&mut engine, dir.path(), &params, &opts, |_| {
            Err("kaputt".to_string())
        });
        assert_eq!(code, 1);
        assert_eq!(payload["failed"], json!(1));

        // Leerer Ordner: Exit 2.
        let empty = tempfile::tempdir().unwrap();
        let (code, _) = run_eval(&mut engine, empty.path(), &params, &opts, |_| Ok(vec![]));
        assert_eq!(code, 2);
    }

    #[test]
    fn a_missed_target_is_exit_3() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s.wav"), b"x").unwrap();
        std::fs::write(
            dir.path().join("s.rttm"),
            "SPEAKER s 1 0.000 8.000 <NA> <NA> A <NA> <NA>\n",
        )
        .unwrap();
        let mut engine = Oracle(vec![]); // hoert nichts -> 100 % Miss
        let opts = EvalOptions {
            collar_s: 0.25,
            rttm_out: None,
            model: "test".into(),
        };
        let (code, payload) = run_eval(
            &mut engine,
            dir.path(),
            &DiarizeParams::new("m.gguf"),
            &opts,
            |_| Ok(vec![0.0; 10 * super::super::SAMPLE_RATE]),
        );
        assert_eq!(code, 3, "{payload}");
        assert_eq!(payload["targets"]["de_ok"], json!(false));
    }
}
