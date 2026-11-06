//! Belege fuer die Abo-Vorschau (lva#334): was auf DIESEM Geraet fuer genau
//! diese CLI-Version gemessen bzw. geprueft wurde. Grundsatz: was nicht belegt
//! ist, sperrt (fail-closed). Es gibt keine eingebauten Zahlen und keine
//! eingebauten Faehigkeiten: ohne Eintrag in einer der beiden Dateien ist die
//! Vorschau fuer diese (CLI, Version, Programm, Modell) gesperrt.
//!
//! Beide Dateien liegen im App-Datenordner, werden vom Pruefskript
//! (`tooling/buch-gespraech/isolation-check.ps1` bzw. `.sh`) geschrieben und
//! von der App nur gelesen. Eine unlesbare Datei, ein unlesbarer oder
//! unvollstaendiger Eintrag zaehlt wie ein fehlender.
//!
//! # `book_talk_isolation_caps.json` (Faehigkeitstabelle, je CLI + Version)
//!
//! ```json
//! { "schema": 1,
//!   "entries": [ {
//!     "cli": "claude_cli",                     // oder "codex_cli"
//!     "cli_version": "2.1.318 (Claude Code)",  // erste Zeile von `--version`, exakt
//!     "kind": "empirisch",                     // nur gemessene Eintraege zaehlen
//!     "checked_at": "2026-11-07T10:00:00Z",
//!     "caps": {
//!       "instructions_blocked": true,          // Nutzer-Anweisungen (CLAUDE.md/AGENTS.md) wirken nicht
//!       "project_instructions_blocked": true,  // Anweisungen in Elternordnern des Arbeitsordners wirken nicht
//!       "hooks_blocked": true,                 // Hooks laufen nicht
//!       "memory_blocked": true,                // Gedaechtnis/Memory wird nicht geladen
//!       "tools_blocked": true,                 // Werkzeuge, MCP, Plugins, Skills, Apps aus
//!       "event_contract": true                 // Kompaktierung/Abschneiden erscheinen als Ereignis (Vertrag bestaetigt)
//!     } } ] }
//! ```
//!
//! Nur wenn ALLE sechs Felder `true` sind, meldet [`IsolationCaps::complete`]
//! „vollstaendig“. Fehlende Felder gelten als `false`.
//!
//! # `book_talk_preview_measurements.json` (Messreihe, je CLI + Version + Programm + Modell)
//!
//! ```json
//! { "schema": 1,
//!   "entries": [ {
//!     "cli": "claude_cli",
//!     "cli_version": "2.1.318 (Claude Code)",
//!     "binary_sha256": "<64 Hexzeichen, SHA-256 des tatsaechlich gestarteten Programms>",
//!     "model": "sonnet@high",                  // genau wie in der Konfiguration (inkl. Effort)
//!     "reported_model": "claude-sonnet-5-5",   // was die CLI in der Ausgabe als Modell nennt
//!     "runs": 10,                              // N >= 10
//!     "max_overhead_tokens": 31234,            // Maximum der Roh-Eingabetoken bei leerem Prompt
//!     "reasoning_reserve_tokens": 3100,        // Maximum der Roh-Ausgabetoken (Denken) bei kurzer Antwort
//!     "compaction_threshold_percent": 80,      // Anteil des Kontexts, ab dem nicht mehr geplant wird (1..=100)
//!     "measured_at": "2026-11-07T10:00:00Z",
//!     "kind": "empirisch",
//!     "raw_usage": [ { "input": 31200, "output": 40 } ]   // optional, Rohwerte der Laeufe
//!   } ] }
//! ```
//!
//! Gebunden ist die Messung an (`cli`, `cli_version`, `binary_sha256`,
//! `model`); jede Abweichung ist „keine Messung“. Mehrere passende Eintraege
//! werden zum unguenstigsten Wert zusammengefasst; nennen sie verschiedene
//! `reported_model`, gilt keiner.

use std::path::Path;

use serde::Deserialize;

/// Name der Faehigkeitstabelle im App-Datenordner.
pub const CAPS_FILE: &str = "book_talk_isolation_caps.json";
/// Name der Messdatei im App-Datenordner.
pub const MEASUREMENTS_FILE: &str = "book_talk_preview_measurements.json";
/// Einzige zulaessige Herkunft eines Eintrags.
pub const KIND_EMPIRICAL: &str = "empirisch";
/// Mindestzahl der Laeufe einer Messreihe.
pub const MIN_RUNS: u32 = 10;

