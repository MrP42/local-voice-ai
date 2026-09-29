//! Performance-Werkzeug `--bench-search` (M4 §11): misst Listensuche, Wortsuche
//! und hybride Suche auf einer SYNTHETISCHEN Sandbox-Datenbank.
//!
//! - Die Datenbank entsteht in einem eigenen Temp-Ordner (`lva-bench-search-*`)
//!   und wird beim Verlassen der Funktion geloescht, auch bei einem Fehler
//!   (voller Datentraeger) oder einer Panik (Drop). Die produktive
//!   `meetings.db` wird nie geoeffnet.
//! - Der Wortschatz hat nur ~100 Woerter, der Text also viele haeufige Terme:
//!   das ist die SCHLECHTESTE Annahme fuer die Volltextsuche (echte Sprache ist
//!   guenstiger). Die Werte gelten als obere Schranke (M4 §14).
//! - `hybrid_ms` misst die Suche mit festem Abfragevektor, ohne Embedding-Aufruf.
//!
//! Speicherbedarf bei 500 x 200 Chunks: ~1,1 GB Platte im Temp-Ordner,
//! ~110 MB RAM fuer den int8-Index. Bei vollem Datentraeger endet der Lauf mit
//! Fehler und raeumt auf; ein anderes Laufwerk waehlt `--bench-dir`.

use anyhow::{anyhow, Result};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Instant;

use super::super::store::{MeetingSource, MeetingStore};
use super::chunking::{fts_query_words, ChunkDraft, ChunkSource};
use super::hybrid::hybrid_search_with_vector;
use super::index::{IndexState, MeetingFilter, STATUS_LEXICAL};
use super::vectors::{SplitMix64, VectorCache, COARSE_CANDIDATES};

pub const MODEL: &str = "bench-model";
/// Ziel aus dem Entwurf: p95 darunter, sonst Exit 3.
pub const P95_LIMIT_MS: f64 = 500.0;
const MAX_MEETINGS: usize = 20_000;
const MAX_CHUNKS_PER_MEETING: usize = 5_000;
const MAX_DIM: usize = 4_096;
const CHUNK_CHARS: usize = 1_000;

/// ~100 Woerter Geschaeftsdeutsch, mit Komposita und Umlauten, gleichverteilt
/// gezogen: jedes kommt in ~75 % der Chunks vor (Worst Case).
#[rustfmt::skip]
const VOCAB: &[&str] = &[
    "Budget", "Marketingbudget", "Kundenbetreuung", "Projekt", "Planung", "Termin", "Angebot",
    "Vertrag", "Rechnung", "Zahlung", "Preis", "Kosten", "Umsatz", "Vertrieb", "Einkauf",
    "Lieferung", "Qualität", "Prüfung", "Freigabe", "Entscheidung", "Aufgabe", "Verantwortung",
    "Zeitplan", "Meilenstein", "Risiko", "Ressource", "Team", "Abteilung", "Geschäftsführung",
    "Kunde", "Lieferant", "Partner", "Wettbewerb", "Markt", "Strategie", "Ziel", "Ergebnis",
    "Analyse", "Bericht", "Präsentation", "Workshop", "Schulung", "Einführung", "Migration",
    "Schnittstelle", "Datenbank", "Sicherheit", "Datenschutz", "Vertraulichkeit", "Support",
    "Wartung", "Betrieb", "Störung", "Änderung", "Anforderung", "Konzept", "Entwurf",
    "Umsetzung", "Test", "Abnahme", "Dokumentation", "Übergabe", "Rückmeldung", "Frage",
    "Antwort", "Vorschlag", "Bedenken", "Einwand", "Zustimmung", "Ablehnung", "nächste", "Woche",
    "Monat", "Quartal", "Jahr", "heute", "morgen", "gestern", "bitte", "danke", "genau",
    "richtig", "wichtig", "schnell", "langsam", "gut", "schlecht", "neu", "alt", "groß", "klein",
    "KI", "API", "Cloud", "Server", "Fehler", "Straße", "Übung",
];
/// Kommt nur in ~0,1 % der Chunks vor (seltener Suchbegriff).
const RARE: &str = "Sonderposten";

