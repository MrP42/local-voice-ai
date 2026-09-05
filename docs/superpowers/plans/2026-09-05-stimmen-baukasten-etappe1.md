# Stimmen-Baukasten Etappe 1 — Umsetzungsplan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ein Assistent in der Stimmenkarte erzeugt aus Name und Beschreibung mehrere Stimm-Kandidaten, die man anhört, mit einem Tiefe-Regler nachregelt und als lokale Stimme speichert — mit einem Entwurf, der einen Absturz übersteht.

**Architecture:** Der Entwurf lebt als `draft.json` plus Kandidaten-WAVs im Dateisystem (`<fish_dir>/builder/<id>/`), atomar geschrieben — der React-Zustand ist nur Anzeige. Kandidaten entstehen über den vorhandenen Fish-Speech-Weg (`/v1/tts` ohne Referenz, je Kandidat ein anderer Seed); der Tiefe-Regler streckt die gewählte Kandidaten-WAV per linearem Resampling, bevor sie zur Referenz der neuen Stimme wird. Gespeichert wird über dieselbe Strecke wie `save_seed_voice_v2`, damit es keinen zweiten Pfad mit eigenen Fehlern gibt.

**Tech Stack:** Rust (Tauri 2, tokio, serde, ulid), TypeScript/React 18, Tailwind, i18next, tauri-specta-Bindings (von Hand nachgezogen).

**Spec:** `docs/superpowers/specs/2026-09-05-stimmen-baukasten-design.md`

## Global Constraints

- **Kommentare und Nutzertexte auf Deutsch**, Code-Bezeichner englisch — wie im ganzen Projekt.
- **Keine Umlaute in Rust-Kommentaren ohne Not**: bestehende Dateien mischen; neuer Code folgt der Datei, in der er steht.
- **Alle Nutzertexte über i18next** (`t("…")`), niemals fest im JSX — ESLint erzwingt es. Neue Schlüssel in **allen 24** Locales unter `src/i18n/locales/*/translation.json`, deutscher Text nur in `de`, sonst englischer.
- **`bindings.ts` wird von Hand gepflegt** — tauri-specta regeneriert nur beim `tauri dev`-Lauf. Jeder neue Command braucht dort einen Eintrag, sonst bricht `tsc`.
- **Kein neuer Menüpunkt, kein neuer Reiter** (`AGENTS.md`): der Assistent gehört in die Stimmenkarte unter „Vorlesen".
- **Touch-Ziele ≥ 44 px**, `cursor-pointer` auf allen klickbaren Flächen, Fokusringe — die A11y-Regeln der bestehenden TTS-Oberfläche.
- **Prüfbefehle** (aus `apps/local-voice/`): `cargo test --manifest-path src-tauri/Cargo.toml <filter>`, `pnpm exec tsc --noEmit`, `pnpm exec eslint src`, `pnpm exec prettier --check <dateien>`.
- **Vorbestehend rot, nicht reparieren**: `prettier --check` auf `TagPalette.tsx`/`tagProvider.tsx`, `cargo clippy` (`approx_constant` in `settings.rs`), `pnpm check:translations` (kein `tsx` installiert). Nur eigene Dateien prüfen.
- **Branch `feat/stimmen-baukasten`**, kein Push auf `main` (Regeln in `AGENTS.md` der Repo-Wurzel).

---

### Task 1: Lineares Resampling in der Signalverarbeitung

Der Tiefe-Regler braucht eine Funktion, die eine WAV streckt: 15 % länger heißt Tonhöhe und Formanten zugleich tiefer. Das ist reine Rechnerei ohne Abhängigkeit zu allem anderen und deshalb die erste Aufgabe.

**Files:**
- Modify: `apps/local-voice/src-tauri/src/managers/tts/dsp.rs` (ans Dateiende anfügen)
- Test: dieselbe Datei, `#[cfg(test)] mod tests` am Ende

**Interfaces:**
- Consumes: nichts
- Produces: `pub fn resample_stretch(samples: &[f32], factor: f32) -> Vec<f32>` — `factor > 1.0` macht das Signal länger und damit tiefer; `factor <= 1.0` gibt eine Kopie zurück (kein Anheben, der Regler geht nur nach unten).

- [ ] **Step 1: Den fehlschlagenden Test schreiben**

