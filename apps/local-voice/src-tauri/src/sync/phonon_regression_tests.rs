//! Drittreview S1: Fehler zwischen Sammlungen und dauerhaftem Stimmenindex.
use super::*;
use crate::managers::tts::phonon_voices::ModelTarget;

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
fn dateigrab_wartet_zwei_pulls_auch_bei_mehrfacher_bruecke_und_neustart() {
    for recursive in [false, true] {
        let hub = FakeHub::neu();
        let a = geraet("a");
        let lock = Mutex::new(());
        let root = a._t.path().join("phonon");
        let store = VoiceStore { root: &root, targets: &[], lock: &lock };
        store.add(neu(1), false).unwrap(); abgleich(&a, &store); sync_stimmen(&a, &hub);
        let d = stimmen(&a)[0].clone();
        let mut graves = vec![grab_von_anderem_geraet(SAMMLUNG_DATEI, &d.id,
            hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().revision)];
        if recursive { graves.push(grab_von_anderem_geraet(SAMMLUNG_ORDNER, &d.ordner.wert,
            hub.objekt(SAMMLUNG_ORDNER, &d.ordner.wert).unwrap().revision)); }
        let race = FehlerHub { hub: &hub, vor_dateipull: Mutex::new(graves), stueckfehler: false };
        lauf_mit(&a, &race);
        assert!(a.zustand().phonon_grab_vorgemerkt.contains(&d.id));
        assert!(!a.zustand().phonon_grab_bestaetigt.contains(&d.id));
        for _ in 0..3 { abgleich(&a, &store); }
        assert_eq!(store.list().len(), 1, "Gegenprobe: ein vollständiger Pull reicht nicht");
        let restarted = VoiceStore { root: &root, targets: &[], lock: &lock };
        sync_stimmen(&a, &hub); abgleich(&a, &restarted);
        assert_eq!(restarted.list().len(), usize::from(recursive));
        if recursive {
            assert!(!a.zustand().phonon_grab_vorgemerkt.contains(&d.id));
            assert!(!a.zustand().phonon_grab_bestaetigt.contains(&d.id));
            assert!(stimmen(&a).iter().any(|f| f.ordner.wert == crate::ablage::system::kennung(&schluessel_konto(), true, 1, "")));
        }
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
        for s in &stores {
            assert_eq!(s.list().len(), count - 1);
            assert!(index_lesen(s)["voices"].as_array().unwrap().iter().all(|r| r["uid"] != format!("{:032x}", 1)),
                "Gegenprobe: Löschen ohne Link importiert dieselbe uid erneut");
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
    assert!(a.bestand().begraben.contains(&d.id));
    sync_stimmen(&b, &hub); // Lebende Datei kommt erst nach der lokalen Löschung am Hub an.
    for _ in 0..4 { abgleich(&a, &store); sync_stimmen(&a, &hub); }
    assert!(store.list().is_empty(), "Gegenprobe: lebende Ablagedatei importiert die gelöschte uid erneut");
    assert!(hub.objekt(SAMMLUNG_DATEI, &d.id).unwrap().deleted);
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
