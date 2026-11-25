//! S1: nur synthetische Stimmen, derselbe Hub-Nachbau wie beim Ablage-Abgleich.
use super::*;
use crate::managers::tts::phonon_voices::{sample_voice_bytes, NewVoice, VoiceStore, ORIGIN_IMPORT};

fn neu(seed: u32) -> NewVoice {
    NewVoice { name: "Probe".into(), bytes: sample_voice_bytes(seed), origin: ORIGIN_IMPORT,
        voice_id: String::new(), language: "de".into(), consent_at: None, source_note: None }
}

fn abgleich(g: &Geraet, s: &VoiceStore<'_>) {
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &g.id, hub_url: HUB_URL };
    crate::sync::phonon::abgleichen(&g.a, s, &konto).unwrap();
}

fn stimmen(g: &Geraet) -> Vec<Datei> {
    g.bestand().dateien.into_values().filter(|d| crate::ablage::system::ist_datei(&d.id)).collect()
}

#[test]
fn stimmen_hochziehen_verbergen_entfernen_und_neu_anlegen() {
    let hub = FakeHub::neu();
    let a = geraet("stimme-a");
    let b = geraet("stimme-b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    let sb = VoiceStore { root: &rb, targets: &[], lock: &lb };
    let new = |seed| NewVoice {
        name: "Probe".into(), bytes: sample_voice_bytes(seed), origin: ORIGIN_IMPORT,
        voice_id: String::new(), language: "de".into(), consent_at: None, source_note: None,
    };
    sa.add(new(1), false).unwrap();
    assert!(a.bestand().ordner.keys().all(|id| !crate::ablage::system::ist_ordner(id)),
        "ein lokaler Import allein legt keinen Sync-Systemordner an");
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &a.id, hub_url: HUB_URL };
    crate::sync::phonon::abgleichen(&a.a, &sa, &konto).unwrap();
    a.sync(&hub);
    b.sync(&hub);
    crate::sync::phonon::abgleichen(&b.a, &sb, &konto).unwrap();
    assert_eq!(sb.list().len(), 1);
    let voice = a.bestand().dateien.values().find(|d| crate::ablage::system::ist_datei(&d.id)).unwrap().clone();
    let folder = voice.ordner.wert.clone();
    let normal = b.ordner(".lv-stimmen");
    let sicht = b.a.ansicht().unwrap();
    assert!(sicht.ordner.iter().any(|o| o.id == normal));
    assert!(!sicht.ordner.iter().any(|o| o.id == folder));
    assert!(!sicht.dateien.iter().any(|d| d.id == voice.id));
    assert!(!b.a.ki_bestand().unwrap().dateien.contains_key(&voice.id));
    assert!(b.a.index_quellen().unwrap().iter().all(|d| d.dat != voice.id));
    assert!(b.a.original_pfad(&voice.id).is_err());
    assert!(b.a.datei_umbenennen(&voice.id, "Andere").is_err());
    assert!(b.a.datei_verschieben(&voice.id, &normal).is_err());
    assert!(b.a.ordner_umbenennen(&folder, "Andere").is_err());
    assert!(b.a.in_papierkorb(AblageArt::Datei, &voice.id).is_err());
    assert!(b.a.in_papierkorb(AblageArt::Ordner, &folder).is_err());
    assert!(b.a.endgueltig_loeschen(AblageArt::Datei, &voice.id).is_err());
    assert!(b.a.endgueltig_loeschen(AblageArt::Ordner, &folder).is_err());
    assert!(b.a.wiederherstellen(AblageArt::Datei, &voice.id).is_err());
    assert!(b.a.fassungen(&voice.id).is_err());
    assert!(b.a.ordner_anlegen(Some(&folder), "Unterordner").is_err());
    assert!(b.a.ordner_verschieben(&folder, Some(&normal)).is_err());
    assert!(b.a.ordner_verschieben(&normal, Some(&folder)).is_err());
    assert!(b.a.ki_setzen(&folder, Some(crate::ablage::freigabe::KiStufe::An)).is_err());
    assert!(b.a.text_auftraege().unwrap().is_empty());
    assert!(b.a.treffer_infos(&[voice.id.clone()]).unwrap().is_empty());
    assert_eq!(b.a.ansicht().unwrap().belegt, 0, "Systembytes nicht als Dokumentenspeicher zählen");
    assert!(b.bestand().dateien.contains_key(&voice.id), "normales Sichern erhält Systemdaten");
    sa.remove("Probe").unwrap();
    crate::sync::phonon::abgleichen(&a.a, &sa, &konto).unwrap();
    a.sync(&hub);
    b.sync(&hub);
    crate::sync::phonon::abgleichen(&b.a, &sb, &konto).unwrap();
    assert!(sb.list().is_empty());
    sa.add(new(1), false).unwrap();
    crate::sync::phonon::abgleichen(&a.a, &sa, &konto).unwrap();
    assert!(a.bestand().dateien.keys().any(|id| crate::ablage::system::ist_datei(id) && *id != voice.id));
}

