//! Kontextgroesse des lokalen Servers je verfuegbarem Grafikspeicher (P1g).
//!
//! Vorher fest 8 192 Token. Das reicht fuer ein kurzes Protokoll, aber ein
//! laengeres Gespraech laeuft dann in die Map-Reduce-Stufe, und ein Modell,
//! das mehr Platz braucht (Denkmodelle, lange Vorlagen), scheitert am vollen
//! Kontext. Die Wahl ist eine reine Funktion: Modellgroesse, KV-Form aus den
//! GGUF-Metadaten und freier VRAM rein, Kontext raus.
//!
//! Konservativ, nie geraten:
//! - Nur mit GPU-Backend, gemessenem freiem VRAM einer dedizierten Karte und
//!   KV-Zahlen aus den Metadaten. Sonst bleibt es beim Standard.
//! - Die Prognose (`estimate`) muss mit der Reserve (`HEADROOM`) passen
//!   (`FitVerdict::Fits`), nicht nur "gerade noch".
//! - Nie unter dem Standard (der war bisher immer gestartet) und nie ueber
//!   `MAX_AUTO_CONTEXT_TOKENS`.
//!
//! Kein Modul hier darf `settings::get_settings(&AppHandle)` rufen.

use super::estimate::{estimate, verdict, FitVerdict, KvShape};
use super::resources::GpuMemory;
use super::DEFAULT_CONTEXT_TOKENS;

/// Groesster Kontext, den die App von sich aus waehlt. Darueber wird ein
/// Einzeldurchlauf auf einem 9B-Modell langsam und in der Qualitaet
/// unzuverlaessig; sehr lange Besprechungen gehen ohnehin in die Map-Reduce-
/// Stufe. Der KV-Cache waechst linear mit dem Kontext.
pub const MAX_AUTO_CONTEXT_TOKENS: u32 = 16_384;

/// Stufen von gross nach klein; der Standard ist der Boden und steht nicht
/// in der Liste.
const STEPS: [u32; 2] = [MAX_AUTO_CONTEXT_TOKENS, 12_288];

/// Freier Grafikspeicher (MiB), gegen den ein lokales Modell antritt: der
/// knappste Wert unter den dedizierten Karten (Budget minus systemweite
/// Belegung). Geteilter Speicher (iGPU) zaehlt nicht -- dort konkurriert das
/// Modell mit dem Arbeitsspeicher, und den prueft das RAM-Start-Gate.
/// `None`, wenn keine dedizierte Karte messbar ist.
pub fn free_dedicated_vram_mb(gpus: &[GpuMemory]) -> Option<u64> {
    gpus.iter()
        .filter(|g| !g.shared && g.budget_mb > 0)
        .map(|g| g.budget_mb.saturating_sub(g.used_mb))
        .min()
}

/// Kontext fuer den Serverstart. `free_vram_mb` ist der freie Speicher VOR
/// dem Laden des Modells.
pub fn choose_context_tokens(
    file_size_bytes: u64,
    shape: Option<KvShape>,
    backend_cpu: bool,
    free_vram_mb: Option<u64>,
) -> u32 {
    let (Some(shape), Some(free)) = (shape, free_vram_mb) else {
        return DEFAULT_CONTEXT_TOKENS;
    };
    if backend_cpu {
        return DEFAULT_CONTEXT_TOKENS;
    }
    STEPS
        .into_iter()
        .find(|&ctx| {
            let need = estimate(file_size_bytes, Some(shape), ctx).total_mb;
            verdict(need, free, true) == FitVerdict::Fits
        })
        .unwrap_or(DEFAULT_CONTEXT_TOKENS)
}

/// Umgebungsvariable fuer Entwickler und Messlaeufe (z. B. `--eval-notes` mit
/// dem Standard-Kontext, um Map-Reduce live zu erzwingen). Nicht fuer den
/// Normalbetrieb.
pub const CONTEXT_ENV: &str = "LVA_LLM_CONTEXT_TOKENS";
const OVERRIDE_RANGE: std::ops::RangeInclusive<u32> = 2_048..=32_768;

/// Kontext aus der Umgebungsvariablen; unbrauchbare oder unvernuenftige Werte
/// (Text, unter 2 048, ueber 32 768) gelten als nicht gesetzt.
pub fn context_override(raw: Option<&str>) -> Option<u32> {
    raw?.trim()
        .parse::<u32>()
        .ok()
        .filter(|v| OVERRIDE_RANGE.contains(v))
}

