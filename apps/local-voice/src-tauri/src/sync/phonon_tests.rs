//! S1: nur synthetische Stimmen, derselbe Hub-Nachbau wie beim Ablage-Abgleich.
use super::*;
use crate::managers::tts::phonon_voices::{sample_voice_bytes, NewVoice, VoiceStore, ORIGIN_IMPORT};

#[path = "phonon_regression_tests.rs"]
mod regression;

fn neu(seed: u32) -> NewVoice {
    NewVoice { name: "Probe".into(), bytes: sample_voice_bytes(seed), origin: ORIGIN_IMPORT,
        voice_id: String::new(), language: "de".into(), consent_at: None, source_note: None }
}

#[test]
fn ios_nulltensor_vertragsvektor() {
    // Bitgleich in VoiceCore-XCTest und Apple-Python-Vertrag, keine echte Stimme.
    let mut bytes = sample_voice_bytes(0);
    let header = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    bytes[8 + header..].fill(0);
    assert_eq!(bytes.len(), 256136);
    assert_eq!(blobs::sha256_hex(&bytes),
        "143e40a6fe706240586e1fa8f426d40b81d9e05546f2d21c20bd53feb3449b87");
    crate::managers::tts::phonon_voices::validate_voice_bytes(&bytes).unwrap();
    bytes[0] = 255;
    assert!(crate::managers::tts::phonon_voices::validate_voice_bytes(&bytes).is_err());
}

#[test]
fn ios_emb_nulltensor_vertragsvektor() {
    let h = r#"{"emb":{"dtype":"F32","shape":[1,125,512],"data_offsets":[0,256000]},"__metadata__":{"sig":"ba7a5a45","epoch":"300"}}"#;
    let mut h = h.as_bytes().to_vec();
    while h.len() % 8 != 0 { h.push(b' '); }
    let mut b = (h.len() as u64).to_le_bytes().to_vec();
    b.extend(h); b.extend(vec![0; 256000]);
    assert_eq!(b.len(), 256128);
    assert_eq!(blobs::sha256_hex(&b), "84b82cbe56a0ad65412bf5b4ab357da1d22f9e34649b1f835e150cbf96a33664");
}

#[test]
fn emb_reist_pro_modell_im_selben_objekt_ohne_desktop_rangwechsel() {
    use crate::managers::tts::phonon_voices::ModelTarget;
    use base64::{engine::general_purpose::STANDARD, Engine};
    let a = geraet("a");
    let root = a._t.path().join("phonon");
    let lock = Mutex::new(());
    let ohne = VoiceStore { root: &root, targets: &[], lock: &lock };
    ohne.add(neu(1), false).unwrap();
    abgleich(&a, &ohne);
    let before = stimmen(&a)[0].clone();
    assert!(!before.extra.keys().any(|k| k.starts_with("phonon_emb_")));
    let mut targets = Vec::new();
    let weight = "flow_lm.condition_provider.conditioners.speaker_wavs.output_proj.weight";
    for (id, scale) in [("phonon-40m-de", 1f32), ("phonon-31m-en", 2f32)] {
        let dir = root.join("models").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes()); b.extend(1u64.to_le_bytes()); b.extend(0u64.to_le_bytes());
        b.extend((weight.len() as u64).to_le_bytes()); b.extend(weight.as_bytes());
        b.extend(2u32.to_le_bytes()); b.extend(512u64.to_le_bytes()); b.extend(512u64.to_le_bytes());
        b.extend(0u32.to_le_bytes()); b.extend(0u64.to_le_bytes());
        while b.len() % 32 != 0 { b.push(0); }
        for i in 0..512*512 { b.extend((if i % 513 == 0 { scale } else { 0. }).to_le_bytes()); }
        std::fs::write(dir.join("model.q8.gguf"), b).unwrap();
        targets.push(ModelTarget { id: id.into(), dir });
    }
    let store = VoiceStore { root: &root, targets: &targets, lock: &lock };
    abgleich(&a, &store);
    let after = stimmen(&a)[0].clone();
    assert_eq!(before.fassungen, after.fassungen);
    assert_eq!(before.blob_refs, after.blob_refs);
    assert_eq!(before.extra["phonon_voice"], after.extra["phonon_voice"]);
    for (id, lang, scale) in [("phonon-40m-de", "de", 1f32), ("phonon-31m-en", "en", 2f32)] {
        let v = &after.extra[&format!("phonon_emb_{id}")];
        assert_eq!(v[1], before.fassungen[0].blob);
        assert_eq!(v[2]["language"], lang);
        let b = STANDARD.decode(v[2]["data"].as_str().unwrap()).unwrap();
        assert_eq!(v[2]["sha256"], blobs::sha256_hex(&b));
        assert_eq!(v[2]["size"], b.len());
        assert_eq!(b.len(), 256080);
        let n = u64::from_le_bytes(b[..8].try_into().unwrap()) as usize;
        let value = f32::from_le_bytes(b[8+n..12+n].try_into().unwrap());
        assert_eq!(value, 7f32.sin() * scale);
        assert!(crate::managers::tts::phonon_voices::sync_datei_gueltig(&after, &schluessel_konto(), None));
    }
    abgleich(&a, &store);
    assert_eq!(stimmen(&a)[0], after, "Wiederanlauf ohne neue Zeit oder Bytes");
    let hub = FakeHub::neu();
    sync_stimmen(&a, &hub);
    let b = geraet("b");
    sync_stimmen(&b, &hub);
    let other_root = b._t.path().join("phonon");
    let other_lock = Mutex::new(());
    let other = VoiceStore { root: &other_root, targets: &[], lock: &other_lock };
    abgleich(&b, &other);
    assert_eq!(stimmen(&b)[0].extra, after.extra, "Zusätze überleben den normalen Transport");
    assert_eq!(std::fs::read(other_root.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(1));
    store.add(neu(2), true).unwrap();
    abgleich(&a, &store);
    let changed = stimmen(&a)[0].clone();
    for target in &targets {
        let field = format!("phonon_emb_{}", target.id);
        assert_eq!(changed.extra[&field][1], changed.extra["phonon_voice"][1]["sha256"]);
        assert_ne!(changed.extra[&field][1], after.extra[&field][1], "Neue Bytes lösen alte Projektionen ab");
    }
}

