//! wai-retrieval: Retrieval-Kern nach ADR-0003 E1 auf SQLite.
//!
//! * Lexikalisch: FTS5 (BM25). Vektoren: `sqlite-vec` wird nur erkannt (`vec_version()`);
//!   das Ranking laeuft ueber Brute-Force-Kosinus auf BLOB-Vektoren (ADR-Fallback).
//! * Hybrid per RRF. Der Rechtefilter (Tenant, Principal, Quellrecht) steckt in der SQL-Abfrage
//!   selbst und wirkt damit VOR Ranking und Zitaterzeugung.
//! * Ohne Beleg: Status `NoEvidence` und keine Zitate.
//! * Identitaet nur ueber `wai_security::Principal`; ein frei gebauter Actor ist nicht moeglich:
//!
//! ```compile_fail
//! let _ = wai_retrieval::Actor { tenant: "x".into(), principal: "y".into() };
//! ```

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use wai_security::Principal;

pub const SCHEMA_VERSION: &str = "0.1.0";
const RRF_K: f64 = 60.0;
const CANDIDATES: usize = 20;
/// Mindest-Kosinus, damit ein reiner Vektortreffer als Beleg gilt.
pub const MIN_COSINE: f32 = 0.25;
const MAX_CHUNK_CHARS: usize = 220;

#[derive(Debug, thiserror::Error)]
pub enum RetrievalError {
    #[error("Datenbankfehler: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Ungueltige Eingabe: {0}")]
    Invalid(String),
    #[error("Embeddings nicht verfuegbar: {0}")]
    Embedding(String),
    /// Parallele Neuindexierung desselben Mandanten (R3-4); Index unveraendert, erneut starten.
    #[error("Parallele Aenderung: {0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, RetrievalError>;

/// Embedding-Anbieter: lokaler HTTP-Dienst oder (gekennzeichneter) Hash-Rueckfall.
pub trait Embedder: Send + Sync {
    /// Kennung, die der Index traegt; bei Wechsel ist eine Neuindexierung noetig.
    fn id(&self) -> String;
    fn embed(&self, text: &str) -> impl std::future::Future<Output = Result<Vec<f32>>> + Send;
    fn embed_batch(
        &self,
        texts: &[String],
    ) -> impl std::future::Future<Output = Result<Vec<Vec<f32>>>> + Send {
        async move {
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                out.push(self.embed(t).await?);
            }
            Ok(out)
        }
    }
    /// `true`, wenn nur ein Behelf ohne semantische Qualitaet laeuft (muss sichtbar gemacht werden).
    fn is_fallback(&self) -> bool {
        false
    }
    /// Ergebnis des letzten Aufrufs (HTTP: Dienst erreichbar).
    fn healthy(&self) -> bool {
        true
    }
}

/// Deterministischer Fake: gehashter Bag-of-Words, L2-normiert. Keine semantische Suche.
pub struct FakeEmbedder {
    dim: usize,
}

impl FakeEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
    pub fn dim(&self) -> usize {
        self.dim
    }
    fn vector(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; self.dim];
        for t in content_tokens(text) {
            let h = Sha256::digest(t.as_bytes());
            let idx = u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize % self.dim;
            v[idx] += 1.0;
        }
        l2_normalize(&mut v);
        v
    }
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self::new(256)
    }
}

impl Embedder for FakeEmbedder {
    fn id(&self) -> String {
        format!("fake-hash-{}", self.dim)
    }
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.vector(text))
    }
    fn is_fallback(&self) -> bool {
        true
    }
}

fn l2_normalize(v: &mut [f32]) {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
}

/// Embedder gegen einen lokalen OpenAI-kompatiblen `/v1/embeddings`-Endpunkt
/// (llama-server, Ollama). Nur Loopback: ein entferntes Ziel wird schon beim Bauen abgelehnt
/// (dasselbe Egress-Prinzip wie `wai-providers`: kein Byte verlaesst den Rechner).
pub struct HttpEmbedder {
    url: String,
    model: String,
    client: reqwest::Client,
    ok: AtomicBool,
}

fn is_loopback_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl HttpEmbedder {
    /// `endpoint`: z. B. `http://127.0.0.1:11434` (Ollama) oder `http://127.0.0.1:8080/v1`.
    pub fn new(endpoint: &str, model: &str) -> Result<Self> {
        let bad = |m: &str| RetrievalError::Invalid(format!("Embedding-Endpunkt: {m}"));
        let rest = endpoint
            .trim()
            .strip_prefix("http://")
            .ok_or_else(|| bad("nur http:// auf Loopback erlaubt"))?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if authority.contains('@') || authority.is_empty() {
            return Err(bad("ungueltige Adresse"));
        }
        let host = match authority.strip_prefix('[') {
            Some(r) => r.split(']').next().unwrap_or(""),
            None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
        };
        if !is_loopback_host(host) {
            return Err(bad("Ziel ist nicht Loopback (Egress-Gate)"));
        }
        let path = path.trim_end_matches('/');
        let url = if path.ends_with("/embeddings") {
            format!("http://{authority}{path}")
        } else if path.ends_with("/v1") {
            format!("http://{authority}{path}/embeddings")
        } else {
            format!("http://{authority}{path}/v1/embeddings")
        };
        if model.trim().is_empty() {
            return Err(bad("Modellname fehlt"));
        }
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(std::time::Duration::from_secs(2))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| RetrievalError::Embedding(e.to_string()))?;
        Ok(Self {
            url,
            model: model.trim().into(),
            client,
            ok: AtomicBool::new(true),
        })
    }

    async fn request(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let fail = |m: String| RetrievalError::Embedding(m);
        let resp = self
            .client
            .post(&self.url)
            .json(&serde_json::json!({"model": self.model, "input": texts}))
            .send()
            .await
            .map_err(|e| fail(format!("Dienst nicht erreichbar ({e})")))?;
        if !resp.status().is_success() {
            return Err(fail(format!("HTTP {}", resp.status())));
        }
        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| fail(format!("Antwort nicht lesbar ({e})")))?;
        let data = body["data"]
            .as_array()
            .ok_or_else(|| fail("Antwort ohne data".into()))?;
        if data.len() != texts.len() {
            return Err(fail("Anzahl der Vektoren passt nicht".into()));
        }
        let mut rows: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
        for (pos, d) in data.iter().enumerate() {
            let idx = d["index"].as_u64().map_or(pos, |i| i as usize);
            let mut v: Vec<f32> = d["embedding"]
                .as_array()
                .ok_or_else(|| fail("Eintrag ohne embedding".into()))?
                .iter()
                .map(|x| x.as_f64().unwrap_or(0.0) as f32)
                .collect();
            if v.is_empty() {
                return Err(fail("leerer Vektor".into()));
            }
            l2_normalize(&mut v);
            rows.push((idx, v));
        }
        rows.sort_by_key(|r| r.0);
        Ok(rows.into_iter().map(|r| r.1).collect())
    }
}

impl Embedder for HttpEmbedder {
    fn id(&self) -> String {
        format!("http:{}", self.model)
    }
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let r = self.request(&[text.to_string()]).await;
        self.ok.store(r.is_ok(), Ordering::Relaxed);
        r.map(|mut v| v.remove(0))
    }
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut out = Vec::with_capacity(texts.len());
        for part in texts.chunks(16) {
            match self.request(part).await {
                Ok(v) => out.extend(v),
                Err(e) => {
                    self.ok.store(false, Ordering::Relaxed);
                    return Err(e);
                }
            }
        }
        self.ok.store(true, Ordering::Relaxed);
        Ok(out)
    }
    fn healthy(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }
}