Ans Ende von `dsp.rs`, in einem neuen `#[cfg(test)] mod tests`-Block (existiert dort noch keiner — sonst die Tests dort einfügen):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strecken_verlaengert_um_den_faktor() {
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let out = resample_stretch(&input, 1.15);
        // 100 Eingabewerte, 15 Prozent laenger: 115 Ausgabewerte.
        assert_eq!(out.len(), 115);
    }

    #[test]
    fn faktor_eins_und_kleiner_laesst_das_signal_unveraendert() {
        let input: Vec<f32> = vec![0.0, 0.5, -0.5, 1.0];
        assert_eq!(resample_stretch(&input, 1.0), input);
        assert_eq!(resample_stretch(&input, 0.5), input);
    }

    #[test]
    fn gestreckte_rampe_bleibt_monoton_und_haelt_die_raender() {
        // Eine Rampe ist der einfachste Fall mit pruefbarer Zwischenstufe:
        // linear interpoliert bleibt sie eine Rampe.
        let input: Vec<f32> = (0..50).map(|i| i as f32).collect();
        let out = resample_stretch(&input, 1.2);
        assert_eq!(out[0], 0.0, "der erste Wert bleibt der erste");
        assert!(
            out.windows(2).all(|w| w[1] >= w[0]),
            "eine gestreckte Rampe darf nirgends fallen"
        );
        assert!(
            *out.last().unwrap() <= 49.0,
            "es wird interpoliert, nicht extrapoliert"
        );
    }

    #[test]
    fn leeres_signal_bleibt_leer() {
        assert!(resample_stretch(&[], 1.15).is_empty());
    }
}
```

- [ ] **Step 2: Test laufen lassen, Fehlschlag bestätigen**

Aus `apps/local-voice/`:
`cargo test --manifest-path src-tauri/Cargo.toml dsp::tests`
Erwartet: FAIL, `cannot find function 'resample_stretch'`.

- [ ] **Step 3: Die Implementierung schreiben**

Vor den Test-Block in `dsp.rs`:

```rust
/// Streckt ein Signal per linearer Interpolation: `factor` 1,15 macht es 15 %
/// laenger und damit hoerbar tiefer — Tonhoehe UND Formanten sinken zusammen,
/// die klassische "tiefer und aelter"-Methode.
///
/// Bewusst linear und nicht bandbegrenzt: das Ergebnis ist die REFERENZ, aus
/// der das Modell anschliessend neu synthetisiert. Was an Interpolationsrauschen
/// entsteht, ueberlebt diesen Schritt nicht — ein teurerer Resampler brachte
/// hier nichts ausser Rechenzeit.
///
/// `factor <= 1.0` gibt eine unveraenderte Kopie: der Regler senkt nur.
pub fn resample_stretch(samples: &[f32], factor: f32) -> Vec<f32> {
    if samples.is_empty() || !(factor > 1.0) || !factor.is_finite() {
        return samples.to_vec();
    }
    let out_len = ((samples.len() as f32) * factor).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        // Position im Original: rueckwaerts gerechnet, damit der erste Wert
        // exakt der erste bleibt.
        let pos = (i as f32) / factor;
        let left = pos.floor() as usize;
        if left + 1 >= samples.len() {
            out.push(samples[samples.len() - 1]);
            continue;
        }
        let frac = pos - (left as f32);
        out.push(samples[left] * (1.0 - frac) + samples[left + 1] * frac);
    }
    out
}
```

- [ ] **Step 4: Tests laufen lassen, grün bestätigen**

`cargo test --manifest-path src-tauri/Cargo.toml dsp::tests`
Erwartet: PASS, 4 Tests.

- [ ] **Step 5: Formatieren und committen**

```bash
cd apps/local-voice/src-tauri && cargo fmt && cd ../../..
git add apps/local-voice/src-tauri/src/managers/tts/dsp.rs
git commit -m "feat(tts): lineares Strecken fuer den Tiefe-Regler"
```

---

### Task 2: Entwurfs-Persistenz des Baukastens

Der Zustand des Assistenten muss einen Absturz überstehen. Diese Aufgabe baut nur das Lesen und Schreiben — ohne Synthese, ohne Tauri, damit sie für sich testbar ist.

**Files:**
- Create: `apps/local-voice/src-tauri/src/managers/tts/builder.rs`
- Modify: `apps/local-voice/src-tauri/src/managers/tts/mod.rs` (eine Zeile: `pub mod builder;` zu den anderen `pub mod`-Zeilen, alphabetisch vor `compile_cache`)

**Interfaces:**
- Consumes: nichts
- Produces:
  - `pub struct BuilderDraft { pub id: String, pub display_name: String, pub description: String, pub probe_text: String, pub tags: Vec<String>, pub depth: f32, pub candidates: Vec<Candidate>, pub selected: Option<i64>, pub created_at: i64, pub updated_at: i64 }`
  - `pub struct Candidate { pub seed: i64, pub file: String, pub created_at: i64 }`
  - `pub fn builder_root(fish_dir: &Path) -> PathBuf`
  - `pub fn draft_dir(fish_dir: &Path, id: &str) -> PathBuf`
  - `pub fn save_draft(fish_dir: &Path, draft: &BuilderDraft) -> Result<(), String>`
  - `pub fn load_draft(fish_dir: &Path, id: &str) -> Result<BuilderDraft, String>`
  - `pub fn list_drafts(fish_dir: &Path) -> Vec<BuilderDraft>`
  - `pub fn delete_draft(fish_dir: &Path, id: &str) -> Result<(), String>`
  - `pub fn prune_drafts(fish_dir: &Path, max_age_days: i64) -> usize`

- [ ] **Step 1: Die fehlschlagenden Tests schreiben**

Neue Datei `builder.rs`, vorerst nur mit dem Test-Modul (die Implementierung kommt in Schritt 3):

```rust
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
```

- [ ] **Step 2: Test laufen lassen, Fehlschlag bestätigen**

Zuerst `pub mod builder;` in `mod.rs` ergänzen (sonst wird die Datei nicht kompiliert), dann:
`cargo test --manifest-path src-tauri/Cargo.toml builder::tests`
Erwartet: FAIL, `cannot find type 'BuilderDraft'`.

Prüfen, ob `tempfile` und `chrono` als dev- bzw. normale Abhängigkeit vorhanden sind (`grep -n "tempfile\|chrono" src-tauri/Cargo.toml`). Beide werden im Projekt bereits benutzt; fehlt eines, in Schritt 3 die entsprechende Zeile ergänzen und im Commit erwähnen.

- [ ] **Step 3: Die Implementierung schreiben**

An den Anfang von `builder.rs`, vor den Test-Block:

```rust
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
```

- [ ] **Step 4: Tests laufen lassen, grün bestätigen**

`cargo test --manifest-path src-tauri/Cargo.toml builder::tests`
Erwartet: PASS, 7 Tests.

- [ ] **Step 5: Formatieren und committen**

```bash
cd apps/local-voice/src-tauri && cargo fmt && cd ../../..
git add apps/local-voice/src-tauri/src/managers/tts/builder.rs apps/local-voice/src-tauri/src/managers/tts/mod.rs
git commit -m "feat(tts): absturzsichere Entwuerfe fuer den Stimmen-Baukasten"
```

---

### Task 3: Kandidaten erzeugen und Entwurf zur Stimme machen

Die Verbindung zwischen Entwurf und Fish-Speech: Kandidaten würfeln, Kandidat mit Tiefe hören, gewählten Kandidaten als Stimme speichern.

**Files:**
- Modify: `apps/local-voice/src-tauri/src/managers/tts/mod.rs` (neue `impl TtsManager`-Methoden, ans Ende des bestehenden `impl`-Blocks vor der schliessenden Klammer bei `save_seed_voice_v2`)

**Interfaces:**
- Consumes: `builder::{BuilderDraft, Candidate, save_draft, load_draft, draft_dir}` aus Task 2; `dsp::resample_stretch` aus Task 1
- Produces (alle auf `TtsManager`):
  - `pub async fn builder_generate(&self, draft_id: &str, count: usize, cancel: tokio::sync::watch::Receiver<bool>, on_candidate: impl FnMut(usize, usize, &Candidate)) -> Result<BuilderDraft, String>`
  - `pub fn builder_candidate_wav(&self, draft_id: &str, seed: i64) -> Result<Vec<u8>, String>` — Kandidat mit dem `depth` des Entwurfs
  - `pub async fn builder_commit(&self, draft_id: &str, meta: registry::VoiceMeta) -> Result<String, String>`

- [ ] **Step 1: Die fehlschlagenden Tests schreiben**

Die Synthese braucht einen laufenden Server und ist deshalb nicht als Einheitstest prüfbar. Testbar ist die **Anwendung der Tiefe** und die **Vollständigkeitsregel** beim Speichern. In den bestehenden `#[cfg(test)] mod tests` in `mod.rs` (existiert bereits, u. a. mit `WavCache::key`-Tests) anfügen:

```rust
    #[test]
    fn tiefe_streckt_die_kandidaten_wav_und_laesst_sie_eine_wav_bleiben() {
        // Ein winziges, gueltiges WAV bauen und durch die Tiefe schicken.
        let wav = super::test_support::sine_wav(16_000, 200);
        let tiefer = super::apply_depth(&wav, 1.15).expect("Tiefe muss rechnen");
        assert!(protocol::looks_like_wav(&tiefer), "bleibt ein WAV");
        assert!(
            tiefer.len() > wav.len(),
            "gestreckt heisst mehr Bytes: {} vs {}",
            tiefer.len(),
            wav.len()
        );
    }

    #[test]
    fn tiefe_eins_gibt_die_bytes_unveraendert_zurueck() {
        let wav = super::test_support::sine_wav(16_000, 200);
        assert_eq!(super::apply_depth(&wav, 1.0).unwrap(), wav);
    }
```

Dazu ein Hilfsmodul für Test-WAVs, direkt vor dem Test-Modul in `mod.rs`:

```rust
/// Winzige WAV-Erzeugung fuer Tests — echte Audiodateien im Repo waeren fuer
/// diese Pruefungen unnoetiger Ballast.
#[cfg(test)]
mod test_support {
    /// Mono, 16 Bit, `rate` Hz, `samples` Werte einer Sinusschwingung.
    pub fn sine_wav(rate: u32, samples: usize) -> Vec<u8> {
        let data_len = samples * 2;
        let mut out = Vec::with_capacity(44 + data_len);
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // mono
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data_len as u32).to_le_bytes());
        for i in 0..samples {
            let v = ((i as f32 / 8.0).sin() * 12_000.0) as i16;
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }
}
```

- [ ] **Step 2: Test laufen lassen, Fehlschlag bestätigen**

`cargo test --manifest-path src-tauri/Cargo.toml tiefe_`
Erwartet: FAIL, `cannot find function 'apply_depth'`.

