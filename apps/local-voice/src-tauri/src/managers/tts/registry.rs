//! Sprecher-Registry: Metadaten je Referenzstimme (`VoiceMeta`), die MIT dem
//! Ordner `<fish_dir>/references/<voice_id>/meta.json` wandern.
//!
//! Bewusst in reine Funktionen (Default, Validierung, Analyse-Heuristik) und
//! I/O (Lesen/Schreiben) getrennt — die reinen Teile sind ohne Dateisystem
//! testbar. Fehlt `meta.json` (jede Stimme vor dieser Registry, oder eine von
//! Hand angelegte), liefert [`read_meta`] sofort einen Default zurück statt
//! einer Migration: Bestandsstimmen funktionieren ohne weiteres Zutun.

use std::path::Path;

use super::voices;

/// Aktuelle Metadaten-Version. Steigt nur bei einer migrationspflichtigen
/// Formatänderung — ein zusätzliches Feld braucht keine neue Version, weil
/// jedes optionale Feld über `#[serde(default)]` toleriert wird (siehe unten).
pub const CURRENT_META_VERSION: u32 = 1;

/// Farbpalette in fester Reihenfolge — die Reihenfolge IST der Vertrag: Index
/// `i` in dieser Liste muss immer derselbe Palette-Key bleiben, weil
/// [`default_color`] über den Index in genau diese Liste indiziert und das
/// Frontend dieselbe Reihenfolge separat nachbildet (siehe dort).
pub const PALETTE: [&str; 10] = [
    "slate", "red", "orange", "amber", "green", "teal", "sky", "violet", "fuchsia", "rose",
];

/// Metadaten einer Referenzstimme.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct VoiceMeta {
    #[serde(default = "current_version")]
    pub version: u32,
    /// Freier Anzeigename (Umlaute erlaubt) — NICHT die voice_id (die bleibt
    /// der sanierte Ordnername, siehe `voices::sanitize_voice_id`).
    pub display_name: String,
    /// Palette-Key (`"teal"`, `"rose"`, …), KEIN Hex-Wert.
    pub color: String,
    #[serde(default)]
    pub avatar: Option<Avatar>,
    /// BCP-47-Sprachcode, z. B. `"de-DE"`.
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Tags, die beim Vorlesen mit dieser Stimme automatisch gelten sollen —
    /// z. B. `["volume up"]` als dauerhafte Kompensation einer leisen
    /// Referenz (siehe [`ReferenceAnalysis`]).
    #[serde(default)]
    pub default_tags: Vec<String>,
    #[serde(default)]
    pub default_style: Option<String>,
    #[serde(default)]
    pub styles: Vec<VoiceStyle>,
}

fn current_version() -> u32 {
    CURRENT_META_VERSION
}

/// Avatarquelle einer Stimme: ein hochgeladenes Bild oder ein Icon-Name aus
/// dem eingebauten Satz.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Avatar {
    Image { file: String },
    Icon { name: String },
}

/// Ein benannter Stil einer Stimme (z. B. „fluesternd") mit eigener,
/// optionaler Referenzaufnahme.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct VoiceStyle {
    /// Saniert, eindeutig je Stimme.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Interne reference_id `__style_<voice>_<style>` — siehe
    /// [`super::voices::style_dir`].
    #[serde(default)]
    pub reference: Option<String>,
}

/// Herkunft einer Stimme: aus einem Seed abgeleitet (Standardstimme
/// festgehalten) oder aus einer echten Aufnahme/einem Import.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum VoiceOrigin {
    Seed(i64),
    Recording,
}

/// Eine Stimme mit ihren Metadaten, für die Stimmenübersicht.
#[derive(Debug, Clone, serde::Serialize, specta::Type)]
pub struct VoiceInfo {
    pub id: String,
    pub meta: VoiceMeta,
    pub origin: VoiceOrigin,
    /// Absoluter Pfad zur Avatar-Datei, falls eine existiert — die
    /// Oberfläche spielt sie über das asset-Protokoll aus, ohne sie zu
    /// kopieren (Muster von `VoiceSample::wav_path`).
    pub avatar_path: Option<String>,
}

/// Name der Metadatendatei je Stimmenordner.
const META_FILE: &str = "meta.json";