/// Konfigurierbare Auswahl (`Retriever<AnyEmbedder>`).
pub enum AnyEmbedder {
    /// Gekennzeichneter Rueckfall ohne Semantik.
    Fake(FakeEmbedder),
    Http(HttpEmbedder),
}

impl Embedder for AnyEmbedder {
    fn id(&self) -> String {
        match self {
            Self::Fake(e) => e.id(),
            Self::Http(e) => e.id(),
        }
    }
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        match self {
            Self::Fake(e) => e.embed(text).await,
            Self::Http(e) => e.embed(text).await,
        }
    }
    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        match self {
            Self::Fake(e) => e.embed_batch(texts).await,
            Self::Http(e) => e.embed_batch(texts).await,
        }
    }
    fn is_fallback(&self) -> bool {
        matches!(self, Self::Fake(_))
    }
    fn healthy(&self) -> bool {
        match self {
            Self::Fake(e) => e.healthy(),
            Self::Http(e) => e.healthy(),
        }
    }
}

/// Sichtbarer Zustand der Vektorsuche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStatus {
    pub embedder: String,
    /// Semantische Vektorsuche wirkt gerade.
    pub semantic: bool,
    /// Nur Hash-Behelf aktiv (keine echte Semantik).
    pub fallback: bool,
    pub reindex_needed: bool,
    /// Chunks ohne Vektor (z. B. Dienst war bei der Aufnahme nicht erreichbar).
    pub missing_vectors: i64,
    pub message: String,
}

const STOP: &[&str] = &[
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einer", "einen", "und", "oder",
    "ist", "sind", "wie", "was", "wer", "wo", "wann", "im", "in", "am", "an", "auf", "zu", "zum",
    "zur", "von", "vom", "mit", "bei", "fuer", "für", "es", "sie", "er", "wir", "ihr", "nicht",
    "auch", "wird", "werden", "hat", "haben", "nach", "als", "ab",
];

/// Kleingeschriebene Woerter ohne deutsche Stoppwoerter.
pub fn content_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Zeichen, die `unicode61 remove_diacritics 2` nicht auf Buchstaben des Grundalphabets faltet, und ihre
/// ASCII-Entsprechung (lva#463). Gemessen am SQLite des Index (Test
/// `ablage::tests_index_paritaet::faltung_ergaenzt_nur_was_fts5_nicht_faltet`): ohne diese Tabelle findet
/// „Strasse“ kein „Straße“. Gilt vor dem Indexieren UND bei der Anfrage; der gespeicherte Text und das
/// Zitat bleiben unverändert. Die Swift-Seite (iPhone) führt dieselbe Tabelle (`ablage-index-vektoren.json`,
/// Abschnitt `normalisierung`).
pub const FTS_FALTUNG: &[(char, &str)] = &[
    ('ß', "ss"),
    ('ẞ', "ss"),
    ('æ', "ae"),
    ('Æ', "ae"),
    ('œ', "oe"),
    ('Œ', "oe"),
    ('ø', "o"),
    ('Ø', "o"),
    ('ł', "l"),
    ('Ł', "l"),
    ('đ', "d"),
    ('Đ', "d"),
    ('ð', "d"),
    ('Ð', "d"),
    ('þ', "th"),
    ('Þ', "th"),
    ('ħ', "h"),
    ('Ħ', "h"),
    ('ı', "i"),
    ('ĳ', "ij"),
    ('Ĳ', "ij"),
];

/// Text für den FTS5-Index und die Anfrage: ersetzt die Zeichen aus [`FTS_FALTUNG`] (ß → ss usw.); alles
/// andere bleibt (Groß-/Kleinschreibung und Diakritika faltet der Tokenizer selbst).
pub fn fts_normalisieren(text: &str) -> String {
    let mut aus = String::with_capacity(text.len());
    for c in text.chars() {
        let ersatz = if c.is_ascii() {
            None
        } else {
            FTS_FALTUNG.iter().find(|(z, _)| *z == c).map(|(_, e)| *e)
        };
        match ersatz {
            Some(e) => aus.push_str(e),
            None => aus.push(c),
        }
    }
    aus
}

/// BM25-Konstanten von FTS5 (`bm25()`).
const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// Eine Zeile der lexikalischen Rangliste: ein Stück mit seinem Rohrang (`bm25`, negativ, kleiner = besser).
#[derive(Debug, Clone, PartialEq)]
pub struct LexZeile {
    pub chunk_id: i64,
    pub source_id: String,
    pub page: u32,
    pub bm25: f64,
}

/// Länge eines Stücks in Wörtern aus `chunks_fts_docsize.sz` (erstes SQLite-Varint).
fn docsize_laenge(b: &[u8]) -> i64 {
    let mut n: i64 = 0;
    for (i, x) in b.iter().enumerate() {
        if i == 8 {
            return (n << 8) | *x as i64;
        }
        n = (n << 7) | (*x & 0x7f) as i64;
        if *x & 0x80 == 0 {
            break;
        }
    }
    n
}

/// Zerlegt die Wörter der Anfrage (nach [`fts_normalisieren`]) mit dem Tokenizer des Index in Phrasen: ein
/// Anfragewort kann in mehrere FTS5-Wörter zerfallen (z. B. Schriften mit Vokalzeichen). Ein Wort ohne
/// Zerlegung liefert eine leere Phrase (zählt nicht).
async fn phrasen_zerlegen(
    c: &mut sqlx::SqliteConnection,
    toks: &[String],
) -> Result<Vec<Vec<String>>> {
    sqlx::query("CREATE VIRTUAL TABLE IF NOT EXISTS temp.wai_tok USING fts5(text, tokenize='unicode61 remove_diacritics 2')")
        .execute(&mut *c)
        .await?;
    sqlx::query(
        "CREATE VIRTUAL TABLE IF NOT EXISTS temp.wai_tok_v USING fts5vocab(wai_tok, 'instance')",
    )
    .execute(&mut *c)
    .await?;
    sqlx::query("DELETE FROM temp.wai_tok")
        .execute(&mut *c)
        .await?;
    for (i, t) in toks.iter().enumerate() {
        sqlx::query("INSERT INTO temp.wai_tok(rowid, text) VALUES(?, ?)")
            .bind(i as i64 + 1)
            .bind(fts_normalisieren(t))
            .execute(&mut *c)
            .await?;
    }
    let rows =
        sqlx::query("SELECT doc, \"offset\", term FROM temp.wai_tok_v ORDER BY doc, \"offset\"")
            .fetch_all(&mut *c)
            .await?;
    let mut phrasen: Vec<Vec<String>> = vec![Vec::new(); toks.len()];
    for r in rows {
        let doc: i64 = r.get(0);
        phrasen[doc as usize - 1].push(r.get(2));
    }
    sqlx::query("DELETE FROM temp.wai_tok")
        .execute(&mut *c)
        .await?;
    Ok(phrasen)
}