- [ ] **Step 3: Die Implementierung schreiben**

`apply_depth` als freie Funktion in `mod.rs` (neben `normalize_wav_bytes`, das dort schon steht — dieselbe Bauart: WAV rein, WAV raus):

```rust
/// Den Tiefe-Regler auf WAV-Bytes anwenden: dekodieren, strecken, wieder als
/// WAV kodieren. `factor <= 1.0` gibt die Bytes unveraendert zurueck, damit
/// der Normalfall keine Rechenzeit und keine Requantisierung kostet.
fn apply_depth(wav: &[u8], factor: f32) -> Option<Vec<u8>> {
    if !(factor > 1.0) {
        return Some(wav.to_vec());
    }
    let (mono, rate, _peak) = decode_wav(wav)?;
    let stretched = dsp::resample_stretch(&mono, factor);
    Some(encode_wav_mono(&stretched, rate))
}
```

Falls `encode_wav_mono` in `mod.rs` noch nicht existiert (prüfen mit `grep -n "fn encode_wav" src-tauri/src/managers/tts/mod.rs`), zusätzlich schreiben:

```rust
/// Mono-f32 zurueck in ein 16-Bit-PCM-WAV. Gegenstueck zu `decode_wav`.
fn encode_wav_mono(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}
```

Dann die drei Manager-Methoden, ans Ende des `impl TtsManager`-Blocks (direkt nach `save_seed_voice_v2`):

```rust
    // ---- Stimmen-Baukasten (Etappe 1) --------------------------------------

    /// Kandidaten fuer einen Entwurf erzeugen: je Kandidat ein zufaelliger
    /// Seed, derselbe Probesatz. Nach JEDEM fertigen Kandidaten wird der
    /// Entwurf geschrieben — bricht der Lauf ab (Abbruch, Absturz,
    /// Serverfehler), bleibt alles bereits Gewuerfelte erhalten.
    ///
    /// Der Seed ist der einzige Regler fuer die Stimmidentitaet (Fish-Speech
    /// kennt keine Konditionierung auf eine Beschreibung) — deshalb ist das
    /// Wuerfeln hier der Kern und nicht ein Beiwerk.
    pub async fn builder_generate(
        &self,
        draft_id: &str,
        count: usize,
        mut cancel: tokio::sync::watch::Receiver<bool>,
        mut on_candidate: impl FnMut(usize, usize, &builder::Candidate),
    ) -> Result<builder::BuilderDraft, String> {
        let fish_dir = self.fish_dir();
        let mut draft = builder::load_draft(&fish_dir, draft_id)?;
        if draft.probe_text.trim().is_empty() {
            return Err("Ohne Probesatz gibt es nichts zu sprechen".to_string());
        }
        self.refresh_from_settings();
        self.ensure_server().await?;
        let port = *self.core.port.lock().unwrap();
        let dir = builder::draft_dir(&fish_dir, draft_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("could not create {}: {e}", dir.display()))?;

        // Die Tags des Entwurfs gehoeren in den Probesatz: die Prosodie der
        // Referenz ueberträgt sich beim Klonen, ein "[slow]" hier wirkt also
        // auf die spaetere Stimme, nicht nur auf diese eine Aufnahme.
        let mut text = String::new();
        for tag in &draft.tags {
            text.push('[');
            text.push_str(tag);
            text.push_str("] ");
        }
        text.push_str(&draft.probe_text);

        for index in 0..count {
            if *cancel.borrow_and_update() {
                break;
            }
            let seed: i64 = rand::random::<u32>() as i64;
            let body = protocol::tts_request_body_in_format(&text, seed, None, "wav");
            let resp = self
                .core
                .http
                .post(format!("{}/v1/tts", protocol::base_url(port)))
                .json(&body)
                .timeout(TTS_TIMEOUT)
                .send()
                .await
                .map_err(|e| format!("Kandidat {} nicht erzeugt: {e}", index + 1))?;
            if !resp.status().is_success() {
                return Err(format!(
                    "Kandidat {} nicht erzeugt: Server antwortete {}",
                    index + 1,
                    resp.status()
                ));
            }
            let audio = resp
                .bytes()
                .await
                .map_err(|e| format!("Kandidat {} unvollstaendig: {e}", index + 1))?
                .to_vec();
            if !protocol::looks_like_wav(&audio) {
                return Err(format!("Kandidat {} war kein WAV", index + 1));
            }
            let audio = normalize_wav_bytes(&audio).unwrap_or(audio);
            let file = format!("cand_{seed}.wav");
            std::fs::write(dir.join(&file), &audio)
                .map_err(|e| format!("Kandidat {} nicht gespeichert: {e}", index + 1))?;
            let candidate = builder::Candidate {
                seed,
                file,
                created_at: chrono::Utc::now().timestamp(),
            };
            draft.candidates.push(candidate.clone());
            draft.updated_at = chrono::Utc::now().timestamp();
            // Erst die Datei, dann der Entwurf: ein Absturz dazwischen laesst
            // eine verwaiste WAV zurueck (harmlos), nie einen Entwurf, der auf
            // eine fehlende Datei zeigt.
            builder::save_draft(&fish_dir, &draft)?;
            on_candidate(index + 1, count, &candidate);
        }
        Ok(draft)
    }

    /// Einen Kandidaten zum Anhoeren liefern — mit dem aktuellen Tiefe-Regler
    /// des Entwurfs. Die Original-WAV bleibt unveraendert liegen, damit der
    /// Regler beliebig oft neu gestellt werden kann, ohne neu zu wuerfeln.
    pub fn builder_candidate_wav(&self, draft_id: &str, seed: i64) -> Result<Vec<u8>, String> {
        let fish_dir = self.fish_dir();
        let draft = builder::load_draft(&fish_dir, draft_id)?;
        let candidate = draft
            .candidates
            .iter()
            .find(|c| c.seed == seed)
            .ok_or_else(|| format!("Kandidat {seed} gehoert nicht zu diesem Entwurf"))?;
        let path = builder::draft_dir(&fish_dir, draft_id).join(&candidate.file);
        let raw = std::fs::read(&path)
            .map_err(|e| format!("could not read {}: {e}", path.display()))?;
        apply_depth(&raw, draft.depth).ok_or_else(|| "Kandidat liess sich nicht lesen".to_string())
    }

    /// Den gewaehlten Kandidaten als Stimme speichern.
    ///
    /// Bewusst dieselbe Strecke wie `save_seed_voice_v2` — nur die Quelle der
    /// WAV ist eine andere: Kandidat statt Seed-Referenz. Zwei Speicherwege
    /// wuerden garantiert auseinanderlaufen.
    pub async fn builder_commit(
        &self,
        draft_id: &str,
        meta: registry::VoiceMeta,
    ) -> Result<String, String> {
        let fish_dir = self.fish_dir();
        let draft = builder::load_draft(&fish_dir, draft_id)?;
        let seed = draft
            .selected
            .ok_or_else(|| "Kein Kandidat gewaehlt".to_string())?;
        let id = voices::sanitize_voice_id(&meta.display_name)
            .ok_or_else(|| "Der Name ergibt keinen brauchbaren Stimmennamen".to_string())?;
        if voices::voice_is_complete(&fish_dir, &id) {
            return Err(format!("Die Stimme '{id}' existiert bereits"));
        }
        let others = registry::other_voice_names(&fish_dir, Some(&id));
        registry::validate_meta(&meta, &others)?;

        let audio = self.builder_candidate_wav(draft_id, seed)?;
        let target = voices::voice_dir(&fish_dir, &id);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("could not create {}: {e}", target.display()))?;
        // Vollstaendig oder gar nicht — die Regel aus `save_seed_voice_v2`:
        // ein halbes Verzeichnis meldet beim naechsten Versuch faelschlich
        // "existiert bereits".
        if let Err(e) = std::fs::write(target.join("sample.wav"), &audio) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(format!("could not write sample.wav: {e}"));
        }
        if let Err(e) = std::fs::write(target.join("sample.lab"), draft.probe_text.as_bytes()) {
            let _ = std::fs::remove_dir_all(&target);
            return Err(format!("could not write sample.lab: {e}"));
        }
        voices::write_seed_marker(&fish_dir, &id, seed);
        registry::write_meta(&fish_dir, &id, &meta)?;
        voices::update_registry(&fish_dir);
        builder::delete_draft(&fish_dir, draft_id)?;
        log::info!("Baukasten: Entwurf {draft_id} als Stimme '{id}' gespeichert");
        Ok(id)
    }
```

