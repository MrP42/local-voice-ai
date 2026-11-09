//! D4a/D4b (Spec §9): zwei simulierte Geräte gegen einen nachgebauten Hub im Prozess. Der Hub bildet die
//! Semantik von wai-portal nach (`VoiceSyncService`, `VoiceBlobService`, Test 56): CAS über `revision`,
//! `blob_refs` in einer Transaktion geprüft (fehlend → `rejected`/`blob_missing`), Reservierungen 24 h je
//! Gerät, bedingtes Löschen (409 referenziert/reserviert), endgültige Grabsteine. Nur synthetische Daten.

use super::*;
use crate::ablage::speicher::{
    Ablage, AblageArt, AblageEinstellungen, AblageImportStand, AblageKollisionswahl,
};
use crate::ablage::zusammenfuehren::Text;
use crate::sync::client::{BlobChunkInfo, BlobInfo, CommitOutcome, RemoteObject};
use std::path::PathBuf;
use std::sync::Mutex;

const USER: i64 = 7;
const HUB_URL: &str = "https://hub.example.org";
const STUNDE: i64 = 3600;

fn schluessel_konto() -> [u8; KEY_LEN] {
    let mut k = [0u8; KEY_LEN];
    for (i, b) in k.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    k
}

// ---------------------------------------------------------------------------
// Nachgebauter Hub
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct HubObj {
    revision: i64,
    payload: String,
    key_id: String,
    deleted: bool,
    device_id: String,
    server_seq: i64,
    refs: Vec<String>,
}

#[derive(Default, Debug)]
struct HubBlob {
    stuecke: BTreeMap<usize, Vec<u8>>,
    complete: bool,
}

#[derive(Default)]
struct HubInnen {
    seq: i64,
    objekte: BTreeMap<(String, String), HubObj>,
    blobs: BTreeMap<String, HubBlob>,
    /// (blob, gerät) → Ablauf (Sekunden der Testuhr)
    reservierungen: BTreeMap<(String, String), i64>,
    uhr: i64,
    quota: u64,
    /// Jeder empfangene Push (Mitschnitt).
    mitschnitt: Vec<PushObject>,
    /// Blob-Kennungen, für die Stücke hochgeladen wurden.
    hochgeladen: Vec<String>,
}

type Haken = Box<dyn FnOnce(&mut HubInnen) + Send>;

struct FakeHub {
    innen: Mutex<HubInnen>,
    vor_push: Mutex<Vec<Haken>>,
}

impl FakeHub {
    fn neu() -> FakeHub {
        FakeHub {
            innen: Mutex::new(HubInnen {
                quota: u64::MAX / 2,
                ..Default::default()
            }),
            vor_push: Mutex::new(Vec::new()),
        }
    }

    fn mit<T>(&self, f: impl FnOnce(&mut HubInnen) -> T) -> T {
        f(&mut self.innen.lock().unwrap())
    }

    /// Läuft einmal unmittelbar vor dem nächsten Push (ein „anderes Gerät“ handelt dazwischen).
    fn vor_naechstem_push(&self, h: impl FnOnce(&mut HubInnen) + Send + 'static) {
        self.vor_push.lock().unwrap().push(Box::new(h));
    }

    fn objekt(&self, coll: &str, id: &str) -> Option<HubObj> {
        self.mit(|i| i.objekte.get(&(coll.into(), id.into())).cloned())
    }
}

impl HubInnen {
    fn referenziert(&self, blob: &str) -> bool {
        self.objekte.iter().any(|((c, _), o)| {
            c == SAMMLUNG_DATEI && !o.deleted && o.refs.iter().any(|r| r == blob)
        })
    }

    fn reservierungen_raeumen(&mut self) {
        let uhr = self.uhr;
        self.reservierungen.retain(|_, ablauf| *ablauf > uhr);
    }

    fn loeschen(&mut self, blob: &str, geraet: &str) -> BlobLoeschung {
        if self.referenziert(blob) {
            return BlobLoeschung::InGebrauch("referenced".into());
        }
        self.reservierungen_raeumen();
        if self
            .reservierungen
            .keys()
            .any(|(b, g)| b == blob && g != geraet)
        {
            return BlobLoeschung::InGebrauch("reserved".into());
        }
        self.reservierungen.retain(|(b, _), _| b != blob);
        BlobLoeschung::Geloescht(self.blobs.remove(blob).is_some())
    }

    fn fertig(&self, blob: &str) -> bool {
        self.blobs.get(blob).is_some_and(|b| b.complete)
    }

    fn als_remote(&self, coll: &str, id: &str, o: &HubObj) -> RemoteObject {
        RemoteObject {
            collection: coll.into(),
            object_id: id.into(),
            revision: o.revision,
            payload: o.payload.clone(),
            key_id: o.key_id.clone(),
            deleted: o.deleted,
            device_id: o.device_id.clone(),
            server_seq: o.server_seq,
            blob_refs: (coll == SAMMLUNG_DATEI).then(|| o.refs.clone()),
        }
    }

