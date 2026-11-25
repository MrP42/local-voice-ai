//! Ablage-Brücke unter der VoiceStore-Sperre. Keine Netzaufrufe und keine Inhaltslogs.
use super::*;
use crate::ablage::speicher::{Ablage, Eingang};
use crate::ablage::system;
use crate::ablage::zusammenfuehren::{self as zf, Datei, Fassung, Ordner, Register};
use crate::sync::ablage::{konto_kennung, Konto, Zustand, SAMMLUNG_DATEI, SAMMLUNG_ORDNER};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Link {
    pub konto: String,
    pub file: String,
    pub sha256: String,
    pub voice: String,
    pub updated: i64,
    pub generation: u32,
    pub snapshot: String,
    pub name: String,
    pub display: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct Meta {
    id: String,
    uid: String,
    name: String,
    language: String,
    origin: String,
    voice_id: String,
    source_note: Option<String>,
    consent_at: Option<String>,
    created: String,
    sha256: String,
    fas: String,
    generation: u32,
}

/// Write-ahead für die lokale Projektion, auch bei Namenswechseln und -tausch.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Projection {
    pub(super) voices: Vec<Record>,
    installs: Vec<Record>,
    old_ids: Vec<String>,
}

fn fehler() -> VoiceError { VoiceError::Io("Stimmabgleich nicht abgeschlossen.".into()) }

pub(super) fn neue_stimmkennung() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

fn meta(d: &Datei, key: &[u8; 32]) -> Option<(i64, Meta)> {
    let v = d.extra.get("phonon_voice")?.as_array()?;
    if v.len() != 2 { return None; }
    let rank = v[0].as_str()?;
    let updated = rank.parse::<i64>().ok()?;
    if updated < 0 || rank != format!("{updated:020}") { return None; }
    let mut m: Meta = serde_json::from_value(v[1].clone()).ok()?;
    let parsed = parse_name(&m.name).ok()?;
    if !parsed.id.eq_ignore_ascii_case(&m.id) || parsed.display != m.name || !origin_is_own(&m.origin)
        || !matches!(m.language.as_str(), "und" | "de" | "en" | "fr" | "es" | "pt")
        || !hex(&m.sha256, 64) || !hex(&m.uid, 32)
        || chrono::DateTime::parse_from_rfc3339(&m.created).is_err()
        || d.id != system::kennung(key, false, m.generation, &m.uid)
        || d.ordner.wert != system::kennung(key, true, m.generation, "")
        || d.papierkorb.wert.is_some() || d.fassungen.is_empty()
        || d.fassungen.iter().any(|f| f.typ != "lvstimme" || !hex(&f.blob, 64)
            || f.blob != f.original_sha256 || !(MIN_VOICE_BYTES..=MAX_VOICE_BYTES).contains(&f.size)
            || !f.texte.is_empty())
    { return None; }
    let f = d.fassungen.iter().find(|f| f.fas == m.fas)?;
    if f.blob != m.sha256 { return None; }
    // Der Name ist ein eigenes Register: ein paralleler Bytewechsel benennt nicht zurück.
    let name = d.extra.get("phonon_name")?.as_array()?;
    if name.len() != 3 { return None; }
    let rank = name[0].as_str()?;
    let n = rank.parse::<i64>().ok()?;
    if n < 0 || rank != format!("{n:020}") { return None; }
    let id = name[1].as_str()?;
    let display = name[2].as_str()?;
    let parsed = parse_name(display).ok()?;
    if !parsed.id.eq_ignore_ascii_case(id) || parsed.display != display { return None; }
    m.id = id.into();
    m.name = display.into();
    Some((updated, m))
}

fn pruefen(bytes: &[u8]) -> Result<(), VoiceError> {
    validate_voice_bytes(bytes)?;
    let n = u64::from_le_bytes(bytes[..8].try_into().map_err(|_| fehler())?) as usize;
    let h: Value = serde_json::from_slice(&bytes[8..8 + n]).map_err(|_| fehler())?;
    if h["speaker_wavs"]["dtype"] != "F32" || h["speaker_wavs"]["shape"] != json!([1,512,125]) {
        return Err(VoiceError::InvalidFile("Stimmformat nicht unterstützt.".into()));
    }
    Ok(())
}