Oben in `mod.rs` sicherstellen, dass `use` für `rand` vorhanden ist (`grep -n "rand::" src-tauri/src/managers/tts/mod.rs` — wird für Seeds bereits benutzt; falls nicht, `rand` in `Cargo.toml` prüfen).

- [ ] **Step 4: Tests laufen lassen, grün bestätigen**

`cargo test --manifest-path src-tauri/Cargo.toml --lib`
Erwartet: PASS, alle bisherigen Tests plus die zwei neuen.

- [ ] **Step 5: Formatieren und committen**

```bash
cd apps/local-voice/src-tauri && cargo fmt && cd ../../..
git add apps/local-voice/src-tauri/src/managers/tts/mod.rs
git commit -m "feat(tts): Kandidaten erzeugen, Tiefe anwenden, Entwurf als Stimme speichern"
```

---

### Task 4: Tauri-Commands und Bindings

**Files:**
- Modify: `apps/local-voice/src-tauri/src/commands/tts.rs` (ans Dateiende)
- Modify: `apps/local-voice/src-tauri/src/lib.rs` (Registrierung in `collect_commands!`, bei den anderen `commands::tts::…`-Zeilen; ausserdem `.manage(commands::tts::BuilderRun::default())` neben dem vorhandenen `AutoTagRun`)
- Modify: `apps/local-voice/src/bindings.ts` (von Hand)

**Interfaces:**
- Consumes: die drei `TtsManager`-Methoden aus Task 3, `builder::{BuilderDraft, Candidate}` aus Task 2
- Produces (TypeScript-Namen in `bindings.ts`):
  - `ttsBuilderCreateDraft(displayName: string, description: string, probeText: string, tags: string[]): Promise<Result<BuilderDraft, string>>`
  - `ttsBuilderListDrafts(): Promise<BuilderDraft[]>`
  - `ttsBuilderUpdateDraft(draft: BuilderDraft): Promise<Result<null, string>>`
  - `ttsBuilderDeleteDraft(id: string): Promise<Result<null, string>>`
  - `ttsBuilderGenerate(id: string, count: number): Promise<Result<BuilderDraft, string>>`
  - `ttsBuilderCancel(): Promise<Result<null, string>>`
  - `ttsBuilderCandidateWav(id: string, seed: number): Promise<Result<number[], string>>`
  - `ttsBuilderCommit(id: string, meta: VoiceMeta): Promise<Result<string, string>>`
  - Event `tts-builder-progress` mit `{ done: number, total: number, seed: number }`

- [ ] **Step 1: Commands schreiben**

Ans Ende von `commands/tts.rs`:

```rust
// ---- Stimmen-Baukasten (Etappe 1) -----------------------------------------

/// Zustand des laufenden Kandidaten-Laufs — wie `AutoTagRun`, damit
/// `tts_builder_cancel` ihn abbrechen kann. `None` = kein Lauf aktiv.
#[derive(Default)]
pub struct BuilderRun(pub std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>);

#[tauri::command]
#[specta::specta]
pub fn tts_builder_create_draft(
    app: AppHandle,
    display_name: String,
    description: String,
    probe_text: String,
    tags: Vec<String>,
) -> Result<builder::BuilderDraft, String> {
    let tts = app.state::<Arc<TtsManager>>();
    let now = chrono::Utc::now().timestamp();
    let draft = builder::BuilderDraft {
        id: ulid::Ulid::new().to_string(),
        display_name,
        description,
        probe_text,
        tags,
        depth: 1.0,
        candidates: Vec::new(),
        selected: None,
        created_at: now,
        updated_at: now,
    };
    builder::save_draft(&tts.fish_dir_public(), &draft)?;
    Ok(draft)
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_list_drafts(app: AppHandle) -> Vec<builder::BuilderDraft> {
    let tts = app.state::<Arc<TtsManager>>();
    builder::list_drafts(&tts.fish_dir_public())
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_update_draft(
    app: AppHandle,
    mut draft: builder::BuilderDraft,
) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>();
    // Der Regler kommt aus der Oberflaeche und wird hier hart begrenzt:
    // ueber 1,15 klingt es kuenstlich, unter 1,0 hebt es die Stimme an, was
    // der Regler nicht anbietet.
    draft.depth = draft.depth.clamp(1.0, 1.15);
    draft.updated_at = chrono::Utc::now().timestamp();
    builder::save_draft(&tts.fish_dir_public(), &draft)
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_delete_draft(app: AppHandle, id: String) -> Result<(), String> {
    let tts = app.state::<Arc<TtsManager>>();
    builder::delete_draft(&tts.fish_dir_public(), &id)
}

/// Kandidaten erzeugen. Je fertigem Kandidaten geht ein
/// `tts-builder-progress`-Event `{done, total, seed}` an die Oberflaeche —
/// Kandidaten erscheinen damit einzeln statt alle am Ende.
#[tauri::command]
#[specta::specta]
pub async fn tts_builder_generate(
    app: AppHandle,
    id: String,
    count: usize,
) -> Result<builder::BuilderDraft, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    let (tx, rx) = tokio::sync::watch::channel(false);
    *app.state::<BuilderRun>().0.lock().unwrap() = Some(tx);
    let progress_app = app.clone();
    let result = tts
        .builder_generate(&id, count.clamp(1, 12), rx, move |done, total, cand| {
            let _ = progress_app.emit(
                "tts-builder-progress",
                serde_json::json!({ "done": done, "total": total, "seed": cand.seed }),
            );
        })
        .await;
    *app.state::<BuilderRun>().0.lock().unwrap() = None;
    result
}

#[tauri::command]
#[specta::specta]
pub fn tts_builder_cancel(app: AppHandle) -> Result<(), String> {
    if let Some(tx) = app.state::<BuilderRun>().0.lock().unwrap().as_ref() {
        let _ = tx.send(true);
    }
    Ok(())
}

/// WAV-Bytes eines Kandidaten mit dem aktuellen Tiefe-Regler. Roh als
/// `Vec<u8>` wie beim Avatar-Upload — das Projekt hat kein base64-Crate.
#[tauri::command]
#[specta::specta]
pub fn tts_builder_candidate_wav(
    app: AppHandle,
    id: String,
    seed: i64,
) -> Result<Vec<u8>, String> {
    app.state::<Arc<TtsManager>>()
        .builder_candidate_wav(&id, seed)
}

#[tauri::command]
#[specta::specta]
pub async fn tts_builder_commit(
    app: AppHandle,
    id: String,
    meta: VoiceMeta,
) -> Result<String, String> {
    let tts = app.state::<Arc<TtsManager>>().inner().clone();
    tts.builder_commit(&id, meta).await
}
```