    fn ergebnis(o: &PushObject, outcome: &str, revision: i64) -> PushResult {
        PushResult {
            collection: o.collection.clone(),
            object_id: o.object_id.clone(),
            outcome: outcome.into(),
            revision,
            current: None,
            reason: None,
            missing: Vec::new(),
        }
    }

    fn abgewiesen(o: &PushObject, grund: &str) -> PushResult {
        let mut r = Self::ergebnis(o, "rejected", 0);
        r.reason = Some(grund.into());
        r
    }

    /// `VoiceSyncController::validateObject` + `VoiceSyncService::pushObject`.
    fn push_eins(&mut self, o: &PushObject, geraet: &str) -> PushResult {
        if !ist_ablage(&o.collection) {
            return Self::abgewiesen(o, "collection");
        }
        if o.collection != SAMMLUNG_DATEI && o.blob_refs.is_some() {
            return Self::abgewiesen(o, "blob_refs_not_allowed");
        }
        if o.collection == SAMMLUNG_DATEI && !o.deleted {
            let Some(refs) = &o.blob_refs else {
                return Self::abgewiesen(o, "blob_refs");
            };
            let eindeutig: BTreeSet<&String> = refs.iter().collect();
            if eindeutig.len() != refs.len() || !refs.iter().all(|r| blobs::valid_blob_id(r)) {
                return Self::abgewiesen(o, "blob_refs");
            }
        }
        if o.deleted != o.payload.is_empty() {
            return Self::abgewiesen(o, "payload");
        }
        let k = (o.collection.clone(), o.object_id.clone());
        let Some(zeile) = self.objekte.get(&k).cloned() else {
            if o.base_revision != 0 {
                return Self::ergebnis(o, "superseded", 0);
            }
            return self.schreiben(o, 1, geraet);
        };
        if zeile.deleted {
            if o.base_revision == zeile.revision - 1 && o.deleted {
                return Self::ergebnis(o, "accepted", zeile.revision);
            }
            let mut r = Self::abgewiesen(o, "tombstoned");
            r.revision = zeile.revision;
            return r;
        }
        if o.base_revision == zeile.revision {
            return self.schreiben(o, zeile.revision + 1, geraet);
        }
        let mut gleiche_refs = o.blob_refs.clone().unwrap_or_default();
        gleiche_refs.sort();
        if o.base_revision == zeile.revision - 1
            && zeile.payload == o.payload
            && zeile.deleted == o.deleted
            && (o.collection != SAMMLUNG_DATEI || o.deleted || zeile.refs == gleiche_refs)
        {
            return Self::ergebnis(o, "accepted", zeile.revision);
        }
        let mut r = Self::ergebnis(o, "superseded", zeile.revision);
        r.current = Some(self.als_remote(&o.collection, &o.object_id, &zeile));
        r
    }

    fn schreiben(&mut self, o: &PushObject, revision: i64, geraet: &str) -> PushResult {
        let mut refs = if o.deleted {
            Vec::new()
        } else {
            o.blob_refs.clone().unwrap_or_default()
        };
        refs.sort();
        if o.collection == SAMMLUNG_DATEI {
            let fehlt: Vec<String> = refs.iter().filter(|r| !self.fertig(r)).cloned().collect();
            if !fehlt.is_empty() {
                let mut r = Self::abgewiesen(o, "blob_missing");
                r.missing = fehlt;
                return r;
            }
            for b in &refs {
                self.reservierungen.retain(|(rb, _), _| rb != b);
            }
        }
        self.seq += 1;
        self.objekte.insert(
            (o.collection.clone(), o.object_id.clone()),
            HubObj {
                revision,
                payload: if o.deleted {
                    String::new()
                } else {
                    o.payload.clone()
                },
                key_id: o.key_id.clone(),
                deleted: o.deleted,
                device_id: geraet.into(),
                server_seq: self.seq,
                refs,
            },
        );
        Self::ergebnis(o, "accepted", revision)
    }
}