pub(crate) fn sync_fingerprint(d: &Datei) -> String {
    sha256_hex(crate::sync::collect::canonical(&serde_json::to_value(d).unwrap_or(Value::Null)).as_bytes())
}

pub(crate) fn sync_datei_gueltig(d: &Datei, key: &[u8; 32], bytes: Option<&[u8]>) -> bool {
    meta(d, key).is_some_and(|_| bytes.is_none_or(|b|
        pruefen(b).is_ok() && d.fassungen.iter().any(|f| f.blob == sha256_hex(b))))
}

fn zeit(r: &Record) -> i64 {
    if r.updated > 0 { r.updated } else {
        chrono::DateTime::parse_from_rfc3339(&r.created_at).map(|t| t.timestamp_millis().max(0)).unwrap_or(0)
    }
}

fn snapshot(r: &Record, sha: &str) -> String {
    sha256_hex(crate::sync::collect::canonical(&json!([r.id, r.uid, r.name, r.language,
        r.origin, r.voice_id, r.source_note, r.consent_at, r.created_at, r.updated, sha])).as_bytes())
}

fn set_meta(d: &mut Datei, updated: i64, m: &Meta) {
    d.extra.insert("phonon_voice".into(), json!([format!("{updated:020}"), m]));
}

fn folder(key: &[u8; 32], generation: u32) -> Ordner {
    let reg = |s: &str| Register::neu(s.to_string(), 1, "phonon-v1");
    Ordner { id: system::kennung(key, true, generation, ""),
        eltern: Register::neu(None, 1, "phonon-v1"), name: reg(".lv-stimmen"),
        datenklasse: reg("internal"), nur_geraet: Register::neu(false, 1, "phonon-v1"),
        zielordner_ki: Register::neu(false, 1, "phonon-v1"),
        papierkorb: Register::neu(None, 1, "phonon-v1"), extra: BTreeMap::new() }
}