Oben in `commands/tts.rs` den `use`-Block um `use crate::managers::tts::builder;` ergänzen.

In `mod.rs` `fish_dir` ist privat — für die Commands eine öffentliche Hülle direkt daneben ergänzen:

```rust
    /// `fish_dir` fuer die Command-Schicht: die Commands brauchen den Pfad,
    /// der Manager haelt ihn aber bewusst privat, damit niemand daran
    /// vorbei am Server vorbei arbeitet.
    pub fn fish_dir_public(&self) -> std::path::PathBuf {
        self.fish_dir()
    }
```

- [ ] **Step 2: Registrieren**

In `lib.rs` bei den `commands::tts::…`-Zeilen in `collect_commands!` ergänzen:

```rust
            commands::tts::tts_builder_create_draft,
            commands::tts::tts_builder_list_drafts,
            commands::tts::tts_builder_update_draft,
            commands::tts::tts_builder_delete_draft,
            commands::tts::tts_builder_generate,
            commands::tts::tts_builder_cancel,
            commands::tts::tts_builder_candidate_wav,
            commands::tts::tts_builder_commit,
```

Und neben dem vorhandenen `.manage(commands::tts::AutoTagRun::default())`:

```rust
        .manage(commands::tts::BuilderRun::default())
```

- [ ] **Step 3: Kompilieren**

`cargo test --manifest-path src-tauri/Cargo.toml --lib`
Erwartet: PASS. Bei `ulid`-Fehler: `ulid` ist über `managers/meetings/store.rs` bereits Abhängigkeit, ggf. `use ulid::Ulid;` statt des vollen Pfads.

- [ ] **Step 4: Bindings von Hand nachziehen**

In `apps/local-voice/src/bindings.ts` bei den anderen `tts…`-Methoden einfügen (Muster: `ttsListVoiceInfos` für Nicht-`Result`, `ttsSaveVoice` für `Result`):

```typescript
async ttsBuilderCreateDraft(displayName: string, description: string, probeText: string, tags: string[]) : Promise<Result<BuilderDraft, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_create_draft", { displayName, description, probeText, tags }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderListDrafts() : Promise<BuilderDraft[]> {
    return await TAURI_INVOKE("tts_builder_list_drafts");
},
async ttsBuilderUpdateDraft(draft: BuilderDraft) : Promise<Result<null, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_update_draft", { draft }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderDeleteDraft(id: string) : Promise<Result<null, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_delete_draft", { id }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderGenerate(id: string, count: number) : Promise<Result<BuilderDraft, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_generate", { id, count }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderCancel() : Promise<Result<null, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_cancel") };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderCandidateWav(id: string, seed: number) : Promise<Result<number[], string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_candidate_wav", { id, seed }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
async ttsBuilderCommit(id: string, meta: VoiceMeta) : Promise<Result<string, string>> {
    try {
    return { status: "ok", data: await TAURI_INVOKE("tts_builder_commit", { id, meta }) };
} catch (e) {
    if(e instanceof Error) throw e;
    else return { status: "error", error: e  as any };
}
},
```

Und bei den Typen (alphabetisch vor `VoiceInfo`):

```typescript
/**
 * Ein Kandidat des Stimmen-Baukastens: ein Wurf, der als Datei vorliegt.
 */
export type Candidate = { seed: number; file: string; created_at: number }
/**
 * Arbeitsstand einer noch nicht gespeicherten Stimme.
 */
export type BuilderDraft = { id: string; display_name: string; description: string; probe_text: string; tags: string[]; depth: number; candidates: Candidate[]; selected: number | null; created_at: number; updated_at: number }
```

- [ ] **Step 5: Prüfen und committen**

```bash
cd apps/local-voice && pnpm exec tsc --noEmit && cd ../..
git add apps/local-voice/src-tauri/src/commands/tts.rs apps/local-voice/src-tauri/src/lib.rs apps/local-voice/src-tauri/src/managers/tts/mod.rs apps/local-voice/src/bindings.ts
git commit -m "feat(tts): Commands und Bindings fuer den Stimmen-Baukasten"
```

---

### Task 5: Pyrion als erstes Rezept

Unabhängig von allem anderen: reine Daten plus Typ.

**Files:**
- Create: `apps/local-voice/src/lib/voices/recipes.ts`

**Interfaces:**
- Consumes: nichts
- Produces: `export interface VoiceRecipe { id: string; name: string; description: string; probeText: string; tags: string[]; color: string }` und `export const VOICE_RECIPES: VoiceRecipe[]`

- [ ] **Step 1: Die Datei schreiben**

```typescript
/**
 * Startpunkte fuer den Stimmen-Baukasten.
 *
 * Ein Rezept ist KEINE fertige Stimme — Fish-Speech kennt keine
 * Konditionierung auf eine Beschreibung, die Stimmidentitaet haengt allein am
 * Seed. Ein Rezept fuellt deshalb vor, was die Beschreibung wirklich steuern
 * kann: den Probesatz (dessen Prosodie sich beim Klonen ueberträgt), die
 * Emotions-Tags und die Metadaten. Gewuerfelt und ausgesucht wird danach.
 *
 * `color` ist ein Palette-Key aus `registry.rs` (siehe `palette.ts`).
 */
export interface VoiceRecipe {
  id: string;
  /** Vorgeschlagener Anzeigename — frei aenderbar. */
  name: string;
  /** Geht als Beschreibung in die Stimme und steuert den Probesatz-Vorschlag. */
  description: string;
  /** Referenzsatz, hoechstens etwa 150 Zeichen, in der Rolle gesprochen. */
  probeText: string;
  tags: string[];
  color: string;
}

export const VOICE_RECIPES: VoiceRecipe[] = [
  {
    id: "pyrion",
    name: "Pyrion",
    description:
      "Sehr tiefe, erwachsene Männerstimme mit viel Resonanz und ruhiger Autorität. " +
      "Alt, mächtig und würdevoll, als hätte er Jahrhunderte erlebt. Langsames bis " +
      "mittleres Tempo, klare Artikulation, kaum Hektik. Klanglich warm und dunkel, " +
      "mit leicht rauer, steiniger Textur — nicht dämonisch, nicht monströs. Selbst " +
      "wütend bleibt die Stimme kontrolliert und schwer statt schrill. Grundstimmung: " +
      "majestätisch, ernst, geheimnisvoll, erfahren. Ein uralter Wächter oder König " +
      "aus einem Fantasyfilm — gewaltig und respekteinflößend, aber vertrauenswürdig. " +
      "Keine Karikatur, kein übertriebenes Bösewicht-Lachen, kein Brüllen.",
    probeText:
      "Ich habe Königreiche kommen und vergehen sehen. Hört mir gut zu, denn ich sage es nur einmal.",
    tags: ["slow", "serious"],
    color: "amber",
  },
];
```

- [ ] **Step 2: Prüfen**

```bash
cd apps/local-voice && pnpm exec tsc --noEmit && pnpm exec eslint src/lib/voices && pnpm exec prettier --check src/lib/voices/recipes.ts
```

Erwartet: alles grün. Meldet Prettier etwas, `--write` darauf laufen lassen.

- [ ] **Step 3: Committen**

```bash
git add apps/local-voice/src/lib/voices/recipes.ts
git commit -m "feat(tts): Pyrion als erstes Rezept des Stimmen-Baukastens"
```

---

### Task 6: Übersetzungen

**Files:**
- Modify: alle 24 `apps/local-voice/src/i18n/locales/*/translation.json`

**Interfaces:**
- Consumes: nichts
- Produces: Schlüssel unter `tts.builder.*`, die Task 7 benutzt.