#[async_trait::async_trait]
impl BlobTransport for FakeHub {
    async fn blob_info(&self, blob_id: &str) -> Result<Option<BlobInfo>, HubError> {
        Ok(self.mit(|i| {
            i.blobs.get(blob_id).map(|b| BlobInfo {
                complete: b.complete,
                chunks: b
                    .stuecke
                    .keys()
                    .map(|k| BlobChunkInfo { idx: *k as i64 })
                    .collect(),
            })
        }))
    }
    async fn put_chunk(&self, blob_id: &str, index: usize, sealed: &[u8]) -> Result<(), HubError> {
        self.mit(|i| {
            let belegt: u64 = i
                .blobs
                .values()
                .flat_map(|b| b.stuecke.values())
                .map(|s| s.len() as u64)
                .sum();
            if belegt + sealed.len() as u64 > i.quota {
                return Err(HubError::Other(
                    "quota_exceeded: Der Speicher im Konto ist voll.".into(),
                ));
            }
            let b = i.blobs.entry(blob_id.into()).or_default();
            if b.complete {
                return Err(HubError::Other("immutable".into()));
            }
            b.stuecke.insert(index, sealed.to_vec());
            i.hochgeladen.push(blob_id.into());
            Ok(())
        })
    }
    async fn commit_blob(&self, blob_id: &str, chunks: usize) -> Result<CommitOutcome, HubError> {
        self.mit(|i| {
            let b = i
                .blobs
                .get_mut(blob_id)
                .ok_or_else(|| HubError::Other("not_found".into()))?;
            let fehlt: Vec<i64> = (0..chunks)
                .filter(|n| !b.stuecke.contains_key(n))
                .map(|n| n as i64)
                .collect();
            if fehlt.is_empty() {
                b.complete = true;
                Ok(CommitOutcome::Complete)
            } else {
                Ok(CommitOutcome::Missing(fehlt))
            }
        })
    }
    async fn get_chunk(&self, blob_id: &str, index: usize) -> Result<Vec<u8>, HubError> {
        self.mit(|i| {
            i.blobs
                .get(blob_id)
                .filter(|b| b.complete)
                .and_then(|b| b.stuecke.get(&index).cloned())
                .ok_or_else(|| HubError::Other("not_found".into()))
        })
    }
}

