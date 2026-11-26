//! Gegenproben für das Example gegen den vorhandenen Hub-Nachbau, ohne echte Konten.
use super::*;

fn plan(a: &Geraet) -> Vec<(Ordner, Datei, Vec<u8>)> {
    let b = a.bestand();
    b.dateien.values().filter(|d| crate::ablage::system::ist_datei(&d.id)).map(|d| {
        let path = a.a.original_quelle(&d.id, &d.fassungen[0].fas).unwrap().unwrap();
        (b.ordner[&d.ordner.wert].clone(), d.clone(), std::fs::read(path).unwrap())
    }).collect()
}

#[test]
fn einmaliger_upload_erhaelt_fassungen_und_schreibt_keinen_lokalen_zustand() {
    let a = geraet("a");
    let root = a._t.path().join("phonon");
    let lock = Mutex::new(());
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store);
    let before = std::fs::read(root.join("eigene-stimmen/stimmen.json")).unwrap();
    let hub = FakeHub::neu();
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &a.id, hub_url: HUB_URL };
    let data = plan(&a);
    let run = |data| tauri::async_runtime::block_on(stimmen_einmalig::hochladen(&hub, &konto, data, || true));
    assert_eq!(run(data.clone()), Ok(1));
    assert_eq!(run(data.clone()), Ok(1));
    assert_eq!(std::fs::read(root.join("eigene-stimmen/stimmen.json")).unwrap(), before);
    assert!(!a.a.dir().join(ZUSTAND).exists(), "kein lokales Ablage-Ledger");
    let obj = hub.objekt(SAMMLUNG_DATEI, &data[0].1.id).unwrap();
    let (Eingang::Datei(d), _) = oeffnen(&konto, &obj.key_id, SAMMLUNG_DATEI, &data[0].1.id, &obj.refs, &obj.payload).unwrap()
        else { panic!("Datei erwartet"); };
    assert_eq!(d.fassungen, data[0].1.fassungen);
    hub.mit(|i| assert!(i.mitschnitt.iter().all(|o| !o.deleted
        && ((o.collection == SAMMLUNG_ORDNER && crate::ablage::system::ist_ordner(&o.object_id))
            || (o.collection == SAMMLUNG_DATEI && crate::ablage::system::ist_datei(&o.object_id))))));
    // Gegenprobe: fremde Kennung wird vor jedem Netzwerk-Schreiben abgelehnt.
    let mut fremd = data.clone();
    fremd[0].1.id = crate::ablage::neue_id("dat_");
    let n = hub.mit(|i| i.mitschnitt.len());
    assert!(run(fremd).is_err());
    assert_eq!(hub.mit(|i| i.mitschnitt.len()), n);
    // Ein lokaler Kontowechsel erlaubt ebenfalls keinen Push.
    assert!(tauri::async_runtime::block_on(stimmen_einmalig::hochladen(&hub, &konto, data, || false)).is_err());
    assert_eq!(hub.mit(|i| i.mitschnitt.len()), n);
}

#[test]
fn einmaliger_upload_bricht_bei_cas_konflikt_und_nach_teilergebnis_ab() {
    let a = geraet("a");
    let root = a._t.path().join("phonon");
    let lock = Mutex::new(());
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    let mut zweite = neu(2);
    zweite.name = "Zweite".into();
    store.add(zweite, false).unwrap();
    abgleich(&a, &store);
    let hub = FakeHub::neu();
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &a.id, hub_url: HUB_URL };
    let data = plan(&a);
    hub.mit(|i| { i.abweisen.insert(data[1].1.id.clone()); });
    let r = tauri::async_runtime::block_on(stimmen_einmalig::hochladen(&hub, &konto, data.clone(), || true));
    assert_eq!(r.unwrap_err().0, 1, "erste Stimme bestätigt, zweite abgewiesen");
    hub.mit(|i| i.abweisen.clear());
    let folder = data[0].0.id.clone();
    hub.vor_naechstem_push(move |i| {
        i.objekte.get_mut(&(SAMMLUNG_ORDNER.into(), folder)).unwrap().revision += 1;
    });
    let r = tauri::async_runtime::block_on(stimmen_einmalig::hochladen(&hub, &konto, data, || true));
    assert_eq!(r.unwrap_err().0, 0, "veraltete Basis wird nicht erzwungen");
}