/// Was die Isolation der CLI in einer bestimmten Version nachweislich leistet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct IsolationCaps {
    #[serde(default)]
    pub instructions_blocked: bool,
    #[serde(default)]
    pub project_instructions_blocked: bool,
    #[serde(default)]
    pub hooks_blocked: bool,
    #[serde(default)]
    pub memory_blocked: bool,
    #[serde(default)]
    pub tools_blocked: bool,
    #[serde(default)]
    pub event_contract: bool,
}

impl IsolationCaps {
    /// Alle Faehigkeiten belegt?
    pub fn complete(&self) -> bool {
        self.instructions_blocked
            && self.project_instructions_blocked
            && self.hooks_blocked
            && self.memory_blocked
            && self.tools_blocked
            && self.event_contract
    }

    /// Nur, was beide belegen.
    fn both(self, other: Self) -> Self {
        Self {
            instructions_blocked: self.instructions_blocked && other.instructions_blocked,
            project_instructions_blocked: self.project_instructions_blocked
                && other.project_instructions_blocked,
            hooks_blocked: self.hooks_blocked && other.hooks_blocked,
            memory_blocked: self.memory_blocked && other.memory_blocked,
            tools_blocked: self.tools_blocked && other.tools_blocked,
            event_contract: self.event_contract && other.event_contract,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct CapsEntry {
    cli: String,
    cli_version: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    checked_at: String,
    #[serde(default)]
    caps: IsolationCaps,
}

/// Eine Messreihe (siehe Moduldoku).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Measurement {
    pub cli: String,
    pub cli_version: String,
    pub binary_sha256: String,
    pub model: String,
    pub reported_model: String,
    pub runs: u32,
    pub max_overhead_tokens: u64,
    pub reasoning_reserve_tokens: u32,
    pub compaction_threshold_percent: u64,
    pub measured_at: String,
    #[serde(default)]
    pub kind: String,
}

impl Measurement {
    /// Ist der Eintrag in sich schluessig? Ein unvollstaendiger gilt als fehlend.
    pub fn is_valid(&self) -> bool {
        self.kind == KIND_EMPIRICAL
            && self.runs >= MIN_RUNS
            && self.max_overhead_tokens > 0
            && self.reasoning_reserve_tokens > 0
            && (1..=100).contains(&self.compaction_threshold_percent)
            && !self.measured_at.trim().is_empty()
            && !self.reported_model.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.cli_version.trim().is_empty()
            && self.binary_sha256.len() == 64
            && self.binary_sha256.chars().all(|c| c.is_ascii_hexdigit())
    }

    /// Der ungeuenstigere von zwei Eintraegen (hoeherer Overhead und
    /// Reserve, niedrigere Schwelle und Laufzahl).
    fn worst(mut self, other: &Self) -> Self {
        self.max_overhead_tokens = self.max_overhead_tokens.max(other.max_overhead_tokens);
        self.reasoning_reserve_tokens = self
            .reasoning_reserve_tokens
            .max(other.reasoning_reserve_tokens);
        self.compaction_threshold_percent = self
            .compaction_threshold_percent
            .min(other.compaction_threshold_percent);
        self.runs = self.runs.min(other.runs);
        self
    }
}

/// Faehigkeiten und Messungen dieses Geraets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Evidence {
    caps: Vec<CapsEntry>,
    measurements: Vec<Measurement>,
}

/// Die Eintraege einer Datei; was sich nicht lesen laesst, faellt weg.
fn entries<T: for<'de> Deserialize<'de>>(raw: &str) -> Vec<T> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    if v.get("schema").and_then(|s| s.as_u64()) != Some(1) {
        return Vec::new();
    }
    v.get("entries")
        .and_then(|e| e.as_array())
        .into_iter()
        .flatten()
        .filter_map(|e| serde_json::from_value::<T>(e.clone()).ok())
        .collect()
}

impl Evidence {
    /// Aus den Rohtexten der beiden Dateien.
    pub fn parse(caps_json: &str, measurements_json: &str) -> Self {
        Self {
            caps: entries(caps_json),
            measurements: entries(measurements_json),
        }
    }

    /// Liest beide Dateien aus dem App-Datenordner; fehlende Dateien geben
    /// leere Belege (alles gesperrt).
    pub fn load(dir: &Path) -> Self {
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap_or_default();
        Self::parse(&read(CAPS_FILE), &read(MEASUREMENTS_FILE))
    }

    /// Faehigkeiten fuer (CLI, Version): nur gemessene Eintraege, bei mehreren
    /// gilt, was alle belegen. `None`: nichts belegt.
    pub fn caps_for(&self, cli: &str, version: &str) -> Option<IsolationCaps> {
        self.caps
            .iter()
            .filter(|e| {
                e.cli == cli
                    && e.cli_version == version
                    && e.kind == KIND_EMPIRICAL
                    && !e.checked_at.trim().is_empty()
            })
            .map(|e| e.caps)
            .reduce(IsolationCaps::both)
    }

