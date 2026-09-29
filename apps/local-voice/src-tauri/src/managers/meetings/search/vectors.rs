//! Vektorindex im RAM (M4 §4, Entscheidung D1): int8-Brute-Force in Rust
//! statt sqlite-vec. Die Wahrheit sind die f32-BLOBs in `meeting_chunk_vectors`;
//! hier liegt nur eine verkleinerte Kopie (1 Byte je Dimension plus ein
//! Skalenfaktor je Vektor). Die Suche laeuft zweistufig: int8 liefert grob die
//! besten `COARSE_CANDIDATES`, `rescore` bewertet diese exakt in f32 aus der DB
//! nach. Damit ist der Index selbst verlustbehaftet, das Ergebnis nicht.
//!
//! Gemessen (Spike M3, 100 000 x 1024, ein Thread): int8 ~17 ms, RAM ~100 MB.
//!
//! Speicher: der Index waechst nur mit `try_reserve` und bis `MAX_INDEX_BYTES`;
//! ein zu grosser oder nicht allozierbarer Index ist ein FEHLER (`index_too_large`
//! / `index_out_of_memory`), kein Absturz. Der Aufrufer faellt dann auf die
//! lexikalische Suche zurueck.
//!
//! Veraltung ist ungefaehrlich: `rescore` nimmt nur Kandidaten, die es in der DB
//! (Chunk lebend, Besprechung lebend, Vektor vorhanden) noch gibt. Ein Chunk,
//! der neu indexiert wurde und dessen alter Vektor noch im RAM steht, faellt so
//! weg statt falsch zu treffen.

use anyhow::{anyhow, Result};
use rusqlite::params;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant};

use super::super::store::MeetingStore;

/// So viele Kandidaten liefert die int8-Stufe an die f32-Nachbewertung (D1).
pub const COARSE_CANDIDATES: usize = 300;
/// Obergrenze fuer den Speicher des int8-Index (Vektoren + Verwaltung).
pub const MAX_INDEX_BYTES: usize = 1 << 30;
/// Nach so langer Zeit ohne Nutzung wird der Index verworfen (M4 §4).
pub const IDLE_TTL: Duration = Duration::from_secs(600);
const MAX_DIM: usize = 8_192;

// ---------------------------------------------------------------------------
// Kleine Helfer (Quantisierung, Skalarprodukte)
// ---------------------------------------------------------------------------

/// Symmetrische int8-Quantisierung: `v ~ out * scale`. Liefert 0.0 (und Nullen)
/// fuer einen Nullvektor oder nicht endliche Werte.
fn quantize(v: &[f32], out: &mut [i8]) -> f32 {
    if v.iter().any(|x| !x.is_finite()) {
        out.fill(0);
        return 0.0;
    }
    let max = v.iter().fold(0f32, |m, x| m.max(x.abs()));
    if max <= 0.0 {
        out.fill(0);
        return 0.0;
    }
    let inv = 127.0 / max;
    for (o, x) in out.iter_mut().zip(v) {
        *o = (x * inv).round().clamp(-127.0, 127.0) as i8;
    }
    max / 127.0
}

/// Skalarprodukt zweier int8-Vektoren in i32 (acht Bahnen, damit LLVM es vektorisiert).
#[inline]
fn dot_i8(a: &[i8], b: &[i8]) -> i32 {
    let mut lanes = [0i32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            lanes[i] += i32::from(x[i]) * i32::from(y[i]);
        }
    }
    let mut sum: i32 = lanes.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        sum += i32::from(*x) * i32::from(*y);
    }
    sum
}

#[inline]
fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..8 {
            lanes[i] += x[i] * y[i];
        }
    }
    let mut sum: f32 = lanes.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        sum += x * y;
    }
    sum
}

fn norm(v: &[f32]) -> f32 {
    v.iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt() as f32
}

/// f32 little endian -> Vec; `None`, wenn die Laenge nicht zu `dim` passt.
fn decode_blob(blob: &[u8], dim: usize, out: &mut Vec<f32>) -> Option<()> {
    if dim == 0 || blob.len() != dim * 4 {
        return None;
    }
    out.clear();
    out.extend(
        blob.chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
    );
    Some(())
}

/// Heap-Eintrag: kleinster Wert oben, damit der schlechteste der besten k
/// zuerst ersetzt wird. Gleicher Wert: der spaetere Index ist "kleiner"
/// (frueher eingefuegte gewinnen den Gleichstand, das Ergebnis ist deterministisch).
#[derive(PartialEq)]
struct Scored {
    score: f32,
    row: u32,
}

impl Eq for Scored {}

impl PartialOrd for Scored {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for Scored {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Umgekehrt zur Guete: BinaryHeap ist ein Max-Heap, oben soll der Schlechteste liegen.
        other
            .score
            .total_cmp(&self.score)
            .then_with(|| self.row.cmp(&other.row))
    }
}

// ---------------------------------------------------------------------------
// Der Index
// ---------------------------------------------------------------------------

