//! Ablage-Brücke unter der VoiceStore-Sperre. Keine Netzaufrufe und keine Inhaltslogs.
use super::*;
use crate::ablage::speicher::{Ablage, Eingang};
use crate::ablage::system;
use crate::ablage::zusammenfuehren::{Datei, Fassung, Ordner, Register};
use crate::sync::ablage::{konto_kennung, Konto, Zustand, SAMMLUNG_DATEI, SAMMLUNG_ORDNER};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Link {
    pub konto: String,
    pub file: String,
    pub sha256: String,
    pub voice: String,
    pub updated: i64,
}

#[derive(Clone, Serialize, Deserialize)]
struct Meta {
    id: String,
    name: String,
    language: String,
    origin: String,
    consent_at: Option<String>,
    sha256: String,
    generation: u32,
    nonce: u32,
}

fn fehler() -> VoiceError { VoiceError::Io("Stimmabgleich nicht abgeschlossen.".into()) }

fn meta(d: &Datei, key: &[u8; 32]) -> Option<(i64, Meta)> {
    let v = d.extra.get("phonon_voice")?.as_array()?;
    if v.len() != 2 { return None; }
    let rank = v[0].as_str()?;
    let updated = rank.parse::<i64>().ok()?;
    if updated < 0 || rank != format!("{updated:020}") { return None; }
    let m: Meta = serde_json::from_value(v[1].clone()).ok()?;
    let parsed = parse_name(&m.name).ok()?;
    if !parsed.id.eq_ignore_ascii_case(&m.id) || parsed.display != m.name || !origin_is_own(&m.origin)
        || !matches!(m.language.as_str(), "und" | "de" | "en" | "fr" | "es" | "pt")
        || m.sha256.len() != 64 || !m.sha256.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        || d.id != system::kennung(key, false, m.generation, &format!("{}:{}", m.sha256, m.nonce))
        || d.ordner.wert != system::kennung(key, true, m.generation, "")
        || d.fassungen.len() != 1 || !d.entfernt.is_empty() || !d.wahlen.is_empty()
        || d.papierkorb.wert.is_some()
    { return None; }
    let f = &d.fassungen[0];
    if f.typ != "safetensors" || f.blob != m.sha256 || f.original_sha256 != m.sha256
        || f.fas != d.id.replacen("dat_", "fas_", 1) || f.vorgaenger.is_some()
        || !(MIN_VOICE_BYTES..=MAX_VOICE_BYTES).contains(&f.size) || !f.texte.is_empty()
    { return None; }
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
    meta(d, key).is_some_and(|(_, m)| bytes.is_none_or(|b|
        pruefen(b).is_ok() && sha256_hex(b) == m.sha256))
}

fn zeit(r: &Record) -> i64 {
    if r.updated > 0 { r.updated } else {
        chrono::DateTime::parse_from_rfc3339(&r.created_at).map(|t| t.timestamp_millis().max(0)).unwrap_or(0)
    }
}