fn abgleich(g: &Geraet, s: &VoiceStore<'_>) {
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &g.id, hub_url: HUB_URL };
    crate::sync::phonon::abgleichen(&g.a, s, &konto).unwrap();
}

// Die verschränkten Modellschritte brauchen den Transport, nicht bei jedem Schritt einen
// Suchindex. Dessen Integration und Systemfilter prüfen die Ablage- und tests_system-Tests.
fn sync_stimmen(g: &Geraet, hub: &FakeHub) {
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &g.id, hub_url: HUB_URL };
    tauri::async_runtime::block_on(lauf(hub, &g.a, &konto)).unwrap();
}

#[test]
fn modellschritt_belegt_nicht_die_globale_indexsperre() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let (worker, fertig) = crate::ablage::index::mit_sperre(|| {
            let (a, hub) = (&a, &hub);
            let worker = scope.spawn(move || {
                sync_stimmen(a, hub);
                tx.send(()).unwrap();
            });
            (worker, rx.recv_timeout(std::time::Duration::from_secs(10)))
        });
        // Auch die Gegenprobe (Geraet::sync mit Index-Nachziehen) endet nach Freigabe,
        // bevor die Behauptung fehlschlägt; kein zurückgelassener blockierter Testthread.
        worker.join().unwrap();
        assert!(fertig.is_ok(), "Modellschritte dürfen andere Ablagen nicht am Index aufhalten");
    });
    assert_eq!(stimmen(&a).len(), 1);
    assert_eq!(hub.objekt(SAMMLUNG_DATEI, &stimmen(&a)[0].id).unwrap().revision, 1);
}

#[test]
fn entfernen_bleibt_lokal_und_erzeugt_keinen_grabstein() {
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
    sa.remove("Probe").unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(sb.list().len(), 1, "Entfernen an A betrifft B nicht");
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(sb.list().len(), 1, "keine Löschweitergabe in S1a");
    let index: serde_json::Value = serde_json::from_slice(
        &std::fs::read(rb.join("eigene-stimmen/stimmen.json")).unwrap()).unwrap();
    assert!(index.get("sync_pending").is_none(), "Empfang ist keine Nutzeraktion");
}

#[test]
fn stimmkennung_bleibt_beim_aendern_und_wechselt_bei_neuanlage() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    let index = || -> serde_json::Value { serde_json::from_slice(
        &std::fs::read(root.join("eigene-stimmen/stimmen.json")).unwrap()).unwrap() };
    store.add(neu(1), false).unwrap();
    let uid = index()["voices"][0]["uid"].clone();
    assert_eq!(uid.as_str().unwrap().len(), 32);
    abgleich(&a, &store);
    store.add(neu(2), true).unwrap();
    assert_eq!(index()["voices"][0]["uid"], uid);
    assert!(index().get("sync_pending").is_none(), "Ändern ist keine Nutzerlöschung");
    abgleich(&a, &store);
    store.remove("Probe").unwrap();
    assert_eq!(index()["sync_removed"][0], uid);
    assert!(index().get("sync_pending").is_none());
    store.add(neu(1), false).unwrap();
    assert_ne!(index()["voices"][0]["uid"], uid, "gleicher Name und Inhalt beleben keine Kennung wieder");
}

#[test]
fn gleiche_bytes_unter_zwei_namen_bleiben_zwei_stimmen() {
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
    let mut andere = neu(1);
    andere.name = "Andere".into();
    sa.add(andere, false).unwrap();
    abgleich(&a, &sa);
    assert_eq!(stimmen(&a).len(), 2);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(sb.list().len(), 2);
    sa.remove("Probe").unwrap();
    abgleich(&a, &sa);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &sb);
    b.sync(&hub);
    abgleich(&b, &sb);
    assert_eq!(sb.list().len(), 2, "Entfernen einer UID bleibt am ausführenden Gerät");
}

fn stimmen(g: &Geraet) -> Vec<Datei> {
    g.bestand().dateien.into_values().filter(|d| crate::ablage::system::ist_datei(&d.id)).collect()
}

#[test]
fn aenderungen_sind_fassungen_derselben_datei() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let first = stimmen(&a)[0].clone();
    store.add(neu(2), true).unwrap();
    abgleich(&a, &store);
    let files = stimmen(&a);
    assert_eq!(files.len(), 1, "Gegenprobe: Transportdatei je updated erzeugt zwei Dateien");
    assert_eq!(files[0].id, first.id);
    assert_eq!(files[0].fassungen.len(), 2);
    assert_eq!(zf::koepfe(&files[0]).len(), 1);
}

fn index_lesen(s: &VoiceStore<'_>) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(s.root.join("eigene-stimmen/stimmen.json")).unwrap()).unwrap()
}