pub struct VectorIndex {
    model: String,
    dim: usize,
    /// Chunk-ID je Zeile.
    ids: Vec<i64>,
    /// Index in `meetings` je Zeile (fuer den Scope-Filter).
    meeting_ix: Vec<u32>,
    meetings: Vec<String>,
    meeting_lookup: HashMap<String, u32>,
    /// `rows * dim` Bytes.
    q8: Vec<i8>,
    /// Skalenfaktor je Zeile (`max|x| / 127`).
    scale: Vec<f32>,
    pos: HashMap<i64, usize>,
    /// Beim Laden uebersprungene Zeilen (kaputte BLOB-Laenge, fremde Dimension).
    skipped: usize,
    max_bytes: usize,
}

impl VectorIndex {
    pub fn empty(model: &str, dim: usize) -> Self {
        Self {
            model: model.to_string(),
            dim,
            ids: Vec::new(),
            meeting_ix: Vec::new(),
            meetings: Vec::new(),
            meeting_lookup: HashMap::new(),
            q8: Vec::new(),
            scale: Vec::new(),
            pos: HashMap::new(),
            skipped: 0,
            max_bytes: MAX_INDEX_BYTES,
        }
    }

    /// Wie `empty`, aber mit eigener Speicherobergrenze (Tests, Sonderfaelle).
    pub fn with_limit(model: &str, dim: usize, max_bytes: usize) -> Self {
        Self {
            max_bytes,
            ..Self::empty(model, dim)
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// Ungefaehrer Speicherbedarf.
    pub fn approx_bytes(&self) -> usize {
        self.q8.len() + self.ids.len() * (8 + 4 + 4 + 24)
    }

    fn meeting_slot(&mut self, meeting_id: &str) -> u32 {
        if let Some(ix) = self.meeting_lookup.get(meeting_id) {
            return *ix;
        }
        let ix = self.meetings.len() as u32;
        self.meetings.push(meeting_id.to_string());
        self.meeting_lookup.insert(meeting_id.to_string(), ix);
        ix
    }

    /// Reserviert Platz fuer eine weitere Zeile oder meldet, warum es nicht geht.
    fn reserve_row(&mut self) -> Result<()> {
        if self.approx_bytes() + self.dim + 48 > self.max_bytes {
            return Err(anyhow!("index_too_large"));
        }
        let oom = |_| anyhow!("index_out_of_memory");
        self.q8.try_reserve(self.dim).map_err(oom)?;
        self.ids.try_reserve(1).map_err(oom)?;
        self.meeting_ix.try_reserve(1).map_err(oom)?;
        self.scale.try_reserve(1).map_err(oom)?;
        Ok(())
    }

    /// Haengt eine Zeile an oder ueberschreibt die der Chunk-ID.
    fn put_row(&mut self, chunk_id: i64, meeting_id: &str, vec: &[f32]) -> Result<()> {
        if let Some(&row) = self.pos.get(&chunk_id) {
            let dim = self.dim;
            let scale = quantize(vec, &mut self.q8[row * dim..(row + 1) * dim]);
            self.scale[row] = scale;
            let meeting = self.meeting_slot(meeting_id);
            self.meeting_ix[row] = meeting;
            return Ok(());
        }
        self.reserve_row()?;
        self.pos
            .try_reserve(1)
            .map_err(|_| anyhow!("index_out_of_memory"))?;
        let meeting = self.meeting_slot(meeting_id);
        let start = self.q8.len();
        self.q8.resize(start + self.dim, 0);
        let scale = quantize(vec, &mut self.q8[start..]);
        self.pos.insert(chunk_id, self.ids.len());
        self.ids.push(chunk_id);
        self.meeting_ix.push(meeting);
        self.scale.push(scale);
        Ok(())
    }

    /// Laedt alle Vektoren von `model` lebender Besprechungen. Zeilen mit
    /// kaputter BLOB-Laenge oder anderer Dimension als die erste werden
    /// uebersprungen und gezaehlt (`skipped`). Der Speicher der f32-BLOBs wird
    /// nie komplett gehalten: Zeile fuer Zeile quantisieren.
    pub fn load(store: &MeetingStore, model: &str) -> Result<Self> {
        Self::load_with_limit(store, model, MAX_INDEX_BYTES)
    }

    /// Wie `load` mit eigener Speicherobergrenze (`index_too_large` beim Ueberschreiten).
    pub fn load_with_limit(store: &MeetingStore, model: &str, max_bytes: usize) -> Result<Self> {
        let conn = store.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT v.chunk_id, c.meeting_id, v.dim, v.vec
             FROM meeting_chunk_vectors v
             JOIN meeting_chunks c ON c.id = v.chunk_id
             JOIN meetings m ON m.id = c.meeting_id AND m.deleted_at IS NULL
             WHERE v.model = ?1",
        )?;
        let mut rows = stmt.query(params![model])?;
        let mut index: Option<VectorIndex> = None;
        let mut skipped = 0usize;
        let mut scratch: Vec<f32> = Vec::new();
        while let Some(row) = rows.next()? {
            let chunk_id: i64 = row.get(0)?;
            let dim = row.get::<_, i64>(2)?.max(0) as usize;
            let (meeting_id, blob) = (row.get_ref(1)?.as_str()?, row.get_ref(3)?.as_blob()?);
            if dim == 0 || dim > MAX_DIM || decode_blob(blob, dim, &mut scratch).is_none() {
                skipped += 1;
                continue;
            }
            // Die Dimension bestimmt die erste brauchbare Zeile, nicht eine kaputte davor.
            let index = index.get_or_insert_with(|| Self::with_limit(model, dim, max_bytes));
            if dim != index.dim {
                skipped += 1;
                continue;
            }
            index.put_row(chunk_id, meeting_id, &scratch)?;
        }
        let mut index = index.unwrap_or_else(|| Self::with_limit(model, 0, max_bytes));
        index.skipped = skipped;
        Ok(index)
    }

    /// Fuegt Vektoren ein oder ersetzt sie: `(chunk_id, meeting_id, Vektor)`.
    /// Ein Vektor mit falscher Laenge oder ohne endliche Werte ist ein Fehler
    /// (`invalid_vector`), dann bleibt der Index bis dahin veraendert (die
    /// Zeilen davor sind drin); der Aufrufer laedt notfalls neu.
    pub fn upsert(&mut self, rows: &[(i64, String, Vec<f32>)]) -> Result<()> {
        for (chunk_id, meeting_id, vec) in rows {
            if self.dim == 0 && !vec.is_empty() && vec.len() <= MAX_DIM && self.ids.is_empty() {
                self.dim = vec.len();
            }
            if vec.len() != self.dim || self.dim == 0 || vec.iter().any(|x| !x.is_finite()) {
                return Err(anyhow!("invalid_vector"));
            }
            self.put_row(*chunk_id, meeting_id, vec)?;
        }
        Ok(())
    }

    fn remove_row(&mut self, row: usize) {
        let last = self.ids.len() - 1;
        self.pos.remove(&self.ids[row]);
        if row != last {
            self.ids[row] = self.ids[last];
            self.meeting_ix[row] = self.meeting_ix[last];
            self.scale[row] = self.scale[last];
            let dim = self.dim;
            self.q8.copy_within(last * dim..(last + 1) * dim, row * dim);
            self.pos.insert(self.ids[row], row);
        }
        self.ids.truncate(last);
        self.meeting_ix.truncate(last);
        self.scale.truncate(last);
        self.q8.truncate(last * self.dim);
    }

    /// Entfernt alle Zeilen einer Besprechung (geloescht oder neu indexiert).
    /// Liefert die Zahl entfernter Zeilen.
    pub fn remove_meeting(&mut self, meeting_id: &str) -> usize {
        let Some(&ix) = self.meeting_lookup.get(meeting_id) else {
            return 0;
        };
        let mut rows: Vec<usize> = (0..self.ids.len())
            .filter(|&i| self.meeting_ix[i] == ix)
            .collect();
        // Von hinten nach vorn: nachgerueckte letzte Zeilen sind nie selbst noch zu loeschen.
        rows.sort_unstable_by(|a, b| b.cmp(a));
        for &row in &rows {
            self.remove_row(row);
        }
        rows.len()
    }

    /// Maske ueber die Besprechungen des Index: `true` fuer die in `meeting_ids`.
    pub fn scope_mask(&self, meeting_ids: &[String]) -> Vec<bool> {
        let mut mask = vec![false; self.meetings.len()];
        for id in meeting_ids {
            if let Some(&ix) = self.meeting_lookup.get(id.as_str()) {
                mask[ix as usize] = true;
            }
        }
        mask
    }

    /// Die `k` aehnlichsten Zeilen (int8, ein Thread), absteigend nach Score
    /// (Kosinus-Naeherung), Gleichstand nach Einfuegereihenfolge. `mask` kommt
    /// aus `scope_mask`. Falsche Abfragelaenge, Nullvektor oder leerer Index
    /// ergeben eine leere Liste.
    pub fn topk(&self, q: &[f32], k: usize, mask: Option<&[bool]>) -> Vec<(i64, f32)> {
        if k == 0 || self.ids.is_empty() || q.len() != self.dim || q.iter().any(|x| !x.is_finite())
        {
            return Vec::new();
        }
        let mut qq = vec![0i8; self.dim];
        let qscale = quantize(q, &mut qq);
        if qscale == 0.0 {
            return Vec::new();
        }
        let q_norm = norm(q);
        let mut heap: BinaryHeap<Scored> = BinaryHeap::with_capacity(k + 1);
        for row in 0..self.ids.len() {
            if let Some(mask) = mask {
                if !mask
                    .get(self.meeting_ix[row] as usize)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            let dot = dot_i8(&qq, &self.q8[row * self.dim..(row + 1) * self.dim]);
            let score = dot as f32 * self.scale[row] * qscale / q_norm;
            if heap.len() < k {
                heap.push(Scored {
                    score,
                    row: row as u32,
                });
            } else if let Some(worst) = heap.peek() {
                if score > worst.score {
                    heap.pop();
                    heap.push(Scored {
                        score,
                        row: row as u32,
                    });
                }
            }
        }
        let mut best = heap.into_vec();
        best.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.row.cmp(&b.row)));
        best.into_iter()
            .map(|s| (self.ids[s.row as usize], s.score))
            .collect()
    }

