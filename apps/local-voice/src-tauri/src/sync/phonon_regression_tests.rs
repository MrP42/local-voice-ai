//! Drittreview S1: Fehler zwischen Sammlungen und dauerhaftem Stimmenindex.
use super::*;
use crate::managers::tts::phonon_voices::ModelTarget;

#[test]
fn defekte_optionale_modellkopie_blockiert_keinen_pflichtabschluss() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let locks = [Mutex::new(()), Mutex::new(())];
    let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
    let model = b._t.path().join("model");
    std::fs::create_dir_all(model.join("voices")).unwrap();
    std::fs::write(model.join("voices/SOURCE.json"), b"{").unwrap();
    let targets = [ModelTarget { id: "synthetisch".into(), dir: model.clone() }];
    let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
    let sb = VoiceStore { root: &roots[1], targets: &targets, lock: &locks[1] };
    sa.add(neu(1), false).unwrap();
    abgleich(&a, &sa); sync_stimmen(&a, &hub); sync_stimmen(&b, &hub);
    abgleich(&b, &sb); // Gegenprobe: SOURCE.json-Fehler bricht das Pflichtjournal ab.
    assert_eq!(sb.list().len(), 1);
    assert!(index_lesen(&sb).get("sync_receive").is_none());
    assert_eq!(index_lesen(&sb)["sync_models_open"], true);
    let mut extra = neu(2); extra.name = "Weitere".into();
    sb.add(extra, false).unwrap();
    sb.remove("Probe").unwrap(); // Auch optionale Entfernung darf lokal nicht blockieren.
    assert_eq!(sb.list().len(), 1);
    std::fs::write(model.join("voices/SOURCE.json"), b"{\"voices\":[]}").unwrap();
    sb.sync(); // Vorhandener Modellstart-/Installationsweg.
    assert_eq!(index_lesen(&sb)["sync_models_open"], false);
    assert!(!model.join("voices/Probe.safetensors").exists());
    assert!(model.join("voices/Weitere.safetensors").is_file());
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(model.join("voices/SOURCE.json")).unwrap()).unwrap();
    assert_eq!(source["voices"].as_array().unwrap().len(), 1);
}

#[test]
fn entfernte_erstsync_dublette_bleibt_lokal_entfernt_aber_neuanlagen_kommen_an() {
    // Beide UID-Reihenfolgen: auch ein später bekannt werdender Gewinner bleibt entfernt.
    for (removed_uid, created, updated) in [
        (1u32, "2099-01-01T00:00:00Z", 1767225600000i64),
        (2, "2026-01-01T00:00:00Z", 4070908800000),
    ] {
        let hub = FakeHub::neu();
        let a = geraet("a");
        let b = geraet("b");
        let locks = [Mutex::new(()), Mutex::new(())];
        let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
        let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
        let sb = VoiceStore { root: &roots[1], targets: &[], lock: &locks[1] };
        for (s, uid) in [(&sa, removed_uid), (&sb, 3 - removed_uid)] {
            s.add(neu(1), false).unwrap();
            let mut v = index_lesen(s);
            v["voices"][0]["uid"] = serde_json::json!(format!("{uid:032x}"));
            v["voices"][0]["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
            v["voices"][0]["updated"] = serde_json::json!(1767225600000i64);
            index_schreiben(s, &v);
            abgleich(if uid == removed_uid { &a } else { &b }, s);
        }
        sa.remove("Probe").unwrap();
        sync_stimmen(&b, &hub); sync_stimmen(&a, &hub); abgleich(&a, &sa);
        assert!(sa.list().is_empty(), "Gegenprobe: entfernte Erstsync-Kopie fehlt bei der Zuordnung");
        assert!(index_lesen(&sa)["sync_removed"].as_array().unwrap()
            .contains(&serde_json::json!(format!("{:032x}", 3 - removed_uid))));
        assert_eq!(sb.list().len(), 1, "keine Löschweitergabe");
        sb.remove("Probe").unwrap();
        sb.add(neu(1), false).unwrap();
        let mut v = index_lesen(&sb);
        // Synthetisch strikt später als die Entfernung, ohne Uhr-/Sleep-Abhängigkeit.
        v["voices"][0]["created_at"] = serde_json::json!(created);
        v["voices"][0]["updated"] = serde_json::json!(updated);
        index_schreiben(&sb, &v);
        abgleich(&b, &sb); sync_stimmen(&b, &hub); sync_stimmen(&a, &hub); abgleich(&a, &sa);
        assert_eq!(sa.list().len(), 1, "spätere Neuanlage ist keine Erstsync-Dublette");
        sa.remove("Probe").unwrap();
        let removed = index_lesen(&sa)["sync_removed"].clone();
        sa.add(neu(1), false).unwrap();
        assert_eq!(sa.list().len(), 1, "bewusste lokale Neuanlage bleibt möglich");
        assert_eq!(index_lesen(&sa)["sync_removed"], removed);
        assert!(index_lesen(&sa).get("sync_removed_initial").is_none());
    }
}

/// Behält den vorhandenen Hub unverändert; Fehler entstehen an der Transportgrenze.
struct FehlerHub<'a> {
    hub: &'a FakeHub,
    vor_dateipull: Mutex<Vec<PushObject>>,
    stueckfehler: bool,
}