- [ ] **Step 1: Schlüssel in alle Locales einfügen**

Ein Skript im Scratchpad, nach dem Muster des Sprecher-Blocks (der `tts.speakers`-Block entstand genauso). Der Block wird in `tts` direkt hinter `speakers` einsortiert. Englischer Text in allen Sprachen ausser `de`:

Englisch:
```json
"builder": {
  "open": "Create new voice",
  "title": "Create a voice",
  "nameLabel": "Name",
  "namePlaceholder": "e.g. Pyrion",
  "descriptionLabel": "Description",
  "descriptionPlaceholder": "How should this voice sound? Age, mood, tempo, character.",
  "probeLabel": "Sample sentence",
  "probeHint": "The candidates speak this sentence. Its delivery carries into the saved voice.",
  "generate": "Generate candidates",
  "generating": "Generating {{done}} of {{total}}…",
  "cancel": "Stop",
  "candidates": "Candidates",
  "empty": "No candidates yet — generate a few and listen.",
  "play": "Play",
  "choose": "Use this one",
  "chosen": "Chosen",
  "depthLabel": "Depth",
  "depthHint": "Stretches the reference before cloning — deeper and older. Beyond the top of the range it starts to sound artificial.",
  "save": "Save voice",
  "saving": "Saving…",
  "discard": "Discard draft",
  "resume": "Resume draft",
  "drafts": "Unfinished drafts",
  "recipes": "Start from a character",
  "seedNote": "Fish-Speech cannot build a voice from a description — each candidate is a different random voice. The description shapes the sample sentence, the tags and the saved metadata.",
  "errorNoServer": "The speech server is not running — start it on the Read aloud page.",
  "errorNoCandidate": "Pick a candidate first."
}
```

Deutsch (`de`):
```json
"builder": {
  "open": "Neue Stimme erschaffen",
  "title": "Stimme erschaffen",
  "nameLabel": "Name",
  "namePlaceholder": "z. B. Pyrion",
  "descriptionLabel": "Beschreibung",
  "descriptionPlaceholder": "Wie soll die Stimme klingen? Alter, Stimmung, Tempo, Charakter.",
  "probeLabel": "Probesatz",
  "probeHint": "Diesen Satz sprechen die Kandidaten. Sein Vortrag geht in die gespeicherte Stimme über.",
  "generate": "Kandidaten erzeugen",
  "generating": "Erzeuge {{done}} von {{total}}…",
  "cancel": "Anhalten",
  "candidates": "Kandidaten",
  "empty": "Noch keine Kandidaten — erzeuge welche und hör sie an.",
  "play": "Abspielen",
  "choose": "Diesen nehmen",
  "chosen": "Gewählt",
  "depthLabel": "Tiefe",
  "depthHint": "Streckt die Referenz vor dem Klonen — tiefer und älter. Am oberen Ende beginnt es künstlich zu klingen.",
  "save": "Stimme speichern",
  "saving": "Speichere…",
  "discard": "Entwurf verwerfen",
  "resume": "Entwurf fortsetzen",
  "drafts": "Unfertige Entwürfe",
  "recipes": "Mit einer Figur beginnen",
  "seedNote": "Fish-Speech kann eine Stimme nicht aus einer Beschreibung bauen — jeder Kandidat ist eine andere Zufallsstimme. Die Beschreibung prägt Probesatz, Tags und die gespeicherten Angaben.",
  "errorNoServer": "Der Sprachserver läuft nicht — starte ihn auf der Vorlesen-Seite.",
  "errorNoCandidate": "Wähle zuerst einen Kandidaten."
}
```

- [ ] **Step 2: Parität prüfen**

```bash
cd apps/local-voice/src/i18n/locales && python -c "
import json,io,os
ref=json.load(io.open('en/translation.json',encoding='utf-8'))['tts']['builder']
bad=[l for l in sorted(os.listdir('.')) if os.path.isfile(os.path.join(l,'translation.json')) and set(json.load(io.open(os.path.join(l,'translation.json'),encoding='utf-8'))['tts'].get('builder',{}))!=set(ref)]
print('unvollstaendig:', bad)"
```
Erwartet: `unvollstaendig: []`

- [ ] **Step 3: Committen**

```bash
git add apps/local-voice/src/i18n/locales
git commit -m "i18n: Texte des Stimmen-Baukastens in allen 24 Sprachen"
```

---

### Task 7: Die Oberfläche des Assistenten