    /// Bewertet Kandidaten exakt in f32 aus der DB nach und liefert die besten
    /// `k` (Kosinus, absteigend; Gleichstand: kleinere Chunk-ID zuerst).
    /// Kandidaten ohne Vektor, mit toter Besprechung oder falscher Laenge
    /// entfallen.
    pub fn rescore(
        &self,
        store: &MeetingStore,
        q: &[f32],
        cands: &[(i64, f32)],
        k: usize,
    ) -> Result<Vec<(i64, f32)>> {
        let q_norm = norm(q);
        if cands.is_empty() || k == 0 || q.len() != self.dim || q_norm <= 0.0 {
            return Ok(Vec::new());
        }
        let ids: Vec<i64> = cands.iter().map(|c| c.0).collect();
        let conn = store.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT v.chunk_id, v.dim, v.vec
             FROM meeting_chunk_vectors v
             JOIN meeting_chunks c ON c.id = v.chunk_id
             JOIN meetings m ON m.id = c.meeting_id AND m.deleted_at IS NULL
             WHERE v.model = ?1 AND v.chunk_id IN (SELECT value FROM json_each(?2))",
        )?;
        let mut rows = stmt.query(params![self.model, serde_json::to_string(&ids)?])?;
        let mut scored: Vec<(i64, f32)> = Vec::with_capacity(cands.len());
        let mut scratch: Vec<f32> = Vec::new();
        while let Some(row) = rows.next()? {
            let dim = row.get::<_, i64>(1)?.max(0) as usize;
            if dim != self.dim
                || decode_blob(row.get_ref(2)?.as_blob()?, dim, &mut scratch).is_none()
            {
                continue;
            }
            scored.push((row.get(0)?, dot_f32(q, &scratch) / q_norm));
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(k);
        Ok(scored)
    }
}