#[async_trait::async_trait]
impl BlobTransport for FehlerHub<'_> {
    async fn blob_info(&self, id: &str) -> Result<Option<BlobInfo>, HubError> { self.hub.blob_info(id).await }
    async fn put_chunk(&self, id: &str, n: usize, bytes: &[u8]) -> Result<(), HubError> { self.hub.put_chunk(id, n, bytes).await }
    async fn commit_blob(&self, id: &str, n: usize) -> Result<CommitOutcome, HubError> { self.hub.commit_blob(id, n).await }
    async fn get_chunk(&self, id: &str, n: usize, limit: usize) -> Result<Vec<u8>, HubError> {
        if self.stueckfehler {
            self.hub.mit(|h| h.heruntergeladen.push(id.into()));
            Err(HubError::Other("Stückantwort zu groß".into()))
        } else { self.hub.get_chunk(id, n, limit).await }
    }
}

#[async_trait::async_trait]
impl AblageHub for FehlerHub<'_> {
    async fn push(&self, device: &str, objects: &[PushObject]) -> Result<Vec<PushResult>, HubError> { self.hub.push(device, objects).await }
    async fn pull_sammlung(&self, since: i64, limit: u32, coll: &str) -> Result<PullResponse, HubError> {
        if coll == SAMMLUNG_DATEI {
            let graves = std::mem::take(&mut *self.vor_dateipull.lock().unwrap());
            if !graves.is_empty() { self.hub.push("alt", &graves).await?; }
        }
        self.hub.pull_sammlung(since, limit, coll).await
    }
    async fn reservieren(&self, id: &str, device: &str) -> Result<(), HubError> { self.hub.reservieren(id, device).await }
    async fn blob_loeschen(&self, id: &str, device: &str) -> Result<BlobLoeschung, HubError> { self.hub.blob_loeschen(id, device).await }
    async fn verbrauch(&self) -> Result<BlobVerbrauch, HubError> { self.hub.verbrauch().await }
}

fn lauf_mit(g: &Geraet, hub: &impl AblageHub) {
    let key = schluessel_konto();
    let konto = Konto { key: &key, user_id: USER, device_id: &g.id, hub_url: HUB_URL };
    tauri::async_runtime::block_on(lauf(hub, &g.a, &konto)).unwrap();
}

pub(super) fn rekursiv_zwischen_sammlungen(g: &Geraet, hub: &FakeHub, graves: Vec<PushObject>) {
    lauf_mit(g, &FehlerHub { hub, vor_dateipull: Mutex::new(graves), stueckfehler: false });
}