#[test]
fn erster_sync_zweier_pcs_dedupliziert_inhalt_und_cas() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    let sb = VoiceStore { root: &rb, targets: &[], lock: &lb };
    sa.add(neu(1), false).unwrap();
    sb.add(neu(1), false).unwrap();
    // Beide legen vor dem ersten Pull unabhängig ihren Bestand an (identische IDs).
    abgleich(&a, &sa);
    abgleich(&b, &sb);
    assert_eq!(stimmen(&a)[0].id, stimmen(&b)[0].id);
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    abgleich(&a, &sa);
    abgleich(&b, &sb);
    assert_eq!(stimmen(&a).len(), 1);
    assert_eq!(zf::koepfe(&stimmen(&a)[0]).len(), 1);
    assert_eq!(sa.list().len(), 1);
    assert_eq!(sb.list().len(), 1);
    let vorher = hub.mit(|i| i.mitschnitt.len());
    abgleich(&a, &sa);
    a.sync(&hub);
    assert_eq!(hub.mit(|i| i.mitschnitt.len()), vorher, "kein Rückupload gezogener Stimmen");
    assert_eq!(hub.mit(|i| i.objekte.iter().filter(|((c, _), o)| c == SAMMLUNG_ORDNER && !o.deleted).count()), 2,
        "Allgemein und genau ein Systemordner");
}

#[test]
fn namenskonflikt_gewinnt_nach_updated_und_neuanlage_ueberlebt_alten_grabstein() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    let sb = VoiceStore { root: &rb, targets: &[], lock: &lb };
    sa.add(neu(1), false).unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    // Der Importweg vergibt auch bei gleicher Millisekunde einen höheren updated-Wert.
    sb.add(neu(2), true).unwrap();
    abgleich(&b, &sb);
    b.sync(&hub);
    a.sync(&hub);
    abgleich(&a, &sa);
    assert_eq!(std::fs::read(ra.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(2));
    sa.remove("Probe").unwrap();
    // Lokale Neuanlage gleichen Namens vor Empfang des alten Grabsteins.
    sb.remove("Probe").unwrap();
    sb.add(neu(3), false).unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(std::fs::read(rb.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(3));
}

#[test]
fn kaputte_stimme_wird_einmal_verworfen_ohne_grabstein_und_dokumente_laufen_weiter() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let sa = VoiceStore { root: &root, targets: &[], lock: &lock };
    sa.add(neu(1), false).unwrap();
    abgleich(&a, &sa);
    let mut d = stimmen(&a)[0].clone();
    // Gültiger Blob samt passendem SHA, aber kein safetensors-Kopf (synthetisch).
    let bytes = vec![0u8; 256_136];
    let sha = blobs::sha256_hex(&bytes);
    let key = schluessel_konto();
    d.id = crate::ablage::system::kennung(&key, false, 0, &format!("{sha}:0"));
    d.fassungen[0].fas = d.id.replacen("dat_", "fas_", 1);
    d.fassungen[0].blob = sha.clone();
    d.fassungen[0].original_sha256 = sha.clone();
    d.blob_refs = vec![sha.clone()];
    d.extra.get_mut("phonon_voice").unwrap()[1]["sha256"] = serde_json::json!(sha);
    a.a.sync_einspielen(&[Eingang::Datei(d.clone())]).unwrap();
    let tmp = a.a.sync_zwischenpfad().unwrap();
    std::fs::write(&tmp, &bytes).unwrap();
    a.a.original_einlegen(&d.id, &d.fassungen[0].fas, &tmp).unwrap();
    let ord = a.ordner("Dokumente");
    let doc = a.ablegen("probe.txt", "Synthetische Gegenprobe", &ord);
    a.sync(&hub);
    b.sync(&hub);
    let z = b.zustand();
    assert_eq!(z.phonon_verworfen.len(), 1);
    assert!(!b.bestand().originale.contains(&(d.id.clone(), d.fassungen[0].fas.clone())));
    assert!(!b.bestand().begraben.contains(&d.id));
    assert_eq!(b.original(&doc).as_deref(), Some("Synthetische Gegenprobe"));
    b.sync(&hub);
    assert_eq!(b.zustand().phonon_verworfen, z.phonon_verworfen);
    assert!(!hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().deleted);
}