#[test]
fn systemdatei_verwendet_die_normale_fassungsaufbewahrung() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    for seed in 0..12 { store.add(neu(seed), seed > 0).unwrap(); abgleich(&a, &store); }
    let file = stimmen(&a)[0].clone();
    let future = chrono::Utc::now().timestamp_millis() + 91 * 24 * 60 * 60 * 1000;
    assert_eq!(a.a.sync_fassungen_aufraeumen(&file.id, future).unwrap(), 2,
        "Gegenprobe: der öffentliche Dokumentfilter sperrt die System-Aufbewahrung");
    let files = stimmen(&a);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].fassungen.len(), 10);
    assert_eq!(files[0].entfernt.len(), 2);
    assert_eq!(files[0].blob_refs.len(), 10);
}

fn index_schreiben(s: &VoiceStore<'_>, v: &serde_json::Value) {
    std::fs::write(s.root.join("eigene-stimmen/stimmen.json"), serde_json::to_vec(v).unwrap()).unwrap();
}

#[test]
fn namenswechsel_gegen_byteaenderung_behaelt_beides() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let locks = [Mutex::new(()), Mutex::new(())];
    let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
    let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
    let sb = VoiceStore { root: &roots[1], targets: &[], lock: &locks[1] };
    sa.add(neu(1), false).unwrap();
    for _ in 0..4 { a.sync(&hub); abgleich(&a, &sa); b.sync(&hub); abgleich(&b, &sb); }
    let uid = index_lesen(&sa)["voices"][0]["uid"].clone();
    let mut v = index_lesen(&sa);
    v["voices"][0]["name"] = serde_json::json!("Neu");
    v["voices"][0]["id"] = serde_json::json!("Neu");
    std::fs::rename(roots[0].join("eigene-stimmen/Probe.safetensors"),
        roots[0].join("eigene-stimmen/Neu.safetensors")).unwrap();
    index_schreiben(&sa, &v);
    sb.add(neu(2), true).unwrap();
    // Beide veröffentlichen lokal auf derselben Basis, erst danach werden sie verschränkt.
    abgleich(&a, &sa);
    abgleich(&b, &sb);
    for _ in 0..8 { a.sync(&hub); b.sync(&hub); abgleich(&a, &sa); abgleich(&b, &sb); }
    for s in [&sa, &sb] {
        assert_eq!(s.list().len(), 1);
        assert_eq!(s.list()[0].id, "Neu", "Gegenprobe: ein gemeinsames Metadaten-LWW verliert den Namen");
        assert_eq!(index_lesen(s)["voices"][0]["uid"], uid);
        assert_eq!(std::fs::read(s.root.join("eigene-stimmen/Neu.safetensors")).unwrap(), sample_voice_bytes(2));
    }
}