#[test]
fn alte_clients_begraben_ordner_und_dateien_in_jeder_reihenfolge_ohne_stimmverlust() {
    for order in [[0, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        let hub = FakeHub::neu();
        let devices = [geraet("a"), geraet("b")];
        let locks = [Mutex::new(()), Mutex::new(())];
        let roots = devices.iter().map(|g| g._t.path().join("phonon")).collect::<Vec<_>>();
        let stores = (0..2).map(|i| VoiceStore { root: &roots[i], targets: &[], lock: &locks[i] }).collect::<Vec<_>>();
        stores[0].add(neu(1), false).unwrap();
        let mut other = neu(2); other.name = "Andere".into();
        stores[0].add(other, false).unwrap();
        let settle = || { for _ in 0..4 { for i in 0..2 {
            abgleich(&devices[i], &stores[i]); sync_stimmen(&devices[i], &hub);
        } } };
        settle();
        let files = stimmen(&devices[0]);
        let folder = files[0].ordner.wert.clone();
        let objects = [(SAMMLUNG_ORDNER, folder.as_str()),
            (SAMMLUNG_DATEI, files[0].id.as_str()), (SAMMLUNG_DATEI, files[1].id.as_str())];
        let uid = index_lesen(&stores[0])["voices"].as_array().unwrap().iter()
            .find(|r| r["id"] == "Probe").unwrap()["uid"].clone();
        stores[0].remove("Probe").unwrap();
        for n in order {
            let (coll, id) = objects[n];
            let grave = grab_von_anderem_geraet(coll, id, hub.objekt(coll, id).unwrap().revision);
            // Auch die Lücke zwischen den Sammlungspulls wird durchlaufen.
            rekursiv_zwischen_sammlungen(&devices[1], &hub, vec![grave]);
            for i in [1, 0] {
                sync_stimmen(&devices[i], &hub); abgleich(&devices[i], &stores[i]);
                assert_eq!(stores[i].list().len(), if i == 0 { 1 } else { 2 },
                    "Gegenprobe: kein Grabstein darf eine lokale Stimme entfernen: {order:?}");
            }
        }
        settle();
        assert!(index_lesen(&stores[0])["voices"].as_array().unwrap().iter().all(|r| r["uid"] != uid));
        assert!(index_lesen(&stores[1])["voices"].as_array().unwrap().iter().any(|r| r["uid"] == uid));
        let key = schluessel_konto();
        for s in &stores {
            for r in index_lesen(s)["voices"].as_array().unwrap() {
                let generation = r["sync_file"]["generation"].as_u64().unwrap() as u32;
                assert_eq!(r["sync_file"]["file"], crate::ablage::system::kennung(&key, false,
                    generation, r["uid"].as_str().unwrap()));
                assert!(generation > 0 && generation <= 2, "begrenzte Wiederbereitstellung");
            }
        }
        let writes = hub.mit(|h| h.mitschnitt.len());
        let uploads = hub.mit(|h| h.hochgeladen.len());
        let indexes = stores.iter().map(index_lesen).collect::<Vec<_>>();
        settle();
        assert_eq!(hub.mit(|h| h.mitschnitt.len()), writes, "keine Endlosschleife");
        assert_eq!(hub.mit(|h| h.hochgeladen.len()), uploads);
        for i in 0..2 { assert_eq!(index_lesen(&stores[i]), indexes[i]); }
    }
}

#[test]
fn bereitstellen_heilt_absturz_vor_original_und_vor_link_ohne_neues_schreiben() {
    for original_fertig in [false, true] {
        let a = geraet("a");
        let lock = Mutex::new(());
        let root = a._t.path().join("phonon");
        let store = VoiceStore { root: &root, targets: &[], lock: &lock };
        store.add(neu(1), false).unwrap();
        let before = index_lesen(&store);
        abgleich(&a, &store);
        let files = stimmen(&a);
        let d = &files[0];
        if !original_fertig {
            std::fs::remove_file(a.a.original_quelle(&d.id, &d.fassungen[0].fas).unwrap().unwrap()).unwrap();
        }
        index_schreiben(&store, &before);
        let restarted = VoiceStore { root: &root, targets: &[], lock: &lock };
        abgleich(&a, &restarted);
        assert_eq!(stimmen(&a), files, "Gegenprobe: kein künstlicher Rang nach Absturz vor Link");
        assert_eq!(index_lesen(&restarted)["voices"][0]["sync_file"]["file"], d.id);
        assert_eq!(std::fs::read(a.a.original_quelle(&d.id, &d.fassungen[0].fas).unwrap().unwrap()).unwrap(), sample_voice_bytes(1));
        let idx = index_lesen(&restarted);
        abgleich(&a, &restarted);
        assert_eq!(stimmen(&a), files);
        assert_eq!(index_lesen(&restarted), idx);
    }
}

#[test]
fn erst_beim_push_gelernte_graeber_erhalten_uid_und_lokale_aenderung() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    abgleich(&a, &store); sync_stimmen(&a, &hub);
    let d = stimmen(&a)[0].clone();
    let uid = index_lesen(&store)["voices"][0]["uid"].clone();
    let graves = [grab_von_anderem_geraet(SAMMLUNG_ORDNER, &d.ordner.wert,
        hub.objekt(SAMMLUNG_ORDNER, &d.ordner.wert).unwrap().revision),
        grab_von_anderem_geraet(SAMMLUNG_DATEI, &d.id,
            hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().revision)];
    store.add(neu(2), true).unwrap(); abgleich(&a, &store);
    hub.vor_naechstem_push(move |h| { for grave in &graves {
        assert_eq!(h.push_eins(grave, "alt").outcome, "accepted");
    } });
    sync_stimmen(&a, &hub); abgleich(&a, &store);
    for _ in 0..4 { sync_stimmen(&a, &hub); abgleich(&a, &store); }
    assert_eq!(store.list().len(), 1);
    assert_eq!(index_lesen(&store)["voices"][0]["uid"], uid);
    assert_eq!(index_lesen(&store)["voices"][0]["sync_file"]["generation"], 1);
    assert_eq!(std::fs::read(root.join("eigene-stimmen/Probe.safetensors")).unwrap(), sample_voice_bytes(2));
    let writes = hub.mit(|h| h.mitschnitt.len());
    sync_stimmen(&a, &hub); abgleich(&a, &store);
    assert_eq!(hub.mit(|h| h.mitschnitt.len()), writes);
}

