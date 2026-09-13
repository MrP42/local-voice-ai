//! „Passt es rein?“ -- die Speicherprognose fuer ein lokales Modell.
//!
//! Eine Prognose, kein Versprechen: Gewichte aus der Dateigroesse, KV-Cache
//! aus den GGUF-Metadaten (Schichten, KV-Koepfe, Kopfbreite), dazu die
//! Rechenpuffer des Backends und eine Reserve. Der Spike vom 13.09. hat die
//! Groessenordnung bestaetigt: 378-MB-Datei + Kontext 4096 = 867 MiB real,
//! die Formel liefert ~950. Genauer wird es erst nach dem ersten echten
//! Laden -- die gemessene Belegung schlaegt dann jede Formel.

use serde::Serialize;
use specta::Type;

use crate::managers::gguf_meta::{self, GgufError, GgufMetadata};

/// Faktor auf die Dateigroesse: Gewichte liegen selten 1:1 im Speicher
/// (Ausrichtung, Puffer je Tensor).
const WEIGHT_FACTOR: f64 = 1.15;
/// Rechen- und Backend-Puffer, die kein Metadatum verraet (MiB).
const OVERHEAD_MB: u64 = 512;
/// Anteil des freien Budgets, der frei bleiben soll, damit Treiber und
/// Anzeige nicht ins Stocken geraten.
const HEADROOM: f64 = 0.15;
/// Bytes je KV-Element: f16 fuer K und fuer V.
const KV_BYTES_PER_ELEMENT: u64 = 2;