**Files:**
- Create: `apps/local-voice/src/components/settings/tts/builder/useBuilderDraft.ts`
- Create: `apps/local-voice/src/components/settings/tts/builder/CandidateCard.tsx`
- Create: `apps/local-voice/src/components/settings/tts/builder/VoiceBuilder.tsx`
- Create: `apps/local-voice/src/components/settings/tts/builder/index.ts`
- Modify: `apps/local-voice/src/components/settings/tts/VoicesCard.tsx` (Knopf „Neue Stimme erschaffen" plus Panel)

**Interfaces:**
- Consumes: `commands.ttsBuilder*` und die Typen `BuilderDraft`, `Candidate` aus Task 4; `VOICE_RECIPES` aus Task 5; `voiceColor` aus `src/lib/voices/palette.ts`; die `tts.builder.*`-Schlüssel aus Task 6
- Produces: `<VoiceBuilder onSaved={(voiceId: string) => void} />`

- [ ] **Step 1: Den Hook schreiben**

`useBuilderDraft.ts` hält den Entwurf, schreibt Änderungen entprellt zurück ans Backend und hört auf `tts-builder-progress`:

```typescript
import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands, type BuilderDraft } from "@/bindings";

/** Wie lange nach der letzten Eingabe gewartet wird, bevor der Entwurf auf
 *  die Platte geht. Kurz genug, dass ein Absturz hoechstens einen Satz
 *  kostet, lang genug, dass Tippen keine Schreiblast erzeugt. */
const SAVE_DEBOUNCE_MS = 600;

export interface BuilderProgress {
  done: number;
  total: number;
}

export function useBuilderDraft() {
  const [draft, setDraft] = useState<BuilderDraft | null>(null);
  const [drafts, setDrafts] = useState<BuilderDraft[]>([]);
  const [progress, setProgress] = useState<BuilderProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const saveTimer = useRef<number | null>(null);

  const reloadDrafts = useCallback(() => {
    void commands.ttsBuilderListDrafts().then(setDrafts).catch(() => undefined);
  }, []);

  useEffect(() => {
    reloadDrafts();
  }, [reloadDrafts]);

  // Der Fortschritt kommt als Ereignis, weil das Erzeugen Minuten dauert und
  // die Kandidaten einzeln erscheinen sollen.
  useEffect(() => {
    const un = listen<{ done: number; total: number }>(
      "tts-builder-progress",
      (event) => {
        setProgress({ done: event.payload.done, total: event.payload.total });
        void commands
          .ttsBuilderListDrafts()
          .then((all) => {
            setDrafts(all);
            setDraft((current) =>
              current ? (all.find((d) => d.id === current.id) ?? current) : current,
            );
          })
          .catch(() => undefined);
      },
    );
    return () => {
      void un.then((f) => f());
    };
  }, []);

  /** Aendert den Entwurf sofort in der Anzeige und entprellt das Schreiben. */
  const patch = useCallback((changes: Partial<BuilderDraft>) => {
    setDraft((current) => {
      if (!current) return current;
      const next = { ...current, ...changes };
      if (saveTimer.current !== null) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void commands.ttsBuilderUpdateDraft(next).catch(() => undefined);
      }, SAVE_DEBOUNCE_MS);
      return next;
    });
  }, []);

  return { draft, setDraft, drafts, reloadDrafts, progress, setProgress, error, setError, patch };
}
```

- [ ] **Step 2: Die Kandidatenkarte schreiben**

`CandidateCard.tsx` — Abspielen über einen Blob-URL aus den WAV-Bytes:

```typescript
import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, Play } from "lucide-react";
import { commands, type Candidate } from "@/bindings";

export const CandidateCard: React.FC<{
  draftId: string;
  candidate: Candidate;
  chosen: boolean;
  onChoose: (seed: number) => void;
}> = ({ draftId, candidate, chosen, onChoose }) => {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);

  // Die Bytes kommen bei JEDEM Abspielen frisch: der Tiefe-Regler kann
  // sich zwischendurch geaendert haben, und ein zwischengespeicherter
  // Blob wuerde dann die alte Fassung abspielen.
  const play = async () => {
    setBusy(true);
    try {
      const res = await commands.ttsBuilderCandidateWav(draftId, candidate.seed);
      if (res.status !== "ok") return;
      const blob = new Blob([new Uint8Array(res.data)], { type: "audio/wav" });
      const url = URL.createObjectURL(blob);
      const audio = new Audio(url);
      audio.onended = () => URL.revokeObjectURL(url);
      await audio.play();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className={`flex items-center gap-2 rounded-lg border p-2 ${
        chosen ? "border-logo-primary bg-logo-primary/10" : "border-mid-gray/40"
      }`}
    >
      <button
        type="button"
        onClick={() => void play()}
        disabled={busy}
        title={t("tts.builder.play")}
        aria-label={t("tts.builder.play")}
        className="flex min-h-[44px] min-w-[44px] cursor-pointer items-center justify-center rounded-md text-text/70 transition-colors hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-default disabled:opacity-50"
      >
        <Play width={16} height={16} aria-hidden="true" />
      </button>
      <span className="min-w-0 flex-1 truncate text-xs text-text/45">
        #{candidate.seed}
      </span>
      <button
        type="button"
        onClick={() => onChoose(candidate.seed)}
        className={`flex min-h-[44px] cursor-pointer items-center gap-1 rounded-md px-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary ${
          chosen
            ? "text-logo-primary"
            : "text-text/70 hover:bg-mid-gray/15 hover:text-text"
        }`}
      >
        {chosen && <Check width={14} height={14} aria-hidden="true" />}
        {chosen ? t("tts.builder.chosen") : t("tts.builder.choose")}
      </button>
    </div>
  );
};
```

- [ ] **Step 3: Den Assistenten schreiben**

`VoiceBuilder.tsx` — drei Abschnitte untereinander (Beschreiben, Kandidaten, Speichern), Rezeptauswahl oben, offene Entwürfe zum Fortsetzen. Vollständiger Code im Anhang A dieses Plans.

- [ ] **Step 4: In die Stimmenkarte einhängen**

In `VoicesCard.tsx` einen Knopf `t("tts.builder.open")` ergänzen, der `<VoiceBuilder onSaved={…} />` einblendet; nach dem Speichern `window.dispatchEvent(new CustomEvent("lv-voices-changed"))` auslösen, damit Stimmenliste und Sprecher-Chips sofort nachziehen.

- [ ] **Step 5: Prüfen und committen**

```bash
cd apps/local-voice && pnpm exec tsc --noEmit && pnpm exec eslint src/components/settings/tts && pnpm exec prettier --check src/components/settings/tts/builder && cd ../..
git add apps/local-voice/src/components/settings/tts
git commit -m "feat(tts): Oberflaeche des Stimmen-Baukastens"
```

---

### Task 8: Aufräumen alter Entwürfe beim Start

**Files:**
- Modify: `apps/local-voice/src-tauri/src/lib.rs` (im `setup`, wo die anderen Manager initialisiert werden)

**Interfaces:**
- Consumes: `builder::prune_drafts` aus Task 2

- [ ] **Step 1: Aufruf ergänzen**

Im `setup`-Block, nachdem der `TtsManager` im State liegt:

```rust
            // Entwuerfe des Stimmen-Baukastens aelter als 30 Tage entfernen:
            // Kandidaten-WAVs sind gross, und ein vergessener Entwurf haelt
            // sonst dauerhaft Platz. Best effort — ein Fehler hier darf den
            // Start nicht aufhalten.
            {
                let fish_dir = std::path::PathBuf::from(
                    crate::settings::get_settings(app.handle()).tts_fish_dir,
                );
                let removed =
                    crate::managers::tts::builder::prune_drafts(&fish_dir, 30);
                if removed > 0 {
                    log::info!("Baukasten: {removed} alte Entwuerfe entfernt");
                }
            }
```

- [ ] **Step 2: Kompilieren und committen**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
git add apps/local-voice/src-tauri/src/lib.rs
git commit -m "chore(tts): alte Baukasten-Entwuerfe beim Start aufraeumen"
```

---

## Anhang A — VoiceBuilder.tsx

Der Assistent hält keinen eigenen dauerhaften Zustand: alles steht im Entwurf im Backend. Aufbau von oben nach unten — Rezepte, Beschreiben, Kandidaten, Speichern.

```typescript
import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Trash2, Wand2 } from "lucide-react";
import { commands, type VoiceMeta } from "@/bindings";
import { Input } from "@/components/ui/Input";
import { VOICE_RECIPES } from "@/lib/voices/recipes";
import { voiceColor } from "@/lib/voices/palette";
import { CandidateCard } from "./CandidateCard";
import { useBuilderDraft } from "./useBuilderDraft";

const CANDIDATE_COUNT = 6;