#[test]
fn empfangsjournal_heilt_abbruch_vor_und_nach_einer_datei() {
    for blocked in ["Probe", "Probe-2"] {
        let hub = FakeHub::neu();
        let a = geraet("a");
        let b = geraet("b");
        let locks = [Mutex::new(()), Mutex::new(())];
        let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
        let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
        let targets = [crate::managers::tts::phonon_voices::ModelTarget {
            id: "synthetisch".into(), dir: b._t.path().join("model"),
        }];
        let sb = VoiceStore { root: &roots[1], targets: &targets, lock: &locks[1] };
        sa.add(neu(1), false).unwrap();
        sb.add(neu(2), false).unwrap();
        for (s, n) in [(&sa, 1u32), (&sb, 2)] {
            let mut v = index_lesen(s);
            v["voices"][0]["uid"] = serde_json::json!(format!("{n:032x}"));
            v["voices"][0]["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
            index_schreiben(s, &v);
        }
        abgleich(&a, &sa); abgleich(&b, &sb);
        a.sync(&hub); b.sync(&hub); a.sync(&hub);
        let obstacle = roots[1].join(format!("eigene-stimmen/{blocked}.safetensors.partial"));
        std::fs::create_dir(&obstacle).unwrap();
        let key = schluessel_konto();
        let konto = Konto { key: &key, user_id: USER, device_id: &b.id, hub_url: HUB_URL };
        assert!(crate::sync::phonon::abgleichen(&b.a, &sb, &konto).is_err());
        assert!(index_lesen(&sb)["sync_receive"].is_object(), "Write-ahead vor dem ersten Ersetzen");
        if blocked == "Probe-2" {
            assert_eq!(std::fs::read(roots[1].join("eigene-stimmen/Probe.safetensors")).unwrap(),
                sample_voice_bytes(2), "Verlierer vor Gewinner verschieben");
        }
        // Auch einen bereits teilweise ausgeführten Plan des alten Clients fortsetzen.
        std::fs::write(roots[1].join("eigene-stimmen/Probe.safetensors"), sample_voice_bytes(1)).unwrap();
        let moved = roots[1].join("eigene-stimmen/Probe-2.safetensors");
        if moved.exists() { std::fs::remove_file(&moved).unwrap(); }
        assert_eq!(sb.list().iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["Probe", "Probe-2"], "Gegenprobe: auch die bisherige lokale Stimme bleibt sichtbar");
        sb.sync();
        for (name, seed) in [("Probe", 1), ("Probe-2", 2)] {
            assert_eq!(std::fs::read(targets[0].dir.join(format!("voices/{name}.safetensors"))).unwrap(),
                sample_voice_bytes(seed), "Abgleich verwendet geprüfte Staging-Bytes");
        }
        let staged = roots[1].join(format!("eigene-stimmen/.sync-{:032x}.safetensors", 2));
        std::fs::write(&staged, sample_voice_bytes(99)).unwrap();
        assert_eq!(sb.list().len(), 2, "offenes Journal erhält auch vorübergehend unlesbare Einträge");
        sb.sync();
        assert_eq!(std::fs::read(targets[0].dir.join("voices/Probe-2.safetensors")).unwrap(), sample_voice_bytes(2));
        std::fs::write(staged, sample_voice_bytes(2)).unwrap();
        std::fs::remove_dir(obstacle).unwrap();
        // Neustart: schon ein normales Lesen beendet das Journal, ohne erneuten Hub-Zugriff.
        assert_eq!(sb.list().len(), 2);
        assert!(index_lesen(&sb).get("sync_receive").is_none());
        for (name, seed) in [("Probe", 1), ("Probe-2", 2)] {
            assert_eq!(std::fs::read(roots[1].join(format!("eigene-stimmen/{name}.safetensors"))).unwrap(),
                sample_voice_bytes(seed), "Gegenprobe ohne uid-Staging überschreibt die zweite Quelle");
        }
    }
}

#[test]
fn indexwiederaufbau_uebernimmt_keine_verwaisten_empfangskopien() {
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let s = VoiceStore { root: &root, targets: &[], lock: &lock };
    s.add(neu(1), false).unwrap();
    std::fs::write(root.join("eigene-stimmen/.sync-00112233445566778899aabbccddeeff.safetensors"),
        sample_voice_bytes(2)).unwrap();
    std::fs::write(root.join("eigene-stimmen/stimmen.json"), b"defekt").unwrap();
    assert_eq!(s.list().len(), 1, "Gegenprobe: Staging darf beim Wiederaufbau keine Stimme werden");
    assert_eq!(s.list()[0].id, "Probe");
}

#[test]
fn unabhaengige_gleichnamige_importe_bleiben_sichtbar_und_einzeln_loeschbar() {
    for delete in [0usize, 1] {
        let hub = FakeHub::neu();
        let a = geraet("a");
        let b = geraet("b");
        let locks = [Mutex::new(()), Mutex::new(())];
        let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
        let stores = [VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] },
            VoiceStore { root: &roots[1], targets: &[], lock: &locks[1] }];
        stores[0].add(neu(1), false).unwrap();
        stores[1].add(neu(2), false).unwrap();
        let settle = || { for _ in 0..8 { for (g, s) in [(&a, &stores[0]), (&b, &stores[1])] {
            g.sync(&hub); abgleich(g, s);
        } } };
        settle();
        let names = stores[0].list().iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(names.len(), 2, "Gegenprobe: Namens-LWW verliert eine unabhängige Stimme");
        assert_eq!(stores[1].list().iter().map(|r| r.id.clone()).collect::<Vec<_>>(), names);
        let deleted_bytes = std::fs::read(roots[0].join("eigene-stimmen")
            .join(format!("{}.safetensors", names[delete]))).unwrap();
        stores[0].remove(&names[delete]).unwrap();
        settle();
        assert_eq!(stores[1].list().len(), 2, "keine fremde Nutzeraktion");
        for s in &stores[..1] {
            assert_eq!(s.list().len(), 1);
            let bytes = std::fs::read(s.root.join("eigene-stimmen")
                .join(format!("{}.safetensors", s.list()[0].id))).unwrap();
            assert_ne!(bytes, deleted_bytes, "Löschen trifft nur die gewählte uid");
        }
        let writes = hub.mit(|h| h.mitschnitt.len());
        settle();
        assert_eq!(hub.mit(|h| h.mitschnitt.len()), writes, "kein Umbenennungs-Pingpong");
    }
}

#[test]
fn konfliktname_kuerzt_vor_dem_suffix_und_reserviert_basis_und_modellnamen() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let locks = [Mutex::new(()), Mutex::new(())];
    let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
    let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
    let sb = VoiceStore { root: &roots[1], targets: &[], lock: &locks[1] };
    let base = "A".repeat(40);
    let taken = format!("{}-2", "A".repeat(38));
    for (s, seed) in [(&sa, 1), (&sb, 2)] {
        let mut n = neu(seed); n.name = base.clone(); s.add(n, false).unwrap();
    }
    let mut n = neu(9); n.name = taken.clone(); sa.add(n, false).unwrap();
    let model = crate::catalog::tts_entries(crate::catalog::Purpose::PhononVoice).into_iter()
        .filter_map(|e| e.voice).find_map(|v|
            crate::managers::tts::phonon_voices::parse_name(&v.name).ok()).unwrap();
    let mut n = neu(8); n.name = model.display.clone(); sa.add(n, false).unwrap();
    for (s, start) in [(&sa, 1usize), (&sb, 10)] {
        let mut v = index_lesen(s);
        for (i, r) in v["voices"].as_array_mut().unwrap().iter_mut().enumerate() {
            r["uid"] = serde_json::json!(format!("{:032x}", start + i));
            r["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
        }
        index_schreiben(s, &v);
    }
    for _ in 0..8 { a.sync(&hub); abgleich(&a, &sa); b.sync(&hub); abgleich(&b, &sb); }
    for s in [&sa, &sb] {
        let voices = s.list();
        assert_eq!(voices.len(), 4);
        assert!(voices.iter().any(|r| r.id == base));
        assert!(voices.iter().any(|r| r.id == taken));
        assert!(voices.iter().any(|r| r.id == format!("{}-3", "A".repeat(38))),
            "Gegenprobe: Abschneiden nach dem Suffix oder fehlende Basisreservierung kollidiert");
        assert!(voices.iter().all(|r| r.id.len() <= 40 && !r.id.eq_ignore_ascii_case(&model.id)),
            "Gegenprobe ohne Katalogreservierung belegt einen Modellnamen");
    }
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
    b.sync(&hub);
    crate::sync::phonon::abgleichen(&b.a, &sb, &konto).unwrap();
    assert_eq!(sb.list().len(), 1, "S1a: Entfernen wirkt nur lokal");
    assert!(sa.list().is_empty(), "und kehrt hier nicht zurueck");
    sa.add(new(1), false).unwrap();
    crate::sync::phonon::abgleichen(&a.a, &sa, &konto).unwrap();
    assert!(a.bestand().dateien.keys().any(|id| crate::ablage::system::ist_datei(id) && *id != voice.id));
}