/// Metadaten einer Stimme lesen. Fehlt `meta.json` (oder ist sie kaputt),
/// kommt [`default_meta`] zurück — nie ein Fehler: eine Stimme ohne
/// Metadaten ist der Normalfall jeder Stimme vor dieser Registry.
pub fn read_meta(fish_dir: &Path, id: &str) -> VoiceMeta {
    let path = voices::voice_dir(fish_dir, id).join(META_FILE);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| default_meta(id, &voices::list_voices(fish_dir)))
}

/// Metadaten atomar schreiben (temp-Datei + rename), damit ein Absturz
/// mitten im Schreiben nie eine halbe `meta.json` hinterlässt.
pub fn write_meta(fish_dir: &Path, id: &str, meta: &VoiceMeta) -> Result<(), String> {
    let dir = voices::voice_dir(fish_dir, id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| format!("could not serialize meta for {id}: {e}"))?;
    let path = dir.join(META_FILE);
    let tmp = dir.join(format!("{META_FILE}.tmp"));
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("could not finalize {}: {e}", path.display()))
}

/// FNV-1a (64-bit) über die rohen UTF-8-Bytes der id.
///
/// Bewusst kein externes Hash-Crate (keine neue Dependency): FNV-1a ist ein
/// Dutzend Zeilen, feststehend spezifiziert und deshalb 1:1 in JavaScript
/// nachbaubar — das Frontend spiegelt GENAU diesen Algorithmus, damit ein
/// noch nie gespeicherter Stimmenname schon vor dem ersten Backend-Rundlauf
/// dieselbe Vorschaufarbe zeigt wie später die Registry.
///
/// Algorithmus (auch für die JS-Seite): `hash = 0xcbf29ce484222325`; für
/// jedes Byte `b` der UTF-8-Kodierung: `hash = (hash XOR b) * 0x100000001b3`
/// (Multiplikation modulo 2^64, also mit Wrap-Around). Die Farbe ist dann
/// `PALETTE[hash % 10]` in der oben festgehaltenen Reihenfolge.
fn fnv1a_hash(input: &str) -> u64 {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET_BASIS;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// Deterministische Default-Farbe einer id — reiner Hash, unabhängig von
/// allen anderen Stimmen (siehe [`fnv1a_hash`] für den Algorithmus, der auch
/// vom Frontend nachgebildet wird).
pub fn default_color(id: &str) -> &'static str {
    let index = (fnv1a_hash(id) % PALETTE.len() as u64) as usize;
    PALETTE[index]
}