export const VoiceBuilder: React.FC<{ onSaved: (voiceId: string) => void }> = ({
  onSaved,
}) => {
  const { t } = useTranslation();
  const {
    draft,
    setDraft,
    drafts,
    reloadDrafts,
    progress,
    setProgress,
    error,
    setError,
    patch,
  } = useBuilderDraft();
  const [busy, setBusy] = useState(false);

  const startFromRecipe = async (recipeId: string) => {
    const recipe = VOICE_RECIPES.find((r) => r.id === recipeId);
    if (!recipe) return;
    const res = await commands.ttsBuilderCreateDraft(
      recipe.name,
      recipe.description,
      recipe.probeText,
      recipe.tags,
    );
    if (res.status === "ok") {
      setDraft(res.data);
      reloadDrafts();
    } else {
      setError(res.error);
    }
  };

  const startEmpty = async () => {
    const res = await commands.ttsBuilderCreateDraft("", "", "", []);
    if (res.status === "ok") {
      setDraft(res.data);
      reloadDrafts();
    } else {
      setError(res.error);
    }
  };

  const generate = async () => {
    if (!draft) return;
    setBusy(true);
    setError(null);
    setProgress({ done: 0, total: CANDIDATE_COUNT });
    const res = await commands.ttsBuilderGenerate(draft.id, CANDIDATE_COUNT);
    setBusy(false);
    setProgress(null);
    if (res.status === "ok") setDraft(res.data);
    else setError(res.error);
  };

  const save = async () => {
    if (!draft) return;
    if (draft.selected === null) {
      setError(t("tts.builder.errorNoCandidate"));
      return;
    }
    const recipe = VOICE_RECIPES.find((r) => r.name === draft.display_name);
    const meta: VoiceMeta = {
      version: 1,
      display_name: draft.display_name,
      color: recipe?.color ?? "slate",
      avatar: null,
      language: "de-DE",
      description: draft.description,
      default_tags: draft.tags,
      default_style: null,
      styles: [],
    };
    setBusy(true);
    const res = await commands.ttsBuilderCommit(draft.id, meta);
    setBusy(false);
    if (res.status === "ok") {
      setDraft(null);
      reloadDrafts();
      onSaved(res.data);
    } else {
      setError(res.error);
    }
  };

  const discard = async () => {
    if (!draft) return;
    await commands.ttsBuilderDeleteDraft(draft.id);
    setDraft(null);
    reloadDrafts();
  };

  if (!draft) {
    return (
      <div className="space-y-3 rounded-lg border border-mid-gray/40 p-3">
        <p className="text-sm font-medium text-text">{t("tts.builder.recipes")}</p>
        <div className="flex flex-wrap gap-2">
          {VOICE_RECIPES.map((recipe) => (
            <button
              key={recipe.id}
              type="button"
              onClick={() => void startFromRecipe(recipe.id)}
              className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded-md border border-mid-gray/40 px-3 text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              <span
                aria-hidden="true"
                className="size-2.5 rounded-full"
                style={{ backgroundColor: voiceColor(recipe.color) }}
              />
              {recipe.name}
            </button>
          ))}
          <button
            type="button"
            onClick={() => void startEmpty()}
            className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded-md border border-dashed border-mid-gray/40 px-3 text-sm text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
          >
            <Wand2 width={15} height={15} aria-hidden="true" />
            {t("tts.builder.title")}
          </button>
        </div>

        {drafts.length > 0 && (
          <div className="space-y-1 border-t border-mid-gray/15 pt-2">
            <p className="text-xs font-semibold uppercase tracking-wide text-text/40">
              {t("tts.builder.drafts")}
            </p>
            {drafts.map((d) => (
              <button
                key={d.id}
                type="button"
                onClick={() => setDraft(d)}
                className="flex min-h-[44px] w-full cursor-pointer items-center justify-between gap-2 rounded-md px-2 text-start text-sm text-text/80 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
              >
                <span className="truncate">{d.display_name || t("tts.builder.title")}</span>
                <span className="shrink-0 text-xs text-text/45">
                  {d.candidates.length}
                </span>
              </button>
            ))}
          </div>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-4 rounded-lg border border-mid-gray/40 p-3">
      <p className="text-[11px] leading-4 text-text/45">{t("tts.builder.seedNote")}</p>

      <div className="space-y-2">
        <label className="block text-sm text-text/80" htmlFor="builder-name">
          {t("tts.builder.nameLabel")}
        </label>
        <Input
          id="builder-name"
          type="text"
          value={draft.display_name}
          onChange={(e) => patch({ display_name: e.target.value })}
          placeholder={t("tts.builder.namePlaceholder")}
          className="w-full"
        />
        <label className="block text-sm text-text/80" htmlFor="builder-desc">
          {t("tts.builder.descriptionLabel")}
        </label>
        <textarea
          id="builder-desc"
          value={draft.description}
          onChange={(e) => patch({ description: e.target.value })}
          placeholder={t("tts.builder.descriptionPlaceholder")}
          rows={4}
          className="block w-full rounded-md border border-mid-gray/80 bg-mid-gray/10 px-3 py-2 text-sm focus:border-logo-primary focus:outline-none"
        />
        <label className="block text-sm text-text/80" htmlFor="builder-probe">
          {t("tts.builder.probeLabel")}
        </label>
        <Input
          id="builder-probe"
          type="text"
          value={draft.probe_text}
          onChange={(e) => patch({ probe_text: e.target.value })}
          className="w-full"
        />
        <p className="text-[11px] leading-4 text-text/45">{t("tts.builder.probeHint")}</p>
      </div>

      <div className="space-y-2">
        <label className="block text-sm text-text/80" htmlFor="builder-depth">
          {t("tts.builder.depthLabel")}
        </label>
        <input
          id="builder-depth"
          type="range"
          min={1}
          max={1.15}
          step={0.01}
          value={draft.depth}
          onChange={(e) => patch({ depth: Number(e.target.value) })}
          className="w-full cursor-pointer"
        />
        <p className="text-[11px] leading-4 text-text/45">{t("tts.builder.depthHint")}</p>
      </div>

      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void generate()}
            disabled={busy}
            className="flex min-h-[44px] cursor-pointer items-center gap-2 rounded-md bg-logo-primary px-3 text-sm font-medium text-ink hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-default disabled:opacity-50"
          >
            {busy && <Loader2 width={15} height={15} className="animate-spin" aria-hidden="true" />}
            {progress
              ? t("tts.builder.generating", { done: progress.done, total: progress.total })
              : t("tts.builder.generate")}
          </button>
          {busy && (
            <button
              type="button"
              onClick={() => void commands.ttsBuilderCancel()}
              className="flex min-h-[44px] cursor-pointer items-center rounded-md px-3 text-sm text-text/70 hover:bg-mid-gray/15 hover:text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
            >
              {t("tts.builder.cancel")}
            </button>
          )}
        </div>

        {draft.candidates.length === 0 ? (
          <p className="text-xs text-text/50">{t("tts.builder.empty")}</p>
        ) : (
          <div className="space-y-1">
            {draft.candidates.map((candidate) => (
              <CandidateCard
                key={candidate.seed}
                draftId={draft.id}
                candidate={candidate}
                chosen={draft.selected === candidate.seed}
                onChoose={(seed) => patch({ selected: seed })}
              />
            ))}
          </div>
        )}
      </div>

      {error && <p className="text-sm text-red-500">{error}</p>}

      <div className="flex items-center gap-2 border-t border-mid-gray/15 pt-3">
        <button
          type="button"
          onClick={() => void save()}
          disabled={busy || draft.selected === null}
          className="flex min-h-[44px] cursor-pointer items-center rounded-md bg-logo-primary px-3 text-sm font-medium text-ink hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary disabled:cursor-default disabled:opacity-50"
        >
          {busy ? t("tts.builder.saving") : t("tts.builder.save")}
        </button>
        <button
          type="button"
          onClick={() => void discard()}
          className="flex min-h-[44px] cursor-pointer items-center gap-1 rounded-md px-3 text-sm text-text/70 hover:bg-mid-gray/15 hover:text-red-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-logo-primary"
        >
          <Trash2 width={15} height={15} aria-hidden="true" />
          {t("tts.builder.discard")}
        </button>
      </div>
    </div>
  );
};
```

---

## Reihenfolge und Parallelität

Unabhängig, können gleichzeitig laufen: **Task 1, Task 2, Task 5, Task 6** — sie berühren disjunkte Dateien (`dsp.rs` / `builder.rs`+`mod.rs`-Modulzeile / `recipes.ts` / Locales).

Danach seriell, weil aufeinander aufbauend: **Task 3** (braucht 1 und 2) → **Task 4** (braucht 3) → **Task 7** (braucht 4, 5, 6) → **Task 8** (braucht 2).

Achtung bei Task 2 und Task 3: beide fassen `mod.rs` an. Task 2 nur die eine `pub mod builder;`-Zeile am Dateikopf, Task 3 den `impl`-Block — kein Konflikt, solange Task 3 nach Task 2 läuft.

## Abnahme

Nach Task 8: `pnpm tauri build`, Installer starten, unter **Vorlesen → Stimmen** den Assistenten öffnen, „Pyrion" wählen, Kandidaten erzeugen, anhören. **Nur Patrick kann entscheiden, ob eine der Stimmen nach Pyrion klingt.** Reicht keine, ist der Tiefe-Regler der nächste Hebel und danach Etappe 2 (eigene Aufnahme).