// ---------------------------------------------------------------------------
// Prozessweiter Cache mit Leerlauf-Verfall
// ---------------------------------------------------------------------------

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

struct Slot {
    db_path: PathBuf,
    index: VectorIndex,
    last_used: AtomicU64,
}

/// Haelt EINEN Index (den der zuletzt benutzten Datenbank und des Modells).
/// Suchen laufen unter dem Lesesperre, Aenderungen (`update`) unter der
/// Schreibsperre; beides dauert Millisekunden. Nur das erste Laden (~1 s bei
/// 100 000 Vektoren) haelt die Schreibsperre lang.
///
/// Vertrag fuer den Indexer (P4b): nach `put_vectors` -> `update(|i| i.upsert(..))`,
/// vor einem erneuten Indexieren oder Loeschen -> `update(|i| i.remove_meeting(..))`.
/// Unterbleibt das, ist es harmlos (siehe Modulkopf: `rescore` filtert), nur
/// findet die Vektorsuche neue Chunks dann erst nach dem naechsten Laden.
pub struct VectorCache {
    slot: RwLock<Option<Slot>>,
}

impl VectorCache {
    pub const fn new() -> Self {
        Self {
            slot: RwLock::new(None),
        }
    }

    /// Fuehrt `f` auf dem (bei Bedarf geladenen) Index von `store` und `model` aus.
    pub fn with_index<R>(
        &self,
        store: &MeetingStore,
        model: &str,
        f: impl FnOnce(&VectorIndex) -> R,
    ) -> Result<R> {
        let matches = |slot: &Slot| slot.db_path == store.db_path() && slot.index.model == model;
        {
            let read = self.slot.read().unwrap_or_else(PoisonError::into_inner);
            if let Some(slot) = read.as_ref().filter(|s| matches(s)) {
                slot.last_used.store(now_ms(), Ordering::Relaxed);
                return Ok(f(&slot.index));
            }
        }
        let mut write = self.slot.write().unwrap_or_else(PoisonError::into_inner);
        if !write.as_ref().is_some_and(matches) {
            *write = None; // erst freigeben, dann laden: nie zwei Indizes zugleich im RAM
            *write = Some(Slot {
                db_path: store.db_path().to_path_buf(),
                index: VectorIndex::load(store, model)?,
                last_used: AtomicU64::new(now_ms()),
            });
        }
        let slot = write.as_ref().expect("gerade gesetzt");
        slot.last_used.store(now_ms(), Ordering::Relaxed);
        Ok(f(&slot.index))
    }

    /// Aendert den geladenen Index; ohne geladenen Index passiert nichts
    /// (er wird beim naechsten Bedarf frisch aus der DB gelesen).
    pub fn update<R>(&self, f: impl FnOnce(&mut VectorIndex) -> R) -> Option<R> {
        let mut write = self.slot.write().unwrap_or_else(PoisonError::into_inner);
        write.as_mut().map(|slot| f(&mut slot.index))
    }

    /// Verwirft den Index sofort (Neuaufbau, "Index neu erstellen", Modellwechsel).
    pub fn invalidate(&self) {
        *self.slot.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Verwirft den Index, wenn er `ttl` lang nicht benutzt wurde. Liefert, ob er verworfen wurde.
    pub fn drop_if_idle(&self, ttl: Duration) -> bool {
        let mut write = self.slot.write().unwrap_or_else(PoisonError::into_inner);
        let idle = write.as_ref().is_some_and(|slot| {
            now_ms().saturating_sub(slot.last_used.load(Ordering::Relaxed))
                >= ttl.as_millis() as u64
        });
        if idle {
            *write = None;
        }
        idle
    }

    pub fn is_loaded(&self) -> bool {
        self.slot
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }
}

impl Default for VectorCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Der Cache der laufenden App.
pub fn global_cache() -> &'static VectorCache {
    static CACHE: VectorCache = VectorCache::new();
    &CACHE
}

