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

/// Dauerhafter Auftrag für exakt eine uid; bei Erstsync erst nach bestätigtem Gewinner ausführen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Grave {
    pub konto: String,
    pub file: String,
    pub voice: String,
    pub after: Option<String>,
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
    linked: bool,
    // Nur Erstsync-Zusammenführungen, keine Löschbelege für fremde Kennungen.
    merged: Vec<String>,
}

/// Write-ahead für die lokale Projektion, auch bei Namenswechseln und -tausch.
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Projection {
    voices: Vec<Record>,
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
        || m.merged.iter().any(|u| !hex(u, 32) || u <= &m.uid)
        || m.merged.windows(2).any(|w| w[0] >= w[1])
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
            if self.targets.iter().any(|t| self.target_state(t, &r.id, Some(&link.sha256)) == TargetState::Foreign) {
                return Err(fehler());
            }
        }
        // Alle Quellen liegen vor dem Journal dauerhaft unter uid, niemals unter Zielnamen.
        for r in &p.installs {
            if !hex(&r.uid, 32) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { return Err(fehler()); }
            let bytes = read_voice_file(&self.receive_path(&r.uid))?;
            if r.sync_file.as_ref().is_none_or(|l| sha256_hex(&bytes) != l.sha256) { return Err(fehler()); }
            write_atomic(&self.master_path(&r.id), &bytes)?;
            for t in self.targets {
                if self.target_state(t, &r.id, r.sync_file.as_ref().map(|l| l.sha256.as_str())) == TargetState::Foreign {
                    return Err(fehler());
                }
                self.write_to_target(t, r, &bytes)?;
            }
        }
        for id in &p.old_ids {
            if p.voices.iter().any(|r| r.id.eq_ignore_ascii_case(id)) { continue; }
            if !parse_name(id).is_ok_and(|n| n.id == *id) { return Err(fehler()); }
            for t in self.targets {
                if self.target_state(t, id, None) == TargetState::Ours {
                    let path = Self::voice_file(t, id);
                    if path.exists() { fs::remove_file(path).map_err(|e| io_err(&e))?; }
                    remove_row(&Self::voices_dir(t), id, true)?;
                }
            }
            let path = self.master_path(id);
            if path.exists() { fs::remove_file(path).map_err(|e| io_err(&e))?; }
        }
        let p = idx.sync_receive.take().ok_or_else(fehler)?;
        idx.voices = p.voices;
        self.write_index(idx)?;
        for r in p.installs { let _ = fs::remove_file(self.receive_path(&r.uid)); }
        Ok(())
    }

    /// Erst nach einem vollständigen Pull aufrufen. Ordnergräber haben Vorrang vor Dateigräbern.
    pub(crate) fn sync_ablage(&self, a: &Ablage, konto: &Konto<'_>) -> Result<(), VoiceError> {
        let _g = self.guard();
        let kennung = konto_kennung(konto.user_id, konto.key, konto.hub_url);
        let mut idx = self.read_index()?;
        for r in idx.voices.iter().filter(|r| idx.sync_removed.contains(&r.uid)) {
            self.remove_received(&r.id)?;
        }
        idx = self.read_index()?;
        for l in idx.sync_pending.clone().iter().filter(|l| l.konto == kennung && l.after.is_none()) {
            if let Some(r) = idx.voices.iter().find(|r| r.uid == l.voice).cloned() {
                self.remove_received(&r.id)?;
                idx = self.read_index()?;
            }
        }
        let mut b = a.sync_bestand().map_err(|_| fehler())?;
        let z = Zustand::laden(a);
        if z.konto == kennung && z.phonon_pull_offen { return Ok(()); }
        let tot = |collection: &str, id: &str| b.begraben.contains(id)
            || (z.konto == kennung && z.objekte.get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted));
        let mut generation = 0u32;
        while tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, generation, "")) {
            generation = generation.checked_add(1).ok_or_else(fehler)?;
        }
        // Nutzerlöschung betrifft exakt die verknüpfte Datei, nie einen Namen oder Hash.
        // Bei einem alten rekursiven Ordnergrab bleiben lokale Stimmen ausdrücklich erhalten.
        for r in idx.voices.clone() {
            let Some(l) = r.sync_file.as_ref().filter(|l| l.konto == kennung && l.voice == r.uid
                && l.file == system::kennung(konto.key, false, l.generation, &r.uid)) else { continue; };
            if tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, l.generation, "")) { continue; }
            // Erstsync-Zusammenführungen binden um, anstatt die lokale Stimme zu entfernen.
            let merged = b.dateien.values().any(|d| !tot(SAMMLUNG_DATEI, &d.id)
                && meta(d, konto.key).is_some_and(|(_, m)| m.generation == generation
                    && m.merged.contains(&r.uid)));
            if !merged && !tot(SAMMLUNG_DATEI, &l.file) { continue; }
            let sha = sha256_file(&self.master_path(&r.id)).unwrap_or_default();
            if merged && snapshot(&r, &sha) == l.snapshot { continue; }
            let ordner = system::kennung(konto.key, true, l.generation, "");
            if !merged && (z.konto != kennung || !z.phonon_grab_bestaetigt.contains(&l.file)
                || b.ordner.get(&ordner).is_none_or(|o| o.papierkorb.wert.is_some())
                || !z.objekte.get(&format!("{SAMMLUNG_ORDNER}/{ordner}"))
                    .is_some_and(|o| !o.deleted && !o.angehalten)) { continue; }
            if snapshot(&r, &sha) == l.snapshot {
                self.remove_received(&r.id)?;
                idx = self.read_index()?;
            } else {
                let local = idx.voices.iter_mut().find(|x| x.uid == r.uid).ok_or_else(fehler)?;
                local.uid = neue_stimmkennung();
                local.sync_file = None;
                local.sync_linked = true;
                self.write_index(&idx)?;
            }
        }
        // Ein Publish kann vor dem Link abstürzen. UID-Aufträge entstehen deshalb beim
        // Entfernen immer; ihre kontogebundenen Dateikennungen werden hier abgeleitet.
        for uid in &idx.sync_removed {
            for g in 0..=generation {
                let file = system::kennung(konto.key, false, g, uid);
                if (b.dateien.contains_key(&file) || z.objekte.contains_key(&format!("{SAMMLUNG_DATEI}/{file}")))
                    && !b.begraben.contains(&file)
                    && !z.objekte.get(&format!("{SAMMLUNG_DATEI}/{file}")).is_some_and(|o| o.deleted)
                    && !idx.sync_pending.iter().any(|l| l.konto == kennung && l.file == file) {
                    idx.sync_pending.push(Grave { konto: kennung.clone(), file, voice: uid.clone(), after: None });
                }
            }
        }
        self.write_index(&idx)?;
        // Ausschließlich explizit vorgemerkte Dateien begraben. Der normale CAS-Pfad ist zuständig.
        let graves = idx.sync_pending.iter().filter(|l| l.konto == kennung && l.after.as_ref().is_none_or(|id|
            z.objekte.get(&format!("{SAMMLUNG_DATEI}/{id}")).is_some_and(|o| o.deleted || b.dateien.get(id)
                .is_some_and(|d| o.hash == crate::sync::ablage::hash_datei(d) && meta(d, konto.key)
                    .is_some_and(|(_, m)| m.merged.contains(&l.voice))))))
            .map(|l| Eingang::Grab(l.file.clone())).collect::<Vec<_>>();
        if !graves.is_empty() { a.sync_einspielen(&graves).map_err(|_| fehler())?; }
        idx.sync_pending.retain(|l| l.konto != kennung || !z.objekte
            .get(&format!("{SAMMLUNG_DATEI}/{}", l.file)).is_some_and(|o| o.deleted));
        self.write_index(&idx)?;
        b = a.sync_bestand().map_err(|_| fehler())?;
        for r in idx.voices.clone() {
            if !origin_is_own(&r.origin) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { continue; }
            let bytes = match read_voice_file(&self.master_path(&r.id)) {
                Ok(v) if pruefen(&v).is_ok() => v,
                _ => continue,
            };
            let sha = sha256_hex(&bytes);
            let file = system::kennung(konto.key, false, generation, &r.uid);
            if b.begraben.contains(&file) { continue; }
            let existing = b.dateien.get(&file);
            let old = existing.and_then(|d| meta(d, konto.key));
            let link = r.sync_file.as_ref().filter(|l| l.konto == kennung);
            let unchanged = link.is_some_and(|l| l.snapshot == snapshot(&r, &sha));
            if unchanged && old.is_some() { continue; }
            // Datei-vor-Index: den bereits gezogenen Stand übernehmen, nicht erneut veröffentlichen.
            if old.as_ref().is_some_and(|(u, m)| *u >= zeit(&r) && m.sha256 == sha)
                && link.is_some_and(|l| l.sha256 != sha) { continue; }
            let language = if r.language.is_empty() { "und" } else { &r.language };
            if !matches!(language, "und" | "de" | "en" | "fr" | "es" | "pt") { continue; }
            let updated = if let Some((u, _)) = &old {
                zeit(&r).max(u.checked_add(1).ok_or_else(fehler)?)
            } else { zeit(&r) };
            let reg = |s: &str| Register::neu(s.to_string(), 1, "phonon-v1");
            let mut d = existing.cloned().unwrap_or_else(|| Datei {
                id: file.clone(), ordner: reg(&system::kennung(konto.key, true, generation, "")),
                name: reg("voice.safetensors"), klasse_min: reg("internal"), nur_geraet_min: false,
                fassungen: vec![], wahlen: vec![], entfernt: vec![], blob_refs: vec![],
                papierkorb: Register::neu(None, 1, "phonon-v1"), extra: BTreeMap::new(),
            });
            let content_changed = link.is_none_or(|l| l.sha256 != sha);
            let fas = if !content_changed && old.is_some() { old.as_ref().unwrap().1.fas.clone() }
                else if let Some((_, m)) = old.as_ref().filter(|(_, m)| m.sha256 == sha) { m.fas.clone() }
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
                d.extra.insert("phonon_name".into(), json!([format!("{updated:020}"), wanted.id, wanted.display]));
            }
            let content_sha = d.fassungen.iter().find(|f| f.fas == fas).ok_or_else(fehler)?.blob.clone();
            let m = Meta { id: wanted.id, uid: r.uid.clone(), name: wanted.display,
                language: language.into(), origin: r.origin.clone(), consent_at: r.consent_at.clone(),
                voice_id: r.voice_id.clone(), source_note: r.source_note.clone(),
                created: r.created_at.clone(), sha256: content_sha, fas: fas.clone(), generation,
                linked: r.sync_linked || link.is_some(), merged: old.as_ref().map(|(_, m)| m.merged.clone()).unwrap_or_default() };
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
        b = a.sync_bestand().map_err(|_| fehler())?;
        let mut candidates = BTreeMap::<String, (Datei, i64, Meta, Vec<u8>)>::new();
        for d in b.dateien.values().filter(|d| system::ist_datei(&d.id)) {
            let fingerprint = sync_fingerprint(d);
            if idx.sync_rejected.contains(&fingerprint) || b.begraben.contains(&d.id) { continue; }
            let Some((mut updated, mut m)) = meta(d, konto.key) else {
                idx.sync_rejected.insert(fingerprint); continue;
            };
            if idx.sync_removed.contains(&m.uid) { continue; }
            if m.generation != generation || b.ordner.get(&d.ordner.wert).is_none_or(|o| o.papierkorb.wert.is_some())
                || idx.sync_pending.iter().any(|l| l.konto == kennung && l.file == d.id) { continue; }
            let heads = zf::koepfe(d);
            let Some(selected) = d.fassungen.iter().filter(|f| heads.contains(&f.fas))
                .max_by(|a, b| (a.zeit, &a.fas).cmp(&(b.zeit, &b.fas))) else { continue; };
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
            candidates.insert(m.uid.clone(), (d.clone(), updated, m, bytes));
        }
        // Nur noch nie empfangene Erstimporte zusammenführen. Kleinste uid gewinnt.
        let mut aliases = BTreeMap::<String, String>::new();
        let ids = candidates.keys().cloned().collect::<Vec<_>>();
        for (i, uid) in ids.iter().enumerate() {
            let m = &candidates[uid].2;
            if m.linked { continue; }
            for other in &ids[..i] {
                let n = &candidates[other].2;
                if !n.linked && m.id.eq_ignore_ascii_case(&n.id) && m.sha256 == n.sha256 {
                    aliases.insert(uid.clone(), aliases.get(other).cloned().unwrap_or_else(|| other.clone()));
                    break;
                }
            }
        }
        for (uid, (_, _, m, _)) in &candidates {
            for old in &m.merged { aliases.insert(old.clone(), uid.clone()); }
        }
        for (loser, winner) in &aliases {
            let Some(lm) = candidates.get(loser).map(|c| &c.2) else { continue; };
            let loser_file = system::kennung(konto.key, false, lm.generation, loser);
            let Some((d, updated, m, _)) = candidates.get_mut(winner) else { continue; };
            if !m.merged.contains(loser) {
                m.merged.push(loser.clone()); m.merged.sort(); m.merged.dedup();
                *updated = updated.checked_add(1).ok_or_else(fehler)?;
                set_meta(d, *updated, m);
                // Gewinner samt Zuordnung dauerhaft VOR dem Grab; beim Neustart wiederholbar.
                a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
            }
            // Write-ahead bleibt auch erhalten, wenn der Nutzer den Gewinner vor dem nächsten
            // Push entfernt. Das unabhängige Verlierergrab ist Teil der Zusammenführung.
            if !idx.sync_pending.iter().any(|l| l.konto == kennung && l.file == loser_file) {
                idx.sync_pending.push(Grave { konto: kennung.clone(), file: loser_file,
                    voice: loser.clone(), after: Some(d.id.clone()) });
                self.write_index(&idx)?;
            }
        }
        candidates.retain(|uid, _| !aliases.contains_key(uid));
        for (d, updated, m, _) in candidates.values_mut() {
            if zf::aktuell(d).as_deref() != Some(m.fas.as_str()) {
                let count = zf::datei_zaehler_max(d).checked_add(1).ok_or_else(fehler)?;
                let w = zf::neue_wahl(d, &crate::ablage::neue_id("wahl_"), &m.fas, count, "phonon-v1");
                d.wahlen.push(w);
                a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
            }
            // Erst Empfang auf einem anderen Gerät beendet das Erstsync-Fenster.
            if !m.linked && !idx.voices.iter().any(|r| r.uid == m.uid) {
                m.linked = true;
                *updated = updated.checked_add(1).ok_or_else(fehler)?;
                set_meta(d, *updated, m);
                a.sync_einspielen(&[Eingang::Datei(d.clone())]).map_err(|_| fehler())?;
            }
        }
        // Kapazität vor dem Gesamtplan prüfen: niemand darf einen Namen freigeben,
        // dessen bisherige UID anschließend aus der Projektion herausfällt.
        let mut slots = MAX_OWN_VOICES.saturating_sub(idx.voices.len());
        candidates.retain(|uid, _| {
            if idx.voices.iter().any(|r| r.uid == *uid || aliases.get(&r.uid) == Some(uid)) { return true; }
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
        for (d, _, _, _) in candidates.values() {
            for (key, value) in &d.extra {
                if value == &json!(true) {
                    if let Some(id) = key.strip_prefix("phonon_blocked_") {
                        if parse_name(id).is_ok_and(|n| n.id == id) { occupied.insert(id.to_string()); }
                    }
                }
            }
        }
        for r in &idx.voices {
            if !candidates.contains_key(&r.uid) && !aliases.get(&r.uid).is_some_and(|u| candidates.contains_key(u)) {
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
                }
            }
        }
        let mut rest = candidates.keys().filter(|uid| !assigned.contains_key(*uid)).cloned().collect::<Vec<_>>();
        rest.sort();
        for uid in rest {
            let (d, _, m, _) = candidates.get_mut(&uid).ok_or_else(fehler)?;
            for n in 2.. {
                let suffix = format!("-{n}");
                let base = m.id.chars().take(MAX_NAME_CHARS - suffix.len()).collect::<String>();
                let name = format!("{base}{suffix}");
                let parsed = parse_name(&name)?;
                if occupied.insert(parsed.id.to_ascii_lowercase()) && !self.name_blocked(a, d, &parsed.id)? {
                    assigned.insert(uid, name); break;
                }
            }
        }
        let mut voices = idx.voices.clone();
        let mut installs = Vec::new();
        let old_ids = idx.voices.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        for (uid, (d, updated, m, bytes)) in &candidates {
            let local = voices.iter().find(|r| r.uid == *uid || aliases.get(&r.uid) == Some(uid)).cloned();
            let name = &assigned[uid];
            let parsed = parse_name(name)?;
            let id = local.as_ref().filter(|r| r.id.eq_ignore_ascii_case(&parsed.id))
                .map(|r| r.id.clone()).unwrap_or(parsed.id);
            let mut r = Record { id, uid: uid.clone(), name: name.clone(), origin: m.origin.clone(),
                voice_id: m.voice_id.clone(),
                language: if m.language == "und" { String::new() } else { m.language.clone() },
                created_at: m.created.clone(), consent_at: m.consent_at.clone(),
                source_note: m.source_note.clone(), updated: *updated,
                sync_file: None, sync_linked: m.linked };
            r.sync_file = Some(Link { konto: kennung.clone(), file: d.id.clone(), sha256: m.sha256.clone(),
                voice: uid.clone(), updated: *updated, generation, snapshot: snapshot(&r, &m.sha256),
                name: m.name.clone(), display: r.name.clone() });
            if local.as_ref() == Some(&r) && sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str()) { continue; }
            write_atomic(&self.receive_path(uid), bytes)?;
            voices.retain(|v| v.uid != *uid && aliases.get(&v.uid) != Some(uid));
            voices.push(r.clone());
            installs.push(r);
        }
        self.write_index(&idx)?;
        if !installs.is_empty() {
            idx.sync_receive = Some(Projection { voices, installs, old_ids });
            self.write_index(&idx)?;
            self.finish_receive(&mut idx)?;
        }
        Ok(())
    }
}