#[test]
fn fremde_modellnamen_erhalten_eigen_suffix_mit_laengengrenze() {
    let a = geraet("a");
    let b = geraet("b");
    let locks = [Mutex::new(()), Mutex::new(())];
    let roots = [a._t.path().join("phonon"), b._t.path().join("phonon")];
    let sa = VoiceStore { root: &roots[0], targets: &[], lock: &locks[0] };
    let base = "A".repeat(40);
    let eigen = format!("{}-eigen", "A".repeat(34));
    let expected = format!("{}-eigen-2", "A".repeat(32));
    let model = b._t.path().join("model");
    std::fs::create_dir_all(model.join("voices")).unwrap();
    for name in [&base, &eigen] {
        std::fs::write(model.join("voices").join(format!("{name}.safetensors")), sample_voice_bytes(99)).unwrap();
    }
    let targets = [ModelTarget { id: "synthetisch".into(), dir: model.clone() }];
    let sb = VoiceStore { root: &roots[1], targets: &targets, lock: &locks[1] };
    let mut voice = neu(1); voice.name = base.clone();
    sa.add(voice, false).unwrap();
    let hub = FakeHub::neu();
    for _ in 0..4 {
        abgleich(&a, &sa); sync_stimmen(&a, &hub);
        sync_stimmen(&b, &hub); abgleich(&b, &sb);
    }
    for s in [&sa, &sb] {
        assert_eq!(s.list()[0].id, expected, "Gegenprobe: numerischer oder ungekürzter Suffix");
        assert_eq!(s.list()[0].id.len(), 40);
    }
    for name in [&base, &eigen] {
        assert_eq!(std::fs::read(model.join("voices").join(format!("{name}.safetensors"))).unwrap(), sample_voice_bytes(99));
    }
}