/// Die Metadaten-Schluessel, die die Prognose braucht -- je Architektur
/// mit deren Praefix (`qwen3.block_count`, `llama.block_count`, …).
pub const PROBE_KEYS: &[&str] = &[
    "general.architecture",
    "general.name",
    "qwen3.block_count",
    "qwen3.attention.head_count",
    "qwen3.attention.head_count_kv",
    "qwen3.embedding_length",
    "qwen3.attention.key_length",
    "qwen3.context_length",
    "gemma3.block_count",
    "gemma3.attention.head_count",
    "gemma3.attention.head_count_kv",
    "gemma3.embedding_length",
    "gemma3.attention.key_length",
    "gemma3.context_length",
    "llama.block_count",
    "llama.attention.head_count",
    "llama.attention.head_count_kv",
    "llama.embedding_length",
    "llama.attention.key_length",
    "llama.context_length",
    "qwen2.block_count",
    "qwen2.attention.head_count",
    "qwen2.attention.head_count_kv",
    "qwen2.embedding_length",
    "qwen2.attention.key_length",
    "qwen2.context_length",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FitVerdict {
    /// Bedarf unter dem freien Budget abzueglich Reserve.
    Fits,
    /// Bedarf passt gerade noch -- ohne Reserve.
    Tight,
    /// Bedarf uebersteigt das freie Budget.
    Unlikely,
    /// Kein GPU-Budget messbar -- Urteil nur gegen den RAM.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct MemoryEstimate {
    pub weights_mb: u64,
    pub kv_mb: u64,
    pub overhead_mb: u64,
    pub total_mb: u64,
    pub context_tokens: u32,
    /// Woher die KV-Zahlen kommen: aus den Metadaten oder geraten.
    pub from_metadata: bool,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct FitReport {
    pub estimate: MemoryEstimate,
    /// Freies Budget des massgeblichen Speichers (MiB) -- GPU, sonst RAM.
    pub free_mb: u64,
    pub on_gpu: bool,
    pub verdict: FitVerdict,
}

/// Architekturzahlen, die den KV-Cache bestimmen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KvShape {
    pub layers: u64,
    pub kv_heads: u64,
    pub head_dim: u64,
}

impl KvShape {
    /// Liest die Form aus den Metadaten; `None`, wenn ein Wert fehlt --
    /// dann wird geschaetzt, aber als geschaetzt ausgewiesen.
    pub fn from_metadata(meta: &GgufMetadata) -> Option<Self> {
        let arch = meta.get_str("general.architecture")?;
        let get = |suffix: &str| meta.get_u64(&format!("{arch}.{suffix}"));
        let layers = get("block_count")?;
        let heads = get("attention.head_count")?;
        let kv_heads = get("attention.head_count_kv").unwrap_or(heads);
        let head_dim = get("attention.key_length")
            .or_else(|| get("embedding_length").map(|e| e / heads.max(1)))?;
        Some(Self {
            layers,
            kv_heads,
            head_dim,
        })
    }

    /// KV-Cache fuer `context_tokens` Token (MiB): Token × Schichten ×
    /// KV-Koepfe × Kopfbreite × (K + V) × Bytes.
    pub fn kv_mb(&self, context_tokens: u32) -> u64 {
        let elements = u64::from(context_tokens) * self.layers * self.kv_heads * self.head_dim;
        elements * 2 * KV_BYTES_PER_ELEMENT / (1024 * 1024)
    }
}

/// Grobe Form, wenn keine Metadaten vorliegen: ein 4B-Modell mittlerer Bauart.
fn fallback_shape(file_size_bytes: u64) -> KvShape {
    // Kleinere Dateien = weniger Schichten; das ist eine Daumenregel, mehr
    // nicht -- und sie wird als solche ausgewiesen.
    let layers = ((file_size_bytes / (1024 * 1024 * 100)).clamp(12, 48)) as u64;
    KvShape {
        layers,
        kv_heads: 8,
        head_dim: 128,
    }
}

pub fn estimate(file_size_bytes: u64, shape: Option<KvShape>, context_tokens: u32) -> MemoryEstimate {
    let (kv_shape, from_metadata) = match shape {
        Some(s) => (s, true),
        None => (fallback_shape(file_size_bytes), false),
    };
    let weights_mb = ((file_size_bytes as f64) * WEIGHT_FACTOR / (1024.0 * 1024.0)).ceil() as u64;
    let kv_mb = kv_shape.kv_mb(context_tokens);
    MemoryEstimate {
        weights_mb,
        kv_mb,
        overhead_mb: OVERHEAD_MB,
        total_mb: weights_mb + kv_mb + OVERHEAD_MB,
        context_tokens,
        from_metadata,
    }
}

pub fn verdict(total_mb: u64, free_mb: u64, on_gpu: bool) -> FitVerdict {
    if !on_gpu {
        return FitVerdict::Unknown;
    }
    let with_headroom = (free_mb as f64 * (1.0 - HEADROOM)) as u64;
    if total_mb <= with_headroom {
        FitVerdict::Fits
    } else if total_mb <= free_mb {
        FitVerdict::Tight
    } else {
        FitVerdict::Unlikely
    }
}

/// Metadaten aus dem Kopf einer lokalen Datei (waechst nach, bis der Kopf
/// vollstaendig ist -- dasselbe Muster wie bei den Diktatmodellen).
pub fn shape_from_file(path: &std::path::Path) -> Option<KvShape> {
    use std::io::Read;
    let mut size = 256usize << 10;
    let max = 16usize << 20;
    loop {
        let mut file = std::fs::File::open(path).ok()?;
        let mut buf = vec![0u8; size];
        let mut filled = 0;
        while filled < buf.len() {
            match file.read(&mut buf[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
        buf.truncate(filled);
        match gguf_meta::parse_header(&buf, PROBE_KEYS) {
            Ok(meta) => return KvShape::from_metadata(&meta),
            Err(GgufError::Truncated { needed }) if needed > filled && size < max => {
                size = needed.max(size * 2).min(max);
            }
            Err(_) => return None,
        }
    }
}

/// Metadaten aus dem Kopf einer entfernten Datei per HTTP-Range -- fuer die
/// Prognose *vor* dem Download. Faellt still auf `None` zurueck: ohne Netz
/// gibt es eben nur die Daumenregel.
pub async fn shape_from_url(url: &str) -> Option<KvShape> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let mut size = 512usize << 10;
    let max = 8usize << 20;
    loop {
        let resp = client
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes=0-{}", size - 1))
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let buf = resp.bytes().await.ok()?;
        match gguf_meta::parse_header(&buf, PROBE_KEYS) {
            Ok(meta) => return KvShape::from_metadata(&meta),
            Err(GgufError::Truncated { needed }) if needed > buf.len() && size < max => {
                size = needed.max(size * 2).min(max);
            }
            Err(_) => return None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::gguf_meta::GgufValue;
    use std::collections::HashMap;

    fn qwen3_0_6b() -> GgufMetadata {
        let mut kv = HashMap::new();
        kv.insert("general.architecture".to_string(), GgufValue::String("qwen3".into()));
        kv.insert("qwen3.block_count".to_string(), GgufValue::U32(28));
        kv.insert("qwen3.attention.head_count".to_string(), GgufValue::U32(16));
        kv.insert("qwen3.attention.head_count_kv".to_string(), GgufValue::U32(8));
        kv.insert("qwen3.attention.key_length".to_string(), GgufValue::U32(128));
        kv.insert("qwen3.embedding_length".to_string(), GgufValue::U32(1024));
        GgufMetadata { kv }
    }

    /// Die Zahlen des Spikes: 378 MB Datei, Kontext 4096 -> gemessen 867 MiB.
    /// Die Prognose muss in derselben Groessenordnung liegen und darueber,
    /// nie darunter -- eine zu optimistische Prognose waere die gefaehrliche.
    #[test]
    fn the_spike_measurement_is_covered_by_the_estimate() {
        let shape = KvShape::from_metadata(&qwen3_0_6b()).expect("Form aus Metadaten");
        assert_eq!(shape, KvShape { layers: 28, kv_heads: 8, head_dim: 128 });
        let e = estimate(396_705_472, Some(shape), 4096);
        assert!(e.from_metadata);
        assert_eq!(e.kv_mb, 448, "KV: 4096*28*8*128*4 Byte");
        assert!(e.total_mb >= 867, "Prognose {} MiB unter der Messung", e.total_mb);
        assert!(e.total_mb <= 1400, "Prognose {} MiB unplausibel hoch", e.total_mb);
    }

    #[test]
    fn head_dim_falls_back_to_embedding_over_heads() {
        let mut meta = qwen3_0_6b();
        meta.kv.remove("qwen3.attention.key_length");
        let shape = KvShape::from_metadata(&meta).unwrap();
        assert_eq!(shape.head_dim, 64, "1024 / 16 Koepfe");
    }

    #[test]
    fn missing_metadata_is_estimated_and_flagged_as_such() {
        let e = estimate(2_497_281_312, None, 8192);
        assert!(!e.from_metadata);
        assert!(e.kv_mb > 0);
    }

    #[test]
    fn verdicts_follow_the_free_budget_with_headroom() {
        assert_eq!(verdict(3000, 10_000, true), FitVerdict::Fits);
        // 8600 von 10000 frei: unter dem Budget, aber ueber 85 % davon.
        assert_eq!(verdict(8600, 10_000, true), FitVerdict::Tight);
        assert_eq!(verdict(12_000, 10_000, true), FitVerdict::Unlikely);
        assert_eq!(verdict(3000, 10_000, false), FitVerdict::Unknown);
    }
}