/// Stücke, die die Phrase enthalten (nur Stücke, die der Akteur lesen darf), mit der Zahl ihrer Vorkommen.
async fn phrase_treffer(
    c: &mut sqlx::SqliteConnection,
    actor: &Actor,
    phrase: &[String],
) -> Result<HashMap<i64, u32>> {
    let sql = format!(
        "SELECT v.doc, v.\"offset\" FROM chunks_fts_v v JOIN chunks c ON c.id = v.doc JOIN docs d ON d.source_id = c.source_id WHERE v.term = ?3 AND {ACL_SQL}"
    );
    let mut lagen: Vec<HashMap<i64, Vec<i64>>> = Vec::with_capacity(phrase.len());
    for wort in phrase {
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&actor.tenant)
            .bind(&actor.principal)
            .bind(wort)
            .fetch_all(&mut *c)
            .await?;
        let mut je: HashMap<i64, Vec<i64>> = HashMap::new();
        for r in rows {
            je.entry(r.get(0)).or_default().push(r.get(1));
        }
        if je.is_empty() {
            return Ok(HashMap::new());
        }
        lagen.push(je);
    }
    let mut aus: HashMap<i64, u32> = HashMap::new();
    for (doc, starts) in &lagen[0] {
        let f = starts
            .iter()
            .filter(|p| {
                lagen[1..].iter().enumerate().all(|(k, l)| {
                    l.get(doc)
                        .is_some_and(|v| v.contains(&(**p + k as i64 + 1)))
                })
            })
            .count() as u32;
        if f > 0 {
            aus.insert(*doc, f);
        }
    }
    Ok(aus)
}

/// Wer fragt: intern aus dem authentisierten `Principal` abgeleitet, nie frei konstruierbar.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Actor {
    tenant: String,
    principal: String,
}

impl Actor {
    fn from_principal(p: &Principal) -> Self {
        Actor {
            tenant: p.tenant().0.clone(),
            principal: p.actor().0.clone(),
        }
    }
}

