//! Engine-Abstraktion des Vorlesens (Paket A3, Piper seit Paket E2).
//!
//! Hier liegt die NAHT, über die neben Fish Speech eine zweite Engine
//! ([`super::piper::PiperEngine`], CPU) spricht, ohne dass die
//! Satz-Pipeline, die Caches oder der Server-Lebenszyklus in `mod.rs`
//! umgebaut werden mussten. Fish-Verhalten bleibt bit-identisch: der Kern
//! dispatcht Fish direkt auf die BESTEHENDEN Pfade (HTTP-POST in
//! `TtsCore::fish_synthesize`, Startlogik in `TtsManager::ensure_server`).
//! Der frühere `FishEngine`-Wrapper ist entfallen: sein `synthesize` zog
//! den Port aus einer eigenen Quelle und lief in Produktion nie — toter
//! Code mit Drift-Risiko (Review-Befund zu A3/E1).
//!
//! ## Entscheidung: async ohne `async-trait`, Dispatch per Enum
//!
//! `async-trait` ist keine Dependency dieses Crates, und eine neue kommt
//! dafür nicht ins Haus. Die async-Methoden des Traits sind deshalb als
//! `-> impl Future + Send` deklariert (RPITIT, seit Rust 1.75) — Impls
//! schreiben ganz normal `async fn`. Ein so deklarierter Trait ist nicht
//! dyn-kompatibel; der Kern dispatcht darum über ein internes Enum
//! (`EngineImpl` in mod.rs) mit derselben öffentlichen Formsprache. Das ist
//! im Task-Brief ausdrücklich zugelassen: Ziel ist die Naht, nicht die
//! Ideologie. Ein `dyn TtsEngine` wäre hier ohnehin unhandlich, weil die
//! Fish-Synthese `&TtsCore` braucht (HTTP-Client, Port, Caches) — ein
//! besitzendes Trait-Objekt im Kern ergäbe eine Besitz-Schleife.

/// Welche Synthese-Engine spricht.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, specta::Type,
)]
pub enum TtsEngineKind {
    Fish,
    Piper,
}

impl TtsEngineKind {
    /// Settings-Wert (`tts_engine`) in eine Engine-Art übersetzen.
    /// Unbekannte Werte fallen auf Fish zurück — der sichere Standard.
    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "piper" => Self::Piper,
            _ => Self::Fish,
        }
    }
}

/// Was eine Engine kann. Die Oberfläche und der Manager entscheiden daran,
/// welche Bedienelemente sinnvoll sind und ob die GPU als belegt gilt.
///
/// dead_code: außer `needs_gpu` liest die Felder erst das UI-Paket (E4) —
/// der Vertrag ist trotzdem schon vollständig definiert.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct EngineCaps {
    pub style_tags: bool,
    pub cloning: bool,
    pub voice_switching: bool,
    /// Heute überall false; vorbereitet für Engines, die Häppchen liefern.
    pub streaming: bool,
    pub needs_gpu: bool,
    pub export_formats: &'static [&'static str],
}

/// Fähigkeiten von Fish Speech: klont Stimmen, wechselt sie je Satz,
/// versteht Stil-Tags, encodiert Exporte direkt — und belegt dafür die GPU.
pub const FISH_CAPS: EngineCaps = EngineCaps {
    style_tags: true,
    cloning: true,
    voice_switching: true,
    streaming: false,
    needs_gpu: true,
    export_formats: &["wav", "mp3", "opus"],
};

/// Ein Syntheseauftrag: genau ein Satz, eine Stimme, ein Seed.
#[derive(Clone, Copy, Debug)]
pub struct SynthesisRequest<'a> {
    pub text: &'a str,
    pub voice: Option<&'a str>,
    pub seed: i64,
    /// Fish: ignoriert. Piper: wird zu `--length_scale` (Kehrwert, geklemmt).
    /// Der Kern übergibt heute für BEIDE Engines `None`: das Tempo regelt
    /// einheitlich die Wiedergabe — eingebacken läge es dauerhaft im Cache
    /// und gälte doppelt (der Player skaliert bereits).
    pub speed: Option<f32>,
}

/// Die Naht selbst. Jede Engine beantwortet dieselben fünf Fragen:
/// wer bist du, was kannst du, wie heißt dein Cache-Fach, bist du bereit,
/// und: sprich diesen Satz.
///
/// In Produktion übt die Piper-Engine den Trait aus (Fish dispatcht der
/// Kern per Enum direkt auf seine bestehenden Pfade); in den Tests
/// zusätzlich die Mock-Engine. dead_code: `shutdown` ruft heute niemand —
/// der Server-Lebenszyklus gehört dem Manager.
#[allow(dead_code)]
pub trait TtsEngine: Send + Sync {
    fn kind(&self) -> TtsEngineKind;
    fn caps(&self) -> EngineCaps;
    /// Anteil der Engine am Cache-Schlüssel. Fish: `""` (Legacy-Regel —
    /// siehe [`fish_cache_tag`]); andere Engines: `"<engine>/<stimme>"`,
    /// damit sich zwei Engines nie einen Cache-Eintrag teilen.
    fn cache_tag(&self, voice: Option<&str>) -> String;
    fn ensure_ready(&self) -> impl std::future::Future<Output = Result<(), String>> + Send;
    /// Einen Satz synthetisieren; Rückgabe sind WAV-Bytes.
    fn synthesize(
        &self,
        req: SynthesisRequest<'_>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, String>> + Send;
    fn shutdown(&self) {}
}

/// Cache-Tag der Fish-Engine: IMMER leer, unabhängig von der Stimme.
///
/// KRITISCH (Legacy-Regel): der leere Tag geht in `WavCache::key` NICHT in
/// den Hash ein. Jeder vor der Engine-Abstraktion erzeugte Schlüssel — im
/// RAM wie im Platten-Cache (`{key:016x}.wav`) — bleibt damit byte-identisch
/// gültig, und bereits synthetisierte Bücher sind weiterhin offline hörbar.
/// Die Stimme braucht der Tag nicht: sie steckt bereits selbst im Schlüssel.
pub fn fish_cache_tag(_voice: Option<&str>) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbekannte_settings_werte_fallen_auf_fish_zurueck() {
        assert_eq!(TtsEngineKind::from_setting("fish"), TtsEngineKind::Fish);
        assert_eq!(TtsEngineKind::from_setting("piper"), TtsEngineKind::Piper);
        assert_eq!(TtsEngineKind::from_setting(" Piper "), TtsEngineKind::Piper);
        assert_eq!(TtsEngineKind::from_setting("kokoro"), TtsEngineKind::Fish);
        assert_eq!(TtsEngineKind::from_setting(""), TtsEngineKind::Fish);
    }

    /// Die Legacy-Regel in einem Satz: Fish trägt keinen Tag, egal welche
    /// Stimme spricht — sonst verlöre jeder Bestandsnutzer seinen Cache.
    #[test]
    fn der_fish_cache_tag_ist_leer_und_ignoriert_die_stimme() {
        assert_eq!(fish_cache_tag(None), "");
        assert_eq!(fish_cache_tag(Some("patrick")), "");
    }

    #[test]
    fn fish_braucht_die_gpu_und_streamt_nicht() {
        assert!(FISH_CAPS.needs_gpu);
        assert!(!FISH_CAPS.streaming);
        assert!(FISH_CAPS.cloning && FISH_CAPS.voice_switching && FISH_CAPS.style_tags);
        assert_eq!(FISH_CAPS.export_formats, &["wav", "mp3", "opus"]);
    }
}
