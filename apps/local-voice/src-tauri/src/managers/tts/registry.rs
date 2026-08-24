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
    /// Catch-all fuer Felder, die diese Version nicht kennt. OHNE das
    /// waere `write_meta` verlustbehaftet: eine AELTERE App-Version wuerde
    /// beim naechsten Speichern jedes Feld einer NEUEREN stillschweigend
    /// wegwerfen (Read→Write→Read haette dann NICHT mehr denselben Inhalt).
    /// `#[specta(skip)]`, weil `serde_json::Map` kein `specta::Type` hat —
    /// die generierte TS-Definition braucht dieses Feld ohnehin nicht, das
    /// Frontend liest/schreibt nur die benannten Felder.
    #[serde(flatten)]
    #[specta(skip)]
    pub extra: serde_json::Map<String, serde_json::Value>,
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
        extra: serde_json::Map::new(),
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

// ---- Traversal-/Existenz-Schutz und darauf aufbauende Operationen --------
//
// Jede der folgenden Funktionen nimmt eine `fish_dir: &Path` PLUS eine vom
// Aufrufer gelieferte id entgegen und ist damit ohne AppHandle/Tauri per
// tempdir testbar — bewusst so geschnitten, damit `TtsManager` nur noch
// `self.fish_dir()` ermittelt und hierher durchreicht (siehe `mod.rs`).

/// Sanitiert `id` UND prueft, dass sie zu einer WIRKLICH existierenden
/// Stimme gehoert — der zentrale Schutz gegen Pfad-Traversal.
///
/// Zwei Schranken, beide noetig:
/// 1. [`voices::sanitize_voice_id`] laesst nur `a-z0-9_-` durch — `.`, `/`
///    und `\` landen nie im Ergebnis, ein `..`-Segment kann so gar nicht
///    erst entstehen. Weicht das sanierte Ergebnis vom Original ab, war
///    die Eingabe nicht schon die kanonische id (die JEDE echte Stimme
///    traegt, weil sie beim Anlegen genau so saniert wurde) — das wird
///    abgelehnt statt still umgeschrieben: sonst koennte z. B. `../anna`
///    unbemerkt auf eine ANDERE, zufaellig existierende Stimme `anna`
///    matchen, oder ein Tippfehler eine fremde Stimme treffen.
/// 2. Existenzpruefung gegen [`voices::list_voices`]: eine unbekannte id
///    darf keinen Pfad mehr anfassen — sonst entstuende z. B. ein
///    verwaistes `meta.json` fuer eine nie angelegte Stimme.
pub fn require_known_voice(fish_dir: &Path, id: &str) -> Result<String, String> {
    let id = require_valid_id(id)?;
    if !voices::list_voices(fish_dir).iter().any(|v| v == &id) {
        return Err(format!("Stimme '{id}' nicht gefunden"));
    }
    Ok(id)
}

/// Wie [`require_known_voice`], aber ohne Existenzpruefung — fuer
/// Kennungen (Stile), die nicht in `voices::list_voices` auftauchen. Reine
/// Zeichenfilterung, aber genau die IST hier der Traversal-Schutz.
pub fn require_valid_id(id: &str) -> Result<String, String> {
    voices::sanitize_voice_id(id)
        .filter(|sanitized| sanitized == id)
        .ok_or_else(|| format!("ungueltige Kennung: '{id}'"))
}

/// (voice_id, display_name) aller Stimmen ausser `exclude` — Grundlage
/// fuer [`validate_meta`]s Duplikatpruefung.
pub fn other_voice_names(fish_dir: &Path, exclude: Option<&str>) -> Vec<(String, String)> {
    voices::list_voices(fish_dir)
        .into_iter()
        .filter(|id| Some(id.as_str()) != exclude)
        .map(|id| {
            let display_name = read_meta(fish_dir, &id).display_name;
            (id, display_name)
        })
        .collect()
}

/// Metadaten einer Stimme lesen — mit Existenz-/Traversal-Pruefung.
pub fn get_voice_meta_checked(fish_dir: &Path, id: &str) -> Result<VoiceMeta, String> {
    let id = require_known_voice(fish_dir, id)?;
    Ok(read_meta(fish_dir, &id))
}