#[test]
fn begrabener_systemordner_bekommt_neue_generation_und_nie_wieder_die_alte_id() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    a.sync(&hub);
    let old = stimmen(&a)[0].ordner.wert.clone();
    let revision = hub.objekt(SAMMLUNG_ORDNER, &old).unwrap().revision;
    tauri::async_runtime::block_on(hub.push("alt", &[grab_von_anderem_geraet(SAMMLUNG_ORDNER, &old, revision)])).unwrap();
    a.sync(&hub);
    abgleich(&a, &store);
    a.sync(&hub);
    let key = schluessel_konto();
    let next = crate::ablage::system::kennung(&key, true, 1, "");
    assert!(hub.objekt(SAMMLUNG_ORDNER, &old).unwrap().deleted);
    assert!(a.bestand().ordner.contains_key(&next));
    assert!(stimmen(&a).iter().any(|d| d.ordner.wert == next));
    assert!(a.a.ansicht().unwrap().ordner.iter().all(|o| o.id != old && o.id != next));
}

#[test]
fn importpruefung_hash_sprache_pfad_windowsname_und_modellschutz() {
    use crate::managers::tts::phonon_voices::{sync_datei_gueltig, ModelTarget};
    let a = geraet("a");
    let b = geraet("b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    sa.add(neu(1), false).unwrap();
    abgleich(&a, &sa);
    let d = stimmen(&a)[0].clone();
    let key = schluessel_konto();
    assert!(sync_datei_gueltig(&d, &key, Some(&sample_voice_bytes(1))));
    assert!(!sync_datei_gueltig(&d, &key, Some(&sample_voice_bytes(2))), "falscher SHA");
    // Gültiges safetensors, aber F16 statt F32: der allgemeine Import war bisher großzügiger.
    let header = br#"{"speaker_wavs":{"dtype":"F16","shape":[1,512,125],"data_offsets":[0,128000]}}"#;
    let mut f16 = (header.len() as u64).to_le_bytes().to_vec();
    f16.extend(header);
    f16.extend(vec![0u8; 128000]);
    assert!(crate::managers::tts::phonon_voices::validate_voice_bytes(&f16).is_ok());
    let sha = blobs::sha256_hex(&f16);
    let mut wrong = d.clone();
    wrong.id = crate::ablage::system::kennung(&key, false, 0, &format!("{sha}:0"));
    wrong.fassungen[0].fas = wrong.id.replacen("dat_", "fas_", 1);
    wrong.fassungen[0].blob = sha.clone();
    wrong.fassungen[0].original_sha256 = sha.clone();
    wrong.fassungen[0].size = f16.len() as u64;
    wrong.extra.get_mut("phonon_voice").unwrap()[1]["sha256"] = serde_json::json!(sha);
    assert!(sync_datei_gueltig(&wrong, &key, None));
    assert!(!sync_datei_gueltig(&wrong, &key, Some(&f16)), "strenge Tensorprüfung");
    for (field, value) in [("id", "../Probe"), ("name", "NUL"), ("language", "xx"), ("origin", "model")] {
        let mut kaputt = d.clone();
        kaputt.extra.get_mut("phonon_voice").unwrap()[1][field] = serde_json::json!(value);
        assert!(!sync_datei_gueltig(&kaputt, &key, None), "Metadatenprüfung");
    }
    let dir = b._t.path().join("model");
    std::fs::create_dir_all(dir.join("voices")).unwrap();
    std::fs::write(dir.join("voices/Probe.safetensors"), sample_voice_bytes(9)).unwrap();
    let targets = [ModelTarget { id: "synthetisch".into(), dir: dir.clone() }];
    let sb = VoiceStore { root: &rb, targets: &targets, lock: &lb };
    // Der vorhandene Importweg muss die mitgelieferte Modellstimme auch beim Pull schützen.
    b.a.sync_einspielen(&[Eingang::Ordner(a.bestand().ordner[&d.ordner.wert].clone()), Eingang::Datei(d.clone())]).unwrap();
    let tmp = b.a.sync_zwischenpfad().unwrap();
    std::fs::write(&tmp, sample_voice_bytes(1)).unwrap();
    b.a.original_einlegen(&d.id, &d.fassungen[0].fas, &tmp).unwrap();
    abgleich(&b, &sb);
    assert!(sb.list().is_empty());
    assert_eq!(std::fs::read(dir.join("voices/Probe.safetensors")).unwrap(), sample_voice_bytes(9));
    assert!(!b.bestand().begraben.contains(&d.id));
}

#[test]
fn wiederanlauf_nach_datei_vor_index_erzeugt_keinen_rueckupload() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    let sb = VoiceStore { root: &rb, targets: &[], lock: &lb };
    sa.add(neu(1), false).unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    sa.add(neu(2), true).unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    let before = stimmen(&b);
    // Abbruch im bestehenden Importweg: die neuen Bytes liegen, stimmen.json ist noch alt.
    std::fs::write(rb.join("eigene-stimmen/Probe.safetensors"), sample_voice_bytes(2)).unwrap();
    abgleich(&b, &sb);
    abgleich(&b, &sb);
    assert_eq!(stimmen(&b), before, "keine neue Datei und kein künstlich neuer updated-Wert");
    assert_eq!(std::fs::read(rb.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(2));
}

#[test]
fn wiederanlauf_raeuemt_alten_namen_auch_bei_bereits_gebundenem_neuem_namen_weg() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let path = root.join("eigene-stimmen/stimmen.json");
    let mut idx: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut alt = idx["voices"][0].clone();
    alt["id"] = serde_json::json!("Alt");
    alt["name"] = serde_json::json!("Alt");
    alt["updated"] = serde_json::json!(idx["voices"][0]["updated"].as_i64().unwrap() - 1);
    alt["sync_file"]["voice"] = serde_json::json!("Alt");
    idx["voices"].as_array_mut().unwrap().push(alt);
    std::fs::write(&path, serde_json::to_vec(&idx).unwrap()).unwrap();
    std::fs::write(root.join("eigene-stimmen/Alt.safetensors"), sample_voice_bytes(1)).unwrap();
    abgleich(&a, &store);
    assert_eq!(store.list().len(), 1);
    assert!(!root.join("eigene-stimmen/Alt.safetensors").exists());
    assert!(a.bestand().begraben.is_empty(), "Aufräumen gleicher Inhalte ist kein Entfernen der Sync-Datei");
}