    /// Messung fuer genau (CLI, Version, Programm-Hash, Modell).
    pub fn measurement_for(
        &self,
        cli: &str,
        version: &str,
        sha256: &str,
        model: &str,
    ) -> Option<Measurement> {
        let mut matching = self.measurements.iter().filter(|m| {
            m.is_valid()
                && m.cli == cli
                && m.cli_version == version
                && m.binary_sha256.eq_ignore_ascii_case(sha256)
                && m.model == model
        });
        let first = matching.next()?.clone();
        matching.try_fold(first, |acc, m| {
            (acc.reported_model == m.reported_model).then(|| acc.worst(m))
        })
    }

    #[cfg(test)]
    pub fn from_parts(
        caps: &[(&str, &str, IsolationCaps)],
        measurements: Vec<Measurement>,
    ) -> Self {
        Self {
            caps: caps
                .iter()
                .map(|(cli, version, caps)| CapsEntry {
                    cli: cli.to_string(),
                    cli_version: version.to_string(),
                    kind: KIND_EMPIRICAL.into(),
                    checked_at: "2026-11-07T10:00:00Z".into(),
                    caps: *caps,
                })
                .collect(),
            measurements,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12";

    fn caps_json(extra: &str) -> String {
        format!(
            r#"{{"schema":1,"entries":[{{"cli":"claude_cli","cli_version":"2.1.318 (Claude Code)","kind":"empirisch","checked_at":"2026-11-07T10:00:00Z","caps":{{"instructions_blocked":true,"project_instructions_blocked":true,"hooks_blocked":true,"memory_blocked":true,"tools_blocked":true{extra}}}}}]}}"#
        )
    }

    fn measurement_json(overrides: &[(&str, &str)]) -> String {
        let mut v = serde_json::json!({
            "cli": "claude_cli", "cli_version": "2.1.318 (Claude Code)",
            "binary_sha256": SHA, "model": "sonnet@high",
            "reported_model": "claude-sonnet-5-5", "runs": 10,
            "max_overhead_tokens": 31234, "reasoning_reserve_tokens": 3100,
            "compaction_threshold_percent": 80,
            "measured_at": "2026-11-07T10:00:00Z", "kind": "empirisch"
        });
        for (k, raw) in overrides {
            v[*k] = serde_json::from_str(raw).unwrap();
        }
        serde_json::json!({"schema": 1, "entries": [v]}).to_string()
    }

    /// Form, wie das Pruefskript sie schreibt (mit `notes` und `raw_usage`).
    #[test]
    fn the_files_of_the_check_script_are_read() {
        let caps = r#"{"schema":1,"entries":[{"cli":"codex_cli","cli_version":"codex-cli 9.9.9","kind":"empirisch","checked_at":"2026-11-07T10:00:00Z","caps":{"instructions_blocked":true,"project_instructions_blocked":true,"hooks_blocked":true,"memory_blocked":true,"tools_blocked":true,"event_contract":true},"notes":""}]}"#;
        let measured = r#"{"schema":1,"entries":[{"cli":"codex_cli","cli_version":"codex-cli 9.9.9","binary_sha256":"30f778cb43690b0a19f74a85c5cbe7d0b3901be2dd5c3f166d4bd9149528bc97","model":"gpt-6.1-sol","reported_model":"gpt-6.1-sol","runs":10,"max_overhead_tokens":20075,"reasoning_reserve_tokens":38,"compaction_threshold_percent":80,"measured_at":"2026-11-07T10:00:00Z","kind":"empirisch","raw_usage":[{"input":20001,"output":31}]}]}"#;
        let e = Evidence::parse(caps, measured);
        assert!(e
            .caps_for("codex_cli", "codex-cli 9.9.9")
            .unwrap()
            .complete());
        let m = e
            .measurement_for(
                "codex_cli",
                "codex-cli 9.9.9",
                "30f778cb43690b0a19f74a85c5cbe7d0b3901be2dd5c3f166d4bd9149528bc97",
                "gpt-6.1-sol",
            )
            .unwrap();
        assert_eq!(
            (m.max_overhead_tokens, m.reasoning_reserve_tokens),
            (20_075, 38)
        );
    }

    #[test]
    fn nothing_is_proven_without_files() {
        let e = Evidence::load(&std::env::temp_dir().join("lva-gibt-es-nicht"));
        assert_eq!(e, Evidence::default());
        assert!(e.caps_for("claude_cli", "1").is_none());
        assert!(e
            .measurement_for("claude_cli", "1", SHA, "sonnet")
            .is_none());
    }

    #[test]
    fn caps_are_complete_only_when_every_field_is_true() {
        let e = Evidence::parse(&caps_json(r#","event_contract":true"#), "");
        let caps = e.caps_for("claude_cli", "2.1.318 (Claude Code)").unwrap();
        assert!(caps.complete());
        // Fehlendes Feld = false.
        let e = Evidence::parse(&caps_json(""), "");
        let caps = e.caps_for("claude_cli", "2.1.318 (Claude Code)").unwrap();
        assert!(!caps.event_contract && !caps.complete());
        // Andere Version oder andere CLI: nichts belegt.
        assert!(e.caps_for("claude_cli", "2.1.319 (Claude Code)").is_none());
        assert!(e.caps_for("codex_cli", "2.1.318 (Claude Code)").is_none());
    }

    #[test]
    fn caps_need_the_empirical_kind_and_a_date() {
        let raw = caps_json(r#","event_contract":true"#).replace("empirisch", "geschaetzt");
        assert!(Evidence::parse(&raw, "")
            .caps_for("claude_cli", "2.1.318 (Claude Code)")
            .is_none());
        let raw = caps_json(r#","event_contract":true"#).replace("2026-11-07T10:00:00Z", "");
        assert!(Evidence::parse(&raw, "")
            .caps_for("claude_cli", "2.1.318 (Claude Code)")
            .is_none());
    }

    #[test]
    fn a_file_with_a_wrong_schema_or_garbage_counts_as_missing() {
        for raw in [
            "",
            "kaputt",
            r#"{"schema":2,"entries":[]}"#,
            r#"{"entries":[]}"#,
        ] {
            assert_eq!(Evidence::parse(raw, raw), Evidence::default(), "{raw}");
        }
    }

    #[test]
    fn a_measurement_binds_cli_version_hash_and_model() {
        let e = Evidence::parse("", &measurement_json(&[]));
        let find = |cli, ver, sha: &str, model| e.measurement_for(cli, ver, sha, model);
        let v = "2.1.318 (Claude Code)";
        let m = find("claude_cli", v, SHA, "sonnet@high").expect("passt");
        assert_eq!(m.max_overhead_tokens, 31_234);
        assert!(find("claude_cli", v, &SHA.to_uppercase(), "sonnet@high").is_some());
        assert!(find("codex_cli", v, SHA, "sonnet@high").is_none());
        assert!(find("claude_cli", "2.1.319", SHA, "sonnet@high").is_none());
        assert!(find("claude_cli", v, &"cd".repeat(32), "sonnet@high").is_none());
        // Auch ein anderer Effort ist ein anderes Modell.
        assert!(find("claude_cli", v, SHA, "sonnet@low").is_none());
        assert!(find("claude_cli", v, SHA, "sonnet").is_none());
    }

    #[test]
    fn an_invalid_measurement_is_ignored() {
        for (k, v) in [
            ("runs", "9"),
            ("max_overhead_tokens", "0"),
            ("reasoning_reserve_tokens", "0"),
            ("compaction_threshold_percent", "0"),
            ("compaction_threshold_percent", "101"),
            ("kind", r#""geschaetzt""#),
            ("measured_at", r#""""#),
            ("reported_model", r#""""#),
            ("binary_sha256", r#""abc""#),
        ] {
            let e = Evidence::parse("", &measurement_json(&[(k, v)]));
            assert!(
                e.measurement_for("claude_cli", "2.1.318 (Claude Code)", SHA, "sonnet@high")
                    .is_none(),
                "{k}={v}"
            );
        }
    }

    #[test]
    fn several_entries_merge_to_the_worst_and_disagreement_blocks() {
        let one: serde_json::Value = serde_json::from_str(&measurement_json(&[])).unwrap();
        let mut worse = one["entries"][0].clone();
        worse["max_overhead_tokens"] = 40_000.into();
        worse["compaction_threshold_percent"] = 70.into();
        let mut other_model = one["entries"][0].clone();
        other_model["reported_model"] = "claude-sonnet-5-6".into();
        let merged = serde_json::json!({"schema":1,"entries":[one["entries"][0], worse]});
        let e = Evidence::parse("", &merged.to_string());
        let m = e
            .measurement_for("claude_cli", "2.1.318 (Claude Code)", SHA, "sonnet@high")
            .unwrap();
        assert_eq!(
            (m.max_overhead_tokens, m.compaction_threshold_percent),
            (40_000, 70)
        );
        let clash = serde_json::json!({"schema":1,"entries":[one["entries"][0], other_model]});
        let e = Evidence::parse("", &clash.to_string());
        assert!(e
            .measurement_for("claude_cli", "2.1.318 (Claude Code)", SHA, "sonnet@high")
            .is_none());
    }
}
