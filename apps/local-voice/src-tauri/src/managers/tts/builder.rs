//! Entwuerfe des Stimmen-Baukastens: Zustand auf der Platte statt im Browser.
//!
//! Warum nicht im Frontend-Zustand: das Erzeugen von Kandidaten dauert
//! Minuten, und wer in dieser Zeit die App verliert (Absturz, Neustart,
//! versehentlich geschlossen), hat sonst alles verloren — die gewuerfelten
//! Stimmen sind nicht reproduzierbar, ein zweiter Lauf ergibt andere.
//! Deshalb liegt der Entwurf neben den Stimmen und ueberlebt jeden Absturz.
//!
//! Geschrieben wird atomar (temp + rename) wie `registry::write_meta`: ein
//! Absturz MITTEN im Schreiben darf keine halbe `draft.json` hinterlassen,
//! sonst waere der Rettungsweg selbst die Fehlerquelle.

use std::path::{Path, PathBuf};

/// Ein Kandidat: ein Wurf, der als Datei auf der Platte liegt.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct Candidate {
    pub seed: i64,
    /// Dateiname innerhalb des Entwurfsordners, NICHT der volle Pfad —
    /// damit ein verschobener Stimmenordner den Entwurf nicht entwertet.
    pub file: String,
    pub created_at: i64,
}

/// Der Arbeitsstand einer noch nicht gespeicherten Stimme.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct BuilderDraft {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub probe_text: String,
    pub tags: Vec<String>,
    /// Tiefe-Regler, 1,00 bis 1,15 (siehe `dsp::resample_stretch`).
    pub depth: f32,
    pub candidates: Vec<Candidate>,
    /// Seed des gewaehlten Kandidaten.
    pub selected: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

const DRAFT_FILE: &str = "draft.json";

/// Wurzel aller Entwuerfe — bewusst neben den Stimmen und nicht im TEMP:
/// ein Neustart des Rechners raeumt TEMP ab, und genau dann braucht man den
/// Entwurf.
pub fn builder_root(fish_dir: &Path) -> PathBuf {
    fish_dir.join("builder")
}

pub fn draft_dir(fish_dir: &Path, id: &str) -> PathBuf {
    builder_root(fish_dir).join(id)
}

/// Ein Entwurfs-Bezeichner darf NUR ein Ordnername sein. Ohne diese Pruefung
/// koennte ein `..` aus dem Frontend `delete_draft` auf den Stimmenordner
/// zeigen lassen — dieselbe Haerte wie bei `voices::sanitize_voice_id`.
fn checked_id(id: &str) -> Result<&str, String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("ungueltiger Entwurfs-Bezeichner: {id:?}"));
    }
    Ok(id)
}

/// Atomar schreiben: erst in `draft.json.tmp`, dann umbenennen.
pub fn save_draft(fish_dir: &Path, draft: &BuilderDraft) -> Result<(), String> {
    let id = checked_id(&draft.id)?;
    let dir = draft_dir(fish_dir, id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let json = serde_json::to_string_pretty(draft)
        .map_err(|e| format!("could not serialize draft {id}: {e}"))?;
    let tmp = dir.join(format!("{DRAFT_FILE}.tmp"));
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("could not write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, dir.join(DRAFT_FILE))
        .map_err(|e| format!("could not finalize draft {id}: {e}"))
}

pub fn load_draft(fish_dir: &Path, id: &str) -> Result<BuilderDraft, String> {
    let id = checked_id(id)?;
    let path = draft_dir(fish_dir, id).join(DRAFT_FILE);
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("draft {id} ist unlesbar: {e}"))
}

/// Alle lesbaren Entwuerfe, zuletzt geaenderte zuerst. Ein kaputter Entwurf
/// wird uebersprungen statt die ganze Liste scheitern zu lassen — sonst
/// verloere ein einziges defektes Verzeichnis den Zugang zu allen anderen.
pub fn list_drafts(fish_dir: &Path) -> Vec<BuilderDraft> {
    let root = builder_root(fish_dir);
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<BuilderDraft> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            load_draft(fish_dir, &name).ok()
        })
        .collect();
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    out
}

pub fn delete_draft(fish_dir: &Path, id: &str) -> Result<(), String> {
    let id = checked_id(id)?;
    let dir = draft_dir(fish_dir, id);
    if !dir.exists() {
        return Ok(());
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("could not remove {}: {e}", dir.display()))
}