#[test]
fn modell_foreign_name_und_absturz_vor_link_konvergieren() {
    for (count, seed) in [(2usize, 29u64), (3, 898)] {
        let hub = FakeHub::neu();
        let devices = (0..count).map(|i| geraet(&format!("g{i}"))).collect::<Vec<_>>();
        let locks = (0..count).map(|_| Mutex::new(())).collect::<Vec<_>>();
        let roots = devices.iter().map(|g| g._t.path().join("phonon")).collect::<Vec<_>>();
        let model = devices[1]._t.path().join("model");
        std::fs::create_dir_all(model.join("voices")).unwrap();
        std::fs::write(model.join("voices/Probe-2.safetensors"), sample_voice_bytes(99)).unwrap();
        let targets = [ModelTarget { id: "synthetisch".into(), dir: model.clone() }];
        let stores = (0..count).map(|i| VoiceStore { root: &roots[i],
            targets: if i == 1 { &targets } else { &[] }, lock: &locks[i] }).collect::<Vec<_>>();
        let folder = devices[0].ordner("Dokumente");
        let doc = devices[0].ablegen("probe.txt", "Synthetisch", &folder);
        for (i, s) in stores.iter().enumerate() {
            s.add(neu(i as u32 + 1), false).unwrap();
            let mut v = index_lesen(s);
            v["voices"][0]["uid"] = serde_json::json!(format!("{:032x}", i + 1));
            v["voices"][0]["created_at"] = serde_json::json!("2026-01-01T00:00:00Z");
            index_schreiben(s, &v);
            abgleich(&devices[i], s);
        }
        let settle = || { for _ in 0..6 { for i in 0..count {
            sync_stimmen(&devices[i], &hub); abgleich(&devices[i], &stores[i]);
        } } };
        let mut rng = seed;
        for _ in 0..12 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let i = (rng >> 32) as usize % count;
            sync_stimmen(&devices[i], &hub); abgleich(&devices[i], &stores[i]);
            let v = index_lesen(&stores[i]);
            let rows = v["voices"].as_array().unwrap();
            assert!(rows.iter().any(|r| r["uid"] == format!("{:032x}", i + 1)),
                "ohne Nutzerlöschung bleibt die eigene UID erhalten");
            let names = rows.iter().map(|r| r["id"].as_str().unwrap().to_ascii_lowercase()).collect::<BTreeSet<_>>();
            assert_eq!(names.len(), rows.len(), "Gegenprobe: blockierte Umbenennung erzeugt doppelte Namen");
            for r in rows {
                let uid = usize::from_str_radix(r["uid"].as_str().unwrap(), 16).unwrap();
                assert_eq!(std::fs::read(roots[i].join("eigene-stimmen").join(format!("{}.safetensors", r["id"].as_str().unwrap()))).unwrap(),
                    sample_voice_bytes(uid as u32), "keine UID übernimmt fremde Bytes");
            }
        }
        settle();
        let names = stores[0].list().iter().map(|r| r.id.clone()).collect::<BTreeSet<_>>();
        assert_eq!(names.len(), count);
        for s in &stores { assert_eq!(s.list().iter().map(|r| r.id.clone()).collect::<BTreeSet<_>>(), names); }
        assert!(!names.contains("Probe-2"));
        assert_eq!(std::fs::read(model.join("voices/Probe-2.safetensors")).unwrap(), sample_voice_bytes(99));
        // Erster Publish war dauerhaft, der Link ging beim Absturz verloren.
        let s = &stores[0];
        let mut v = index_lesen(s);
        let row = v["voices"].as_array_mut().unwrap().iter_mut().find(|r| r["uid"] == format!("{:032x}", 1)).unwrap();
        let id = row["id"].as_str().unwrap().to_string();
        row.as_object_mut().unwrap().remove("sync_file");
        index_schreiben(s, &v);
        s.remove(&id).unwrap();
        settle();
        for (i, s) in stores.iter().enumerate() {
            assert_eq!(s.list().len(), if i == 0 { count - 1 } else { count });
            assert_eq!(index_lesen(s)["voices"].as_array().unwrap().iter().any(|r| r["uid"] == format!("{:032x}", 1)),
                i != 0, "Entfernen bleibt lokal, auch ohne Link");
        }
        let writes = hub.mit(|h| h.mitschnitt.len());
        let uploads = hub.mit(|h| h.hochgeladen.len());
        let indexes = stores.iter().map(index_lesen).collect::<Vec<_>>();
        let modified = roots.iter().map(|r| std::fs::metadata(r.join("eigene-stimmen/stimmen.json"))
            .unwrap().modified().unwrap()).collect::<Vec<_>>();
        settle();
        assert_eq!(hub.mit(|h| h.mitschnitt.len()), writes);
        assert_eq!(hub.mit(|h| h.hochgeladen.len()), uploads);
        for i in 0..count {
            assert_eq!(index_lesen(&stores[i]), indexes[i]);
            assert_eq!(std::fs::metadata(roots[i].join("eigene-stimmen/stimmen.json"))
                .unwrap().modified().unwrap(), modified[i]);
            assert_eq!(devices[i].original(&doc).as_deref(), Some("Synthetisch"));
        }
    }
}