impl VoiceStore<'_> {
    pub(crate) fn sync_ablage(&self, a: &Ablage, konto: &Konto<'_>) -> Result<(), VoiceError> {
        let _g = self.guard();
        let kennung = konto_kennung(konto.user_id, konto.key, konto.hub_url);
        let mut idx = self.read_index()?;
        let mut b = a.sync_bestand().map_err(|_| fehler())?;
        let z = Zustand::laden(a);
        let begraben = |collection: &str, id: &str| {
            b.begraben.contains(id) || (z.konto == kennung && z.objekte
                .get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted))
        };
        // Write-ahead beim lokalen Entfernen. Auch ältere gleichnamige Konfliktverlierer begraben;
        // der kleine dauerhafte Beleg verhindert Wiederbelebung durch ein lange offlinees Gerät.
        let mut graeber = Vec::new();
        for link in idx.sync_pending.iter().filter(|l| l.konto == kennung) {
            graeber.push(Eingang::Grab(link.file.clone()));
            for d in b.dateien.values() {
                if let Some((updated, m)) = meta(d, konto.key) {
                    if m.id.eq_ignore_ascii_case(&link.voice) && updated <= link.updated {
                        graeber.push(Eingang::Grab(d.id.clone()));
                    }
                }
            }
        }
        // Ein Grabstein darf ausschließlich die unveränderte, genau dieser Datei zugeordnete Kopie
        // entfernen. Eine lokale Neuanlage hat keinen Link und bleibt immer erhalten.
        let weg = idx.voices.iter().filter(|r| r.sync_file.as_ref().is_some_and(|l|
            l.konto == kennung && begraben(SAMMLUNG_DATEI, &l.file)
                && sha256_file(&self.master_path(&r.id)).as_deref() == Some(l.sha256.as_str())))
            .map(|r| r.id.clone()).collect::<Vec<_>>();
        for id in weg { self.remove_locked(&id)?; }
        if !graeber.is_empty() { a.sync_einspielen(&graeber).map_err(|_| fehler())?; }
        idx = self.read_index()?;
        b = a.sync_bestand().map_err(|_| fehler())?;
        let tot = |collection: &str, id: &str| b.begraben.contains(id)
            || (z.konto == kennung && z.objekte.get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted));
        let mut generation = 0u32;
        while tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, generation, "")) {
            generation = generation.checked_add(1).ok_or_else(fehler)?;
        }
        let folder = system::kennung(konto.key, true, generation, "");
        // Vorhandene eigene Stimmen nachtragen. Gleicher Inhalt auf zwei PCs bekommt dieselbe
        // Datei- UND Fassungskennung; CAS kann deshalb keine doppelten Köpfe erzeugen.
        for r in idx.voices.clone() {
            if !origin_is_own(&r.origin) || !parse_name(&r.name).is_ok_and(|n| n.id.eq_ignore_ascii_case(&r.id)) { continue; }
            let bytes = match read_voice_file(&self.master_path(&r.id)) {
                Ok(bytes) if pruefen(&bytes).is_ok() => bytes,
                _ => continue,
            };
            let sha = sha256_hex(&bytes);
            // Wiederanlauf nach Datei-vor-Index beim Empfang: passen die neuen Bytes bereits zu
            // einem mindestens ebenso neuen entfernten Stand, unten dessen Import fertigstellen.
            // Das ist keine neue lokale Änderung und bekommt weder Kennung noch Zeitstempel neu.
            if r.sync_file.as_ref().is_some_and(|l| l.sha256 != sha)
                && b.dateien.values().any(|d| meta(d, konto.key).is_some_and(|(u, m)|
                    u >= zeit(&r) && m.id.eq_ignore_ascii_case(&r.id) && m.sha256 == sha)) { continue; }
            if r.sync_file.as_ref().is_some_and(|l| l.konto == kennung && l.sha256 == sha
                && b.dateien.get(&l.file).is_some_and(|d| !tot(SAMMLUNG_ORDNER, &d.ordner.wert))
                && !tot(SAMMLUNG_DATEI, &l.file)) { continue; }
            let language = if r.language.is_empty() { "und" } else { &r.language };
            if !matches!(language, "und" | "de" | "en" | "fr" | "es" | "pt") { continue; }
            let mut nonce = 0u32;
            let file = loop {
                let id = system::kennung(konto.key, false, generation, &format!("{sha}:{nonce}"));
                if !tot(SAMMLUNG_DATEI, &id) { break id; }
                nonce = nonce.checked_add(1).ok_or_else(fehler)?;
            };
            let updated = if r.sync_file.as_ref().is_some_and(|l| l.sha256 != sha) {
                chrono::Utc::now().timestamp_millis().max(zeit(&r).saturating_add(1))
            } else { zeit(&r) };
            // Ein älterer lokaler Konfliktverlierer wird nach Empfang des neueren Stands unten ersetzt.
            if idx.sync_pending.iter().any(|l| l.konto == kennung && l.voice.eq_ignore_ascii_case(&r.id)
                && updated <= l.updated) { continue; }
            let m = Meta { id: r.id.clone(), name: r.name.clone(), language: language.into(),
                origin: r.origin.clone(), consent_at: r.consent_at.clone(), sha256: sha.clone(), generation, nonce };
            let reg_s = |s: &str| Register::neu(s.to_string(), 1, "phonon-v1");
            let o = Ordner {
                id: folder.clone(), eltern: Register::neu(None, 1, "phonon-v1"), name: reg_s(".lv-stimmen"),
                datenklasse: reg_s("internal"), nur_geraet: Register::neu(false, 1, "phonon-v1"),
                zielordner_ki: Register::neu(false, 1, "phonon-v1"), papierkorb: Register::neu(None, 1, "phonon-v1"),
                extra: BTreeMap::new(),
            };
            let fas = file.replacen("dat_", "fas_", 1);
            let f = Fassung { fas: fas.clone(), vorgaenger: None, blob: sha.clone(), size: bytes.len() as u64,
                original_sha256: sha.clone(), typ: "safetensors".into(), geraet: "phonon-v1".into(),
                zeit: 0, texte: vec![] };
            let d = Datei {
                id: file.clone(), ordner: reg_s(&folder), name: reg_s("voice.safetensors"),
                klasse_min: reg_s("internal"), nur_geraet_min: false, fassungen: vec![f], wahlen: vec![],
                entfernt: vec![], blob_refs: vec![sha.clone()], papierkorb: Register::neu(None, 1, "phonon-v1"),
                extra: BTreeMap::from([("phonon_voice".into(), json!([format!("{updated:020}"), m]))]),
            };
            a.sync_einspielen(&[Eingang::Ordner(o), Eingang::Datei(d)]).map_err(|_| fehler())?;
            let tmp = a.sync_zwischenpfad().map_err(|_| fehler())?;
            crate::ablage::schreibe_atomar(&tmp, &bytes).map_err(|_| fehler())?;
            a.original_einlegen(&file, &fas, &tmp).map_err(|_| fehler())?;
            if let Some(local) = idx.voices.iter_mut().find(|x| x.id == r.id) {
                local.sync_file = Some(Link { konto: kennung.clone(), file, sha256: sha, voice: r.id, updated });
                local.updated = updated;
            }
            self.write_index(&idx)?;
        }
        b = a.sync_bestand().map_err(|_| fehler())?;
        let mut kandidaten = BTreeMap::<String, (i64, String, Meta, Vec<u8>)>::new();
        for d in b.dateien.values().filter(|d| system::ist_datei(&d.id)) {
            let fingerprint = sync_fingerprint(d);
            if idx.sync_rejected.contains(&fingerprint) { continue; }
            let Some((updated, m)) = meta(d, konto.key) else {
                idx.sync_rejected.insert(fingerprint);
                continue;
            };
            if b.begraben.contains(&d.id) || b.begraben.contains(&d.ordner.wert)
                || (z.konto == kennung && z.objekte.get(&format!("{SAMMLUNG_ORDNER}/{}", d.ordner.wert))
                    .is_some_and(|o| o.deleted))
                || b.ordner.get(&d.ordner.wert).is_none_or(|o| o.papierkorb.wert.is_some())
                || idx.sync_pending.iter().any(|l| l.konto == kennung
                    && l.voice.eq_ignore_ascii_case(&m.id) && updated <= l.updated) { continue; }
            let f = &d.fassungen[0];
            let Some(path) = a.original_quelle(&d.id, &f.fas).map_err(|_| fehler())? else { continue; };
            let bytes = match read_voice_file(&path) {
                Ok(bytes) if pruefen(&bytes).is_ok() && sha256_hex(&bytes) == m.sha256 => bytes,
                _ => { idx.sync_rejected.insert(fingerprint); continue; }
            };
            let candidate = (updated, d.id.clone(), m.clone(), bytes);
            let key = m.id.to_ascii_lowercase();
            if kandidaten.get(&key).is_none_or(|c| (c.0, &c.1) < (updated, &d.id)) {
                kandidaten.insert(key, candidate);
            }
        }
        self.write_index(&idx)?;
        for (_, (updated, file, m, bytes)) in kandidaten {
            idx = self.read_index()?;
            let local = idx.voices.iter().find(|r| r.id.eq_ignore_ascii_case(&m.id))
                .or_else(|| idx.voices.iter().find(|r| sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str())));
            let old_id = local.filter(|r| !r.id.eq_ignore_ascii_case(&m.id)).map(|r| r.id.clone());
            if let Some(r) = local {
                if zeit(r) > updated || (zeit(r) == updated && r.sync_file.as_ref()
                    .is_some_and(|l| l.konto == kennung && l.file > file)) { continue; }
                if r.sync_file.as_ref().is_some_and(|l| l.konto == kennung && l.file == file
                    && l.sha256 == m.sha256 && l.updated == updated)
                    && r.name == m.name && r.origin == m.origin && r.consent_at == m.consent_at
                    && r.language == (if m.language == "und" { "" } else { m.language.as_str() })
                    && sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str()) { continue; }
            }
            // add_bound ist der bestehende Importweg inklusive Kapazität und Modellschutz;
            // Herkunftsbeleg und Eintrag werden zusammen in stimmen.json geschrieben.
            let pending = idx.sync_pending.clone();
            let result = self.add_bound(NewVoice { name: m.name, bytes, origin: if m.origin == ORIGIN_GRADIUM {
                ORIGIN_GRADIUM } else { ORIGIN_IMPORT }, voice_id: String::new(),
                language: if m.language == "und" { String::new() } else { m.language },
                consent_at: m.consent_at, source_note: None }, local.is_some() && old_id.is_none(),
                Some(Link { konto: kennung.clone(), file: file.clone(), sha256: m.sha256.clone(), voice: m.id.clone(), updated }));
            match result {
                Ok(_) => {}
                Err(VoiceError::TakenByModel { .. } | VoiceError::InvalidName(_) | VoiceError::InvalidFile(_)) => {
                    if let Some(d) = b.dateien.get(&file) { idx.sync_rejected.insert(sync_fingerprint(d)); }
                    self.write_index(&idx)?;
                    continue;
                }
                Err(_) => continue,
            }
            if let Some(old) = old_id { self.remove_bound(&old, false)?; }
            idx = self.read_index()?;
            idx.sync_pending = pending;
            if let Some(r) = idx.voices.iter_mut().find(|r| r.id.eq_ignore_ascii_case(&m.id)) {
                r.updated = updated;
                r.sync_file = Some(Link { konto: kennung.clone(), file, sha256: m.sha256, voice: r.id.clone(), updated });
            }
            self.write_index(&idx)?;
        }
        // Abbruch nach dem neuen Namen, vor dem Entfernen des alten: der passende neue Link oben
        // darf das Aufräumen nicht dauerhaft überspringen. Nur unveränderte Kopien derselben Datei.
        idx = self.read_index()?;
        let mut doppelt = Vec::new();
        for d in b.dateien.values() {
            let Some((updated, m)) = meta(d, konto.key) else { continue; };
            let fertig = idx.voices.iter().any(|r| r.id.eq_ignore_ascii_case(&m.id) && r.sync_file.as_ref().is_some_and(|l|
                l.konto == kennung && l.file == d.id && l.updated == updated)
                && sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str()));
            if !fertig { continue; }
            doppelt.extend(idx.voices.iter().filter(|r| !r.id.eq_ignore_ascii_case(&m.id) && r.sync_file.as_ref().is_some_and(|l|
                l.konto == kennung && l.file == d.id && l.sha256 == m.sha256 && zeit(r) <= updated)
                && sha256_file(&self.master_path(&r.id)).as_deref() == Some(m.sha256.as_str()))
                .map(|r| r.id.clone()));
        }
        doppelt.sort();
        doppelt.dedup();
        for id in doppelt { self.remove_bound(&id, false)?; }
        // Nach einem Ordnergrab erst die neue Generation vollständig anlegen, dann alte Dateien
        // begraben. Aus dem Bestand abgeleitet, damit ein Abbruch nach dem Indexschreiben heilbar ist.
        idx = self.read_index()?;
        let alte = b.dateien.values().filter(|d| system::ist_datei(&d.id))
            .filter(|d| z.konto == kennung && z.objekte.get(&format!("{SAMMLUNG_ORDNER}/{}", d.ordner.wert))
                .is_some_and(|o| o.deleted))
            .filter(|d| meta(d, konto.key).is_some_and(|(_, m)| idx.voices.iter().any(|r|
                r.sync_file.as_ref().is_some_and(|l| l.konto == kennung && l.sha256 == m.sha256 && l.file != d.id))))
            .map(|d| Eingang::Grab(d.id.clone())).collect::<Vec<_>>();
        if !alte.is_empty() { a.sync_einspielen(&alte).map_err(|_| fehler())?; }
        Ok(())
    }
}