#[test]
fn unabhaengige_importe_gleicher_bytes_bleiben_zwei_stimmen() {
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
    let mut uids = Vec::new();
    for (s, uid, created) in [(&sa, 2u32, "2026-01-01T00:00:00Z"), (&sb, 1, "2026-01-02T00:00:00Z")] {
        let mut v = index_lesen(s);
        v["voices"][0]["uid"] = serde_json::json!(format!("{uid:032x}"));
        v["voices"][0]["created_at"] = serde_json::json!(created);
        uids.push(v["voices"][0]["uid"].clone());
        index_schreiben(s, &v);
    }
    // Unabhängige Stimmenkennungen, auch bei identischen Bytes.
    abgleich(&a, &sa);
    abgleich(&b, &sb);
    assert_ne!(stimmen(&a)[0].id, stimmen(&b)[0].id);
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    abgleich(&a, &sa);
    abgleich(&b, &sb);
    for _ in 0..6 {
        a.sync(&hub); abgleich(&a, &sa);
        b.sync(&hub); abgleich(&b, &sb);
    }
    assert_eq!(stimmen(&a).len(), 2);
    assert!(!hub.mit(|h| h.objekte.values().any(|d| d.deleted)));
    assert!(stimmen(&a).iter().all(|d| zf::koepfe(d).len() == 1));
    for s in [&sa, &sb] {
        assert_eq!(s.list().len(), 2, "Gegenprobe: Erstsync-Zusammenführung verliert eine unabhängige UID");
        let v = index_lesen(s);
        let rows = v["voices"].as_array().unwrap();
        assert!(uids.iter().all(|u| rows.iter().any(|r| &r["uid"] == u)));
        assert_eq!(rows.iter().find(|r| r["uid"] == uids[0]).unwrap()["id"], "Probe",
            "die ältere Anlage gewinnt auch gegen die kleinere UID");
        assert_eq!(rows.iter().find(|r| r["uid"] == uids[1]).unwrap()["id"], "Probe-2");
    }
    let vorher = hub.mit(|i| i.mitschnitt.len());
    abgleich(&a, &sa);
    a.sync(&hub);
    assert_eq!(hub.mit(|i| i.mitschnitt.len()), vorher, "kein Rückupload gezogener Stimmen");
    assert_eq!(hub.mit(|i| i.objekte.iter().filter(|((c, _), o)| c == SAMMLUNG_ORDNER && !o.deleted).count()), 2,
        "Allgemein und genau ein Systemordner");
    sa.remove("Probe").unwrap();
    for _ in 0..4 {
        abgleich(&a, &sa); sync_stimmen(&a, &hub);
        sync_stimmen(&b, &hub); abgleich(&b, &sb);
    }
    assert_eq!(sa.list().len(), 1, "nur die entfernte UID bleibt hier ausgeblendet");
    assert_eq!(sb.list().len(), 2);
    assert_eq!(stimmen(&a).len(), 2);
    assert!(!hub.mit(|h| h.objekte.values().any(|d| d.deleted)));
}

#[test]
fn lokale_metadatenaenderung_behaelt_ihre_uid() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let locks = [Mutex::new(()), Mutex::new(())];
    let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
    let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
    let sb = VoiceStore { root: &roots[1], targets: &[], lock: &locks[1] };
    for (s, uid) in [(&sa, 1u32), (&sb, 2)] {
        s.add(neu(1), false).unwrap();
        let mut v = index_lesen(s);
        v["voices"][0]["uid"] = serde_json::json!(format!("{uid:032x}"));
        index_schreiben(s, &v);
    }
    abgleich(&a, &sa); abgleich(&b, &sb);
    a.sync(&hub); b.sync(&hub); a.sync(&hub);
    abgleich(&a, &sa); a.sync(&hub);
    let mut v = index_lesen(&sb);
    let updated = v["voices"][0]["updated"].as_i64().unwrap();
    let sha = v["voices"][0]["sync_file"]["sha256"].clone();
    v["voices"][0]["source_note"] = serde_json::json!("Synthetische Änderung");
    // Wie add_locked auch reine Metadatenänderungen mit einem neueren Rang versehen.
    v["voices"][0]["updated"] = serde_json::json!(updated + 1);
    index_schreiben(&sb, &v);
    b.sync(&hub); abgleich(&b, &sb);
    for _ in 0..8 { a.sync(&hub); abgleich(&a, &sa); b.sync(&hub); abgleich(&b, &sb); }
    for s in [&sa, &sb] {
        assert_eq!(s.list().len(), 2, "beide unabhängigen UIDs bleiben erhalten");
        let v = index_lesen(s);
        let rows = v["voices"].as_array().unwrap();
        let changed = rows.iter().find(|r| r["uid"] == serde_json::json!(format!("{:032x}", 2))).unwrap();
        assert_eq!(changed["source_note"], "Synthetische Änderung");
        assert_eq!(changed["updated"], updated + 1);
        assert_eq!(changed["sync_file"]["sha256"], sha, "reine Metadatenänderung behält die Bytes");
        assert_eq!(rows.iter().filter(|r| r["source_note"] == "Synthetische Änderung").count(), 1);
    }
}