#[test]
fn loeschen_nach_erstem_ablageschreiben_ohne_link_verhindert_auch_spaeten_import() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap();
    let before = index_lesen(&store);
    abgleich(&a, &store);
    let d = stimmen(&a)[0].clone();
    b.a.sync_einspielen(&[Eingang::Ordner(a.bestand().ordner[&d.ordner.wert].clone()), Eingang::Datei(d.clone())]).unwrap();
    let tmp = b.a.sync_zwischenpfad().unwrap();
    std::fs::write(&tmp, sample_voice_bytes(1)).unwrap();
    b.a.original_einlegen(&d.id, &d.fassungen[0].fas, &tmp).unwrap();
    index_schreiben(&store, &before); // Absturz nach Original, vor erstem Link.
    store.remove("Probe").unwrap();
    abgleich(&a, &store);
    assert!(store.list().is_empty());
    assert!(!a.bestand().begraben.contains(&d.id));
    sync_stimmen(&b, &hub); // Lebende Datei kommt erst nach der lokalen Löschung am Hub an.
    for _ in 0..4 { abgleich(&a, &store); sync_stimmen(&a, &hub); }
    assert!(store.list().is_empty(), "Gegenprobe: lebende Ablagedatei importiert die gelöschte uid erneut");
    assert!(!hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().deleted);
}

#[test]
fn uebergrosse_transportantwort_wird_nur_einmal_angefordert() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let b = geraet("b");
    let lock = Mutex::new(());
    let root = a._t.path().join("phonon");
    let store = VoiceStore { root: &root, targets: &[], lock: &lock };
    store.add(neu(1), false).unwrap(); abgleich(&a, &store); sync_stimmen(&a, &hub);
    let failing = FehlerHub { hub: &hub, vor_dateipull: Mutex::new(vec![]), stueckfehler: true };
    lauf_mit(&b, &failing);
    assert_eq!(b.zustand().phonon_verworfen.len(), 1);
    let downloads = hub.mit(|h| h.heruntergeladen.len());
    lauf_mit(&b, &failing);
    assert_eq!(hub.mit(|h| h.heruntergeladen.len()), downloads, "Gegenprobe: fester Clientfehler wird endlos geladen");
}
