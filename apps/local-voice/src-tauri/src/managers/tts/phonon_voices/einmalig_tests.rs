use super::*;

#[test]
fn trockenlauf_listet_nur_metadaten_und_beide_emb_fassungen() {
    let bytes = sample_voice_bytes(17);
    let r: Record = serde_json::from_value(json!({
        "id": "Test", "uid": "00112233445566778899aabbccddeeff", "name": "Test",
        "origin": "import", "language": "de", "created_at": "2026-11-26T12:00:00Z"
    })).unwrap();
    let key = [7; 32];
    let konto = Konto { key: &key, user_id: 1, device_id: "test", hub_url: "https://invalid.test" };
    let mut d = voice_sync::stimmdatei(&r, &bytes, &konto, 0, None).unwrap();
    let mut w = vec![0.; 512 * 512];
    for i in 0..512 { w[i * 512 + i] = 1.; }
    let emb = embedding::encode(&bytes, &w).unwrap();
    let entries = crate::catalog::tts_entries(crate::catalog::Purpose::PhononModel);
    for id in ["phonon-40m-de", "phonon-31m-en"] {
        let entry = entries.iter().find(|e| e.id == id).unwrap();
        voice_sync::emb_eintragen(&mut d, entry, &sha256_hex(&bytes), zeit(&r), &emb).unwrap();
    }
    let zeile = listenzeile(&r, &bytes, &d).unwrap();
    assert_eq!(zeile.name, "Test");
    assert_eq!(zeile.sprache, "de");
    assert_eq!(zeile.sha256, &sha256_hex(&bytes)[..8]);
    assert_eq!(zeile.emb.len(), 2);
    assert!(zeile.emb.iter().all(|(_, size)| *size == emb.len()));
    assert_eq!(zeile.objekt_bytes, crate::sync::ablage::stimmobjekt_groesse(&d));
    let aus = zeile.ausgabe();
    assert!(!aus.contains("data"));
    assert!(!aus.contains("source_note"));
    assert!(!aus.contains(&sha256_hex(&bytes)));
    // Gegenprobe zum App-Vertrag: eine neue Modellrevision behält den vorhandenen Rang.
    let mut alt = d.clone();
    alt.extra.insert("unbekannt".into(), json!(["bleibt"]));
    alt.extra.get_mut("phonon_emb_phonon-40m-de").unwrap()[0] = json!("00009000000000000000");
    alt.extra.get_mut("phonon_emb_phonon-40m-de").unwrap()[2]["revision"] = json!("0000");
    voice_sync::emb_uebernehmen(&mut alt, &d).unwrap();
    assert_eq!(alt.extra["phonon_emb_phonon-40m-de"][0], "00009000000000000000");
    assert_eq!(alt.extra["phonon_emb_phonon-40m-de"][2], d.extra["phonon_emb_phonon-40m-de"][2]);
    assert_eq!(alt.extra["unbekannt"], json!(["bleibt"]));
    // Gegenprobe: fehlende oder zum falschen Master gehörende emb darf nicht als fertig gelten.
    d.extra.remove("phonon_emb_phonon-31m-en");
    assert!(listenzeile(&r, &bytes, &d).is_err());
    d.extra.get_mut("phonon_emb_phonon-40m-de").unwrap()[1] = json!("falsch");
    assert!(listenzeile(&r, &bytes, &d).is_err());
}

#[test]
fn upload_braucht_den_ausdruecklichen_schalter() {
    assert_eq!(hochladen(&[]), Ok(false));
    assert_eq!(hochladen(&["--hochladen".into()]), Ok(true));
    assert!(hochladen(&["--upload".into()]).is_err());
    assert!(hochladen(&["--hochladen".into(), "--hochladen".into()]).is_err());
}

#[test]
fn listenbildung_laesst_dateien_unveraendert_und_bricht_bei_fehlern_ab() {
    let tmp = tempfile::tempdir().unwrap();
    let own = tmp.path().join(STORE_DIR);
    fs::create_dir_all(&own).unwrap();
    let bytes = sample_voice_bytes(19);
    fs::write(own.join("Probe.safetensors"), &bytes).unwrap();
    let idx_raw = serde_json::to_vec(&json!({"version": 1, "voices": [{
        "id": "Probe", "uid": "00112233445566778899aabbccddeeff", "name": "Probe",
        "origin": "import", "language": "en", "created_at": "2026-11-26T12:00:00Z"
    }]})).unwrap();
    fs::write(own.join(INDEX_FILE), &idx_raw).unwrap();
    let entries = crate::catalog::tts_entries(crate::catalog::Purpose::PhononModel);
    let weights = MODELLE.iter().map(|id| (entries.iter().find(|e| e.id == *id).unwrap().clone(), vec![0.; 512 * 512])).collect::<Vec<_>>();
    let key = [7; 32];
    let konto = Konto { key: &key, user_id: 1, device_id: "test", hub_url: "https://invalid.test" };
    let idx = || serde_json::from_slice::<Index>(&idx_raw).unwrap();
    let stimmen = vorbereiten(tmp.path(), idx(), &konto, &weights).unwrap();
    assert_eq!(stimmen.len(), 1);
    assert_eq!(listenzeile(&stimmen[0].record, &bytes, &stimmen[0].datei).unwrap().emb.len(), 2);
    assert_eq!(fs::read(own.join(INDEX_FILE)).unwrap(), idx_raw);
    assert_eq!(fs::read(own.join("Probe.safetensors")).unwrap(), bytes);
    assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&own).unwrap().count(), 2);
    // Gegenproben: keine automatische Reparatur und kein Überspringen beschädigter Stimmen.
    let mut ohne_uid = idx();
    ohne_uid.voices[0].uid.clear();
    assert!(vorbereiten(tmp.path(), ohne_uid, &konto, &weights).is_err());
    fs::write(own.join("Probe.safetensors"), b"synthetisch kaputt").unwrap();
    assert!(vorbereiten(tmp.path(), idx(), &konto, &weights).is_err());
    assert_eq!(fs::read(own.join(INDEX_FILE)).unwrap(), idx_raw);
}