#[test]
fn byteaenderung_derselben_uid_und_neuanlage_ueberlebt_alten_grabstein() {
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
    // Fassungen sind unveränderlich. Neue Kennung und höherer Metadatenrang verhindern,
    // dass der Merge oder das bereits vorhandene Original die kaputte Probe ersetzt.
    d.fassungen[0].fas = crate::ablage::neue_id("fas_");
    d.fassungen[0].blob = sha.clone();
    d.fassungen[0].original_sha256 = sha.clone();
    d.blob_refs = vec![sha.clone()];
    d.extra.get_mut("phonon_voice").unwrap()[1]["sha256"] = serde_json::json!(sha);
    d.extra.get_mut("phonon_voice").unwrap()[1]["fas"] = serde_json::json!(d.fassungen[0].fas);
    let rank = d.extra["phonon_voice"][0].as_str().unwrap().parse::<i64>().unwrap();
    d.extra.get_mut("phonon_voice").unwrap()[0] = serde_json::json!(format!("{:020}", rank + 1));
    a.a.sync_einspielen(&[Eingang::Datei(d.clone())]).unwrap();
    let tmp = a.a.sync_zwischenpfad().unwrap();
    std::fs::write(&tmp, &bytes).unwrap();
    assert!(a.a.original_einlegen(&d.id, &d.fassungen[0].fas, &tmp).unwrap());
    assert!(std::fs::read(a.a.original_quelle(&d.id, &d.fassungen[0].fas).unwrap().unwrap()).unwrap() == bytes,
        "am Sender müssen die kaputten Testbytes liegen");
    let ord = a.ordner("Dokumente");
    let doc = a.ablegen("probe.txt", "Synthetische Gegenprobe", &ord);
    a.sync(&hub);
    assert!(hub.objekt(SAMMLUNG_DATEI, &d.id).is_some(), "kaputte Probe muss den Empfänger erreichen");
    b.sync(&hub);
    let z = b.zustand();
    assert_eq!(z.phonon_verworfen.len(), 1);
    let blob = hub_blob(&schluessel_konto(), &d.fassungen[0].blob).unwrap();
    let downloads = || hub.mit(|h| h.heruntergeladen.iter().filter(|id| **id == blob).count());
    assert_eq!(downloads(), 1);
    assert!(!b.bestand().originale.contains(&(d.id.clone(), d.fassungen[0].fas.clone())));
    assert!(!b.bestand().begraben.contains(&d.id));
    assert_eq!(b.original(&doc).as_deref(), Some("Synthetische Gegenprobe"));
    b.sync(&hub);
    assert_eq!(downloads(), 1, "Gegenprobe ohne Verwerfungsmerkmal lädt dieselbe kaputte Stimme erneut");
    assert_eq!(b.zustand().phonon_verworfen, z.phonon_verworfen);
    assert!(!hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().deleted);
}

#[test]
fn begrabener_systemordner_bekommt_neue_generation_und_nie_wieder_die_alte_id() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let lock = Mutex::new(());
    let lock_b = Mutex::new(());
    let root = a._t.path().join("phonon");
    let root_b = b._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    let store_b = VoiceStore { root: &root_b, targets: &[], lock: &lock_b };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    a.sync(&hub);
    b.sync(&hub);
    abgleich(&b, &store_b);
    let old = stimmen(&a)[0].ordner.wert.clone();
    let revision = hub.objekt(SAMMLUNG_ORDNER, &old).unwrap().revision;
    tauri::async_runtime::block_on(hub.push("alt", &[grab_von_anderem_geraet(SAMMLUNG_ORDNER, &old, revision)])).unwrap();
    a.sync(&hub);
    abgleich(&a, &store);
    a.sync(&hub);
    // B kennt noch die alte Datei. Der technische Wechsel darf keine Löschkaskade starten.
    b.sync(&hub);
    abgleich(&b, &store_b);
    b.sync(&hub);
    a.sync(&hub);
    abgleich(&a, &store);
    assert_eq!(store.list().len(), 1);
    assert_eq!(store_b.list().len(), 1);
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
    assert_eq!(sb.list().len(), 1);
    assert_eq!(sb.list()[0].id, "Probe-eigen", "Foreign-Basisname erhält einen freien Konfliktnamen");
    assert_eq!(std::fs::read(dir.join("voices/Probe.safetensors")).unwrap(), sample_voice_bytes(9));
    assert!(!b.bestand().begraben.contains(&d.id));
}