/// Metadaten einer Stimme validieren und speichern — mit Existenz-/
/// Traversal-Pruefung.
pub fn set_voice_meta_checked(fish_dir: &Path, id: &str, meta: VoiceMeta) -> Result<(), String> {
    let id = require_known_voice(fish_dir, id)?;
    let others = other_voice_names(fish_dir, Some(&id));
    validate_meta(&meta, &others)?;
    write_meta(fish_dir, &id, &meta)
}

/// Avatar setzen/ersetzen UND `meta.avatar` synchron mitfuehren — ohne
/// diesen Abgleich zeigte die Registry weiter das alte Icon/keinen Avatar,
/// obwohl laengst eine Bilddatei auf der Platte liegt.
pub fn set_voice_avatar_checked(
    fish_dir: &Path,
    id: &str,
    bytes: &[u8],
    ext: &str,
) -> Result<(), String> {
    let id = require_known_voice(fish_dir, id)?;
    let filename = voices::save_avatar(fish_dir, &id, bytes, ext)?;
    let mut meta = read_meta(fish_dir, &id);
    meta.avatar = Some(Avatar::Image { file: filename });
    write_meta(fish_dir, &id, &meta)
}

/// Avatar entfernen UND `meta.avatar` auf `None` zuruecksetzen, wenn er
/// zuvor ein Bild war (ein Icon-Avatar bleibt unberuehrt — das Icon liegt
/// nicht auf der Platte, „Avatar-Datei loeschen" betrifft es nicht).
pub fn clear_voice_avatar_checked(fish_dir: &Path, id: &str) -> Result<(), String> {
    let id = require_known_voice(fish_dir, id)?;
    voices::clear_avatar(fish_dir, &id);
    let mut meta = read_meta(fish_dir, &id);
    if matches!(meta.avatar, Some(Avatar::Image { .. })) {
        meta.avatar = None;
        write_meta(fish_dir, &id, &meta)?;
    }
    Ok(())
}

/// Referenzaufnahme einer GESPEICHERTEN Stimme analysieren (siehe
/// [`analyze_reference`]) — mit Existenz-/Traversal-Pruefung.
pub fn analyze_stored_reference(fish_dir: &Path, voice: &str) -> Result<ReferenceAnalysis, String> {
    let voice = require_known_voice(fish_dir, voice)?;
    let (wav_path, _) = voices::voice_sample(fish_dir, &voice)
        .ok_or_else(|| format!("keine Referenz fuer '{voice}' gefunden"))?;
    let samples = voices::load_wav_mono_16k(&wav_path)?;
    Ok(analyze_reference(&samples, 16_000))
}