/// `anna_m` → `Anna M`: Wortgrenzen an `_`/`-`, jedes Wort großgeschrieben.
/// Leere Stücke (mehrfache Trenner) fallen weg.
fn humanize(id: &str) -> String {
    id.split(['_', '-'])
        .filter(|word| !word.is_empty())
        .map(capitalize_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Default-Anzeigename aus der id, mit Kollisionsschutz gegen andere
/// (ebenfalls noch metadatenlose) Stimmen in `all_ids`.
///
/// Ohne diesen Schutz könnten zwei Ids, die auf denselben Namen humanisieren
/// (z. B. `anna-m` und `anna_m` → beide „Anna M"), zwei Stimmen mit
/// IDENTISCHEM Default-Anzeigenamen ergeben — und `validate_meta` lehnt
/// genau das ab, sobald man auch nur EINE der beiden ohne Änderung am Namen
/// speichert. Bei einer Kollision behält die (nach Ids) alphabetisch
/// ERSTE Stimme den schlichten Namen, jede andere bekommt ihre id in
/// Klammern angehängt — deterministisch, unabhängig von der Reihenfolge in
/// `all_ids`.
fn default_display_name(id: &str, all_ids: &[String]) -> String {
    let name = humanize(id);
    let name = if name.is_empty() {
        id.to_string()
    } else {
        name
    };
    let lower = name.to_lowercase();
    let yields_to_smaller_id = all_ids
        .iter()
        .any(|other| other != id && humanize(other).to_lowercase() == lower && other.as_str() < id);
    if yields_to_smaller_id {
        format!("{name} ({id})")
    } else {
        name
    }
}

/// Default-Metadaten für eine Stimme ohne `meta.json`. `all_ids` sind alle
/// (auch metadatenlosen) Stimmen-Ids — für den Kollisionsschutz des
/// Anzeigenamens, siehe [`default_display_name`]. Die Farbe hängt davon
/// NICHT ab (siehe [`default_color`]).
pub fn default_meta(id: &str, all_ids: &[String]) -> VoiceMeta {
    VoiceMeta {
        version: CURRENT_META_VERSION,
        display_name: default_display_name(id, all_ids),
        color: default_color(id).to_string(),
        avatar: None,
        language: None,
        description: None,
        default_tags: Vec::new(),
        default_style: None,
        styles: Vec::new(),
    }
}

/// Validiert Metadaten vor dem Speichern. `others` sind (voice_id,
/// display_name) aller ANDEREN Stimmen — der Anzeigename muss sich
/// unicode-case-insensitiv gegen jeden von beidem unterscheiden.
///
/// Warum case-insensitiv über `to_lowercase()` und nicht
/// `eq_ignore_ascii_case`: Umlaute müssen genauso erkannt werden wie ASCII
/// (`„MÜLLER"` und `„müller"` sind derselbe Name) — dasselbe Muster wie
/// `protocol::resolve_speaker`.
///
/// Kein `<`, `>`, `:` und keine Zeilenumbrüche: der Anzeigename landet roh
/// in generierten Dateien/Konfigurationen; diese Zeichen dort sind ein
/// Parser-Risiko, kein Tippfehler.
pub fn validate_meta(meta: &VoiceMeta, others: &[(String, String)]) -> Result<(), String> {
    let name = meta.display_name.trim();
    if name.is_empty() {
        return Err("display_name darf nicht leer sein".to_string());
    }
    if name.contains(['<', '>', ':']) || name.contains(['\n', '\r']) {
        return Err(
            "display_name darf keine der Zeichen <, >, : oder Zeilenumbrueche enthalten"
                .to_string(),
        );
    }
    let lower = name.to_lowercase();
    for (other_id, other_name) in others {
        if other_id.to_lowercase() == lower || other_name.to_lowercase() == lower {
            return Err(format!(
                "der Anzeigename '{name}' wird bereits von '{other_id}' verwendet"
            ));
        }
    }
    Ok(())
}

/// Ergebnis der Referenz-Analyse: nur ein Vorschlag, nie automatisch aktiv —
/// der Aufrufer entscheidet, ob `suggested_tags` in `default_tags`
/// übernommen werden.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct ReferenceAnalysis {
    pub quiet: bool,
    pub suggested_tags: Vec<String>,
}

/// Unterhalb dieser integrierten Lautheit (siehe `loudness::loudness_lufs`)
/// gilt eine Referenz als deutlich leiser als der Zielpegel
/// (`loudness::TARGET_LUFS` = -20 LUFS). -30 LUFS liegt 10 LU darunter —
/// spürbar leise gesprochen, nicht nur eine ruhige Passage.
const QUIET_LUFS_THRESHOLD: f32 = -30.0;

/// Zweites, unabhängiges Indiz: die Spitzenamplitude selbst ist niedrig
/// (~ -20 dBFS oder leiser). Eine Aufnahme mit niedriger integrierter
/// Lautheit, aber HOHER Spitze hat eine große Dynamik (z. B. ein einzelner
/// lauter Einsatz in sonst leiser Rede) — das Pegeln (siehe
/// `voices::normalize_gain`) fängt das schon ab, und "volume up" wäre hier
/// eine unnötige Dauerkompensation. Erst wenn BEIDE Werte niedrig sind
/// (leise UND wenig Dynamik nach oben), ist die Referenz durchgehend leise
/// aufgenommen und "volume up" eine sinnvolle Vorgabe.
const LOW_DYNAMIC_PEAK: f32 = 0.1;

/// Heuristik über 16-kHz-Mono-Samples (Muster von `loudness::gain_for_mono`):
/// niedrige integrierte Lautheit UND geringe Dynamik ⇒ Vorschlag
/// `["volume up"]`. Nicht messbar (zu kurz, durchgehend Stille) ⇒ kein
/// Vorschlag — im Zweifel wird nichts vorgeschlagen, nicht geraten.
pub fn analyze_reference(samples: &[f32], sample_rate: u32) -> ReferenceAnalysis {
    let peak = super::loudness::peak(samples);
    let quiet = super::loudness::loudness_lufs(samples, sample_rate)
        .is_some_and(|lufs| lufs < QUIET_LUFS_THRESHOLD && peak < LOW_DYNAMIC_PEAK);
    ReferenceAnalysis {
        quiet,
        suggested_tags: if quiet {
            vec!["volume up".to_string()]
        } else {
            Vec::new()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- default_meta: Determinismus & Frontend-Paritaet ----------------

    /// Fixtures mit den ERWARTETEN Palette-Keys: das Frontend baut denselben
    /// FNV-1a-Hash nach und muss fuer dieselbe id dieselbe Farbe zeigen.
    /// Aendert sich hier eine Zuordnung, muss die JS-Seite mitgezogen werden.
    #[test]
    fn default_color_ist_deterministisch_und_dokumentiert_die_frontend_paritaet() {
        // Werte per FNV-1a nachgerechnet (siehe Doc-Kommentar an
        // `fnv1a_hash`) — genau diese Zuordnung baut das Frontend nach.
        assert_eq!(default_color("patrick"), "violet");
        assert_eq!(default_color("olga"), "sky");
        assert_eq!(default_color("anna_m"), "red");
    }

    #[test]
    fn default_color_haengt_nur_an_der_id_nicht_an_anderen_stimmen() {
        let c1 = default_meta("patrick", &[]).color;
        let c2 = default_meta("patrick", &["a".into(), "b".into(), "c".into()]).color;
        assert_eq!(c1, c2);
    }

    #[test]
    fn display_name_wird_aus_der_id_humanisiert() {
        assert_eq!(default_meta("anna_m", &[]).display_name, "Anna M");
        assert_eq!(
            default_meta("frau-mueller", &[]).display_name,
            "Frau Mueller"
        );
        assert_eq!(default_meta("olga", &[]).display_name, "Olga");
    }

    #[test]
    fn kollidierende_default_namen_werden_deterministisch_entzerrt() {
        let ids = vec!["anna-m".to_string(), "anna_m".to_string()];
        assert_eq!(default_meta("anna-m", &ids).display_name, "Anna M");
        assert_eq!(default_meta("anna_m", &ids).display_name, "Anna M (anna_m)");
    }

    #[test]
    fn ohne_kollision_bleibt_der_name_schlicht() {
        let ids = vec!["anna_m".to_string(), "olga".to_string()];
        assert_eq!(default_meta("anna_m", &ids).display_name, "Anna M");
    }

    // ---- VoiceOrigin: JSON-Form absichern ---------------------------------

    #[test]
    fn voice_origin_serialisiert_adjacently_tagged() {
        // Ein Seed traegt einen Wert (i64), der bei intern getaggten Enums
        // (nur `tag`) nicht darstellbar waere — deshalb `tag` + `content`.
        assert_eq!(
            serde_json::to_value(VoiceOrigin::Seed(42)).unwrap(),
            serde_json::json!({"kind": "seed", "value": 42})
        );
        assert_eq!(
            serde_json::to_value(VoiceOrigin::Recording).unwrap(),
            serde_json::json!({"kind": "recording"})
        );
    }

    // ---- Roundtrip / Persistenz ------------------------------------------

    #[test]
    fn meta_roundtrip_ueber_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let meta = VoiceMeta {
            version: 1,
            display_name: "Anna Müller".to_string(),
            color: "teal".to_string(),
            avatar: Some(Avatar::Icon {
                name: "star".to_string(),
            }),
            language: Some("de-DE".to_string()),
            description: Some("Warm, ruhig".to_string()),
            default_tags: vec!["volume up".to_string()],
            default_style: Some("fluesternd".to_string()),
            styles: vec![VoiceStyle {
                id: "fluesternd".to_string(),
                name: "Flüsternd".to_string(),
                tags: vec!["whisper".to_string()],
                reference: Some("__style_anna_fluesternd".to_string()),
            }],
        };
        write_meta(fish, "anna", &meta).unwrap();
        let read_back = read_meta(fish, "anna");
        assert_eq!(read_back, meta);
    }

    #[test]
    fn fehlende_meta_json_ergibt_default_ohne_fehler() {
        let dir = tempfile::tempdir().unwrap();
        let meta = read_meta(dir.path(), "irgendeine-stimme");
        assert_eq!(meta.display_name, "Irgendeine Stimme");
        assert_eq!(meta.version, CURRENT_META_VERSION);
    }

    #[test]
    fn unbekannte_felder_ueberleben_den_roundtrip_nicht_aber_stoeren_das_lesen_nicht() {
        // Eine kuenftige App-Version koennte ein Feld ergaenzen; eine AELTERE
        // Version muss die Datei trotzdem lesen koennen, statt auf den
        // Default zurueckzufallen (das waere ein Datenverlust).
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        let voice_dir = fish.join("references").join("olga");
        std::fs::create_dir_all(&voice_dir).unwrap();
        std::fs::write(
            voice_dir.join("meta.json"),
            r#"{"version":1,"display_name":"Olga","color":"rose","future_field":{"nested":true}}"#,
        )
        .unwrap();
        let meta = read_meta(fish, "olga");
        assert_eq!(meta.display_name, "Olga");
        assert_eq!(meta.color, "rose");
    }

    // ---- Validierung -------------------------------------------------------

    fn meta_named(name: &str) -> VoiceMeta {
        VoiceMeta {
            version: 1,
            display_name: name.to_string(),
            color: "teal".to_string(),
            avatar: None,
            language: None,
            description: None,
            default_tags: Vec::new(),
            default_style: None,
            styles: Vec::new(),
        }
    }

    #[test]
    fn leerer_anzeigename_wird_abgelehnt() {
        assert!(validate_meta(&meta_named(""), &[]).is_err());
        assert!(validate_meta(&meta_named("   "), &[]).is_err());
    }

    #[test]
    fn verbotene_zeichen_werden_abgelehnt() {
        assert!(validate_meta(&meta_named("Anna <b>"), &[]).is_err());
        assert!(validate_meta(&meta_named("Anna:Style"), &[]).is_err());
        assert!(validate_meta(&meta_named("Anna\nZeile2"), &[]).is_err());
    }

    #[test]
    fn gueltiger_name_wird_akzeptiert() {
        assert!(validate_meta(&meta_named("Anna Müller"), &[]).is_ok());
    }

    #[test]
    fn duplikat_gegen_anzeigenamen_ist_case_insensitiv_ueber_unicode() {
        let others = vec![("olga".to_string(), "Olga Müller".to_string())];
        assert!(validate_meta(&meta_named("OLGA MÜLLER"), &others).is_err());
        assert!(validate_meta(&meta_named("olga müller"), &others).is_err());
    }

    #[test]
    fn duplikat_gegen_eine_voice_id_wird_ebenfalls_abgelehnt() {
        let others = vec![("patrick".to_string(), "Der Chef".to_string())];
        assert!(validate_meta(&meta_named("Patrick"), &others).is_err());
    }

    #[test]
    fn unterschiedliche_namen_sind_erlaubt() {
        let others = vec![("olga".to_string(), "Olga Müller".to_string())];
        assert!(validate_meta(&meta_named("Anna"), &others).is_ok());
    }

    // ---- ReferenceAnalysis --------------------------------------------------

    fn sine(amplitude: f32, secs: f32, rate: u32) -> Vec<f32> {
        let n = (secs * rate as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / rate as f32;
                amplitude * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
            })
            .collect()
    }

    #[test]
    fn eine_leise_referenz_schlaegt_volume_up_vor() {
        let quiet = sine(0.02, 4.0, 16_000);
        let analysis = analyze_reference(&quiet, 16_000);
        assert!(
            analysis.quiet,
            "leise Aufnahme muss als leise erkannt werden"
        );
        assert_eq!(analysis.suggested_tags, vec!["volume up".to_string()]);
    }

    #[test]
    fn eine_normal_ausgesteuerte_referenz_bekommt_keinen_vorschlag() {
        let normal = sine(0.5, 4.0, 16_000);
        let analysis = analyze_reference(&normal, 16_000);
        assert!(!analysis.quiet);
        assert!(analysis.suggested_tags.is_empty());
    }

    #[test]
    fn eine_leise_aber_dynamische_referenz_bekommt_keinen_vorschlag() {
        // Niedrige integrierte Lautheit (viel Stille), aber ein einzelner
        // lauter Einsatz -> hohe Spitze -> keine Dauerkompensation vorschlagen.
        let mut samples = sine(0.02, 4.0, 16_000);
        samples[1000] = 0.9;
        let analysis = analyze_reference(&samples, 16_000);
        assert!(!analysis.quiet, "hohe Spitze verhindert den Vorschlag");
    }

    #[test]
    fn zu_kurze_oder_stille_signale_bekommen_keinen_vorschlag() {
        assert!(!analyze_reference(&[0.0; 100], 16_000).quiet);
        assert!(!analyze_reference(&[], 16_000).quiet);
    }
}
