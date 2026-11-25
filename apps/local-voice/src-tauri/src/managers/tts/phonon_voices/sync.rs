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
    uid: String,
    name: String,
    language: String,
    origin: String,
    consent_at: Option<String>,
    sha256: String,
    generation: u32,
    nonce: u32,
}

fn fehler() -> VoiceError { VoiceError::Io("Stimmabgleich nicht abgeschlossen.".into()) }

pub(super) fn neue_stimmkennung() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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
        || m.uid.len() != 32 || !m.uid.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        || d.id != system::kennung(key, false, m.generation, &format!("{}:{updated}:{}", m.uid, m.nonce))
        || d.ordner.wert != system::kennung(key, true, m.generation, "")
        || d.fassungen.len() != 1 || !d.entfernt.is_empty() || !d.wahlen.is_empty()
        || d.papierkorb.wert.is_some()
    { return None; }
    let f = &d.fassungen[0];
    if f.typ != "lvstimme" || f.blob != m.sha256 || f.original_sha256 != m.sha256
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
        let mut z = Zustand::laden(a);
        let begraben = |collection: &str, id: &str| {
            b.begraben.contains(id) || (z.konto == kennung && z.objekte
                .get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted))
        };
        // Ein Beleg gilt nur für die Stimmkennung und höchstens den entfernten Stand.
        // Auch die ursprünglich verknüpfte Datei muss diese Prüfung bestehen.
        let mut graeber = Vec::new();
        let mut guards = BTreeMap::new();
        for link in idx.sync_pending.iter().filter(|l| l.konto == kennung) {
            for d in b.dateien.values() {
                if let Some((updated, m)) = meta(d, konto.key) {
                    if m.uid == link.voice && updated <= link.updated {
                        graeber.push(Eingang::Grab(d.id.clone()));
                        guards.insert(d.id.clone(), crate::sync::ablage::hash_datei(d));
                    }
                }
            }
        }
        // Ein Grabstein darf ausschließlich die unveränderte, genau dieser Datei zugeordnete Kopie
        // entfernen. Eine lokale Neuanlage hat keinen Link und bleibt immer erhalten.
        let weg = idx.voices.iter().filter(|r| r.sync_file.as_ref().is_some_and(|l|
            l.konto == kennung && begraben(SAMMLUNG_DATEI, &l.file)
                && r.uid == l.voice && zeit(r) == l.updated
                && sha256_file(&self.master_path(&r.id)).as_deref() == Some(l.sha256.as_str())))
            .map(|r| r.id.clone()).collect::<Vec<_>>();
        for id in weg { self.remove_received(&id)?; }
        if !graeber.is_empty() {
            // Write-ahead: der Push prüft nach Pull und nach jedem CAS-Konflikt denselben Stand.
            if z.konto != kennung { z = Zustand { konto: kennung.clone(), ..Zustand::default() }; }
            z.phonon_graeber.extend(guards);
            z.speichern(a).map_err(|_| fehler())?;
            a.sync_einspielen(&graeber).map_err(|_| fehler())?;
        }
        idx = self.read_index()?;
        b = a.sync_bestand().map_err(|_| fehler())?;
        let tot = |collection: &str, id: &str| b.begraben.contains(id)
            || (z.konto == kennung && z.objekte.get(&format!("{collection}/{id}")).is_some_and(|o| o.deleted));
        let mut generation = 0u32;
        while tot(SAMMLUNG_ORDNER, &system::kennung(konto.key, true, generation, "")) {
            generation = generation.checked_add(1).ok_or_else(fehler)?;
        }
        let folder = system::kennung(konto.key, true, generation, "");
        // Jede Stimme und jeder Stand hat eine inhaltsunabhängige Datei. Ein alter Grabstein
        // kann dadurch auch bei einem Rennen keinen späteren Stand derselben Stimme löschen.
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
                    u >= zeit(&r) && m.uid == r.uid && m.sha256 == sha)) { continue; }
            if r.sync_file.as_ref().is_some_and(|l| l.konto == kennung && l.sha256 == sha
                && l.updated == zeit(&r) && l.voice == r.uid
                && b.dateien.get(&l.file).is_some_and(|d| !tot(SAMMLUNG_ORDNER, &d.ordner.wert))
                && !tot(SAMMLUNG_DATEI, &l.file)) { continue; }
            let language = if r.language.is_empty() { "und" } else { &r.language };
            if !matches!(language, "und" | "de" | "en" | "fr" | "es" | "pt") { continue; }
            let updated = if r.sync_file.as_ref().is_some_and(|l| l.sha256 != sha) {
                chrono::Utc::now().timestamp_millis().max(zeit(&r).saturating_add(1))
            } else { zeit(&r) };
            // Ein älterer lokaler Konfliktverlierer wird nach Empfang des neueren Stands unten ersetzt.
            if idx.sync_pending.iter().any(|l| l.konto == kennung && l.voice == r.uid
                && updated <= l.updated) { continue; }
            let mut nonce = 0u32;
            let file = loop {
                let id = system::kennung(konto.key, false, generation, &format!("{}:{updated}:{nonce}", r.uid));
                if !tot(SAMMLUNG_DATEI, &id) { break id; }
                nonce = nonce.checked_add(1).ok_or_else(fehler)?;
            };
            let m = Meta { id: r.id.clone(), uid: r.uid.clone(), name: r.name.clone(), language: language.into(),
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
                original_sha256: sha.clone(), typ: "lvstimme".into(), geraet: "phonon-v1".into(),
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
                local.sync_file = Some(Link { konto: kennung.clone(), file, sha256: sha, voice: r.uid, updated });
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
                    && l.voice == m.uid && updated <= l.updated) { continue; }
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
            let local = idx.voices.iter().find(|r| r.id.eq_ignore_ascii_case(&m.id));
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
            let result = self.add_bound(NewVoice { name: m.name, bytes, origin: if m.origin == ORIGIN_GRADIUM {
                ORIGIN_GRADIUM } else { ORIGIN_IMPORT }, voice_id: String::new(),
                language: if m.language == "und" { String::new() } else { m.language },
                consent_at: m.consent_at, source_note: None }, local.is_some(),
                Some(Link { konto: kennung.clone(), file: file.clone(), sha256: m.sha256.clone(), voice: m.uid.clone(), updated }));
            match result {
                Ok(_) => {}
                Err(VoiceError::TakenByModel { .. } | VoiceError::InvalidName(_) | VoiceError::InvalidFile(_)) => {
                    if let Some(d) = b.dateien.get(&file) { idx.sync_rejected.insert(sync_fingerprint(d)); }
                    self.write_index(&idx)?;
                    continue;
                }
                Err(_) => continue,
            }
        }
        // Unabhängige Importe sind nur bei gleichem Namen UND gleichem Inhalt dieselbe Stimme.
        // Beim Nutzerlöschen bekommt jede beobachtete Kennung ihren eigenen, begrenzten Beleg.
        idx = self.read_index()?;
        for r in &mut idx.voices {
            let Some(sha) = sha256_file(&self.master_path(&r.id)) else { continue; };
            for d in b.dateien.values() {
                let Some((updated, m)) = meta(d, konto.key) else { continue; };
                if m.id.eq_ignore_ascii_case(&r.id) && m.sha256 == sha && m.uid != r.uid {
                    let link = Link { konto: kennung.clone(), file: d.id.clone(), sha256: sha.clone(),
                        voice: m.uid, updated };
                    if !r.sync_equivalent.contains(&link) { r.sync_equivalent.push(link); }
                }
            }
        }
        self.write_index(&idx)?;
        // Ein Ordnergrab erzeugt keine Dateigräber. Sonst könnte ein noch nicht migriertes
        // Gerät den technischen Generationswechsel mit einer Nutzerlöschung verwechseln.
        Ok(())
    }
}