/// Quellrecht eines Dokuments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acl {
    /// Alle Principals des Tenants.
    Tenant,
    /// Nur die genannten Principals des Tenants.
    Principals(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct IngestDoc {
    /// Wird zugleich `artifact_id` (Muster `^[a-z]{2,12}_[0-9A-HJKMNP-TV-Z]{26}$`).
    pub source_id: String,
    pub tenant: String,
    pub version: u32,
    /// Seitentexte (Seite 1 = Index 0).
    pub pages: Vec<String>,
    pub acl: Acl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locator {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Zeichen-Offsets (Unicode-Skalare) innerhalb der Seite.
    pub start_offset: u32,
    pub end_offset: u32,
}

/// Zitat laut contracts/core/evidence-citation.schema.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvidenceCitation {
    pub schema_version: String,
    pub id: String,
    pub ts: String,
    pub trace_id: String,
    pub artifact_id: String,
    pub artifact_version: u32,
    pub artifact_sha256: String,
    pub locator: Locator,
    pub quote: String,
    pub score: f64,
    pub acl_checked_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ok,
    /// "Keine Belege": leere Antwort statt Halluzination.
    NoEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub status: Status,
    pub citations: Vec<EvidenceCitation>,
}

impl Answer {
    fn none() -> Self {
        Self {
            status: Status::NoEvidence,
            citations: vec![],
        }
    }
}

pub struct Retriever<E: Embedder> {
    pool: SqlitePool,
    embedder: E,
    clock: fn() -> u64,
    vec_ext: bool,
    reindex_needed: AtomicBool,
    /// Lokale Abweichung (lva#426): höchstens so viele lexikalische Kandidaten je Quelle, damit ein
    /// Dokument mit vielen Treffern nicht alle Plätze belegt. `None` = unbegrenzt (SDK-Verhalten).
    max_per_source: Option<usize>,
}

fn system_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS docs(source_id TEXT PRIMARY KEY, tenant TEXT NOT NULL, version INTEGER NOT NULL, sha256 TEXT NOT NULL, acl_mode TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS acl(source_id TEXT NOT NULL, principal TEXT NOT NULL, PRIMARY KEY(source_id, principal))",
    "CREATE TABLE IF NOT EXISTS chunks(id INTEGER PRIMARY KEY, source_id TEXT NOT NULL, page INTEGER NOT NULL, start_off INTEGER NOT NULL, end_off INTEGER NOT NULL, text TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS chunks_src ON chunks(source_id)",
    "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(text, tokenize='unicode61 remove_diacritics 2')",
    // Lokale Abweichung (lva#463): Wortvorkommen je Stück für das eigene BM25 (IDF nur über lesbare Stücke).
    "CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts_v USING fts5vocab(chunks_fts, 'instance')",
    "CREATE TABLE IF NOT EXISTS vectors(chunk_id INTEGER PRIMARY KEY, vec BLOB NOT NULL)",
    "CREATE TABLE IF NOT EXISTS qcache(key TEXT PRIMARY KEY, tenant TEXT NOT NULL, ranked TEXT NOT NULL)",
    "CREATE TABLE IF NOT EXISTS qcache_src(key TEXT NOT NULL, source_id TEXT NOT NULL)",
    "CREATE INDEX IF NOT EXISTS qcache_src_i ON qcache_src(source_id)",
    "CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY, value TEXT NOT NULL)",
];

/// Rechtefilter: Tenant-Gleichheit UND (Tenant-weit ODER Principal gelistet).
/// Parameter ?1 = tenant, ?2 = principal.
const ACL_SQL: &str = "d.tenant = ?1 AND (d.acl_mode = 'tenant' OR EXISTS (SELECT 1 FROM acl a WHERE a.source_id = d.source_id AND a.principal = ?2))";

const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn valid_id(s: &str) -> bool {
    let Some((p, u)) = s.split_once('_') else {
        return false;
    };
    (2..=12).contains(&p.len())
        && p.bytes().all(|b| b.is_ascii_lowercase())
        && u.len() == 26
        && u.bytes().all(|b| CROCKFORD.contains(&b))
}

fn valid_trace(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn crockford26(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0);
    for b in bytes {
        acc = (acc << 8) | *b as u32;
        bits += 8;
        while bits >= 5 && out.len() < 26 {
            bits -= 5;
            out.push(CROCKFORD[((acc >> bits) & 31) as usize] as char);
        }
    }
    out
}

/// RFC 3339 UTC mit Millisekunden.
pub fn format_ts(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

/// Chunking: Saetze (Ende bei . ! ? Zeilenumbruch) greedy bis `max` Zeichen; Offsets in Zeichen.
pub fn chunk_page(text: &str, max: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut sentences = vec![];
    let mut start = 0;
    for i in 0..chars.len() {
        if matches!(chars[i], '.' | '!' | '?' | '\n') || i + 1 == chars.len() {
            let (mut s, mut e) = (start, i + 1);
            while s < e && chars[s].is_whitespace() {
                s += 1;
            }
            while e > s && chars[e - 1].is_whitespace() {
                e -= 1;
            }
            if s < e {
                sentences.push((s, e));
            }
            start = i + 1;
        }
    }
    let mut out: Vec<(usize, usize)> = vec![];
    for (s, e) in sentences {
        match out.last_mut() {
            Some(last) if e - last.0 <= max => last.1 = e,
            _ => out.push((s, e)),
        }
    }
    out
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Index-Generation hoch und Query-Cache leeren: jede Schreibaktion macht Rangfolgen ungueltig.
async fn bump_generation(tx: &mut sqlx::SqliteConnection) -> Result<()> {
    sqlx::query(
        "INSERT INTO meta(key, value) VALUES('gen', '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM qcache").execute(&mut *tx).await?;
    sqlx::query("DELETE FROM qcache_src")
        .execute(&mut *tx)
        .await?;
    Ok(())
}

async fn meta_get(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    Ok(sqlx::query("SELECT value FROM meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?
        .map(|r| r.get(0)))
}

async fn meta_set(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES(?,?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

/// Meta-Schluessel der aktuellen Embedder-Kennung eines Mandanten (ueberschreibend, N2).
fn emb_key(tenant: &str) -> String {
    format!("emb:{tenant}")
}

/// Meta-Schluessel der Mandanten-Generation (R3-4): steigt bei jeder Neuindexierung und jedem
/// Wechsel der Embedder-Kennung des Mandanten.
fn tgen_key(tenant: &str) -> String {
    format!("tgen:{tenant}")
}

/// Vektorzustand eines Mandanten, gelesen auf EINER Verbindung (innerhalb einer Transaktion ein
/// konsistenter Snapshot).
struct TenantState {
    /// Vektoren stammen aus dem Embedder (ohne Textstellen: ja; ohne `emb:`-Vermerk: nein, R3-3).
    current: bool,
    /// Der `emb:`-Vermerk entspricht bereits der Kennung.
    marked: bool,
    /// Mandanten-Generation (R3-4).
    generation: i64,
}

async fn tenant_state(
    c: &mut sqlx::SqliteConnection,
    tenant: &str,
    id: &str,
) -> Result<TenantState> {
    let row = sqlx::query(
        "SELECT EXISTS(SELECT 1 FROM chunks c JOIN docs d ON d.source_id = c.source_id
                       WHERE d.tenant = ?1),
                (SELECT value FROM meta WHERE key = ?2),
                COALESCE((SELECT CAST(value AS INTEGER) FROM meta WHERE key = ?3), 0)",
    )
    .bind(tenant)
    .bind(emb_key(tenant))
    .bind(tgen_key(tenant))
    .fetch_one(&mut *c)
    .await?;
    let has: bool = row.get(0);
    let marked = row.get::<Option<String>, _>(1).as_deref() == Some(id);
    Ok(TenantState {
        current: !has || marked,
        marked,
        generation: row.get(2),
    })
}

async fn bump_tenant_generation(c: &mut sqlx::SqliteConnection, tenant: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO meta(key, value) VALUES(?, '1')
         ON CONFLICT(key) DO UPDATE SET value = CAST(value AS INTEGER) + 1",
    )
    .bind(tgen_key(tenant))
    .execute(&mut *c)
    .await?;
    Ok(())
}

/// Mandanten mit Textstellen, deren Vektoren nicht aus Embedder `id` stammen. Ohne eigenen
/// Vermerk (Altbestand) gilt der Mandant als offen (R3-3: fail-closed; die globale Kennung ist
/// kein Nachweis); er braucht eine vollstaendige Neuindexierung, bis dahin lexikalisch.
async fn stale_tenants(c: &mut sqlx::SqliteConnection, id: &str) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "SELECT COUNT(DISTINCT d.tenant) FROM chunks c JOIN docs d ON d.source_id = c.source_id
         WHERE COALESCE((SELECT m.value FROM meta m WHERE m.key = 'emb:' || d.tenant), '') <> ?",
    )
    .bind(id)
    .fetch_one(&mut *c)
    .await?)
}

/// Maximaler RRF-Wert (Rang 1 in beiden Listen), Normierung des Scores auf 0..=1.
const RRF_MAX: f64 = 2.0 / (RRF_K + 1.0);

impl<E: Embedder> Retriever<E> {
    /// In-Memory-Datenbank (eine Verbindung).
    pub async fn open_memory(embedder: E) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(":memory:")
            .pragma("temp_store", "MEMORY");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Self::init(pool, embedder).await
    }

    pub async fn open(path: &std::path::Path, embedder: E) -> Result<Self> {
        // Lokale Abweichung (lva#426): gelöschte Seiten werden überschrieben, damit Text aus
        // entfernten Dokumenten nicht in freien Seiten der Datei überlebt. Gilt je Verbindung.
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .busy_timeout(std::time::Duration::from_secs(10))
            .pragma("secure_delete", "ON")
            // Lokale Abweichung (lva#463): die Hilfstabelle der Anfragezerlegung bleibt im Arbeitsspeicher.
            .pragma("temp_store", "MEMORY");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        Self::init(pool, embedder).await
    }

    /// Wert von `PRAGMA secure_delete` der Verbindung dieses Retrievers (0 aus, 1 an, 2 schnell).
    pub async fn secure_delete(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("PRAGMA secure_delete")
            .fetch_one(&self.pool)
            .await?)
    }

    async fn init(pool: SqlitePool, embedder: E) -> Result<Self> {
        for s in SCHEMA {
            sqlx::query(*s).execute(&pool).await?;
        }
        let vec_ext = sqlx::query("SELECT vec_version()")
            .fetch_one(&pool)
            .await
            .is_ok();
        // Altbestand: Cache-Zeilen ohne Generation im Schluessel koennen veraltet sein.
        if meta_get(&pool, "cache_fmt").await?.as_deref() != Some("3") {
            sqlx::query("DELETE FROM qcache").execute(&pool).await?;
            sqlx::query("DELETE FROM qcache_src").execute(&pool).await?;
            meta_set(&pool, "cache_fmt", "3").await?;
        }
        // Jeder Mandant traegt die Kennung des Embedders seiner Vektoren (N2); ein Mandant mit
        // anderer Kennung braucht Reindex. Global erst aktuell, wenn kein Mandant offen ist.
        let id = embedder.id();
        let stale = stale_tenants(&mut *pool.acquire().await?, &id).await?;
        if stale == 0 {
            meta_set(&pool, "embedder", &id).await?;
        }
        let reindex = stale > 0;
        Ok(Self {
            pool,
            embedder,
            clock: system_ms,
            vec_ext,
            reindex_needed: AtomicBool::new(reindex),
            max_per_source: None,
        })
    }

    /// Sichtbarer Zustand der Vektorsuche (Hinweis fuer die Oberflaeche).
    pub async fn status(&self) -> Result<IndexStatus> {
        let reindex = self.reindex_needed.load(Ordering::Relaxed);
        let missing: i64 = sqlx::query(
            "SELECT COUNT(*) FROM chunks c WHERE NOT EXISTS (SELECT 1 FROM vectors v WHERE v.chunk_id = c.id)",
        )
        .fetch_one(&self.pool)
        .await?
        .get(0);
        Ok(self.build_status(reindex, missing))
    }

    /// Zustand der Vektorsuche EINES Mandanten (Wissensseite): Neuaufbau-Bedarf und Textstellen
    /// ohne Vektor zaehlen nur die Daten dieses Mandanten; fremde Mandanten bleiben unsichtbar.
    pub async fn status_for(&self, tenant: &str) -> Result<IndexStatus> {
        if tenant.is_empty() {
            return Err(RetrievalError::Invalid("tenant noetig".into()));
        }
        let id = self.embedder.id();
        let reindex = !tenant_state(&mut *self.pool.acquire().await?, tenant, &id)
            .await?
            .current;
        let missing: i64 = sqlx::query(
            "SELECT COUNT(*) FROM chunks c JOIN docs d ON d.source_id = c.source_id
             WHERE d.tenant = ? AND NOT EXISTS (SELECT 1 FROM vectors v WHERE v.chunk_id = c.id)",
        )
        .bind(tenant)
        .fetch_one(&self.pool)
        .await?
        .get(0);
        Ok(self.build_status(reindex, missing))
    }

    fn build_status(&self, reindex: bool, missing: i64) -> IndexStatus {
        let id = self.embedder.id();
        let fallback = self.embedder.is_fallback();
        let healthy = self.embedder.healthy();
        let semantic = !reindex && !fallback && healthy;
        let message = if reindex {
            "Embedder gewechselt – Neuindexierung nötig; bis dahin lexikalische Suche aktiv"
                .to_string()
        } else if !healthy {
            "Embeddings nicht verfügbar – lexikalische Suche aktiv".to_string()
        } else if fallback {
            "Behelfs-Einbettung (Hash) aktiv – keine semantische Suche".to_string()
        } else if missing > 0 {
            format!("{missing} Textstellen ohne Vektor – Neuindexierung empfohlen")
        } else {
            format!("Embeddings aktiv ({id})")
        };
        IndexStatus {
            embedder: id,
            semantic,
            fallback,
            reindex_needed: reindex,
            missing_vectors: missing,
            message,
        }
    }

    /// Bettet die Textstellen EINES Mandanten mit dem aktuellen Embedder neu ein (B28/3: der
    /// Mandant ist Pflicht, fremde Mandanten bleiben unberuehrt). Die Kennung des Embedders wird
    /// erst uebernommen, wenn jeder Mandant mit Textstellen neu eingebettet ist (sonst bliebe ein
    /// alter Vektorraum als gueltig markiert). Schlaegt der Dienst fehl, bleibt der Index
    /// unveraendert. R3-4: Die Mandanten-Generation wird vor dem Einbetten erfasst und in der
    /// Schreibtransaktion erneut geprueft; lief parallel eine andere Neuindexierung, bleibt der
    /// Index unveraendert ([`RetrievalError::Conflict`]).
    pub async fn reindex(&self, tenant: &str) -> Result<usize> {
        if tenant.is_empty() {
            return Err(RetrievalError::Invalid("tenant noetig".into()));
        }
        let id = self.embedder.id();
        let generation = tenant_state(&mut *self.pool.acquire().await?, tenant, &id)
            .await?
            .generation;
        let rows = sqlx::query(
            "SELECT c.id, c.text FROM chunks c JOIN docs d ON d.source_id = c.source_id
             WHERE d.tenant = ? ORDER BY c.id",
        )
        .bind(tenant)
        .fetch_all(&self.pool)
        .await?;
        let ids: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
        let texts: Vec<String> = rows.iter().map(|r| r.get(1)).collect();
        let vecs = self.embedder.embed_batch(&texts).await?;
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "DELETE FROM vectors WHERE chunk_id IN (SELECT c.id FROM chunks c
             JOIN docs d ON d.source_id = c.source_id WHERE d.tenant = ?)",
        )
        .bind(tenant)
        .execute(&mut *tx)
        .await?;
        // Schreibsperre ist gehalten: Generation erneut pruefen (R3-4).
        if tenant_state(&mut tx, tenant, &id).await?.generation != generation {
            tx.rollback().await?;
            return Err(RetrievalError::Conflict(format!(
                "Mandant {tenant} wurde parallel neu indexiert; bitte erneut starten"
            )));
        }
        for ((cid, text), v) in ids.iter().zip(&texts).zip(&vecs) {
            // Nur fuer unveraenderte Textstellen (eine parallele Aufnahme kann Ids neu belegen).
            sqlx::query(
                "INSERT INTO vectors(chunk_id, vec) SELECT id, ? FROM chunks WHERE id = ? AND text = ?",
            )
            .bind(to_blob(v))
            .bind(cid)
            .bind(text)
            .execute(&mut *tx)
            .await?;
        }
        bump_generation(&mut tx).await?;
        bump_tenant_generation(&mut tx, tenant).await?;
        // Aktuelle Kennung des Mandanten ueberschreiben (N2: kein "jemals reindexiert"-Vermerk);
        // offen bleiben Mandanten, deren Kennung abweicht.
        sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES(?, ?)")
            .bind(emb_key(tenant))
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        let foreign = stale_tenants(&mut tx, &id).await?;
        if foreign == 0 {
            sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES('embedder', ?)")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if foreign == 0 {
            self.reindex_needed.store(false, Ordering::Relaxed);
        }
        Ok(ids.len())
    }

    /// Lokale Abweichung (lva#395 D6): Übernimmt den Mandanten in den Vektorraum des aktuellen
    /// Embedders, OHNE einzubetten: alte Vektoren des Mandanten weg, Kennung setzen. Danach füllt
    /// [`Self::fill_missing`] die Vektoren schrittweise auf (kurze Schreibtransaktionen statt eines
    /// langen [`Self::reindex`]). Liefert `false`, wenn der Mandant schon im Vektorraum liegt.
    pub async fn adopt_space(&self, tenant: &str) -> Result<bool> {
        if tenant.is_empty() {
            return Err(RetrievalError::Invalid("tenant noetig".into()));
        }
        let id = self.embedder.id();
        let mut tx = self.pool.begin().await?;
        if tenant_state(&mut tx, tenant, &id).await?.current {
            tx.rollback().await?;
            return Ok(false);
        }
        sqlx::query(
            "DELETE FROM vectors WHERE chunk_id IN (SELECT c.id FROM chunks c
             JOIN docs d ON d.source_id = c.source_id WHERE d.tenant = ?)",
        )
        .bind(tenant)
        .execute(&mut *tx)
        .await?;
        bump_generation(&mut tx).await?;
        bump_tenant_generation(&mut tx, tenant).await?;
        sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES(?, ?)")
            .bind(emb_key(tenant))
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        let foreign = stale_tenants(&mut tx, &id).await?;
        if foreign == 0 {
            sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES('embedder', ?)")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if foreign == 0 {
            self.reindex_needed.store(false, Ordering::Relaxed);
        }
        Ok(true)
    }

    /// Lokale Abweichung (lva#395 D6): Bettet höchstens `max` Textstellen des Mandanten OHNE Vektor
    /// ein (aufsteigend nach Id) und schreibt sie in einer kurzen Transaktion. Der Mandant muss im
    /// Vektorraum des Embedders liegen ([`Self::adopt_space`]); hat er ihn während des Einbettens
    /// verlassen oder wurde er neu indexiert, bleibt der Index unverändert
    /// ([`RetrievalError::Conflict`]). Liefert die Zahl geschriebener Vektoren; 0 heißt: nichts fehlt
    /// (mehr).
    pub async fn fill_missing(&self, tenant: &str, max: usize) -> Result<usize> {
        if tenant.is_empty() {
            return Err(RetrievalError::Invalid("tenant noetig".into()));
        }
        let id = self.embedder.id();
        let before = tenant_state(&mut *self.pool.acquire().await?, tenant, &id).await?;
        if !before.current {
            return Err(RetrievalError::Conflict(format!(
                "Mandant {tenant} liegt nicht im Vektorraum des Embedders"
            )));
        }
        let rows = sqlx::query(
            "SELECT c.id, c.text FROM chunks c JOIN docs d ON d.source_id = c.source_id
             WHERE d.tenant = ? AND NOT EXISTS (SELECT 1 FROM vectors v WHERE v.chunk_id = c.id)
             ORDER BY c.id LIMIT ?",
        )
        .bind(tenant)
        .bind(max.max(1) as i64)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Ok(0);
        }
        let ids: Vec<i64> = rows.iter().map(|r| r.get(0)).collect();
        let texts: Vec<String> = rows.iter().map(|r| r.get(1)).collect();
        // Einbetten ohne offene Schreibtransaktion.
        let vecs = self.embedder.embed_batch(&texts).await?;
        let mut tx = self.pool.begin().await?;
        let now = tenant_state(&mut tx, tenant, &id).await?;
        if !now.current || now.generation != before.generation {
            tx.rollback().await?;
            return Err(RetrievalError::Conflict(format!(
                "Mandant {tenant} wurde parallel neu indexiert; bitte erneut starten"
            )));
        }
        let mut n = 0usize;
        for ((cid, text), v) in ids.iter().zip(&texts).zip(&vecs) {
            // Nur fuer unveraenderte Textstellen; eine parallele Aufnahme hat ihre schon geschrieben.
            n += sqlx::query(
                "INSERT OR IGNORE INTO vectors(chunk_id, vec) SELECT id, ? FROM chunks WHERE id = ? AND text = ?",
            )
            .bind(to_blob(v))
            .bind(cid)
            .bind(text)
            .execute(&mut *tx)
            .await?
            .rows_affected() as usize;
        }
        bump_generation(&mut tx).await?;
        tx.commit().await?;
        // Mindestens 1, solange etwas fehlte: der Aufrufer fragt erneut (verschwundene Textstellen
        // fehlen beim nächsten Mal nicht mehr).
        Ok(n.max(1))
    }

    /// Stammen die Vektoren des Mandanten aus dem aktuellen Embedder? (N3: Aufnahme und Suche
    /// pruefen den JEWEILIGEN Mandanten, nicht den globalen Zustand.) Ohne Textstellen: ja; ohne
    /// eigenen Vermerk: nein (R3-3).
    async fn tenant_current(&self, tenant: &str) -> Result<bool> {
        let id = self.embedder.id();
        let st = tenant_state(&mut *self.pool.acquire().await?, tenant, &id).await?;
        Ok(st.current)
    }

    /// Lokale Abweichung (lva#426): höchstens `n` lexikalische Kandidaten je Quelle (mindestens 1).
    pub fn with_max_per_source(mut self, n: usize) -> Self {
        self.max_per_source = Some(n.max(1));
        self
    }

    /// Schließt den Verbindungspool (nach dem Schreiben, vor einem Checkpoint durch Dritte).
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Fester Zeitgeber fuer reproduzierbare Tests.
    pub fn with_clock(mut self, clock: fn() -> u64) -> Self {
        self.clock = clock;
        self
    }

    /// `"sqlite-vec"`, wenn die Erweiterung geladen ist, sonst `"brute-force"`.
    pub fn vector_backend(&self) -> &'static str {
        if self.vec_ext {
            "sqlite-vec"
        } else {
            "brute-force"
        }
    }

    pub async fn ingest(&self, doc: &IngestDoc) -> Result<usize> {
        if !valid_id(&doc.source_id) {
            return Err(RetrievalError::Invalid(format!(
                "source_id {:?} ist keine gueltige Id",
                doc.source_id
            )));
        }
        if doc.version == 0 || doc.tenant.is_empty() {
            return Err(RetrievalError::Invalid(
                "version >= 1 und tenant noetig".into(),
            ));
        }
        // Erst einbetten (kann dauern, kein Schreibzugriff offen), dann ersetzen.
        let mut pieces: Vec<(usize, usize, usize, String)> = vec![];
        for (pi, page) in doc.pages.iter().enumerate() {
            let chars: Vec<char> = page.chars().collect();
            for (s, e) in chunk_page(page, MAX_CHUNK_CHARS) {
                pieces.push((pi, s, e, chars[s..e].iter().collect()));
            }
        }
        let texts: Vec<String> = pieces.iter().map(|p| p.3.clone()).collect();
        // Nicht erreichbarer Dienst: Text bleibt (lexikalisch) suchbar, Vektoren fehlen bis zum Reindex.
        // Mandant noch im alten Vektorraum: keine Vektoren aus dem neuen mischen (bis Reindex).
        let id = self.embedder.id();
        let before = tenant_state(&mut *self.pool.acquire().await?, &doc.tenant, &id).await?;
        let current = before.current;
        let vecs = if current {
            self.embedder.embed_batch(&texts).await.ok()
        } else {
            None
        };
        self.delete(&doc.source_id).await?;
        let mut h = Sha256::new();
        for p in &doc.pages {
            h.update((p.len() as u64).to_le_bytes());
            h.update(p.as_bytes());
        }
        let sha = format!("sha256:{}", hex(&h.finalize()));
        let mut tx = self.pool.begin().await?;
        let (mode, list) = match &doc.acl {
            Acl::Tenant => ("tenant", &[][..]),
            Acl::Principals(p) => ("principals", p.as_slice()),
        };
        sqlx::query(
            "INSERT INTO docs(source_id, tenant, version, sha256, acl_mode) VALUES(?,?,?,?,?)",
        )
        .bind(&doc.source_id)
        .bind(&doc.tenant)
        .bind(doc.version as i64)
        .bind(&sha)
        .bind(mode)
        .execute(&mut *tx)
        .await?;
        // R3-4: Schreibsperre ist gehalten; Zustand und Generation erneut pruefen. Hat sich der
        // Mandant seit der Vorpruefung geaendert (paralleler Reindex, anderer Embedder), keine
        // Vektoren schreiben und keinen Vermerk setzen: Textstellen bleiben (lexikalisch).
        let now = tenant_state(&mut tx, &doc.tenant, &id).await?;
        let current = current && now.current && now.generation == before.generation;
        if !now.current {
            self.reindex_needed.store(true, Ordering::Relaxed);
        }
        let vecs = if current { vecs } else { None };
        for p in list {
            sqlx::query("INSERT OR IGNORE INTO acl(source_id, principal) VALUES(?,?)")
                .bind(&doc.source_id)
                .bind(p)
                .execute(&mut *tx)
                .await?;
        }
        let mut n = 0;
        for (i, (pi, s, e, text)) in pieces.iter().enumerate() {
            let id = sqlx::query(
                "INSERT INTO chunks(source_id, page, start_off, end_off, text) VALUES(?,?,?,?,?)",
            )
            .bind(&doc.source_id)
            .bind(*pi as i64 + 1)
            .bind(*s as i64)
            .bind(*e as i64)
            .bind(text)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid();
            // Lokale Abweichung (lva#463): der Index sieht den gefalteten Text (ß → ss, …), chunks.text bleibt.
            sqlx::query("INSERT INTO chunks_fts(rowid, text) VALUES(?,?)")
                .bind(id)
                .bind(fts_normalisieren(text))
                .execute(&mut *tx)
                .await?;
            if let Some(v) = vecs.as_ref().and_then(|v| v.get(i)) {
                sqlx::query("INSERT INTO vectors(chunk_id, vec) VALUES(?,?)")
                    .bind(id)
                    .bind(to_blob(v))
                    .execute(&mut *tx)
                    .await?;
            }
            n += 1;
        }
        if current && !now.marked {
            sqlx::query("INSERT OR REPLACE INTO meta(key, value) VALUES(?, ?)")
                .bind(emb_key(&doc.tenant))
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            bump_tenant_generation(&mut tx, &doc.tenant).await?;
        }
        // Neue Quelle: alle zwischengespeicherten Rangfolgen sind veraltet (B21).
        bump_generation(&mut tx).await?;
        tx.commit().await?;
        Ok(n)
    }

    /// Loescht/widerruft eine Quelle samt Chunks, FTS-, Vektor- und Cacheeintraegen.
    pub async fn delete(&self, source_id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let sql = [
            "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE source_id = ?1)",
            "DELETE FROM vectors WHERE chunk_id IN (SELECT id FROM chunks WHERE source_id = ?1)",
            "DELETE FROM qcache WHERE key IN (SELECT key FROM qcache_src WHERE source_id = ?1)",
            "DELETE FROM qcache_src WHERE key NOT IN (SELECT key FROM qcache)",
            "DELETE FROM chunks WHERE source_id = ?1",
            "DELETE FROM acl WHERE source_id = ?1",
            "DELETE FROM docs WHERE source_id = ?1",
        ];
        for s in sql {
            let q = sqlx::query(s);
            if s.contains("?1") {
                q.bind(source_id).execute(&mut *tx).await?;
            } else {
                q.execute(&mut *tx).await?;
            }
        }
        bump_generation(&mut tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Anzahl gespeicherter Eintraege: (chunks, fts, vectors, cache).
    pub async fn counts(&self) -> Result<(i64, i64, i64, i64)> {
        let mut out = [0i64; 4];
        for (i, t) in ["chunks", "chunks_fts", "vectors", "qcache"]
            .iter()
            .enumerate()
        {
            out[i] = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {t}")))
                .fetch_one(&self.pool)
                .await?
                .get(0);
        }
        Ok((out[0], out[1], out[2], out[3]))
    }

    pub async fn query(
        &self,
        principal: &Principal,
        trace_id: &str,
        text: &str,
        k: usize,
    ) -> Result<Answer> {
        let actor = &Actor::from_principal(principal);
        if !valid_trace(trace_id) {
            return Err(RetrievalError::Invalid(
                "trace_id muss 32 Hex-Zeichen (klein) haben".into(),
            ));
        }
        let generation = meta_get(&self.pool, "gen").await?.unwrap_or_default();
        // Kennung des Embedders im Schluessel: eine Rangfolge eines anderen Vektorraums (oder der rein
        // lexikalischen Suche) kommt nicht wieder (lva#395 D6).
        let schluessel = format!(
            "{}\0{}\0{}\0{}\0{}\0{:?}\0{}",
            actor.tenant,
            actor.principal,
            text,
            k,
            generation,
            self.max_per_source,
            self.embedder.id()
        );
        let key = hex(&Sha256::digest(schluessel.as_bytes()));
        let cached: Option<String> = sqlx::query("SELECT ranked FROM qcache WHERE key = ?")
            .bind(&key)
            .fetch_optional(&self.pool)
            .await?
            .map(|r| r.get(0));
        // Der Cache haelt nur Rangfolge und Score; Existenz und ACL werden bei jedem Zitat neu geprueft.
        let ranked: Vec<(i64, f64)> = match cached {
            Some(s) => s
                .split(',')
                .filter_map(|p| {
                    let (a, b) = p.split_once(':')?;
                    Some((a.parse().ok()?, b.parse().ok()?))
                })
                .collect(),
            None => {
                let (r, complete) = self.rank(actor, text, k).await?;
                // Ein ohne Vektoren berechnetes Ergebnis nicht festhalten (Dienst kann zurueckkehren).
                if !r.is_empty() && complete {
                    self.cache_put(&key, &actor.tenant, &r).await?;
                }
                r
            }
        };
        let sql = format!(
            "SELECT c.page, c.start_off, c.end_off, c.text, d.source_id, d.version, d.sha256 FROM chunks c JOIN docs d ON d.source_id = c.source_id WHERE c.id = ?3 AND {ACL_SQL}"
        );
        let now = (self.clock)();
        let mut citations = vec![];
        for (id, score) in ranked {
            let Some(r) = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(&actor.tenant)
                .bind(&actor.principal)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
            else {
                continue;
            };
            let source: String = r.get("source_id");
            let seed = Sha256::digest(format!("{trace_id}|{source}|{id}|{now}").as_bytes());
            citations.push(EvidenceCitation {
                schema_version: SCHEMA_VERSION.into(),
                id: format!("cit_{}", crockford26(&seed)),
                ts: format_ts(now),
                trace_id: trace_id.into(),
                artifact_id: source,
                artifact_version: r.get::<i64, _>("version") as u32,
                artifact_sha256: r.get("sha256"),
                locator: Locator {
                    page: Some(r.get::<i64, _>("page") as u32),
                    start_offset: r.get::<i64, _>("start_off") as u32,
                    end_offset: r.get::<i64, _>("end_off") as u32,
                },
                quote: r.get::<String, _>("text").chars().take(2000).collect(),
                score,
                acl_checked_at: format_ts(now),
            });
        }
        if citations.is_empty() {
            return Ok(Answer::none());
        }
        Ok(Answer {
            status: Status::Ok,
            citations,
        })
    }

    async fn cache_put(&self, key: &str, tenant: &str, ranked: &[(i64, f64)]) -> Result<()> {
        let val = ranked
            .iter()
            .map(|(i, s)| format!("{i}:{s}"))
            .collect::<Vec<_>>()
            .join(",");
        sqlx::query("INSERT OR REPLACE INTO qcache(key, tenant, ranked) VALUES(?,?,?)")
            .bind(key)
            .bind(tenant)
            .bind(val)
            .execute(&self.pool)
            .await?;
        for (id, _) in ranked {
            sqlx::query("INSERT INTO qcache_src(key, source_id) SELECT ?, source_id FROM chunks WHERE id = ?")
                .bind(key)
                .bind(id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    /// Lexikalische Rangliste (BM25 wie `bm25(chunks_fts)` von FTS5, k1 = 1,2, b = 0,75), beste zuerst, höchstens
    /// `limit` Zeilen; Gleichstand nach Stück-Kennung. Lokale Abweichung (lva#463): Stückzahl, mittlere Länge und
    /// Dokumenthäufigkeit (IDF) zählen nur Stücke, die der Akteur lesen darf, nie Papierkorb oder für ihn gesperrte
    /// Dateien; FTS5 selbst zählt über die ganze Tabelle. Anfragewörter werden wie der Text gefaltet
    /// ([`fts_normalisieren`]). Mit Stücken, die der Akteur lesen darf, und denselben Wörtern ist der Rohrang
    /// gleich dem von FTS5.
    pub async fn lexikalisch(
        &self,
        principal: &Principal,
        text: &str,
        limit: usize,
    ) -> Result<Vec<LexZeile>> {
        let actor = Actor::from_principal(principal);
        self.lex_zeilen(&actor, &content_tokens(text), limit).await
    }

    async fn lex_zeilen(
        &self,
        actor: &Actor,
        toks: &[String],
        limit: usize,
    ) -> Result<Vec<LexZeile>> {
        if toks.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.pool.acquire().await?;
        let phrasen = phrasen_zerlegen(&mut conn, toks).await?;
        let mut treffer: HashMap<&[String], HashMap<i64, u32>> = HashMap::new();
        for p in phrasen.iter().filter(|p| !p.is_empty()) {
            if !treffer.contains_key(p.as_slice()) {
                let t = phrase_treffer(&mut conn, actor, p).await?;
                treffer.insert(p.as_slice(), t);
            }
        }
        if treffer.values().all(|t| t.is_empty()) {
            return Ok(vec![]);
        }
        // Stückzahl und Wörter aller lesbaren Stücke (N und avgdl), Länge je Stück (D).
        let mut kandidaten: HashMap<i64, i64> = HashMap::new();
        for t in treffer.values() {
            for id in t.keys() {
                kandidaten.insert(*id, 0);
            }
        }
        let sql = format!(
            "SELECT s.id, s.sz FROM chunks_fts_docsize s JOIN chunks c ON c.id = s.id JOIN docs d ON d.source_id = c.source_id WHERE {ACL_SQL}"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(&actor.tenant)
            .bind(&actor.principal)
            .fetch_all(&mut *conn)
            .await?;
        let mut n_gesamt = 0f64;
        let mut woerter = 0f64;
        for r in &rows {
            let laenge = docsize_laenge(r.get::<&[u8], _>(1));
            n_gesamt += 1.0;
            woerter += laenge as f64;
            if let Some(d) = kandidaten.get_mut(&r.get::<i64, _>(0)) {
                *d = laenge;
            }
        }
        let avgdl = woerter / n_gesamt;
        let mut punkte: HashMap<i64, f64> = HashMap::new();
        for p in phrasen.iter().filter(|p| !p.is_empty()) {
            let t = &treffer[p.as_slice()];
            if t.is_empty() {
                continue;
            }
            let n_hit = t.len() as f64;
            let mut idf = ((n_gesamt - n_hit + 0.5) / (n_hit + 0.5)).ln();
            if idf <= 0.0 {
                idf = 1e-6;
            }
            for (id, f) in t {
                let f = *f as f64;
                let d = kandidaten[id] as f64;
                *punkte.entry(*id).or_default() += idf
                    * ((f * (BM25_K1 + 1.0)) / (f + BM25_K1 * (1.0 - BM25_B + BM25_B * d / avgdl)));
            }
        }
        let mut rang: Vec<(i64, f64)> = punkte.into_iter().map(|(i, s)| (i, -1.0 * s)).collect();
        rang.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        rang.truncate(limit);
        let ids = format!(
            "[{}]",
            rang.iter()
                .map(|(i, _)| i.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        let rows = sqlx::query(
            "SELECT id, source_id, page FROM chunks WHERE id IN (SELECT value FROM json_each(?))",
        )
        .bind(ids)
        .fetch_all(&mut *conn)
        .await?;
        let mut herkunft: HashMap<i64, (String, u32)> = HashMap::new();
        for r in rows {
            herkunft.insert(r.get(0), (r.get(1), r.get::<i64, _>(2) as u32));
        }
        Ok(rang
            .into_iter()
            .filter_map(|(id, bm25)| {
                herkunft.remove(&id).map(|(source_id, page)| LexZeile {
                    chunk_id: id,
                    source_id,
                    page,
                    bm25,
                })
            })
            .collect())
    }

    /// Hybrid-Ranking (RRF) ausschliesslich ueber autorisierte Chunks; Score normiert auf 0..=1.
    async fn rank(&self, actor: &Actor, text: &str, k: usize) -> Result<(Vec<(i64, f64)>, bool)> {
        let mut rrf: HashMap<i64, f64> = HashMap::new();
        let toks = content_tokens(text);
        if !toks.is_empty() {
            // Mit Kappung je Quelle mehr Zeilen holen und danach je Quelle kürzen (lva#426).
            let limit = match self.max_per_source {
                Some(_) => k.max(CANDIDATES) * 50,
                None => CANDIDATES,
            };
            let rows = self.lex_zeilen(actor, &toks, limit).await?;
            let mut je_quelle: HashMap<String, usize> = HashMap::new();
            let mut r = 0usize;
            for row in rows.iter() {
                if let Some(max) = self.max_per_source {
                    let n = je_quelle.entry(row.source_id.clone()).or_default();
                    if *n >= max {
                        continue;
                    }
                    *n += 1;
                    if r >= k.max(CANDIDATES) {
                        break;
                    }
                }
                *rrf.entry(row.chunk_id).or_default() += 1.0 / (RRF_K + 1.0 + r as f64);
                r += 1;
            }
        }
        let q = if self.tenant_current(&actor.tenant).await? {
            self.embedder.embed(text).await.ok()
        } else {
            None
        };
        let mut complete = q.is_some();
        if let Some(q) = q.filter(|q| q.iter().any(|x| *x != 0.0)) {
            let sql = format!(
                "SELECT c.id, v.vec, c.source_id FROM vectors v JOIN chunks c ON c.id = v.chunk_id JOIN docs d ON d.source_id = c.source_id WHERE {ACL_SQL}"
            );
            // R3-4: Zustand und Vektoren aus EINEM Snapshot (Lesetransaktion); ist der Mandant
            // inzwischen nicht mehr im Vektorraum des Embedders, werden die Vektoren verworfen.
            let id = self.embedder.id();
            let mut tx = self.pool.begin().await?;
            let rows = if tenant_state(&mut tx, &actor.tenant, &id).await?.current {
                sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                    .bind(&actor.tenant)
                    .bind(&actor.principal)
                    .fetch_all(&mut *tx)
                    .await?
            } else {
                complete = false;
                vec![]
            };
            tx.commit().await?;
            let mut sims: Vec<(i64, f32, String)> = rows
                .iter()
                .map(|r| {
                    let v = from_blob(r.get::<&[u8], _>(1));
                    let sim = if v.len() == q.len() {
                        v.iter().zip(&q).map(|(a, b)| a * b).sum::<f32>()
                    } else {
                        0.0
                    };
                    (r.get::<i64, _>(0), sim, r.get::<String, _>(2))
                })
                .filter(|(_, s, _)| *s >= MIN_COSINE)
                .collect();
            sims.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            // Dieselbe Kappung je Quelle wie in der lexikalischen Liste (lva#426, D6).
            let mut je_quelle: HashMap<&str, usize> = HashMap::new();
            let behalten = sims.iter().filter(|(_, _, src)| match self.max_per_source {
                Some(max) => {
                    let n = je_quelle.entry(src.as_str()).or_default();
                    *n += 1;
                    *n <= max
                }
                None => true,
            });
            for (r, (id, _, _)) in behalten.take(CANDIDATES).enumerate() {
                *rrf.entry(*id).or_default() += 1.0 / (RRF_K + 1.0 + r as f64);
            }
        }
        let mut v: Vec<(i64, f64)> = rrf.into_iter().collect();
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.truncate(k);
        Ok((
            v.into_iter()
                .map(|(i, s)| (i, (s / RRF_MAX).min(1.0)))
                .collect(),
            complete,
        ))
    }
}