impl VoiceStore<'_> {
    pub(super) fn finish_model_changes(&self, idx: &mut Index) {
        if !idx.sync_models_open { return; }
        let before = idx.sync_model_removals.len();
        let mut open = false;
        idx.sync_model_removals.retain(|r| {
            if idx.voices.iter().any(|v| v.id.eq_ignore_ascii_case(&r.id)) { return false; }
            let mut failed = false;
            let master = self.master_path(&r.id);
            if master.exists() && sha256_file(&master) == r.sha256 {
                failed |= fs::remove_file(&master).is_err();
            }
            for t in self.targets {
                let result = (|| -> Result<(), VoiceError> {
                    match self.target_state(t, &r.id, r.sha256.as_deref()) {
                        TargetState::Foreign => return Ok(()),
                        TargetState::Ours => {
                            fs::remove_file(Self::voice_file(t, &r.id)).map_err(|e| io_err(&e))?;
                        }
                        TargetState::Missing => {}
                    }
                    remove_row(&Self::voices_dir(t), &r.id, true)?;
                    Ok(())
                })();
                failed |= result.is_err();
            }
            open |= failed;
            failed
        });
        for r in &idx.voices {
            let Ok(bytes) = read_voice_file(&self.master_path(&r.id)) else { open = true; continue; };
            let sha = sha256_hex(&bytes);
            for t in self.targets {
                if self.target_state(t, &r.id, Some(&sha)) == TargetState::Foreign { continue; }
                if self.write_to_target(t, r, &bytes).is_err() { open = true; }
            }
        }
        if open { log::warn!("Phonon-Stimmen: optionale Modellarbeiten offen"); }
        if idx.sync_models_open != open || before != idx.sync_model_removals.len() {
            idx.sync_models_open = open;
            if self.write_index(idx).is_err() {
                log::warn!("Phonon-Stimmen: Status optionaler Modellarbeiten nicht gespeichert");
            }
        }
    }

    fn name_blocked(&self, a: &Ablage, d: &mut Datei, id: &str) -> Result<bool, VoiceError> {
        if !self.targets.iter().any(|t| self.target_state(t, id, None) == TargetState::Foreign) {
            return Ok(false);
        }
        // Je Name ein monotones Extra-Feld: der vorhandene Merge vereinigt die Sperren
        // auch bei parallelen Funden. Alle Geräte berechnen danach dieselben Suffixe.
        let key = format!("phonon_blocked_{}", id.to_ascii_lowercase());
        if d.extra.get(&key) != Some(&json!(true)) {
            d.extra.insert(key, json!(true));
            a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
        }
        Ok(true)
    }

    fn receive_path(&self, uid: &str) -> PathBuf {
        self.store_dir().join(format!(".sync-{uid}.safetensors"))
    }

    pub(super) fn projected_bytes(&self, r: &Record) -> Result<Vec<u8>, VoiceError> {
        if !hex(&r.uid, 32) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) {
            return Err(fehler());
        }
        for path in [self.master_path(&r.id), self.receive_path(&r.uid)] {
            if let Ok(bytes) = read_voice_file(&path) {
                if r.sync_file.as_ref().is_none_or(|l| sha256_hex(&bytes) == l.sha256) {
                    return Ok(bytes);
                }
            }
        }
        Err(fehler())
    }

    pub(super) fn finish_receive(&self, idx: &mut Index) -> Result<(), VoiceError> {
        let Some(p) = &idx.sync_receive else { return Ok(()); };
        let names = p.voices.iter().map(|r| r.id.to_ascii_lowercase()).collect::<BTreeSet<_>>();
        if names.len() != p.voices.len() { return Err(fehler()); }
        // Den gesamten Plan prüfen, bevor die erste Masterdatei geändert wird.
        for r in &p.installs {
            if !hex(&r.uid, 32) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { return Err(fehler()); }
            let bytes = read_voice_file(&self.receive_path(&r.uid))?;
            let link = r.sync_file.as_ref().ok_or_else(fehler)?;
            if sha256_hex(&bytes) != link.sha256 { return Err(fehler()); }
        }
        // Auch die optionale Aufräumarbeit vor dem Löschen ihrer Hashquelle sichern.
        let before = idx.sync_model_removals.len();
        for id in &p.old_ids {
            if p.voices.iter().any(|r| r.id.eq_ignore_ascii_case(id)) { continue; }
            if !parse_name(id).is_ok_and(|n| n.id == *id) { return Err(fehler()); }
            if !idx.sync_model_removals.iter().any(|r| r.id.eq_ignore_ascii_case(id)) {
                idx.sync_model_removals.push(ModelRemoval { id: id.clone(), sha256: sha256_file(&self.master_path(id)) });
            }
        }
        if before != idx.sync_model_removals.len() { self.write_index(idx)?; }
        // Alle Quellen liegen vor dem Journal dauerhaft unter uid, niemals unter Zielnamen.
        let mut installs = p.installs.iter().collect::<Vec<_>>();
        // Bisherige Namensinhaber zuerst retten, dann die Gewinner installieren.
        installs.sort_by_key(|r| !idx.voices.iter().any(|old|
            old.uid == r.uid && !old.id.eq_ignore_ascii_case(&r.id)));
        for r in installs {
            if !hex(&r.uid, 32) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { return Err(fehler()); }
            let bytes = read_voice_file(&self.receive_path(&r.uid))?;
            if r.sync_file.as_ref().is_none_or(|l| sha256_hex(&bytes) != l.sha256) { return Err(fehler()); }
            write_atomic(&self.master_path(&r.id), &bytes)?;
        }
        for id in &p.old_ids {
            if p.voices.iter().any(|r| r.id.eq_ignore_ascii_case(id)) { continue; }
            if !parse_name(id).is_ok_and(|n| n.id == *id) { return Err(fehler()); }
            let path = self.master_path(id);
            if path.exists() { fs::remove_file(path).map_err(|e| io_err(&e))?; }
        }
        let p = idx.sync_receive.take().ok_or_else(fehler)?;
        idx.voices = p.voices;
        idx.sync_models_open = true;
        self.write_index(idx)?;
        for r in p.installs { let _ = fs::remove_file(self.receive_path(&r.uid)); }
        Ok(())
    }

    /// Grabsteine lösen nur neue Transportkennungen aus, niemals lokale Löschungen.
    pub(crate) fn sync_ablage(&self, a: &Ablage, konto: &Konto<'_>) -> Result<(), VoiceError> {
        let _g = self.guard();
        let kennung = konto_kennung(konto.user_id, konto.key, konto.hub_url);
        let mut idx = self.read_index()?;
        let b = a.sync_bestand().map_err(|_| fehler())?;
        let z = Zustand::laden(a);
        let tot = |collection: &str, id: &str| b.begraben.contains(id)
            || (z.konto == kennung && z.objekte.get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted));
        let mut generation = 0u32;
        while tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, generation, "")) {
            generation = generation.checked_add(1).ok_or_else(fehler)?;
        }
        for r in idx.voices.clone() {
            if idx.sync_removed.contains(&r.uid) { continue; }
            if !origin_is_own(&r.origin) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { continue; }
            let bytes = match read_voice_file(&self.master_path(&r.id)) {
                Ok(v) if pruefen(&v).is_ok() => v,
                _ => continue,
            };
            let sha = sha256_hex(&bytes);
            // Jede bekannte Generation höchstens einmal prüfen; pro Stimme nur einmal bereitstellen.
            let mut generation = generation.max(r.sync_file.as_ref()
                .filter(|l| l.konto == kennung).map(|l| l.generation).unwrap_or(0));
            let mut hub_rank = None;
            for d in b.dateien.values() {
                if let Some((u, m)) = meta(d, konto.key).filter(|(_, m)| m.uid == r.uid) {
                    generation = generation.max(m.generation);
                    if !tot(SAMMLUNG_DATEI, &d.id) && !tot(SAMMLUNG_ORDNER, &d.ordner.wert) {
                        let rank = (u, m.sha256);
                        if hub_rank.as_ref().is_none_or(|old| old < &rank) { hub_rank = Some(rank); }
                    }
                }
            }
            while tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, generation, ""))
                || tot(SAMMLUNG_DATEI, &system::kennung(konto.key, false, generation, &r.uid)) {
                generation = generation.checked_add(1).ok_or_else(fehler)?;
            }
            let file = system::kennung(konto.key, false, generation, &r.uid);
            let existing = b.dateien.get(&file);
            let old = existing.and_then(|d| meta(d, konto.key));
            let link = r.sync_file.as_ref().filter(|l| l.konto == kennung);
            // Auch bei einem alten Link einen Abbruch vor dem neuen Original fertigstellen.
            if let Some((_, m)) = old.as_ref().filter(|(_, m)| m.sha256 == sha) {
                if a.original_quelle(&file, &m.fas).map_err(|_| fehler())?.is_none() {
                    let tmp = a.sync_zwischenpfad().map_err(|_| fehler())?;
                    crate::ablage::schreibe_atomar(&tmp, &bytes).map_err(|_| fehler())?;
                    if !a.original_einlegen(&file, &m.fas, &tmp).map_err(|_| fehler())? { return Err(fehler()); }
                }
            }
            // Wiederanlauf nach Ablagedatei/Original, vor Link: keine neue Fassung oder Zeit.
            let wanted = link.filter(|l| l.display == r.name).map(|l| l.name.as_str()).unwrap_or(&r.name);
            if let Some((updated, m)) = old.as_ref().filter(|(updated, m)|
                *updated >= zeit(&r) && link.is_none_or(|l| l.file != file)
                && m.sha256 == sha && m.name == wanted && m.origin == r.origin
                && m.voice_id == r.voice_id && m.source_note == r.source_note
                && m.consent_at == r.consent_at && m.created == r.created_at
                && (m.language == r.language || (m.language == "und" && r.language.is_empty()))) {
                let local = idx.voices.iter_mut().find(|x| x.uid == r.uid).ok_or_else(fehler)?;
                local.updated = *updated;
                local.sync_file = Some(Link { konto: kennung.clone(), file, sha256: sha.clone(),
                    voice: r.uid, updated: *updated, generation, snapshot: snapshot(local, &sha),
                    name: m.name.clone(), display: local.name.clone() });
                self.write_index(&idx)?;
                continue;
            }
            let content_changed = link.is_none_or(|l| l.updated != zeit(&r) || l.sha256 != sha);
            let unchanged = !content_changed && link.is_some_and(|l| l.snapshot == snapshot(&r, &sha));
            if !hub_rank.as_ref().is_none_or(|(u, s)| {
                let rank = (*u, s.as_str());
                rank <= (zeit(&r), sha.as_str())
            }) {
                // Ein Namensregister bleibt unabhängig vom Gewinner der Stimmbytes.
                if let Some(d) = existing.filter(|_| link.is_some_and(|l| l.display != r.name)) {
                    let mut d = d.clone();
                    let name_rank = d.extra.get("phonon_name").and_then(|v| v[0].as_str())
                        .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                    let rank = zeit(&r).max(name_rank.checked_add(1).ok_or_else(fehler)?);
                    d.extra.insert("phonon_name".into(), json!([format!("{rank:020}"), r.id, r.name]));
                    a.sync_einspielen(&[Eingang::Datei(d)]).map_err(|_| fehler())?;
                }
                continue;
            }
            if unchanged && old.as_ref().is_some_and(|(u, m)|
                (*u, m.sha256.as_str()) >= (zeit(&r), sha.as_str())) { continue; }
            // Datei-vor-Index: den bereits gezogenen Stand übernehmen, nicht erneut veröffentlichen.
            if old.as_ref().is_some_and(|(u, m)| *u >= zeit(&r) && m.sha256 == sha)
                && link.is_some_and(|l| l.sha256 != sha) { continue; }
            let language = if r.language.is_empty() { "und" } else { &r.language };
            if !matches!(language, "und" | "de" | "en" | "fr" | "es" | "pt") { continue; }
            let updated = zeit(&r);
            let reg = |s: &str| Register::neu(s.to_string(), 1, "phonon-v1");
            let mut d = existing.cloned().unwrap_or_else(|| Datei {
                id: file.clone(), ordner: reg(&system::kennung(konto.key, true, generation, "")),
                name: reg("voice.safetensors"), klasse_min: reg("internal"), nur_geraet_min: false,
                fassungen: vec![], wahlen: vec![], entfernt: vec![], blob_refs: vec![],
                papierkorb: Register::neu(None, 1, "phonon-v1"), extra: BTreeMap::new(),
            });
            let fas = if let Some((_, m)) = old.as_ref().filter(|(u, m)| *u == updated && m.sha256 == sha) { m.fas.clone() }
                else {
                    let fas = crate::ablage::neue_id("fas_");
                    d.fassungen.push(Fassung { fas: fas.clone(), vorgaenger: old.as_ref().map(|(_, m)| m.fas.clone()),
                        blob: sha.clone(), original_sha256: sha.clone(), size: bytes.len() as u64,
                        typ: "lvstimme".into(), geraet: konto.device_id.into(), zeit: updated, texte: vec![] });
                    fas
                };
            let wanted = link.filter(|l| l.display == r.name).map(|l| l.name.as_str()).unwrap_or(&r.name);
            let wanted = parse_name(wanted)?;
            if !d.extra.contains_key("phonon_name") || link.is_some_and(|l| l.display != r.name) {
                let name_rank = d.extra.get("phonon_name").and_then(|v| v[0].as_str())
                    .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                let rank = updated.max(name_rank.checked_add(1).ok_or_else(fehler)?);
                d.extra.insert("phonon_name".into(), json!([format!("{rank:020}"), wanted.id, wanted.display]));
            }
            let content_sha = d.fassungen.iter().find(|f| f.fas == fas).ok_or_else(fehler)?.blob.clone();
            let m = Meta { id: wanted.id, uid: r.uid.clone(), name: wanted.display,
                language: language.into(), origin: r.origin.clone(), consent_at: r.consent_at.clone(),
                voice_id: r.voice_id.clone(), source_note: r.source_note.clone(),
                created: r.created_at.clone(), sha256: content_sha, fas: fas.clone(), generation };
            set_meta(&mut d, updated, &m);
            d.blob_refs = zf::blob_refs_aus(&d.fassungen);
            a.sync_einspielen(&[Eingang::Ordner(folder(konto.key, generation)), Eingang::Datei(d)]).map_err(|_| fehler())?;
            if m.sha256 == sha {
                let tmp = a.sync_zwischenpfad().map_err(|_| fehler())?;
                crate::ablage::schreibe_atomar(&tmp, &bytes).map_err(|_| fehler())?;
                if !a.original_einlegen(&file, &fas, &tmp).map_err(|_| fehler())? { return Err(fehler()); }
            }
            let local = idx.voices.iter_mut().find(|x| x.uid == r.uid).ok_or_else(fehler)?;
            local.updated = updated;
            local.sync_file = Some(Link { konto: kennung.clone(), file, sha256: sha.clone(), voice: r.uid,
                updated, generation, snapshot: snapshot(local, &sha), name: m.name, display: local.name.clone() });
            self.write_index(&idx)?;
        }
        let b = a.sync_bestand().map_err(|_| fehler())?;
        let mut candidates = BTreeMap::<String, (Datei, i64, Meta, Vec<u8>)>::new();
        for d in b.dateien.values().filter(|d| system::ist_datei(&d.id)) {
            let fingerprint = sync_fingerprint(d);
            if idx.sync_rejected.contains(&fingerprint) || b.begraben.contains(&d.id) { continue; }
            let Some((mut updated, mut m)) = meta(d, konto.key) else {
                idx.sync_rejected.insert(fingerprint); continue;
            };
            if idx.sync_removed.contains(&m.uid) { continue; }
            if tot(SAMMLUNG_DATEI, &d.id) || tot(SAMMLUNG_ORDNER, &d.ordner.wert)
                || b.ordner.get(&d.ordner.wert).is_none_or(|o| o.papierkorb.wert.is_some()) { continue; }
            let heads = zf::koepfe(d);
            let Some(selected) = d.fassungen.iter().filter(|f| heads.contains(&f.fas))
                .max_by(|a, b| (a.zeit, &a.blob, &a.fas).cmp(&(b.zeit, &b.blob, &b.fas))) else { continue; };
            let mut d = d.clone();
            if selected.fas != m.fas {
                m.fas = selected.fas.clone();
                m.sha256 = selected.blob.clone();
                updated = updated.checked_add(1).ok_or_else(fehler)?;
                set_meta(&mut d, updated, &m);
                a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
            }
            let Some(path) = a.original_quelle(&d.id, &m.fas).map_err(|_| fehler())? else { continue; };
            let bytes = match fs::read(path) {
                Ok(v) if pruefen(&v).is_ok() && sha256_hex(&v) == m.sha256 => v,
                _ => { idx.sync_rejected.insert(fingerprint); continue; }
            };
            if candidates.get(&m.uid).is_none_or(|(_, u, old, _)|
                (*u, &old.sha256) < (updated, &m.sha256)) {
                candidates.insert(m.uid.clone(), (d.clone(), updated, m, bytes));
            }
        }
        // Ältere empfangene Stände dürfen keine neuere lokale Fassung ersetzen.
        candidates.retain(|uid, (_, updated, m, _)| {
            let Some(r) = idx.voices.iter().find(|r| r.uid == *uid) else { return true; };
            let Some(sha) = sha256_file(&self.master_path(&r.id)) else { return true; };
            (*updated, m.sha256.as_str()) >= (zeit(r), sha.as_str())
        });
        for (d, _, m, _) in candidates.values_mut() {
            if zf::aktuell(d).as_deref() != Some(m.fas.as_str()) {
                let count = zf::datei_zaehler_max(d).checked_add(1).ok_or_else(fehler)?;
                let w = zf::neue_wahl(d, &crate::ablage::neue_id("wahl_"), &m.fas, count, "phonon-v1");
                d.wahlen.push(w);
                a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
            }
        }
        // Kapazität vor dem Gesamtplan prüfen: niemand darf einen Namen freigeben,
        // dessen bisherige UID anschließend aus der Projektion herausfällt.
        let mut slots = MAX_OWN_VOICES.saturating_sub(idx.voices.len());
        candidates.retain(|uid, _| {
            if idx.voices.iter().any(|r| r.uid == *uid) { return true; }
            if slots == 0 { return false; }
            slots -= 1;
            true
        });
        // Basisnamen bleiben Metadaten. Echte Basisnamen zuerst reservieren; Suffixe nach uid.
        let mut groups = BTreeMap::<String, Vec<String>>::new();
        for (uid, (_, _, m, _)) in &candidates { groups.entry(m.id.to_ascii_lowercase()).or_default().push(uid.clone()); }
        let mut assigned = BTreeMap::new();
        let mut occupied = crate::catalog::tts_entries(crate::catalog::Purpose::PhononVoice)
            .into_iter().filter_map(|e| e.voice).flat_map(|v| {
                let mut names = Vec::new();
                if let Some(stem) = Path::new(&v.filename).file_stem().and_then(|s| s.to_str()) {
                    names.push(stem.to_ascii_lowercase());
                }
                if let Ok(name) = parse_name(&v.name) { names.push(name.id.to_ascii_lowercase()); }
                names
            })
            .collect::<BTreeSet<_>>();
        let mut blocked = occupied.clone();
        for (d, _, _, _) in candidates.values() {
            for (key, value) in &d.extra {
                if value == &json!(true) {
                    if let Some(id) = key.strip_prefix("phonon_blocked_") {
                        if parse_name(id).is_ok_and(|n| n.id == id) {
                            blocked.insert(id.to_string()); occupied.insert(id.to_string());
                        }
                    }
                }
            }
        }
        for r in &idx.voices {
            if !candidates.contains_key(&r.uid) {
                occupied.insert(r.id.to_ascii_lowercase());
            }
        }
        for (base, ids) in &mut groups {
            ids.sort_by_key(|u| (chrono::DateTime::parse_from_rfc3339(&candidates[u].2.created)
                .map(|t| t.timestamp_millis()).unwrap_or(0), u.clone()));
            if occupied.insert(base.clone()) {
                let (d, _, m, _) = candidates.get_mut(&ids[0]).ok_or_else(fehler)?;
                if !self.name_blocked(a, d, &m.id)? {
                    assigned.insert(ids[0].clone(), m.name.clone());
                } else {
                    blocked.insert(base.clone());
                }
            }
        }
        let mut rest = candidates.keys().filter(|uid| !assigned.contains_key(*uid)).cloned().collect::<Vec<_>>();
        rest.sort();
        for uid in rest {
            let (d, _, m, _) = candidates.get_mut(&uid).ok_or_else(fehler)?;
            let mut eigen = blocked.contains(&m.id.to_ascii_lowercase());
            let mut n = if eigen { 1usize } else { 2 };
            loop {
                let suffix = if eigen {
                    if n == 1 { "-eigen".to_string() } else { format!("-eigen-{n}") }
                } else { format!("-{n}") };
                let base = m.id.chars().take(MAX_NAME_CHARS.saturating_sub(suffix.len())).collect::<String>();
                let name = format!("{base}{suffix}");
                let parsed = parse_name(&name)?;
                let key = parsed.id.to_ascii_lowercase();
                let foreign = blocked.contains(&key) || self.name_blocked(a, d, &parsed.id)?;
                if foreign && !eigen {
                    eigen = true; n = 1; continue;
                }
                if !foreign && occupied.insert(key) {
                    assigned.insert(uid, name); break;
                }
                n = n.checked_add(1).ok_or_else(fehler)?;
            }
        }
        let mut voices = idx.voices.clone();
        let mut installs = Vec::new();
        let old_ids = idx.voices.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        for (uid, (d, updated, m, bytes)) in &candidates {
            let local = voices.iter().find(|r| r.uid == *uid).cloned();
            let name = &assigned[uid];
            let parsed = parse_name(name)?;
            let id = local.as_ref().filter(|r| r.id.eq_ignore_ascii_case(&parsed.id))
                .map(|r| r.id.clone()).unwrap_or(parsed.id);
            let mut r = Record { id, uid: uid.clone(), name: name.clone(), origin: m.origin.clone(),
                voice_id: m.voice_id.clone(),
                language: if m.language == "und" { String::new() } else { m.language.clone() },
                created_at: m.created.clone(), consent_at: m.consent_at.clone(),
                source_note: m.source_note.clone(), updated: *updated,
                sync_file: None };
            r.sync_file = Some(Link { konto: kennung.clone(), file: d.id.clone(), sha256: m.sha256.clone(),
                voice: uid.clone(), updated: *updated, generation: m.generation, snapshot: snapshot(&r, &m.sha256),
                name: m.name.clone(), display: r.name.clone() });
            if local.as_ref() == Some(&r) && sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str()) { continue; }
            // Beim ersten Empfang gibt es noch keinen Index und damit keinen Stimmenordner.
            fs::create_dir_all(self.store_dir()).map_err(|e| io_err(&e))?;
            write_atomic(&self.receive_path(uid), bytes)?;
            voices.retain(|v| v.uid != *uid);
            voices.push(r.clone());
            installs.push(r);
        }
        self.write_index(&idx)?;
        if !installs.is_empty() {
            idx.sync_receive = Some(Projection { voices, installs, old_ids });
            self.write_index(&idx)?;
            self.finish_receive(&mut idx)?;
            self.finish_model_changes(&mut idx);
        }
        Ok(())
    }
}