#[async_trait::async_trait]
impl AblageHub for FakeHub {
    async fn push(
        &self,
        device_id: &str,
        objects: &[PushObject],
    ) -> Result<Vec<PushResult>, HubError> {
        let haken: Vec<Haken> = std::mem::take(&mut *self.vor_push.lock().unwrap());
        self.mit(|i| {
            for h in haken {
                h(i);
            }
            objects
                .iter()
                .map(|o| {
                    i.mitschnitt.push(o.clone());
                    i.push_eins(o, device_id)
                })
                .collect::<Vec<_>>()
        })
        .pipe(Ok)
    }
    async fn pull_sammlung(
        &self,
        since: i64,
        limit: u32,
        collection: &str,
    ) -> Result<PullResponse, HubError> {
        Ok(self.mit(|i| {
            let mut neu: Vec<(&(String, String), &HubObj)> = i
                .objekte
                .iter()
                .filter(|((c, _), o)| c == collection && o.server_seq > since)
                .collect();
            neu.sort_by_key(|(_, o)| o.server_seq);
            let more = neu.len() > limit as usize;
            neu.truncate(limit as usize);
            let cursor = neu.last().map(|(_, o)| o.server_seq).unwrap_or(since);
            PullResponse {
                objects: neu
                    .into_iter()
                    .map(|((c, id), o)| i.als_remote(c, id, o))
                    .collect(),
                cursor,
                more,
            }
        }))
    }
    async fn reservieren(&self, blob_id: &str, device_id: &str) -> Result<(), HubError> {
        self.mit(|i| {
            let ablauf = i.uhr + 24 * STUNDE;
            i.reservierungen
                .insert((blob_id.into(), device_id.into()), ablauf);
        });
        Ok(())
    }
    async fn blob_loeschen(
        &self,
        blob_id: &str,
        device_id: &str,
    ) -> Result<BlobLoeschung, HubError> {
        Ok(self.mit(|i| i.loeschen(blob_id, device_id)))
    }
    async fn verbrauch(&self) -> Result<BlobVerbrauch, HubError> {
        Ok(self.mit(|i| BlobVerbrauch {
            used: i
                .blobs
                .values()
                .flat_map(|b| b.stuecke.values())
                .map(|s| s.len() as u64)
                .sum(),
            quota: i.quota,
            blobs: i.blobs.values().filter(|b| b.complete).count() as u64,
        }))
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

// ---------------------------------------------------------------------------
// Simulierte Geräte
// ---------------------------------------------------------------------------

struct Geraet {
    _t: tempfile::TempDir,
    a: Ablage,
    quellen: PathBuf,
    id: String,
}

fn geraet(name: &str) -> Geraet {
    let t = tempfile::tempdir().unwrap();
    let quellen = t.path().join("quellen");
    std::fs::create_dir_all(&quellen).unwrap();
    let a = Ablage::neu(t.path().join("ablage"));
    assert_eq!(a.wartung(), None);
    Geraet {
        _t: t,
        a,
        quellen,
        id: format!("geraet-{name}-0001"),
    }
}

impl Geraet {
    fn sync(&self, hub: &FakeHub) -> Bericht {
        let key = schluessel_konto();
        let konto = Konto {
            key: &key,
            user_id: USER,
            device_id: &self.id,
            hub_url: HUB_URL,
        };
        let b = tauri::async_runtime::block_on(lauf(hub, &self.a, &konto)).expect("Lauf");
        if b.geaendert {
            index_nachziehen(&self.a);
        }
        b
    }

    fn quelle(&self, name: &str, inhalt: &str) -> PathBuf {
        let p = self.quellen.join(name);
        std::fs::write(&p, inhalt).unwrap();
        p
    }

    fn ordner(&self, name: &str) -> String {
        self.a.ordner_anlegen(None, name).unwrap()
    }

    fn ablegen(&self, name: &str, inhalt: &str, ordner: &str) -> String {
        let q = self.quelle(name, inhalt);
        let z = self.a.importieren(&q, ordner, None).unwrap();
        assert_eq!(z[0].stand, AblageImportStand::Abgelegt, "{:?}", z[0]);
        z[0].dat.clone().unwrap()
    }

    fn neue_fassung(&self, name: &str, inhalt: &str, ordner: &str) {
        let q = self.quelle(name, inhalt);
        let z = self
            .a
            .importieren(&q, ordner, Some(AblageKollisionswahl::NeueFassung))
            .unwrap();
        assert_eq!(z[0].stand, AblageImportStand::NeueFassung, "{:?}", z[0]);
    }

    fn bestand(&self) -> SyncBestand {
        self.a.sync_bestand().unwrap()
    }

    fn datei(&self, dat: &str) -> Option<Datei> {
        self.bestand().dateien.get(dat).cloned()
    }

    fn zustand_von(&self, id: &str) -> Option<AblageSyncZustand> {
        stand(&self.a, true)
            .unwrap()
            .eintraege
            .into_iter()
            .find(|e| e.id == id)
            .map(|e| e.zustand)
    }

    fn original(&self, dat: &str) -> Option<String> {
        let p = self.a.original_pfad(dat).ok()?;
        std::fs::read_to_string(p).ok()
    }

    fn zustand(&self) -> Zustand {
        Zustand::laden(&self.a)
    }
}

fn hid(inhalt: &str) -> String {
    hub_blob(&schluessel_konto(), &blobs::sha256_hex(inhalt.as_bytes())).unwrap()
}

/// Alle Klartexte, die je beim Hub ankamen (Mitschnitt entschlüsselt).
fn mitschnitt_klartexte(hub: &FakeHub) -> Vec<(String, String, String)> {
    let key = schluessel_konto();
    hub.mit(|i| {
        i.mitschnitt
            .iter()
            .map(|o| {
                let refs = o.blob_refs.clone().unwrap_or_default();
                let plain = if o.deleted {
                    String::new()
                } else {
                    String::from_utf8(
                        crypto::open(
                            &key,
                            &crypto::aad_ablage(USER, &o.collection, &o.object_id, &refs),
                            &o.payload,
                        )
                        .unwrap(),
                    )
                    .unwrap()
                };
                (o.collection.clone(), o.object_id.clone(), plain)
            })
            .collect()
    })
}

/// Jede lebende Datei am Hub referenziert nur abgeschlossene Blobs.
fn keine_lebende_fassung_ohne_blob(hub: &FakeHub) {
    hub.mit(|i| {
        for ((c, id), o) in &i.objekte {
            if c == SAMMLUNG_DATEI && !o.deleted {
                for r in &o.refs {
                    assert!(i.fertig(r), "{id}: Blob {r} fehlt am Hub");
                }
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn zwei_geraete_gleichen_ordner_datei_und_original_ab() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Verträge");
    let dat = a.ablegen("Angebot.txt", "Angebot über 12 Stück", &ord);
    a.sync(&hub);
    let bericht = b.sync(&hub);
    assert!(bericht.geaendert);
    assert!(bericht.vollstaendig);
    let bb = b.bestand();
    assert_eq!(bb.ordner[&ord].name.wert, "Verträge");
    assert_eq!(bb.dateien[&dat].name.wert, "Angebot.txt");
    assert_eq!(b.original(&dat).as_deref(), Some("Angebot über 12 Stück"));
    assert_eq!(b.zustand_von(&dat), Some(AblageSyncZustand::Synchronisiert));
    assert_eq!(a.zustand_von(&ord), Some(AblageSyncZustand::Synchronisiert));
    // Das Steuerfeld trägt die Hub-Kennung des Originals; am Hub liegt nur Chiffrat.
    let o = hub.objekt(SAMMLUNG_DATEI, &dat).unwrap();
    assert_eq!(o.refs, vec![hid("Angebot über 12 Stück")]);
    assert!(!o.payload.contains("Angebot"));
    // Ein zweiter Lauf sendet nichts erneut.
    let vorher = hub.mit(|i| i.mitschnitt.len());
    a.sync(&hub);
    b.sync(&hub);
    assert_eq!(
        hub.mit(|i| i.mitschnitt.len()),
        vorher,
        "kein erneuter Push"
    );
}

#[test]
fn blob_refs_sind_an_die_aad_gebunden() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Projekte");
    let dat = a.ablegen("Plan.txt", "Plan A", &ord);
    a.ablegen("Andere.txt", "Plan B", &ord);
    a.sync(&hub);
    // Der Hub vertauscht das Steuerfeld (zeigt auf einen anderen, vorhandenen Blob).
    hub.mit(|i| {
        let o = i
            .objekte
            .get_mut(&(SAMMLUNG_DATEI.into(), dat.clone()))
            .unwrap();
        o.refs = vec![hid("Plan B")];
    });
    b.sync(&hub);
    assert!(
        b.datei(&dat).is_none(),
        "manipuliertes Objekt wird nicht übernommen"
    );
}

#[test]
fn zwei_geraete_konflikt_ergibt_zwei_koepfe_aktuell_nicht_nach_zeit() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Angebote");
    let dat = a.ablegen("Angebot.txt", "v1", &ord);
    a.sync(&hub);
    b.sync(&hub);
    // Beide offline: je eine neue Fassung (B später).
    a.neue_fassung("Angebot.txt", "v2-a", &ord);
    std::thread::sleep(std::time::Duration::from_millis(5));
    b.neue_fassung("Angebot.txt", "v2-b", &ord);
    a.sync(&hub);
    b.sync(&hub); // superseded → zusammenführen → mit neuer Basis pushen
    a.sync(&hub);
    for g in [&a, &b] {
        let d = g.datei(&dat).unwrap();
        assert_eq!(zf::koepfe(&d).len(), 2, "zwei Köpfe");
        assert!(zf::konflikt(&d));
        assert_eq!(zf::aktuell(&d), None, "aktuell nicht nach Zeit");
        assert_eq!(g.zustand_von(&dat), Some(AblageSyncZustand::Konflikt));
    }
    assert_eq!(a.datei(&dat), b.datei(&dat), "beide Geräte gleich");
    // Beide Originale liegen auf beiden Geräten.
    let bb = b.bestand();
    assert_eq!(
        bb.originale.iter().filter(|(d, _)| *d == dat).count(),
        3,
        "{:?}",
        bb.originale
    );
    // Wahl der älteren Fassung (A) auf B: genau ein Kopf überall.
    let fa = a
        .datei(&dat)
        .unwrap()
        .fassungen
        .iter()
        .find(|f| f.blob == blobs::sha256_hex(b"v2-a"))
        .unwrap()
        .fas
        .clone();
    b.a.fassung_waehlen(&dat, &fa).unwrap();
    b.sync(&hub);
    a.sync(&hub);
    for g in [&a, &b] {
        let d = g.datei(&dat).unwrap();
        assert_eq!(zf::koepfe(&d), vec![fa.clone()]);
        assert_eq!(g.zustand_von(&dat), Some(AblageSyncZustand::Synchronisiert));
    }
    // Offline A → neue Fassung auf der gewählten: konfliktfrei.
    a.neue_fassung("Angebot.txt", "v3-a", &ord);
    a.sync(&hub);
    b.sync(&hub);
    assert!(!zf::konflikt(&b.datei(&dat).unwrap()));
}

#[test]
fn wahl_rennen_lamport_deterministisch_in_allen_reihenfolgen() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Rennen");
    let dat = a.ablegen("R.txt", "basis", &ord);
    a.sync(&hub);
    b.sync(&hub);
    a.neue_fassung("R.txt", "von-a", &ord);
    b.neue_fassung("R.txt", "von-b", &ord);
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    let d = a.datei(&dat).unwrap();
    let koepfe = zf::koepfe(&d);
    assert_eq!(koepfe.len(), 2);
    // Gegenwahl: A wählt Kopf 0, B wählt Kopf 1, gleichzeitig.
    a.a.fassung_waehlen(&dat, &koepfe[0]).unwrap();
    b.a.fassung_waehlen(&dat, &koepfe[1]).unwrap();
    let (da, db) = (a.datei(&dat).unwrap(), b.datei(&dat).unwrap());
    let ab = zf::zusammenfuehren_datei(&da, &db);
    let ba = zf::zusammenfuehren_datei(&db, &da);
    assert_eq!(ab, ba, "Reihenfolge egal");
    assert_eq!(zf::koepfe(&ab).len(), 1, "Vorrang ergibt genau einen Kopf");
    // Über den Hub in beiden Reihenfolgen.
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    let (ea, eb) = (a.datei(&dat).unwrap(), b.datei(&dat).unwrap());
    assert_eq!(ea, eb);
    assert_eq!(zf::koepfe(&ea), zf::koepfe(&ab));
    assert_eq!(ea.wahlen.len(), 2);
}

#[test]
fn ordner_verschieberennen_ohne_zyklus() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let x = a.ordner("X");
    let y = a.ordner("Y");
    a.sync(&hub);
    b.sync(&hub);
    a.a.ordner_verschieben(&x, Some(&y)).unwrap();
    b.a.ordner_verschieben(&y, Some(&x)).unwrap();
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    let (ba, bb) = (a.bestand(), b.bestand());
    assert_eq!(ba.ordner, bb.ordner);
    let eltern = zf::wirksame_eltern(&ba.ordner);
    // Genau eine Kante wirkt, keine zyklische Kette.
    let xy = eltern.get(&x) == Some(&Eltern::Ordner(y.clone()));
    let yx = eltern.get(&y) == Some(&Eltern::Ordner(x.clone()));
    assert!(xy ^ yx, "{eltern:?}");
    assert_eq!(eltern, zf::wirksame_eltern(&bb.ordner));
}

#[test]
fn grabstein_offline_geraet_belebt_nichts_und_blob_wird_geloescht() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Alt");
    let dat = a.ablegen("Weg.txt", "wird gelöscht", &ord);
    a.sync(&hub);
    b.sync(&hub);
    assert!(b.original(&dat).is_some());
    // B offline: benennt um. A löscht endgültig.
    b.a.datei_umbenennen(&dat, "Neu.txt").unwrap();
    a.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    a.a.endgueltig_loeschen(AblageArt::Datei, &dat).unwrap();
    a.sync(&hub);
    let o = hub.objekt(SAMMLUNG_DATEI, &dat).unwrap();
    assert!(o.deleted, "Grabstein am Hub");
    assert!(
        hub.mit(|i| !i.blobs.contains_key(&hid("wird gelöscht"))),
        "Blob ohne lebende Fassung gelöscht"
    );
    b.sync(&hub);
    assert!(b.datei(&dat).is_none(), "B belebt nichts");
    assert!(b.bestand().begraben.contains(&dat));
    assert!(
        !b.a.dir().join("dateien").join(&dat).exists(),
        "Original weg"
    );
    a.sync(&hub);
    b.sync(&hub);
    assert!(hub.objekt(SAMMLUNG_DATEI, &dat).unwrap().deleted);
    assert!(a.datei(&dat).is_none());
}

/// Gemeinsamer Aufbau: A hat Datei F (Inhalt C) synchronisiert; B legt G mit demselben Inhalt ab.
fn loeschen_parallel_aufbau() -> (FakeHub, Geraet, Geraet, String, String, String) {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Gemeinsam");
    let f = a.ablegen("F.txt", "gleicher Inhalt", &ord);
    a.sync(&hub);
    b.sync(&hub);
    let g = b.ablegen("G.txt", "gleicher Inhalt", &ord);
    (hub, a, b, f, g, hid("gleicher Inhalt"))
}

#[test]
fn loeschen_parallel_upload_reservierung_gibt_409_und_push_gelingt() {
    let (hub, a, b, f, g, blob) = loeschen_parallel_aufbau();
    let (geraet_a, f2, blob2) = (a.id.clone(), f.clone(), blob.clone());
    let ergebnis = std::sync::Arc::new(Mutex::new(None));
    let e2 = ergebnis.clone();
    // Zwischen Reservierung (B) und Objekt-Push (B): A begräbt F und will den Blob löschen.
    hub.vor_naechstem_push(move |i| {
        let rev = i.objekte[&(SAMMLUNG_DATEI.to_string(), f2.clone())].revision;
        let grab = PushObject {
            collection: SAMMLUNG_DATEI.into(),
            object_id: f2,
            base_revision: rev,
            payload: String::new(),
            key_id: "abcd1234".into(),
            deleted: true,
            blob_refs: None,
        };
        assert_eq!(i.push_eins(&grab, &geraet_a).outcome, "accepted");
        *e2.lock().unwrap() = Some(i.loeschen(&blob2, &geraet_a));
    });
    b.sync(&hub);
    assert_eq!(
        *ergebnis.lock().unwrap(),
        Some(BlobLoeschung::InGebrauch("reserved".into())),
        "reservieren → löschen 409"
    );
    assert_eq!(
        hub.objekt(SAMMLUNG_DATEI, &g).unwrap().refs,
        vec![blob.clone()]
    );
    keine_lebende_fassung_ohne_blob(&hub);
    // A zieht den Stand: der Blob bleibt (lebende Fassung G), nichts wird gelöscht.
    a.sync(&hub);
    assert!(hub.mit(|i| i.fertig(&blob)));
    assert!(a.datei(&g).is_some());
    assert!(a.datei(&f).is_none(), "Grabstein von F gilt auch auf A");
}

#[test]
fn loeschen_parallel_upload_reservierung_abgelaufen_push_422_neu_hochladen() {
    let (hub, _a, b, f, g, blob) = loeschen_parallel_aufbau();
    let (f2, blob2) = (f.clone(), blob.clone());
    hub.vor_naechstem_push(move |i| {
        i.uhr += 25 * STUNDE; // Reservierung abgelaufen
        let rev = i.objekte[&(SAMMLUNG_DATEI.to_string(), f2.clone())].revision;
        let grab = PushObject {
            collection: SAMMLUNG_DATEI.into(),
            object_id: f2,
            base_revision: rev,
            payload: String::new(),
            key_id: "abcd1234".into(),
            deleted: true,
            blob_refs: None,
        };
        assert_eq!(i.push_eins(&grab, "geraet-a-0001").outcome, "accepted");
        assert_eq!(
            i.loeschen(&blob2, "geraet-a-0001"),
            BlobLoeschung::Geloescht(true)
        );
    });
    b.sync(&hub);
    let pushes_g = hub.mit(|i| i.mitschnitt.iter().filter(|o| o.object_id == g).count());
    assert_eq!(
        pushes_g, 2,
        "erst 422 (blob_missing), dann nach neuem Hochladen angenommen"
    );
    let uploads = hub.mit(|i| i.hochgeladen.iter().filter(|h| **h == blob).count());
    assert_eq!(uploads, 2, "von B neu hochgeladen");
    let o = hub.objekt(SAMMLUNG_DATEI, &g).unwrap();
    assert!(!o.deleted);
    keine_lebende_fassung_ohne_blob(&hub);
}

#[test]
fn nur_geraet_und_personal_erscheinen_nie_im_hub() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let privat = a.ordner("Privatordner");
    a.a.ordner_einstellungen(
        &privat,
        &AblageEinstellungen {
            nur_geraet: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    let unter = a.a.ordner_anlegen(Some(&privat), "Unterordner").unwrap();
    let p1 = a.ablegen("Tagebuch.txt", "GEHEIM-nur-geraet", &unter);
    let pers = a.ordner("Persönlich");
    a.a.ordner_einstellungen(
        &pers,
        &AblageEinstellungen {
            datenklasse: Some("personal".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let p2 = a.ablegen("Arzt.txt", "GEHEIM-personal", &pers);
    let vertr = a.ordner("Vertraulich");
    a.a.ordner_einstellungen(
        &vertr,
        &AblageEinstellungen {
            datenklasse: Some("confidential".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let p3 = a.ablegen("Lohn.txt", "GEHEIM-confidential", &vertr);
    let offen = a.ordner("Offen");
    a.ablegen("Notiz.txt", "öffentlich genug", &offen);
    a.sync(&hub);
    a.sync(&hub);
    let ids: BTreeSet<String> =
        hub.mit(|i| i.mitschnitt.iter().map(|o| o.object_id.clone()).collect());
    for id in [&privat, &unter, &p1, &pers, &p2, &vertr, &p3] {
        assert!(!ids.contains(id), "{id} darf nie zum Hub");
    }
    assert!(ids.contains(&offen));
    for (_, _, klar) in mitschnitt_klartexte(&hub) {
        assert!(
            !klar.contains("Privatordner") && !klar.contains("Tagebuch") && !klar.contains("Arzt")
        );
    }
    for inhalt in [
        "GEHEIM-nur-geraet",
        "GEHEIM-personal",
        "GEHEIM-confidential",
    ] {
        assert!(
            hub.mit(|i| !i.hochgeladen.contains(&hid(inhalt))),
            "{inhalt}"
        );
    }
    assert_eq!(a.zustand_von(&p1), Some(AblageSyncZustand::NurGeraet));
    assert_eq!(a.zustand_von(&p3), Some(AblageSyncZustand::NurGeraet));
}

#[test]
fn wechsel_zu_nur_geraet_entfernt_cloud_kopien_und_behaelt_lokale() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Wird privat");
    let dat = a.ablegen("Brief.txt", "Brieftext", &ord);
    a.sync(&hub);
    b.sync(&hub);
    assert!(b.original(&dat).is_some());
    a.a.ordner_einstellungen(
        &ord,
        &AblageEinstellungen {
            nur_geraet: Some(true),
            ..Default::default()
        },
    )
    .unwrap();
    a.sync(&hub);
    assert!(hub.objekt(SAMMLUNG_ORDNER, &ord).unwrap().deleted);
    assert!(hub.objekt(SAMMLUNG_DATEI, &dat).unwrap().deleted);
    assert!(
        hub.mit(|i| !i.blobs.contains_key(&hid("Brieftext"))),
        "Cloud-Kopie weg"
    );
    b.sync(&hub);
    assert!(
        b.datei(&dat).is_none(),
        "andere Geräte entfernen ihre Kopie"
    );
    assert!(!b.bestand().ordner.contains_key(&ord));
    // A behält alles lokal, auch nach weiteren Läufen (eigener Grabstein belebt/löscht nichts).
    a.sync(&hub);
    a.sync(&hub);
    assert_eq!(a.original(&dat).as_deref(), Some("Brieftext"));
    assert_eq!(a.zustand_von(&dat), Some(AblageSyncZustand::NurGeraet));
    assert!(a.zustand().lokalisiert.contains(&dat));
    // Zurück auf sync: Rückweg ist offen, die Kennung bleibt lokal (nie erneut gesendet).
    a.a.ordner_einstellungen(
        &ord,
        &AblageEinstellungen {
            nur_geraet: Some(false),
            ..Default::default()
        },
    )
    .unwrap();
    let vorher = hub.mit(|i| i.mitschnitt.len());
    a.sync(&hub);
    assert_eq!(hub.mit(|i| i.mitschnitt.len()), vorher);
    assert_eq!(a.original(&dat).as_deref(), Some("Brieftext"));
}

fn text_json(f: &zf::Fassung, textrev: &str, inhalt: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "marken_version": 1,
        "textrev": textrev,
        "original_sha256": f.original_sha256,
        "extraktor": "text",
        "extraktor_version": "1",
        "abschnitte": [{"anker": "zeile=1", "pfad": [], "text": inhalt}],
    }))
    .unwrap()
}

#[test]
fn textfassung_kommt_mit_passender_textrev_an() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Texte");
    let dat = a.ablegen("Bericht.txt", "Inhalt des Berichts", &ord);
    let f = a.datei(&dat).unwrap().fassungen[0].clone();
    let rev = crate::ablage::speicher::erwartete_textrev(&f).unwrap();
    let json = text_json(&f, &rev, "Inhalt des Berichts");
    let text = Text {
        textrev: rev.clone(),
        blob: String::new(),
        size: 0,
        extraktor: "text".into(),
        extraktor_version: "1".into(),
        marken_version: 1,
        stand: "ok".into(),
        code: None,
        versuche: 0,
    };
    a.a.text_ablegen(&dat, &f.fas, Some(&json), text).unwrap();
    a.sync(&hub);
    let o = hub.objekt(SAMMLUNG_DATEI, &dat).unwrap();
    assert_eq!(o.refs.len(), 2, "Original und Text referenziert");
    b.sync(&hub);
    let p = b.a.text_pfad(&dat, &rev);
    assert_eq!(std::fs::read(&p).unwrap(), json);
    assert_eq!(b.zustand_von(&dat), Some(AblageSyncZustand::Synchronisiert));
    // Ein Text, der nicht zu seiner Fassung passt (andere textrev), wird abgewiesen.
    let falsch = text_json(&f, &"0".repeat(64), "x");
    assert!(b.a.text_einlegen(&dat, &f.fas, &rev, &falsch).is_err());
}

#[test]
fn fehlende_textfassung_ergibt_wartezustand() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Warten");
    let dat = a.ablegen("Lang.txt", "noch ohne Text", &ord);
    a.sync(&hub);
    b.sync(&hub);
    // B hat das Original: es erstellt den Text selbst (kein Warten).
    let mut bb = b.bestand();
    assert!(!wartet_auf_text(&bb, &bb.dateien[&dat]));
    // Ohne Original (z. B. noch nicht geladen) wartet die Datei auf ihren Text.
    bb.originale.clear();
    assert!(wartet_auf_text(&bb, &bb.dateien[&dat]));
    let z = b.zustand();
    let e = zustaende(&bb, &z);
    assert_eq!(
        e.iter().find(|e| e.id == dat).map(|e| e.zustand),
        Some(AblageSyncZustand::WartetAufText)
    );
    // Text am Hub eingetragen, aber hier noch nicht geladen: ebenfalls Warten.
    let f = a.datei(&dat).unwrap().fassungen[0].clone();
    let rev = crate::ablage::speicher::erwartete_textrev(&f).unwrap();
    let text = Text {
        textrev: rev.clone(),
        blob: String::new(),
        size: 0,
        extraktor: "text".into(),
        extraktor_version: "1".into(),
        marken_version: 1,
        stand: "ok".into(),
        code: None,
        versuche: 0,
    };
    a.a.text_ablegen(&dat, &f.fas, Some(&text_json(&f, &rev, "t")), text)
        .unwrap();
    a.sync(&hub);
    // Der Text-Blob ist (noch) nicht abrufbar.
    let textblob = hub.objekt(SAMMLUNG_DATEI, &dat).unwrap().refs;
    hub.mit(|i| {
        for r in &textblob {
            if r != &hid("noch ohne Text") {
                i.blobs.remove(r);
            }
        }
    });
    b.sync(&hub);
    assert_eq!(b.zustand_von(&dat), Some(AblageSyncZustand::WartetAufText));
}

#[test]
fn kontingent_ueberschritten_stoppt_hochladen_sichtbar() {
    let hub = FakeHub::neu();
    hub.mit(|i| i.quota = 10);
    let a = geraet("a");
    let ord = a.ordner("Groß");
    let dat = a.ablegen("Gross.txt", &"x".repeat(500), &ord);
    let bericht = a.sync(&hub);
    assert!(bericht.warnung.unwrap_or_default().contains("Speicher"));
    assert!(
        hub.objekt(SAMMLUNG_DATEI, &dat).is_none(),
        "ohne Blob kein Objekt"
    );
    let st = stand(&a.a, true).unwrap();
    assert!(st.kontingent_voll);
    assert_eq!(st.kontingent, Some(10.0));
    assert_eq!(a.zustand_von(&dat), Some(AblageSyncZustand::Ausstehend));
    // Der Ordner selbst braucht keinen Speicher und ist synchronisiert.
    assert_eq!(a.zustand_von(&ord), Some(AblageSyncZustand::Synchronisiert));
}

#[test]
fn anderes_konto_beginnt_mit_leerem_zustand() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    a.ordner("Eins");
    a.sync(&hub);
    assert!(!a.zustand().objekte.is_empty());
    let mut z = a.zustand();
    z.konto = "8:ffffffff:https://anderer.example.org".into();
    z.speichern(&a.a).unwrap();
    // Neuer Hub (anderes Konto): alles wird neu gesendet.
    let hub2 = FakeHub::neu();
    a.sync(&hub2);
    assert!(hub2.mit(|i| !i.objekte.is_empty()));
}