/// Die Wahl fuer eine Modelldatei auf diesem Rechner. Blockierend (liest den
/// GGUF-Kopf, fragt DXGI) -- der Aufrufer legt es auf einen Blocking-Thread.
pub fn plan_for_file(path: &std::path::Path, backend: &str) -> u32 {
    if let Some(forced) = context_override(std::env::var(CONTEXT_ENV).ok().as_deref()) {
        return forced;
    }
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let shape = super::estimate::shape_from_file(path);
    let free = free_dedicated_vram_mb(&super::resources::system_memory().gpus);
    choose_context_tokens(size, shape, backend == "cpu", free)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Qwen3.5-9B Q4_K_M (Hybrid: 8 von 32 Schichten mit KV) wie in P1e.
    const QWEN35_9B: u64 = 5_680_522_464;
    fn qwen35() -> KvShape {
        KvShape { layers: 8, kv_heads: 4, head_dim: 256 }
    }
    /// Ein dichtes 8B-Modell mit vollem KV je Schicht: 36 x 8 x 128.
    fn dense_8b() -> KvShape {
        KvShape { layers: 36, kv_heads: 8, head_dim: 128 }
    }
    fn gpu(budget: u64, used: u64, shared: bool) -> GpuMemory {
        GpuMemory {
            name: "Testkarte".into(),
            budget_mb: budget,
            used_mb: used,
            dedicated_mb: if shared { 0 } else { budget },
            shared,
        }
    }

    #[test]
    fn a_big_card_gets_the_maximum_context() {
        // RTX 4090 mit 16 GB frei: passt mit Reserve.
        assert_eq!(
            choose_context_tokens(QWEN35_9B, Some(qwen35()), false, Some(16_308)),
            MAX_AUTO_CONTEXT_TOKENS
        );
        assert_eq!(
            choose_context_tokens(QWEN35_9B, Some(dense_8b()), false, Some(16_308)),
            MAX_AUTO_CONTEXT_TOKENS
        );
    }

    #[test]
    fn the_context_shrinks_with_the_free_vram_but_never_below_the_default() {
        let file = QWEN35_9B;
        let need_16k = estimate(file, Some(dense_8b()), 16_384).total_mb;
        let need_12k = estimate(file, Some(dense_8b()), 12_288).total_mb;
        assert!(need_12k < need_16k);
        // Reserve (15 %) mitgerechnet: gerade genug fuer 12 288, nicht fuer 16 384.
        let free = (need_12k as f64 / 0.85).ceil() as u64 + 1;
        assert!(free < (need_16k as f64 / 0.85) as u64);
        assert_eq!(choose_context_tokens(file, Some(dense_8b()), false, Some(free)), 12_288);
        // Knapp: nur noch der Standard.
        assert_eq!(
            choose_context_tokens(file, Some(dense_8b()), false, Some(need_12k)),
            DEFAULT_CONTEXT_TOKENS
        );
        // Zu wenig fuer alles: weiterhin der Standard, nie darunter.
        assert_eq!(
            choose_context_tokens(file, Some(dense_8b()), false, Some(100)),
            DEFAULT_CONTEXT_TOKENS
        );
    }

    #[test]
    fn nothing_is_guessed_without_a_gpu_measurement_or_kv_metadata() {
        assert_eq!(
            choose_context_tokens(QWEN35_9B, Some(qwen35()), false, None),
            DEFAULT_CONTEXT_TOKENS
        );
        assert_eq!(
            choose_context_tokens(QWEN35_9B, None, false, Some(24_000)),
            DEFAULT_CONTEXT_TOKENS
        );
        // CPU-Backend: der KV-Cache liegt im RAM, dort gilt das Start-Gate.
        assert_eq!(
            choose_context_tokens(QWEN35_9B, Some(qwen35()), true, Some(24_000)),
            DEFAULT_CONTEXT_TOKENS
        );
    }

    #[test]
    fn the_env_override_accepts_only_sane_values() {
        assert_eq!(context_override(Some("8192")), Some(8_192));
        assert_eq!(context_override(Some(" 16384 ")), Some(16_384));
        assert_eq!(context_override(Some("1000")), None, "zu klein");
        assert_eq!(context_override(Some("1000000")), None, "zu gross");
        assert_eq!(context_override(Some("viel")), None);
        assert_eq!(context_override(None), None);
    }

    #[test]
    fn free_vram_is_the_tightest_dedicated_card_and_ignores_shared_memory() {
        let gpus = [gpu(24_000, 7_800, false), gpu(8_000, 6_000, false), gpu(30_000, 100, true)];
        assert_eq!(free_dedicated_vram_mb(&gpus), Some(2_000));
        // Nur eine iGPU: keine Aussage.
        assert_eq!(free_dedicated_vram_mb(&[gpu(30_000, 100, true)]), None);
        assert_eq!(free_dedicated_vram_mb(&[]), None);
        // Belegung ueber Budget: 0, nicht negativ.
        assert_eq!(free_dedicated_vram_mb(&[gpu(4_000, 5_000, false)]), Some(0));
    }
}