// ---------------------------------------------------------------------------
// Deterministische Testdaten (auch fuer den Bench)
// ---------------------------------------------------------------------------

/// SplitMix64: klein, schnell, reproduzierbar. Kein `rand`-Aufwand fuer 100 Mio. Zahlen.
pub(crate) struct SplitMix64(pub u64);

impl SplitMix64 {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Gleichverteilt in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Ungefaehr normalverteilt (Summe von vier Gleichverteilten, Mittel 0).
    pub fn gauss(&mut self) -> f32 {
        self.unit() + self.unit() + self.unit() + self.unit() - 2.0
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// Zufaelliger Einheitsvektor.
    pub fn unit_vector(&mut self, dim: usize) -> Vec<f32> {
        let mut v: Vec<f32> = (0..dim).map(|_| self.gauss()).collect();
        let n = norm(&v);
        v.iter_mut().for_each(|x| *x /= n);
        v
    }

    /// Einheitsvektor nahe `center` (fuer Cluster; `noise` 0 = identisch).
    pub fn near(&mut self, center: &[f32], noise: f32) -> Vec<f32> {
        let mut v: Vec<f32> = center.iter().map(|c| c + noise * self.gauss()).collect();
        let n = norm(&v);
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::super::chunking::ChunkSource;
    use super::super::index::tests::{draft, ready_meeting, state, tmp_store};
    use super::super::index::STATUS_LEXICAL;
    use super::*;

    const DIM: usize = 64;

    /// Baut `meetings` Besprechungen mit je `per` Chunks und deren Vektoren
    /// (gruppiert um 24 Zentren, damit die Naehe nicht trivial ist). Liefert
    /// `(chunk_id, meeting_id, normierter Vektor)` in Anlegereihenfolge.
    fn build(
        s: &MeetingStore,
        meetings: usize,
        per: usize,
        model: &str,
    ) -> Vec<(i64, String, Vec<f32>)> {
        let mut rng = SplitMix64(7);
        let centers: Vec<Vec<f32>> = (0..24).map(|_| rng.unit_vector(DIM)).collect();
        let mut all = Vec::new();
        for i in 0..meetings {
            let m = ready_meeting(s, &format!("M{i}"), i as i64);
            let drafts: Vec<_> = (0..per)
                .map(|j| draft(ChunkSource::Transcript, &format!("Text {i} {j}")))
                .collect();
            let ids = s
                .replace_meeting_chunks(
                    &m.id,
                    &[ChunkSource::Transcript],
                    &drafts,
                    &state(STATUS_LEXICAL),
                )
                .unwrap();
            let rows: Vec<(i64, Vec<f32>)> = ids
                .iter()
                .map(|id| {
                    let c = &centers[rng.below(centers.len())];
                    (*id, rng.near(c, 0.35))
                })
                .collect();
            s.put_vectors(model, &rows).unwrap();
            for (id, v) in rows {
                all.push((id, m.id.clone(), v));
            }
        }
        all
    }

    fn exact_top(
        all: &[(i64, String, Vec<f32>)],
        q: &[f32],
        k: usize,
        allow: Option<&[String]>,
    ) -> Vec<i64> {
        let mut scored: Vec<(i64, f32)> = all
            .iter()
            .filter(|(_, m, _)| allow.is_none_or(|a| a.contains(m)))
            .map(|(id, _, v)| (*id, dot_f32(q, v) / norm(q)))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.into_iter().take(k).map(|(id, _)| id).collect()
    }

    #[test]
    fn int8_rescored_matches_f32_top10() {
        let (_d, s) = tmp_store();
        let all = build(&s, 20, 100, "mdl");
        assert_eq!(all.len(), 2_000);
        let index = VectorIndex::load(&s, "mdl").unwrap();
        assert_eq!((index.len(), index.dim(), index.skipped()), (2_000, DIM, 0));

        let mut rng = SplitMix64(99);
        let mut coarse_overlap = 0usize;
        for round in 0..25 {
            // Abfragen aus dem Datenbestand (Treffer bei Recall) und frei erfundene.
            let q = if round % 2 == 0 {
                let pick = rng.below(all.len());
                rng.near(&all[pick].2, 0.2)
            } else {
                rng.unit_vector(DIM)
            };
            let exact = exact_top(&all, &q, 10, None);
            let coarse = index.topk(&q, COARSE_CANDIDATES, None);
            assert_eq!(coarse.len(), COARSE_CANDIDATES);
            let rescored = index.rescore(&s, &q, &coarse, 10).unwrap();
            let got: Vec<i64> = rescored.iter().map(|(id, _)| *id).collect();
            assert_eq!(
                got, exact,
                "Recall@10 = 1,0 mit gleicher Reihenfolge (Runde {round})"
            );
            // Die reine int8-Stufe liegt nah dran (Diagnose, kein Vertrag).
            coarse_overlap += coarse
                .iter()
                .take(10)
                .filter(|(id, _)| exact.contains(id))
                .count();
        }
        assert!(
            coarse_overlap >= 25 * 8,
            "int8-Top-10 deckt sich im Mittel zu mindestens 80 % mit f32 ({coarse_overlap}/250)"
        );
    }

    #[test]
    fn topk_is_sorted_and_scores_approximate_the_cosine() {
        let (_d, s) = tmp_store();
        let all = build(&s, 3, 50, "mdl");
        let index = VectorIndex::load(&s, "mdl").unwrap();
        let q = all[10].2.clone();
        let top = index.topk(&q, 20, None);
        assert_eq!(top.len(), 20);
        assert!(top.windows(2).all(|w| w[0].1 >= w[1].1), "absteigend");
        assert_eq!(top[0].0, all[10].0, "der Vektor selbst zuerst");
        assert!(
            (top[0].1 - 1.0).abs() < 0.02,
            "Kosinus ~1, war {}",
            top[0].1
        );
        assert_eq!(
            index.topk(&q, 1_000, None).len(),
            150,
            "k groesser als der Index"
        );
    }

    #[test]
    fn topk_respects_the_meeting_mask() {
        let (_d, s) = tmp_store();
        let all = build(&s, 6, 40, "mdl");
        let index = VectorIndex::load(&s, "mdl").unwrap();
        let mut rng = SplitMix64(5);
        let q = rng.unit_vector(DIM);
        let scope: Vec<String> = vec![all[0].1.clone(), all[100].1.clone()];
        let mask = index.scope_mask(&scope);
        assert_eq!(mask.iter().filter(|m| **m).count(), 2);
        let hits = index.topk(&q, 30, Some(&mask));
        assert_eq!(hits.len(), 30);
        let members: Vec<i64> = all
            .iter()
            .filter(|(_, m, _)| scope.contains(m))
            .map(|(id, _, _)| *id)
            .collect();
        assert!(hits.iter().all(|(id, _)| members.contains(id)));
        let got: Vec<i64> = index
            .rescore(&s, &q, &hits, 5)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(got, exact_top(&all, &q, 5, Some(&scope)));
        // Leere Maske und unbekannte Besprechungen: nichts.
        assert!(index.topk(&q, 5, Some(&index.scope_mask(&[]))).is_empty());
        assert!(index
            .topk(&q, 5, Some(&index.scope_mask(&["fremd".to_string()])))
            .is_empty());
    }

    #[test]
    fn degenerate_queries_return_nothing_instead_of_panicking() {
        let (_d, s) = tmp_store();
        build(&s, 1, 10, "mdl");
        let index = VectorIndex::load(&s, "mdl").unwrap();
        assert!(
            index.topk(&vec![0.0; DIM], 5, None).is_empty(),
            "Nullvektor"
        );
        assert!(
            index.topk(&[1.0, 0.0], 5, None).is_empty(),
            "falsche Laenge"
        );
        let mut nan = vec![0.5; DIM];
        nan[3] = f32::NAN;
        assert!(index.topk(&nan, 5, None).is_empty());
        assert!(index.topk(&vec![1.0; DIM], 0, None).is_empty());
        assert!(index
            .rescore(&s, &vec![0.0; DIM], &[(1, 0.0)], 5)
            .unwrap()
            .is_empty());
        assert!(index
            .rescore(&s, &vec![1.0; DIM], &[], 5)
            .unwrap()
            .is_empty());
        let empty = VectorIndex::load(&s, "anderes-modell").unwrap();
        assert!(empty.is_empty());
        assert!(empty.topk(&vec![1.0; DIM], 5, None).is_empty());
    }

    #[test]
    fn upsert_overwrites_in_place_and_appends_new_rows() {
        let mut index = VectorIndex::empty("mdl", 4);
        index
            .upsert(&[
                (1, "A".into(), vec![1.0, 0.0, 0.0, 0.0]),
                (2, "A".into(), vec![0.0, 1.0, 0.0, 0.0]),
            ])
            .unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index.topk(&[1.0, 0.0, 0.0, 0.0], 1, None)[0].0, 1);
        // Chunk 1 zeigt jetzt woanders hin, Chunk 3 kommt dazu.
        index
            .upsert(&[
                (1, "A".into(), vec![0.0, 0.0, 1.0, 0.0]),
                (3, "B".into(), vec![1.0, 0.1, 0.0, 0.0]),
            ])
            .unwrap();
        assert_eq!(index.len(), 3, "kein Duplikat fuer Chunk 1");
        assert_eq!(index.topk(&[1.0, 0.0, 0.0, 0.0], 1, None)[0].0, 3);
        assert_eq!(index.topk(&[0.0, 0.0, 1.0, 0.0], 1, None)[0].0, 1);
        // Fehler: falsche Laenge, NaN.
        assert_eq!(
            index
                .upsert(&[(9, "A".into(), vec![1.0])])
                .unwrap_err()
                .to_string(),
            "invalid_vector"
        );
        assert!(index
            .upsert(&[(9, "A".into(), vec![f32::NAN, 0.0, 0.0, 0.0])])
            .is_err());
        // Ein leerer Index ohne Dimension uebernimmt die der ersten Zeile.
        let mut fresh = VectorIndex::empty("mdl", 0);
        fresh.upsert(&[(1, "A".into(), vec![1.0, 0.0])]).unwrap();
        assert_eq!(fresh.dim(), 2);
    }

    #[test]
    fn remove_meeting_drops_only_its_rows_and_keeps_the_rest_searchable() {
        let (_d, s) = tmp_store();
        let all = build(&s, 5, 30, "mdl");
        let mut index = VectorIndex::load(&s, "mdl").unwrap();
        let victim = all[35].1.clone(); // zweite Besprechung
        assert_eq!(index.remove_meeting(&victim), 30);
        assert_eq!(index.len(), 120);
        assert_eq!(index.remove_meeting(&victim), 0, "zweites Mal nichts");
        assert_eq!(index.remove_meeting("unbekannt"), 0);

        let rest: Vec<(i64, String, Vec<f32>)> = all
            .iter()
            .filter(|(_, m, _)| *m != victim)
            .cloned()
            .collect();
        let mut rng = SplitMix64(3);
        for _ in 0..10 {
            let q = rng.unit_vector(DIM);
            let coarse = index.topk(&q, COARSE_CANDIDATES, None);
            assert!(coarse
                .iter()
                .all(|(id, _)| rest.iter().any(|(r, _, _)| r == id)));
            let got: Vec<i64> = index
                .rescore(&s, &q, &coarse, 10)
                .unwrap()
                .into_iter()
                .map(|(id, _)| id)
                .collect();
            // Die geloeschte Besprechung lebt in der DB noch (nur der RAM-Index wurde bereinigt),
            // deshalb gegen die Restmenge ohne sie vergleichen.
            assert_eq!(got, exact_top(&rest, &q, 10, None));
        }
        // Wieder einfuegen stellt den Bestand her.
        let back: Vec<(i64, String, Vec<f32>)> = all
            .iter()
            .filter(|(_, m, _)| *m == victim)
            .cloned()
            .collect();
        index.upsert(&back).unwrap();
        assert_eq!(index.len(), 150);
    }

    #[test]
    fn load_skips_broken_rows_other_models_and_dead_meetings() {
        let (_d, s) = tmp_store();
        let all = build(&s, 3, 10, "mdl");
        // Fremdes Modell und geloeschte Besprechung.
        let other = ready_meeting(&s, "Andere", 100);
        let ids = s
            .replace_meeting_chunks(
                &other.id,
                &[ChunkSource::Transcript],
                &[
                    draft(ChunkSource::Transcript, "x"),
                    draft(ChunkSource::Transcript, "y"),
                ],
                &state(STATUS_LEXICAL),
            )
            .unwrap();
        let conn = s.get_connection().unwrap();
        let insert = |chunk: i64, model: &str, dim: i64, blob: Vec<u8>| {
            conn.execute(
                "INSERT OR REPLACE INTO meeting_chunk_vectors (chunk_id, model, dim, vec) VALUES (?1, ?2, ?3, ?4)",
                params![chunk, model, dim, blob],
            )
            .unwrap();
        };
        // Zu kurzer BLOB, falsche Dimension.
        insert(ids[0], "mdl", DIM as i64, vec![0u8; 10]);
        insert(ids[1], "mdl", 32, vec![0u8; 32 * 4]);
        drop(conn);
        let index = VectorIndex::load(&s, "mdl").unwrap();
        assert_eq!(index.len(), 30);
        assert_eq!(index.skipped(), 2);
        assert!(VectorIndex::load(&s, "fremd").unwrap().is_empty());

        // Geloeschte Besprechung: ihre Vektoren sind mit den Chunks schon weg.
        s.soft_delete_meeting(&all[0].1).unwrap();
        assert_eq!(VectorIndex::load(&s, "mdl").unwrap().len(), 20);
    }

    #[test]
    fn rescore_drops_candidates_that_vanished_from_the_database() {
        let (_d, s) = tmp_store();
        let all = build(&s, 2, 20, "mdl");
        let index = VectorIndex::load(&s, "mdl").unwrap();
        let q = all[0].2.clone();
        let coarse = index.topk(&q, 40, None);
        assert_eq!(coarse.len(), 40);
        // Besprechung 2 wird geloescht, der RAM-Index weiss davon nichts (kein `update`).
        s.soft_delete_meeting(&all[20].1).unwrap();
        let rescored = index.rescore(&s, &q, &coarse, 40).unwrap();
        assert_eq!(rescored.len(), 20, "nur Chunks lebender Besprechungen");
        assert!(rescored
            .iter()
            .all(|(id, _)| all[..20].iter().any(|(a, _, _)| a == id)));
        assert_eq!(rescored[0].0, all[0].0);
        // Neu indexiert: die alte Chunk-ID gibt es nicht mehr, ihr Vektor ist mit weg.
        s.replace_meeting_chunks(
            &all[0].1,
            &[ChunkSource::Transcript],
            &[draft(ChunkSource::Transcript, "neu")],
            &state(STATUS_LEXICAL),
        )
        .unwrap();
        let after = index.rescore(&s, &q, &coarse, 40).unwrap();
        assert_eq!(
            after.len(),
            0,
            "der neue Chunk hat noch keinen Vektor, die anderen sind tot"
        );
    }

    #[test]
    fn the_index_refuses_to_outgrow_its_memory_cap() {
        let mut index = VectorIndex::with_limit("mdl", 16, 2_000);
        let mut rng = SplitMix64(1);
        let mut stored = 0usize;
        let mut error = None;
        for i in 0..1_000 {
            match index.upsert(&[(i, "A".into(), rng.unit_vector(16))]) {
                Ok(()) => stored += 1,
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        assert_eq!(error.as_deref(), Some("index_too_large"));
        assert!(
            stored > 5 && stored < 100,
            "Obergrenze greift frueh: {stored}"
        );
        assert!(index.approx_bytes() <= 2_000);
        // Was drin ist, bleibt benutzbar.
        assert_eq!(index.topk(&rng.unit_vector(16), 3, None).len(), 3);

        // Laden ueber die DB: gleicher Fehler statt Absturz.
        let (_d, s) = tmp_store();
        build(&s, 2, 20, "mdl");
        assert_eq!(
            VectorIndex::load_with_limit(&s, "mdl", 1_000)
                .err()
                .map(|e| e.to_string())
                .as_deref(),
            Some("index_too_large")
        );
        assert_eq!(VectorIndex::load(&s, "mdl").unwrap().len(), 40);
    }

    #[test]
    fn the_cache_loads_once_serves_readers_and_reloads_for_a_different_model_or_store() {
        let (_d, s) = tmp_store();
        let (_d2, s2) = tmp_store();
        let all = build(&s, 2, 10, "mdl");
        build(&s2, 1, 5, "mdl");
        let cache = VectorCache::new();
        assert!(!cache.is_loaded());
        assert_eq!(cache.update(|i| i.len()), None, "ohne Index: nichts zu tun");

        let n = cache.with_index(&s, "mdl", |i| i.len()).unwrap();
        assert_eq!(n, 20);
        assert!(cache.is_loaded());
        // `update` wirkt auf den geladenen Index; der naechste Leser sieht es.
        cache.update(|i| i.remove_meeting(&all[0].1)).unwrap();
        assert_eq!(
            cache.with_index(&s, "mdl", |i| i.len()).unwrap(),
            10,
            "kein Neuladen"
        );
        // Anderes Modell / anderer Store: frisch aus der jeweiligen DB.
        assert_eq!(cache.with_index(&s, "anderes", |i| i.len()).unwrap(), 0);
        assert_eq!(cache.with_index(&s2, "mdl", |i| i.len()).unwrap(), 5);
        assert_eq!(
            cache.with_index(&s, "mdl", |i| i.len()).unwrap(),
            20,
            "wieder von der Platte"
        );

        cache.invalidate();
        assert!(!cache.is_loaded());
    }

    #[test]
    fn the_cache_drops_an_idle_index() {
        let (_d, s) = tmp_store();
        build(&s, 1, 5, "mdl");
        let cache = VectorCache::new();
        cache.with_index(&s, "mdl", |_| ()).unwrap();
        assert!(
            !cache.drop_if_idle(Duration::from_secs(600)),
            "frisch benutzt bleibt"
        );
        assert!(cache.is_loaded());
        std::thread::sleep(Duration::from_millis(30));
        assert!(cache.drop_if_idle(Duration::from_millis(10)));
        assert!(!cache.is_loaded());
        assert!(
            !cache.drop_if_idle(Duration::ZERO),
            "nichts geladen, nichts zu verwerfen"
        );
        assert_eq!(
            cache.with_index(&s, "mdl", |i| i.len()).unwrap(),
            5,
            "laedt bei Bedarf neu"
        );
    }

    #[test]
    fn a_failing_load_leaves_the_cache_empty_and_usable() {
        let (_d, s) = tmp_store();
        build(&s, 1, 5, "mdl");
        let cache = VectorCache::new();
        // Kaputte Datenbank: die Vektortabelle ist weg.
        s.get_connection()
            .unwrap()
            .execute_batch("DROP TABLE meeting_chunk_vectors;")
            .unwrap();
        assert!(cache.with_index(&s, "mdl", |i| i.len()).is_err());
        assert!(!cache.is_loaded(), "kein halb gefuellter Cache");
    }

    #[test]
    fn quantization_is_symmetric_and_bounded() {
        let mut out = [0i8; 4];
        let scale = quantize(&[1.0, -1.0, 0.5, 0.0], &mut out);
        assert_eq!(out, [127, -127, 64, 0]);
        assert!((scale - 1.0 / 127.0).abs() < 1e-9);
        assert_eq!(quantize(&[0.0; 4], &mut out), 0.0);
        assert_eq!(
            quantize(&[f32::NAN, 1.0, 0.0, 0.0], &mut out),
            0.0,
            "nicht endlich"
        );
        assert_eq!(
            dot_i8(&[1, 2, 3, 4, 5, 6, 7, 8, 9], &[1; 9]),
            45,
            "Rest hinter den Bahnen"
        );
    }
}