#[test]
fn wiederanlauf_nach_datei_vor_index_erfasst_einmalig_lokale_aenderung() {
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
    let before = index_lesen(&sb)["voices"][0]["updated"].as_i64().unwrap();
    // Alter Import ohne Journal: abweichende Masterbytes gelten als neue lokale Änderung.
    std::fs::write(rb.join("eigene-stimmen/Probe.safetensors"), sample_voice_bytes(2)).unwrap();
    abgleich(&b, &sb);
    assert!(index_lesen(&sb)["voices"][0]["updated"].as_i64().unwrap() > before);
    let published = stimmen(&b);
    abgleich(&b, &sb);
    assert_eq!(stimmen(&b), published, "nach einmaliger Erfassung kein erneuter Upload");
    assert_eq!(std::fs::read(rb.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(2));
}

#[test]
fn anderer_name_mit_gleichen_bytes_wird_nicht_aufgeraeumt() {
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
    alt["uid"] = serde_json::json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    alt["updated"] = serde_json::json!(idx["voices"][0]["updated"].as_i64().unwrap() - 1);
    alt["sync_file"] = serde_json::Value::Null;
    idx["voices"].as_array_mut().unwrap().push(alt);
    std::fs::write(&path, serde_json::to_vec(&idx).unwrap()).unwrap();
    std::fs::write(root.join("eigene-stimmen/Alt.safetensors"), sample_voice_bytes(1)).unwrap();
    abgleich(&a, &store);
    assert_eq!(store.list().len(), 2);
    assert!(root.join("eigene-stimmen/Alt.safetensors").exists());
    assert!(a.bestand().begraben.is_empty(), "Aufräumen gleicher Inhalte ist kein Entfernen der Sync-Datei");
}

#[test]
fn inhaltsgleicher_anderer_name_umgeht_nicht_die_kapazitaetsgrenze() {
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
        r["uid"] = serde_json::json!(format!("{i:032x}"));
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
    assert!(sb.list().iter().all(|r| r.id != "Neu"));
    assert!(sb.list().iter().any(|r| r.id == "Probe"));
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
    d.extra.get_mut("phonon_name").unwrap()[2] = serde_json::json!("probe");
    a.a.sync_einspielen(&[Eingang::Datei(d)]).unwrap();
    abgleich(&a, &store);
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].name, "probe");
    assert!(root.join("eigene-stimmen/Probe.safetensors").is_file());
    assert!(a.bestand().begraben.is_empty());
}