/// Entwuerfe aelter als `max_age_days` entfernen. Beim Start aufgerufen:
/// Kandidaten-WAVs sind gross, und ein vergessener Entwurf von vor Monaten
/// haelt sonst dauerhaft Platz.
pub fn prune_drafts(fish_dir: &Path, max_age_days: i64) -> usize {
    let cutoff = chrono::Utc::now().timestamp() - max_age_days * 24 * 60 * 60;
    let mut removed = 0;
    for draft in list_drafts(fish_dir) {
        if draft.updated_at < cutoff && delete_draft(fish_dir, &draft.id).is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(id: &str) -> BuilderDraft {
        BuilderDraft {
            id: id.to_string(),
            display_name: "Pyrion".to_string(),
            description: "Sehr tiefe, wuerdevolle Maennerstimme.".to_string(),
            probe_text: "Ich habe Koenigreiche kommen und gehen sehen.".to_string(),
            tags: vec!["slow".to_string()],
            depth: 1.08,
            candidates: vec![Candidate {
                seed: 4711,
                file: "cand_4711.wav".to_string(),
                created_at: 1_700_000_000,
            }],
            selected: Some(4711),
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        }
    }

    #[test]
    fn schreiben_und_lesen_ergibt_denselben_entwurf() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        let d = draft("01ABC");
        save_draft(fish, &d).unwrap();
        let back = load_draft(fish, "01ABC").unwrap();
        assert_eq!(back.display_name, "Pyrion");
        assert_eq!(back.depth, 1.08);
        assert_eq!(back.candidates.len(), 1);
        assert_eq!(back.selected, Some(4711));
    }

    #[test]
    fn schreiben_hinterlaesst_keine_temp_datei() {
        let tmp = tempfile::tempdir().unwrap();
        save_draft(tmp.path(), &draft("01ABC")).unwrap();
        let dir = draft_dir(tmp.path(), "01ABC");
        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["draft.json".to_string()]);
    }

    #[test]
    fn eine_kaputte_draft_json_kippt_die_liste_nicht() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        save_draft(fish, &draft("01GUT")).unwrap();
        let kaputt = draft_dir(fish, "01KAPUTT");
        std::fs::create_dir_all(&kaputt).unwrap();
        std::fs::write(kaputt.join("draft.json"), b"{kein json").unwrap();
        let list = list_drafts(fish);
        assert_eq!(list.len(), 1, "der lesbare Entwurf bleibt uebrig");
        assert_eq!(list[0].id, "01GUT");
    }

    #[test]
    fn liste_ist_nach_aenderung_absteigend_sortiert() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        let mut alt = draft("01ALT");
        alt.updated_at = 1_000;
        let mut neu = draft("01NEU");
        neu.updated_at = 2_000;
        save_draft(fish, &alt).unwrap();
        save_draft(fish, &neu).unwrap();
        let ids: Vec<String> = list_drafts(fish).into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec!["01NEU".to_string(), "01ALT".to_string()]);
    }

    #[test]
    fn loeschen_entfernt_den_ganzen_ordner() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        save_draft(fish, &draft("01ABC")).unwrap();
        delete_draft(fish, "01ABC").unwrap();
        assert!(!draft_dir(fish, "01ABC").exists());
        assert!(list_drafts(fish).is_empty());
    }

    #[test]
    fn loeschen_akzeptiert_keinen_pfad_ausbruch() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(delete_draft(tmp.path(), "../voices").is_err());
        assert!(delete_draft(tmp.path(), "a/b").is_err());
        assert!(delete_draft(tmp.path(), "").is_err());
    }

    #[test]
    fn aufraeumen_entfernt_nur_die_alten() {
        let tmp = tempfile::tempdir().unwrap();
        let fish = tmp.path();
        let jetzt = chrono::Utc::now().timestamp();
        let mut alt = draft("01ALT");
        alt.updated_at = jetzt - 60 * 60 * 24 * 40;
        let mut frisch = draft("01FRISCH");
        frisch.updated_at = jetzt;
        save_draft(fish, &alt).unwrap();
        save_draft(fish, &frisch).unwrap();
        let entfernt = prune_drafts(fish, 30);
        assert_eq!(entfernt, 1);
        let ids: Vec<String> = list_drafts(fish).into_iter().map(|d| d.id).collect();
        assert_eq!(ids, vec!["01FRISCH".to_string()]);
    }
}