/// Stil samt Referenzaufnahme entfernen — mit Existenz-/Traversal-Pruefung
/// fuer BEIDE Kennungen (Stimme und Stil).
pub fn delete_style_checked(fish_dir: &Path, voice: &str, style_id: &str) -> Result<(), String> {
    let voice = require_known_voice(fish_dir, voice)?;
    let style_id = require_valid_id(style_id)?;
    let dir = voices::style_dir(fish_dir, &voice, &style_id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("could not delete {}: {e}", dir.display()))?;
    }
    let mut meta = read_meta(fish_dir, &voice);
    meta.styles.retain(|s| s.id != style_id);
    write_meta(fish_dir, &voice, &meta)
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
            extra: serde_json::Map::new(),
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
    fn unbekannte_felder_ueberleben_lesen_und_roundtrip() {
        // Eine kuenftige App-Version koennte ein Feld ergaenzen; eine AELTERE
        // Version muss die Datei trotzdem lesen koennen UND darf es beim
        // naechsten Speichern nicht wegwerfen (`extra`/`#[serde(flatten)]`)
        // — sonst waere jedes Oeffnen mit der alten Version ein stiller
        // Datenverlust, sobald irgendetwas anderes neu gespeichert wird.
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
        assert_eq!(
            meta.extra.get("future_field"),
            Some(&serde_json::json!({"nested": true}))
        );

        // Read -> Write -> Read: das unbekannte Feld muss den kompletten
        // Umlauf ueberleben, nicht nur das erste Lesen.
        write_meta(fish, "olga", &meta).unwrap();
        let read_again = read_meta(fish, "olga");
        assert_eq!(
            read_again.extra.get("future_field"),
            Some(&serde_json::json!({"nested": true})),
            "unbekanntes Feld ist beim Schreiben verlorengegangen"
        );
        assert_eq!(read_again, meta);
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
            extra: serde_json::Map::new(),
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

    // ---- Traversal-/Existenz-Schutz (Review-Befund) -----------------------

    /// Legt eine ECHTE, vollstaendige Stimme an (WAV + lab), damit die
    /// Existenzpruefung etwas Reales findet.
    fn real_voice(fish: &Path, id: &str) {
        voices::save_voice(fish, id, &[0.1f32; 4 * 16_000], "Hallo Test.", None).unwrap();
    }

    #[test]
    fn require_valid_id_lehnt_traversal_versuche_ab() {
        for bogus in ["../x", "a/b", "a\\b", "..", ""] {
            assert!(
                require_valid_id(bogus).is_err(),
                "haette '{bogus}' ablehnen muessen"
            );
        }
        assert_eq!(require_valid_id("anna_m").unwrap(), "anna_m");
    }

    #[test]
    fn require_known_voice_lehnt_traversal_und_unbekannte_stimmen_ab() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        real_voice(fish, "anna");

        for bogus in ["../anna", "anna/../anna", "anna\\x", "geist"] {
            assert!(
                require_known_voice(fish, bogus).is_err(),
                "haette '{bogus}' ablehnen muessen"
            );
        }
        assert_eq!(require_known_voice(fish, "anna").unwrap(), "anna");
    }

    #[test]
    fn get_und_set_voice_meta_checked_lehnen_unbekannte_stimmen_ohne_verwaiste_datei_ab() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        real_voice(fish, "anna");

        assert!(get_voice_meta_checked(fish, "geist").is_err());
        assert!(get_voice_meta_checked(fish, "../anna").is_err());
        assert!(set_voice_meta_checked(fish, "geist", meta_named("Geist")).is_err());
        assert!(
            !voices::voice_dir(fish, "geist").join("meta.json").exists(),
            "eine abgelehnte Stimme darf keine meta.json hinterlassen"
        );

        assert!(set_voice_meta_checked(fish, "anna", meta_named("Anna")).is_ok());
        assert_eq!(
            get_voice_meta_checked(fish, "anna").unwrap().display_name,
            "Anna"
        );
    }

    #[test]
    fn set_und_clear_voice_avatar_checked_halten_meta_avatar_synchron() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        real_voice(fish, "anna");

        let png = [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n', 0, 0, 0, 0,
        ];
        set_voice_avatar_checked(fish, "anna", &png, "png").unwrap();
        assert_eq!(
            get_voice_meta_checked(fish, "anna").unwrap().avatar,
            Some(Avatar::Image {
                file: "avatar.png".to_string()
            })
        );

        clear_voice_avatar_checked(fish, "anna").unwrap();
        assert_eq!(get_voice_meta_checked(fish, "anna").unwrap().avatar, None);

        // Traversal/unbekannte Stimme auch hier abgelehnt.
        assert!(set_voice_avatar_checked(fish, "../anna", &png, "png").is_err());
        assert!(set_voice_avatar_checked(fish, "geist", &png, "png").is_err());
        assert!(clear_voice_avatar_checked(fish, "geist").is_err());
    }

    #[test]
    fn analyze_stored_reference_lehnt_traversal_und_unbekannte_stimmen_ab() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        real_voice(fish, "anna");

        assert!(analyze_stored_reference(fish, "geist").is_err());
        assert!(analyze_stored_reference(fish, "../anna").is_err());
        assert!(analyze_stored_reference(fish, "anna").is_ok());
    }

    #[test]
    fn delete_style_checked_lehnt_traversal_in_beiden_kennungen_ab() {
        let dir = tempfile::tempdir().unwrap();
        let fish = dir.path();
        real_voice(fish, "anna");

        assert!(delete_style_checked(fish, "../anna", "stil").is_err());
        assert!(delete_style_checked(fish, "anna", "../stil").is_err());
        assert!(delete_style_checked(fish, "geist", "stil").is_err());
        // Ein nicht existierender, aber gueltig geformter Stil ist ein No-op.
        assert!(delete_style_checked(fish, "anna", "nie-angelegt").is_ok());
    }
}