/// Die gemessenen Anfragen: selten, sehr haeufig, Kompositum-Teil, Zwei-Wort-UND,
/// Zwei-Buchstaben-Wort, Umlaut.
const QUERIES: &[(&str, &str)] = &[
    ("selten", "sonderposten"),
    ("haeufig", "budget"),
    ("kompositum", "betreuung"),
    ("zwei_worte", "budget planung"),
    ("zwei_buchstaben", "ki"),
    ("umlaut", "prüfung"),
];

#[derive(Clone, Debug)]
pub struct BenchConfig {
    pub meetings: usize,
    pub chunks_per_meeting: usize,
    pub dim: usize,
    /// Messlaeufe je Anfrage (nach einem Aufwaermlauf).
    pub iterations: usize,
    pub seed: u64,
    /// Ordner, in dem der Temp-Ordner entsteht (Standard: Temp-Verzeichnis des Systems).
    pub temp_root: Option<PathBuf>,
}

impl Default for BenchConfig {
    fn default() -> Self {
        Self {
            meetings: 500,
            chunks_per_meeting: 200,
            dim: 1_024,
            iterations: 20,
            seed: 42,
            temp_root: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct QueryStat {
    pub name: String,
    pub query: String,
    pub list_p50_ms: f64,
    pub list_p95_ms: f64,
    pub words_p50_ms: f64,
    pub words_p95_ms: f64,
    pub hybrid_p50_ms: f64,
    pub hybrid_p95_ms: f64,
    /// Besprechungen mit Treffern in der Listensuche (Plausibilitaet).
    pub list_hits: u32,
    pub hybrid_hits: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct BenchReport {
    pub mode: &'static str,
    pub profile: &'static str,
    pub meetings: usize,
    pub chunks_per_meeting: usize,
    pub chunks_total: usize,
    pub dim: usize,
    pub iterations: usize,
    pub build_ms: u64,
    pub db_mb: f64,
    pub vector_load_ms: u64,
    pub vector_index_mb: f64,
    /// Nur die int8-Stufe (Top 300 ueber alle Vektoren).
    pub topk_p50_ms: f64,
    pub ui_search_p50_ms: f64,
    pub ui_search_p95_ms: f64,
    pub lexical_p50_ms: f64,
    pub lexical_p95_ms: f64,
    pub hybrid_p50_ms: f64,
    pub hybrid_p95_ms: f64,
    /// Hybrid nur ueber 10 % der Besprechungen (Ordner-Scope).
    pub hybrid_scoped_p95_ms: f64,
    pub limit_ms: f64,
    pub pass: bool,
    pub per_query: Vec<QueryStat>,
}

impl BenchReport {
    /// 0 bei bestanden, 3 wenn ein p95 die Grenze erreicht (M4 §11).
    pub fn exit_code(&self) -> i32 {
        if self.pass {
            0
        } else {
            3
        }
    }
}

/// Nearest-Rank-Perzentil einer bereits sortierten Liste (Millisekunden).
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted[(rank - 1).min(sorted.len() - 1)]
}

fn sorted(mut samples: Vec<f64>) -> Vec<f64> {
    samples.sort_by(|a, b| a.total_cmp(b));
    samples
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1_000.0
}

fn chunk_text(rng: &mut SplitMix64) -> String {
    let mut text = String::with_capacity(CHUNK_CHARS + 32);
    while text.len() < CHUNK_CHARS {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(VOCAB[rng.below(VOCAB.len())]);
    }
    if rng.below(1_000) == 0 {
        text.push(' ');
        text.push_str(RARE);
    }
    text
}

fn validate(cfg: &BenchConfig) -> Result<()> {
    if cfg.meetings == 0
        || cfg.meetings > MAX_MEETINGS
        || cfg.chunks_per_meeting == 0
        || cfg.chunks_per_meeting > MAX_CHUNKS_PER_MEETING
        || cfg.dim == 0
        || cfg.dim > MAX_DIM
        || cfg.iterations == 0
    {
        return Err(anyhow!("bench_invalid_args"));
    }
    Ok(())
}

/// Baut die Sandbox-DB und misst. Die DB liegt in einem Temp-Ordner, der beim
/// Verlassen (auch per Fehler oder Panik) wieder verschwindet.
pub fn run_bench(cfg: &BenchConfig) -> Result<BenchReport> {
    validate(cfg)?;
    let root = cfg.temp_root.clone().unwrap_or_else(std::env::temp_dir);
    let dir = tempfile::Builder::new()
        .prefix("lva-bench-search-")
        .tempdir_in(&root)?;
    let store = MeetingStore::open_at(&dir.path().join("meetings.db"))?;
    let mut rng = SplitMix64(cfg.seed);

    // ---- Aufbau ---------------------------------------------------------
    let built = Instant::now();
    let centers: Vec<Vec<f32>> = (0..32).map(|_| rng.unit_vector(cfg.dim)).collect();
    let state = IndexState {
        status: STATUS_LEXICAL.into(),
        ..IndexState::default()
    };
    let mut meeting_ids: Vec<String> = Vec::with_capacity(cfg.meetings);
    let mut probe: Option<Vec<f32>> = None;
    let flag_conn = store.get_connection()?;
    for i in 0..cfg.meetings {
        let meeting =
            store.create_meeting(&format!("Besprechung {i}"), MeetingSource::Live, Some(1))?;
        flag_conn.execute(
            "UPDATE meetings SET status = 'ready', started_at = ?1 WHERE id = ?2",
            rusqlite::params![1_700_000_000i64 + i as i64 * 86_400, meeting.id],
        )?;
        let drafts: Vec<ChunkDraft> = (0..cfg.chunks_per_meeting)
            .map(|j| ChunkDraft {
                source: ChunkSource::Transcript,
                epoch: 0,
                segment_ids: vec![j as u32],
                ref_keys: Vec::new(),
                document_id: None,
                start_ms: Some(j as u64 * 5_000),
                end_ms: Some(j as u64 * 5_000 + 4_900),
                channel: Some(0),
                text: chunk_text(&mut rng),
                embed_text: String::new(),
            })
            .collect();
        let ids = store.replace_meeting_chunks(
            &meeting.id,
            &[ChunkSource::Transcript],
            &drafts,
            &state,
        )?;
        let center = &centers[i % centers.len()];
        let rows: Vec<(i64, Vec<f32>)> =
            ids.iter().map(|id| (*id, rng.near(center, 0.35))).collect();
        if probe.is_none() {
            probe = rows.first().map(|(_, v)| v.clone());
        }
        store.put_vectors(MODEL, &rows)?;
        meeting_ids.push(meeting.id);
    }
    drop(flag_conn);
    let build_ms = built.elapsed().as_millis() as u64;
    let db_mb = std::fs::metadata(store.db_path())?.len() as f64 / (1024.0 * 1024.0);

    // Fester Abfragevektor: in der Naehe eines vorhandenen Chunks.
    let qvec = rng.near(&probe.expect("mindestens ein Chunk"), 0.2);
    let scoped: Vec<String> = meeting_ids.iter().step_by(10).cloned().collect();

    // ---- Vektorindex laden ----------------------------------------------
    let cache = VectorCache::new();
    let loaded = Instant::now();
    let vector_index_mb =
        cache.with_index(&store, MODEL, |i| i.approx_bytes())? as f64 / (1024.0 * 1024.0);
    let vector_load_ms = loaded.elapsed().as_millis() as u64;

    // ---- Messen ---------------------------------------------------------
    let mut all_list = Vec::new();
    let mut all_words = Vec::new();
    let mut all_hybrid = Vec::new();
    let mut per_query = Vec::new();
    let filter = MeetingFilter::default();
    for (name, query) in QUERIES {
        let fts = fts_query_words(query);
        let (mut list, mut words, mut hybrid) = (Vec::new(), Vec::new(), Vec::new());
        let (mut list_hits, mut hybrid_hits) = (0u32, 0u32);
        // Lauf 0 waermt Caches auf (SQLite-Seiten, Betriebssystem) und zaehlt nicht.
        for run in 0..=cfg.iterations {
            let t = Instant::now();
            let page = store.search_meetings(query, &filter, 0, 25)?;
            let list_ms = ms(t);
            list_hits = page.total;

            let t = Instant::now();
            if let Some(fts) = &fts {
                store.search_words(fts, &meeting_ids, 100)?;
            }
            let words_ms = ms(t);

            let t = Instant::now();
            let res = hybrid_search_with_vector(
                &store,
                &cache,
                MODEL,
                query,
                Some(&qvec),
                &meeting_ids,
                40,
            )?;
            let hybrid_ms = ms(t);
            hybrid_hits = res.hits.len() as u32;

            if run > 0 {
                list.push(list_ms);
                words.push(words_ms);
                hybrid.push(hybrid_ms);
            }
        }
        all_list.extend(list.iter().copied());
        all_words.extend(words.iter().copied());
        all_hybrid.extend(hybrid.iter().copied());
        let (list, words, hybrid) = (sorted(list), sorted(words), sorted(hybrid));
        per_query.push(QueryStat {
            name: (*name).to_string(),
            query: (*query).to_string(),
            list_p50_ms: percentile(&list, 0.5),
            list_p95_ms: percentile(&list, 0.95),
            words_p50_ms: percentile(&words, 0.5),
            words_p95_ms: percentile(&words, 0.95),
            hybrid_p50_ms: percentile(&hybrid, 0.5),
            hybrid_p95_ms: percentile(&hybrid, 0.95),
            list_hits,
            hybrid_hits,
        });
    }

    // Nur die int8-Stufe und der Ordner-Scope.
    let mut topk = Vec::new();
    let mut scoped_hybrid = Vec::new();
    for run in 0..=cfg.iterations {
        let t = Instant::now();
        cache.with_index(&store, MODEL, |i| i.topk(&qvec, COARSE_CANDIDATES, None))?;
        let topk_ms = ms(t);
        let t = Instant::now();
        hybrid_search_with_vector(&store, &cache, MODEL, "budget", Some(&qvec), &scoped, 40)?;
        let scoped_ms = ms(t);
        if run > 0 {
            topk.push(topk_ms);
            scoped_hybrid.push(scoped_ms);
        }
    }
    let (all_list, all_words, all_hybrid) =
        (sorted(all_list), sorted(all_words), sorted(all_hybrid));
    let (topk, scoped_hybrid) = (sorted(topk), sorted(scoped_hybrid));

    let ui_p95 = percentile(&all_list, 0.95);
    let hybrid_p95 = percentile(&all_hybrid, 0.95);
    Ok(BenchReport {
        mode: "bench-search",
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        meetings: cfg.meetings,
        chunks_per_meeting: cfg.chunks_per_meeting,
        chunks_total: cfg.meetings * cfg.chunks_per_meeting,
        dim: cfg.dim,
        iterations: cfg.iterations,
        build_ms,
        db_mb,
        vector_load_ms,
        vector_index_mb,
        topk_p50_ms: percentile(&topk, 0.5),
        ui_search_p50_ms: percentile(&all_list, 0.5),
        ui_search_p95_ms: ui_p95,
        lexical_p50_ms: percentile(&all_words, 0.5),
        lexical_p95_ms: percentile(&all_words, 0.95),
        hybrid_p50_ms: percentile(&all_hybrid, 0.5),
        hybrid_p95_ms: hybrid_p95,
        hybrid_scoped_p95_ms: percentile(&scoped_hybrid, 0.95),
        limit_ms: P95_LIMIT_MS,
        pass: ui_p95 < P95_LIMIT_MS && hybrid_p95 < P95_LIMIT_MS,
        per_query,
    })
}

/// Einstieg fuer die Kommandozeile: Ergebnis als JSON und Exit-Code
/// (0 ok, 1 Fehler, 3 Grenze ueberschritten). Fehler kommen als JSON mit
/// `error`, damit ein aufrufendes Skript sie lesen kann.
pub fn run_cli(
    meetings: Option<usize>,
    chunks: Option<usize>,
    iterations: Option<usize>,
    temp_root: Option<PathBuf>,
) -> (i32, serde_json::Value) {
    let defaults = BenchConfig::default();
    let cfg = BenchConfig {
        meetings: meetings.unwrap_or(defaults.meetings),
        chunks_per_meeting: chunks.unwrap_or(defaults.chunks_per_meeting),
        iterations: iterations.unwrap_or(defaults.iterations),
        temp_root,
        ..defaults
    };
    match run_bench(&cfg) {
        Ok(report) => {
            let code = report.exit_code();
            (
                code,
                serde_json::to_value(&report).unwrap_or(serde_json::Value::Null),
            )
        }
        Err(e) => (
            1,
            serde_json::json!({ "mode": "bench-search", "error": e.to_string() }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small(temp_root: Option<PathBuf>) -> BenchConfig {
        BenchConfig {
            meetings: 12,
            chunks_per_meeting: 30,
            dim: 32,
            iterations: 3,
            seed: 1,
            temp_root,
        }
    }

    #[test]
    fn percentile_is_nearest_rank() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert_eq!(percentile(&v, 0.95), 95.0);
        assert_eq!(percentile(&v, 0.5), 50.0);
        assert_eq!(percentile(&v, 1.0), 100.0);
        assert_eq!(percentile(&[7.0], 0.95), 7.0);
        assert_eq!(percentile(&[], 0.95), 0.0);
        assert_eq!(percentile(&[1.0, 2.0], 0.95), 2.0);
    }

    #[test]
    fn a_small_bench_runs_end_to_end_and_cleans_up_after_itself() {
        let root = tempfile::tempdir().unwrap();
        let report = run_bench(&small(Some(root.path().to_path_buf()))).unwrap();
        assert_eq!(report.chunks_total, 360);
        assert_eq!(report.per_query.len(), QUERIES.len());
        assert!(report.pass, "{report:?}");
        assert_eq!(report.exit_code(), 0);
        let by_name = |n: &str| report.per_query.iter().find(|q| q.name == n).unwrap();
        assert_eq!(
            by_name("haeufig").list_hits,
            12,
            "Allerwelts-Wort steht in jeder Besprechung"
        );
        assert!(
            by_name("kompositum").list_hits > 0,
            "Kompositum-Teil ueber die Trigram-FTS"
        );
        assert!(
            by_name("zwei_buchstaben").list_hits > 0,
            "Zwei-Buchstaben-Wort ueber die Wort-FTS"
        );
        assert!(by_name("haeufig").hybrid_hits > 0);
        assert!(report.db_mb > 0.0 && report.vector_index_mb > 0.0);
        assert_eq!(report.limit_ms, 500.0);
        // Die Sandbox-DB ist weg.
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        // JSON-Form fuer das aufrufende Skript.
        let json = serde_json::to_value(&report).unwrap();
        for key in [
            "ui_search_p95_ms",
            "hybrid_p95_ms",
            "lexical_p95_ms",
            "build_ms",
            "vector_load_ms",
            "pass",
        ] {
            assert!(json.get(key).is_some(), "{key} fehlt");
        }
    }

    #[test]
    fn a_failing_bench_reports_the_error_and_leaves_nothing_behind() {
        // Kein Temp-Ordner anlegbar (Ziel existiert nicht): Fehler statt Panik.
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("gibt-es-nicht");
        assert!(run_bench(&small(Some(missing.clone()))).is_err());
        assert!(!missing.exists());
        let (code, json) = run_cli(Some(5), Some(5), Some(1), Some(missing));
        assert_eq!(code, 1);
        assert!(json["error"].is_string());
        // Ungueltige Argumente.
        for bad in [
            BenchConfig {
                meetings: 0,
                ..small(None)
            },
            BenchConfig {
                chunks_per_meeting: 0,
                ..small(None)
            },
            BenchConfig {
                dim: 0,
                ..small(None)
            },
            BenchConfig {
                iterations: 0,
                ..small(None)
            },
            BenchConfig {
                meetings: MAX_MEETINGS + 1,
                ..small(None)
            },
        ] {
            assert_eq!(
                run_bench(&bad).unwrap_err().to_string(),
                "bench_invalid_args"
            );
        }
    }

    #[test]
    fn the_cli_wrapper_reports_success_as_exit_code_zero() {
        let root = tempfile::tempdir().unwrap();
        let (code, json) = run_cli(Some(6), Some(20), Some(2), Some(root.path().to_path_buf()));
        assert_eq!(code, 0);
        assert_eq!(json["pass"], true);
        assert_eq!(json["meetings"], 6);
        assert_eq!(json["chunks_per_meeting"], 20);
    }

    /// Der eigentliche Messlauf (500 x 200 Chunks, 1024 Dimensionen). Teuer:
    /// `cargo test --release ... -- --ignored --nocapture bench_search_500`.
    #[test]
    #[ignore = "Performance-Messung, dauert Minuten und braucht ~1,2 GB Platte"]
    fn bench_search_500() {
        let report = run_bench(&BenchConfig::default()).unwrap();
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
        assert!(
            report.ui_search_p95_ms < P95_LIMIT_MS,
            "ui_search_p95_ms = {}",
            report.ui_search_p95_ms
        );
        assert!(
            report.hybrid_p95_ms < P95_LIMIT_MS,
            "hybrid_p95_ms = {}",
            report.hybrid_p95_ms
        );
    }
}