#[test]
fn modell_unabhaengige_importe_und_einzelne_loeschungen_mit_festen_saaten() {
    for count in [2usize, 3] {
        for seed in [if count == 2 { 7u64 } else { 898 }] {
            for deleted in 0..count {
                let hub = FakeHub::neu();
                let devices = (0..count).map(|i| geraet(&format!("g{i}"))).collect::<Vec<_>>();
                let locks = (0..count).map(|_| Mutex::new(())).collect::<Vec<_>>();
                let roots = devices.iter().map(|g| g._t.path().join("phonon")).collect::<Vec<_>>();
                let stores = (0..count).map(|i| VoiceStore { root: &roots[i], targets: &[], lock: &locks[i] }).collect::<Vec<_>>();
                for (i, s) in stores.iter().enumerate() {
                    s.add(neu(i as u32 + 1), false).unwrap();
                    let mut v = index_lesen(s);
                    v["voices"][0]["uid"] = serde_json::json!(format!("{:032x}", i + 1));
                    v["voices"][0]["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
                    v["voices"][0]["updated"] = serde_json::json!(1_767_225_600_000i64);
                    index_schreiben(s, &v);
                }
                let folder = devices[0].ordner("Dokumente");
                let doc = devices[0].ablegen("probe.txt", "Synthetisch", &folder);
                let settle = || { for _ in 0..4 { for i in 0..count {
                    sync_stimmen(&devices[i], &hub); abgleich(&devices[i], &stores[i]);
                } } };
                let mut rng = seed;
                let mut seen = stores.iter().map(|s| index_lesen(s)["voices"].as_array().unwrap().iter()
                    .map(|r| r["uid"].as_str().unwrap().to_string()).collect::<BTreeSet<_>>()).collect::<Vec<_>>();
                for _ in 0..12 {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let i = ((rng >> 32) as usize) % count;
                    if rng & 1 == 0 { sync_stimmen(&devices[i], &hub); } else { abgleich(&devices[i], &stores[i]); }
                    for (j, s) in stores.iter().enumerate() {
                        let now = index_lesen(s)["voices"].as_array().unwrap().iter()
                            .map(|r| r["uid"].as_str().unwrap().to_string()).collect::<BTreeSet<_>>();
                        assert!(seen[j].is_subset(&now), "keine bereits vorhandene Stimme verschwindet ohne lokale Nutzeraktion");
                        seen[j] = now;
                        assert!(index_lesen(s)["voices"].as_array().unwrap().iter().any(|r|
                            r["uid"] == serde_json::json!(format!("{:032x}", j + 1))),
                            "keine eigene uid verschwindet ohne Nutzerlöschung: {count}/{seed}");
                    }
                }
                settle();
                for s in &stores {
                    assert_eq!(s.list().len(), count);
                    let v = index_lesen(s);
                    for i in 0..count {
                        let uid = serde_json::json!(format!("{:032x}", i + 1));
                        let r = v["voices"].as_array().unwrap().iter().find(|r| r["uid"] == uid).unwrap();
                        assert_eq!(r["id"], serde_json::json!(if i == 0 { "Probe".into() } else { format!("Probe-{}", i + 1) }));
                    }
                }
                let uid = serde_json::json!(format!("{:032x}", deleted + 1));
                let v = index_lesen(&stores[0]);
                let id = v["voices"].as_array().unwrap().iter().find(|r| r["uid"] == uid).unwrap()["id"].as_str().unwrap();
                stores[0].remove(id).unwrap();
                for _ in 0..12 {
                    rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                    let i = ((rng >> 32) as usize) % count;
                    if rng & 1 == 0 { sync_stimmen(&devices[i], &hub); }
                    else { abgleich(&devices[i], &stores[i]); }
                    for (j, s) in stores.iter().enumerate() {
                        let rows = index_lesen(s);
                        let rows = rows["voices"].as_array().unwrap();
                        assert_eq!(rows.len(), if j == 0 { count - 1 } else { count });
                        assert_eq!(rows.iter().any(|r| r["uid"] == uid), j != 0,
                            "kein Gerät übernimmt fremdes Entfernen oder importiert eigenes Entfernen erneut");
                    }
                }
                settle();
                let baseline = hub.mit(|h| h.mitschnitt.len());
                let revisions = hub.objekt(SAMMLUNG_DATEI, &doc).unwrap().revision;
                for (i, s) in stores.iter().enumerate() {
                    assert_eq!(s.list().len(), if i == 0 { count - 1 } else { count });
                    assert_eq!(index_lesen(s)["voices"].as_array().unwrap().iter().any(|r| r["uid"] == uid), i != 0,
                        "nur die Nutzeraktion an diesem Gerät darf die Stimme entfernen");
                }
                let indexes = stores.iter().map(|s| index_lesen(s)).collect::<Vec<_>>();
                settle();
                assert_eq!(hub.mit(|h| h.mitschnitt.len()), baseline, "endliche Konvergenz");
                assert_eq!(hub.objekt(SAMMLUNG_DATEI, &doc).unwrap().revision, revisions);
                for i in 0..count {
                    assert_eq!(index_lesen(&stores[i]), indexes[i], "keine Indexänderung in Ruhe");
                    assert_eq!(devices[i].original(&doc).as_deref(), Some("Synthetisch"));
                }
            }
        }
    }
}

#[test]
fn systemdiagnose_enthaelt_keine_stimmkennung() {
    let key = schluessel_konto();
    for folder in [false, true] {
        let id = crate::ablage::system::kennung(&key, folder, 0, "synthetisch");
        assert_eq!(log_kennung(&id), "Systemobjekt");
    }
    assert_eq!(log_kennung("normales-objekt"), "normales-objekt");
}

#[test]
fn export_erfuellt_die_lesegrenzen_von_ea44627b() {
    // stand_gueltig, typ_ok und id_gueltig aus ea44627b: keine Sonderbehandlung
    // für Systemobjekte. Zusätzliche Metadaten liegen im schon damals offenen extra-Feld.
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let id_ok = |prefix: &str, id: &str| id.strip_prefix(prefix)
        .is_some_and(|s| s.len() == 26 && ulid::Ulid::from_string(s).is_ok());
    let name_ok = |s: &str| !s.is_empty() && s.chars().count() <= 255 && !s.chars().any(|c| c.is_control());
    let typ_ok = |s: &str| s.len() <= 10 && s.chars().all(|c| c.is_ascii_alphanumeric());
    let bestand = a.bestand();
    for o in bestand.ordner.values() {
        assert!(id_ok("ord_", &o.id) && name_ok(&o.name.wert));
        assert!(o.eltern.wert.as_deref().is_none_or(|p| id_ok("ord_", p)));
    }
    for d in bestand.dateien.values() {
        assert!(id_ok("dat_", &d.id) && name_ok(&d.name.wert) && id_ok("ord_", &d.ordner.wert));
        assert!(d.fassungen.iter().all(|f| id_ok("fas_", &f.fas) && typ_ok(&f.typ)));
        let mut alt_unlesbar = d.clone();
        alt_unlesbar.fassungen[0].typ = "safetensors".into();
        assert!(!alt_unlesbar.fassungen.iter().all(|f| typ_ok(&f.typ)), "Gegenprobe alter Fehler");
        let roundtrip: Datei = serde_json::from_value(serde_json::to_value(d).unwrap()).unwrap();
        assert_eq!(&roundtrip, d, "extra bleibt beim Lesen und Schreiben erhalten");
    }
    let raw: serde_json::Value = serde_json::from_slice(
        &std::fs::read(a.a.dir().join("ablage.json")).unwrap()).unwrap();
    assert!(raw["journal"].as_array().unwrap().is_empty());
    assert!(raw["gefunden"].as_array().is_none_or(|v| v.is_empty()));
    for field in ["lokal", "befreit", "weiterleitung"] {
        assert!(raw.get(field).is_none_or(|v| v.as_array().is_some_and(|x| x.is_empty())
            || v.as_object().is_some_and(|x| x.is_empty())));
    }
}

#[test]
fn zu_grosses_stueck_wird_vor_entschluesselung_verworfen() {
    let hub = FakeHub::neu();
    let key = schluessel_konto();
    let id = "a".repeat(64);
    let sealed = vec![0u8; 256]; // Absichtlich kein gültiges Chiffrat.
    tauri::async_runtime::block_on(hub.put_chunk(&id, 0, &sealed)).unwrap();
    tauri::async_runtime::block_on(hub.commit_blob(&id, 1)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("stimme");
    let error = tauri::async_runtime::block_on(blobs::download(&hub, &key, USER, &id, 1, &target)).unwrap_err();
    assert!(matches!(error, HubError::Other(s) if s == "Datei größer als angekündigt"),
        "Gegenprobe ohne Grenze erreicht stattdessen die Entschlüsselung");
    assert!(!target.exists());
}