#[test]
fn inhaltsgleicher_namenswechsel_geht_auch_an_der_kapazitaetsgrenze() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let la = Mutex::new(());
    let lb = Mutex::new(());
    let ra = a._t.path().join("phonon");
    let rb = b._t.path().join("phonon");
    let sa = VoiceStore { root: &ra, targets: &[], lock: &la };
    let sb = VoiceStore { root: &rb, targets: &[], lock: &lb };
    sb.add(neu(1), false).unwrap();
    let path = rb.join("eigene-stimmen/stimmen.json");
    let mut idx: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    idx["voices"][0]["updated"] = serde_json::json!(1);
    let template = idx["voices"][0].clone();
    // Die übrigen 199 Einträge brauchen für die Kapazitätsprüfung keine Blob-Dateien.
    for i in 1..200 {
        let mut r = template.clone();
        r["id"] = serde_json::json!(format!("Dummy{i}"));
        r["name"] = r["id"].clone();
        idx["voices"].as_array_mut().unwrap().push(r);
    }
    std::fs::write(&path, serde_json::to_vec(&idx).unwrap()).unwrap();
    let mut remote = neu(1);
    remote.name = "Neu".into();
    sa.add(remote, false).unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(sb.list().len(), 200);
    assert!(sb.list().iter().any(|r| r.id == "Neu"));
    assert!(sb.list().iter().all(|r| r.id != "Probe"));
}

#[test]
fn benutzerlesen_beendet_system_loeschjournal_vor_der_projektion() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let system = stimmen(&a)[0].id.clone();
    let folder = a.ordner("Dokumente");
    let doc = a.ablegen("probe.txt", "Synthetisch", &folder);
    let path = a.a.dir().join("ablage.json");
    let mut st: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    st["journal"].as_array_mut().unwrap().push(serde_json::json!({
        "art": "loeschen", "dat": [system.clone()], "ordner": [], "aufheben": true
    }));
    std::fs::write(&path, serde_json::to_vec(&st).unwrap()).unwrap();
    assert!(!a.a.fassungen(&doc).unwrap().is_empty());
    let st: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(st["dateien"].get(&system).is_none(), "kein Wiederbeleben aus Stand.system");
    assert!(st["journal"].as_array().unwrap().is_empty(), "auch lesender Aufruf sichert den Wiederanlauf");
    assert!(st["begraben"].as_array().unwrap().contains(&serde_json::json!(system)));
}

#[test]
fn gleiche_datei_und_zeit_uebernimmt_gewinnende_metadaten_ohne_den_namen_zu_loeschen() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let mut d = stimmen(&a)[0].clone();
    // Kleines p gewinnt bei gleichem Rang im vorhandenen kanonischen Extra-Merge.
    d.extra.get_mut("phonon_voice").unwrap()[1]["name"] = serde_json::json!("probe");
    a.a.sync_einspielen(&[Eingang::Datei(d)]).unwrap();
    abgleich(&a, &store);
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].name, "probe");
    assert!(root.join("eigene-stimmen/Probe.safetensors").is_file());
    assert!(a.bestand().begraben.is_empty());
}
