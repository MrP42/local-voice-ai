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

fn test_konto() -> String {
    konto_kennung(USER, &schluessel_konto(), HUB_URL)
}

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
    /// Jede Reservierung (Blob-Kennung).
    reserviert: Vec<String>,
    /// Reservierung dieser Blobs scheitert (5xx).
    fehl_reservieren: BTreeSet<String>,
    /// Diese Objekte weist der Hub ab (`payload_size`).
    abweisen: BTreeSet<String>,
    /// Hochladen scheitert mit „Zu viele Ablagen“.
    zu_viele: bool,
}

type Haken = Box<dyn FnOnce(&mut HubInnen) + Send>;
type GeraeteHaken = Box<dyn FnOnce() + Send>;

struct FakeHub {
    innen: Mutex<HubInnen>,
    vor_push: Mutex<Vec<Haken>>,
    beim_reservieren: Mutex<Vec<GeraeteHaken>>,
    beim_hochladen: Mutex<Vec<GeraeteHaken>>,
}

impl FakeHub {
    fn neu() -> FakeHub {
        FakeHub {
            innen: Mutex::new(HubInnen {
                quota: u64::MAX / 2,
                ..Default::default()
            }),
            vor_push: Mutex::new(Vec::new()),
            beim_reservieren: Mutex::new(Vec::new()),
            beim_hochladen: Mutex::new(Vec::new()),
        }
    }

    /// Läuft einmal während der nächsten Reservierung (der Nutzer handelt mitten im Lauf).
    fn beim_naechsten_reservieren(&self, h: impl FnOnce() + Send + 'static) {
        self.beim_reservieren.lock().unwrap().push(Box::new(h));
    }

    /// Läuft einmal während des nächsten hochgeladenen Stücks (der Nutzer handelt beim Upload).
    fn beim_naechsten_hochladen(&self, h: impl FnOnce() + Send + 'static) {
        self.beim_hochladen.lock().unwrap().push(Box::new(h));
    }

    /// Ein „Hub“ setzt einen Stand direkt (neue Revision, neuer Cursor).
    fn faelschen(&self, coll: &str, id: &str, f: impl FnOnce(&mut HubObj)) {
        self.mit(|i| {
            i.seq += 1;
            let seq = i.seq;
            let o = i.objekte.get_mut(&(coll.into(), id.into())).unwrap();
            f(o);
            o.revision += 1;
            o.server_seq = seq;
        });
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
        if self.abweisen.contains(&o.object_id) {
            return Self::abgewiesen(o, "payload_size");
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
        // wai-portal#23: Grabsteine der Ablage tragen eine versiegelte Nutzlast (≤ 16 384 Bytes).
        if (!o.deleted && o.payload.is_empty()) || (o.deleted && o.payload.len() > 16_384) {
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
            // Wiederholung nur mit identischer Nutzlast und Schlüsselkennung.
            if o.base_revision == zeile.revision - 1
                && o.deleted
                && o.payload == zeile.payload
                && o.key_id == zeile.key_id
            {
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
                payload: o.payload.clone(),
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
        let haken: Vec<GeraeteHaken> = std::mem::take(&mut *self.beim_hochladen.lock().unwrap());
        for h in haken {
            h();
        }
        self.mit(|i| {
            if i.zu_viele && !i.blobs.contains_key(blob_id) {
                return Err(HubError::Other("too_many_blobs: Zu viele Ablagen".into()));
            }
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
        let haken: Vec<GeraeteHaken> = std::mem::take(&mut *self.beim_reservieren.lock().unwrap());
        for h in haken {
            h();
        }
        self.mit(|i| {
            i.reserviert.push(blob_id.into());
            if i.fehl_reservieren.contains(blob_id) {
                return Err(HubError::Other("HTTP 503".into()));
            }
            let ablauf = i.uhr + 24 * STUNDE;
            i.reservierungen
                .insert((blob_id.into(), device_id.into()), ablauf);
            Ok(())
        })
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
        stand(&self.a, Some(&test_konto()))
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

/// Ein versiegelter Grabstein, wie ihn ein anderes Gerät des Kontos sendet.
fn grab_von_anderem_geraet(coll: &str, id: &str, rev: i64) -> PushObject {
    let key = schluessel_konto();
    PushObject {
        collection: coll.into(),
        object_id: id.into(),
        base_revision: rev,
        payload: grab_nutzlast(&key, USER, coll, id).unwrap(),
        key_id: crypto::key_id(&key),
        deleted: true,
        blob_refs: None,
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
                let plain = String::from_utf8(
                    crypto::open(
                        &key,
                        &crypto::aad_ablage(
                            USER,
                            &o.collection,
                            &o.object_id,
                            &refs,
                            &o.key_id,
                            o.deleted,
                        ),
                        &o.payload,
                    )
                    .unwrap(),
                )
                .unwrap();
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
        let grab = grab_von_anderem_geraet(SAMMLUNG_DATEI, &f2, rev);
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
        let grab = grab_von_anderem_geraet(SAMMLUNG_DATEI, &f2, rev);
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
    assert!(
        a.bestand().lokal.contains(&dat),
        "im Ablagestand festgehalten"
    );
    // Zurück auf sync: ohne Rückweg (läuft im App-Takt, siehe `rueckweg_*`) bleibt die Kennung lokal.
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
    let st = stand(&a.a, Some(&test_konto())).unwrap();
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

// ---------------------------------------------------------------------------
// Sicherheits-Review (Befunde 1–9)
// ---------------------------------------------------------------------------

fn sync_json(g: &Geraet) -> PathBuf {
    g.a.dir().join(ZUSTAND)
}

fn nur_geraet(g: &Geraet, ord: &str, an: bool) {
    g.a.ordner_einstellungen(
        ord,
        &AblageEinstellungen {
            nur_geraet: Some(an),
            ..Default::default()
        },
    )
    .unwrap();
}

fn grab_im_papierkorb(g: &Geraet, dat: &str) -> bool {
    std::fs::read_dir(g.a.dir().join("papierkorb"))
        .map(|r| {
            r.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("grab-") && n.ends_with(dat)
            })
        })
        .unwrap_or(false)
}

/// Befund 1: Die Kennzeichnung steht im Ablagestand, vor dem Push. Weder ein Absturz nach der Annahme am
/// Hub (alter `sync.json`), noch ein defekter `sync.json`, noch ein Kontowechsel lassen den eigenen
/// Grabstein die lokale Kopie löschen.
#[test]
fn festgehaltene_eintraege_ueberleben_absturz_defekten_zustand_und_kontowechsel() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Wird privat");
    let dat = a.ablegen("Brief.txt", "Brieftext", &ord);
    a.sync(&hub);
    b.sync(&hub);
    let alt = std::fs::read(sync_json(&a)).unwrap();
    nur_geraet(&a, &ord, true);
    // Write-ahead: festgehalten, bevor irgendein Abgleich lief.
    let lokal = a.bestand().lokal;
    assert!(lokal.contains(&ord) && lokal.contains(&dat), "{lokal:?}");
    a.sync(&hub);
    assert!(hub.objekt(SAMMLUNG_DATEI, &dat).unwrap().deleted);
    assert!(hub.objekt(SAMMLUNG_ORDNER, &ord).unwrap().deleted);
    let pruefen = |wann: &str| {
        assert_eq!(a.original(&dat).as_deref(), Some("Brieftext"), "{wann}");
        assert!(a.datei(&dat).is_some(), "{wann}");
        assert!(a.bestand().ordner.contains_key(&ord), "{wann}");
        assert_eq!(
            a.zustand_von(&dat),
            Some(AblageSyncZustand::NurGeraet),
            "{wann}"
        );
    };
    // 1. Absturz nach der Annahme am Hub, vor dem Speichern: der alte Zustand liegt wieder da.
    std::fs::write(sync_json(&a), &alt).unwrap();
    a.sync(&hub);
    pruefen("nach Absturz");
    // 2. Defekter Zustand.
    std::fs::write(sync_json(&a), b"{kaputt").unwrap();
    a.sync(&hub);
    pruefen("nach defektem sync.json");
    // 3. Kontowechsel (anderer Hub, gleiche Daten): der Zustand beginnt leer.
    let mut z = a.zustand();
    z.konto = "8:ffffffff:https://anderer.example.org".into();
    z.speichern(&a.a).unwrap();
    a.sync(&hub);
    pruefen("nach Kontowechsel");
    // Nie wieder gesendet.
    let staende = hub.mit(|i| {
        i.mitschnitt
            .iter()
            .filter(|o| (o.object_id == dat || o.object_id == ord) && !o.deleted)
            .count()
    });
    assert_eq!(
        staende, 2,
        "nur die ursprünglichen Stände vor dem Umschalten"
    );
    // B legt seine Kopie in den Papierkorb (30 Tage), nicht sofort weg.
    b.sync(&hub);
    assert!(b.datei(&dat).is_none());
    assert!(
        grab_im_papierkorb(&b, &dat),
        "Inhalt im Papierkorb aufgehoben"
    );
}

/// Befund 2 und 9: Ein Hub, der `deleted = true` fälscht (ohne Siegel oder mit dem Siegel eines
/// lebenden Stands), löscht nichts; ein versiegelter Grabstein gilt, der Inhalt bleibt 30 Tage im
/// Papierkorb und die Wartung entfernt ihn danach.
#[test]
fn gefaelschter_grabstein_loescht_nichts_echter_geht_in_den_papierkorb() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Akten");
    let f = a.ablegen("F.txt", "Inhalt F", &ord);
    let g = a.ablegen("G.txt", "Inhalt G", &ord);
    a.sync(&hub);
    b.sync(&hub);
    // Ohne Nutzlast.
    hub.faelschen(SAMMLUNG_DATEI, &f, |o| {
        o.deleted = true;
        o.payload = String::new();
        o.refs.clear();
    });
    // Mit dem Siegel des lebenden Stands (deleted steht in der AAD).
    hub.faelschen(SAMMLUNG_DATEI, &g, |o| {
        o.deleted = true;
        o.refs.clear();
    });
    let bericht = b.sync(&hub);
    assert_eq!(b.original(&f).as_deref(), Some("Inhalt F"));
    assert_eq!(b.original(&g).as_deref(), Some("Inhalt G"));
    assert!(b.datei(&f).is_some() && b.datei(&g).is_some());
    assert!(bericht.warnung.unwrap_or_default().contains("Siegel"));
    // Fixrunde 6: ein Grabstein ohne gültiges Siegel hält an, gilt aber nicht als gelöscht.
    for id in [&f, &g] {
        let o = b.zustand().objekt(SAMMLUNG_DATEI, id).cloned().unwrap();
        assert!(o.angehalten && !o.deleted, "{o:?}");
        assert_eq!(b.zustand_von(id), Some(AblageSyncZustand::Angehalten));
    }
    assert!(stand(&b.a, Some(&test_konto()))
        .unwrap()
        .fehler
        .unwrap_or_default()
        .contains("Siegel"));
    // Ein echter, versiegelter Grabstein (A löscht endgültig) gilt.
    let h = a.ablegen("H.txt", "Inhalt H", &ord);
    a.sync(&hub);
    b.sync(&hub);
    a.a.in_papierkorb(AblageArt::Datei, &h).unwrap();
    a.a.endgueltig_loeschen(AblageArt::Datei, &h).unwrap();
    a.sync(&hub);
    let o = hub.objekt(SAMMLUNG_DATEI, &h).unwrap();
    assert!(
        o.deleted && !o.payload.is_empty(),
        "Grabstein trägt ein Siegel"
    );
    b.sync(&hub);
    assert!(b.datei(&h).is_none());
    assert!(grab_im_papierkorb(&b, &h));
    // Nach der Frist räumt die Wartung.
    let korb = b.a.dir().join("papierkorb");
    for e in std::fs::read_dir(&korb).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if let Some(rest) = n.strip_prefix("grab-") {
            let (_, hinten) = rest.split_once('-').unwrap();
            std::fs::rename(e.path(), korb.join(format!("grab-1000-{hinten}"))).unwrap();
        }
    }
    assert_eq!(b.a.wartung(), None);
    assert!(!grab_im_papierkorb(&b, &h), "nach 30 Tagen entfernt");
}

/// Deterministisches Siegel: eine Wiederholung nach verlorener Antwort trägt dieselbe Nutzlast und
/// gilt beim Hub als angenommen (wai-portal#23).
#[test]
fn grabstein_wiederholung_traegt_dasselbe_siegel() {
    let key = schluessel_konto();
    let id = "dat_01JC0000000000000000000000";
    assert_eq!(
        grab_nutzlast(&key, USER, SAMMLUNG_DATEI, id).unwrap(),
        grab_nutzlast(&key, USER, SAMMLUNG_DATEI, id).unwrap()
    );
    assert_ne!(
        grab_nutzlast(&key, USER, SAMMLUNG_DATEI, id).unwrap(),
        grab_nutzlast(&key, USER, SAMMLUNG_ORDNER, id).unwrap()
    );
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("O");
    let dat = a.ablegen("D.txt", "d", &ord);
    a.sync(&hub);
    a.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    a.a.endgueltig_loeschen(AblageArt::Datei, &dat).unwrap();
    a.sync(&hub);
    // Antwort verloren: dieselbe Wiederholung (Basis rev-1, gleiches Siegel) gilt als angenommen.
    let rev = hub.objekt(SAMMLUNG_DATEI, &dat).unwrap().revision;
    let wiederholung = PushObject {
        collection: SAMMLUNG_DATEI.into(),
        object_id: dat.clone(),
        base_revision: rev - 1,
        payload: grab_nutzlast(&key, USER, SAMMLUNG_DATEI, &dat).unwrap(),
        key_id: crypto::key_id(&key),
        deleted: true,
        blob_refs: None,
    };
    let gesendet = hub.mit(|i| {
        i.mitschnitt
            .iter()
            .rev()
            .find(|o| o.object_id == dat && o.deleted)
            .map(|o| o.payload.clone())
    });
    assert_eq!(gesendet.as_deref(), Some(wiederholung.payload.as_str()));
    let r = hub.mit(|i| i.push_eins(&wiederholung, &a.id));
    assert_eq!(r.outcome, "accepted");
    let bericht = a.sync(&hub);
    assert_eq!(bericht.warnung, None);
    assert!(a.datei(&dat).is_none());
}

/// Befund 3: Eine Datei, die ein anderes Gerät vor dem Umschalten in den Ordner legte, die hier aber
/// erst danach ankommt, wird nie begraben; sie bleibt synchron (Ordner fehlt → „Wiederhergestellt“).
#[test]
fn spaet_eintreffende_datei_unter_lokalisiertem_ordner_bleibt_synchron() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Projekt");
    let f1 = a.ablegen("Alt.txt", "alt", &ord);
    a.sync(&hub);
    b.sync(&hub);
    // B legt F3 in den Ordner und gleicht ab; A hat das noch nicht gesehen und schaltet um.
    let f3 = b.ablegen("Neu.txt", "von B", &ord);
    b.sync(&hub);
    nur_geraet(&a, &ord, true);
    assert!(!a.bestand().lokal.contains(&f3));
    a.sync(&hub);
    a.sync(&hub);
    assert!(
        hub.objekt(SAMMLUNG_DATEI, &f1).unwrap().deleted,
        "F1 lokalisiert"
    );
    assert!(hub.objekt(SAMMLUNG_ORDNER, &ord).unwrap().deleted);
    let o3 = hub.objekt(SAMMLUNG_DATEI, &f3).unwrap();
    assert!(!o3.deleted, "F3 nie begraben");
    assert!(
        hub.mit(|i| !i.mitschnitt.iter().any(|o| o.object_id == f3 && o.deleted)),
        "kein Grabstein für F3 gesendet"
    );
    let ba = a.bestand();
    assert!(ba.befreit.contains(&f3));
    assert_eq!(ba.teilnahmen().get(&f3), Some(&Teilnahme::Sync));
    assert_eq!(a.original(&f3).as_deref(), Some("von B"));
    // Auf B fehlt der Ordner: F3 bleibt (wie „Wiederhergestellt“) und weiter synchron.
    b.sync(&hub);
    assert!(b.datei(&f3).is_some());
    assert!(b.datei(&f1).is_none());
    b.a.datei_umbenennen(&f3, "Neu2.txt").unwrap();
    b.sync(&hub);
    a.sync(&hub);
    assert_eq!(a.datei(&f3).unwrap().name.wert, "Neu2.txt");
    // Verschiebt A die befreite Datei in einen anderen lokalen Ordner, gilt dieser wieder.
    let privat = a.ordner("Privat");
    nur_geraet(&a, &privat, true);
    a.a.datei_verschieben(&f3, &privat).unwrap();
    assert!(a.bestand().lokal.contains(&f3));
}

/// Befund 4: Teilnahme wird unmittelbar vor Upload und Push neu geprüft.
#[test]
fn teilnahme_wird_vor_upload_und_push_neu_geprueft() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Bald privat");
    let dat = a.ablegen("Geheim.txt", "GEHEIM-spaet", &ord);
    let (ablage, ord2) = (a.a.clone(), ord.clone());
    // Mitten im Lauf (während der Reservierung) stellt der Nutzer den Ordner auf „nur dieses Gerät“.
    hub.beim_naechsten_reservieren(move || {
        ablage
            .ordner_einstellungen(
                &ord2,
                &AblageEinstellungen {
                    nur_geraet: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
    });
    a.sync(&hub);
    let gesendet: Vec<String> =
        hub.mit(|i| i.mitschnitt.iter().map(|o| o.object_id.clone()).collect());
    assert!(!gesendet.contains(&dat), "kein Push nach dem Umschalten");
    assert!(!gesendet.contains(&ord));
    assert_eq!(a.zustand_von(&dat), Some(AblageSyncZustand::NurGeraet));
    // Ein späterer Lauf lädt nichts mehr hoch.
    let uploads = hub.mit(|i| i.hochgeladen.len());
    a.sync(&hub);
    assert_eq!(hub.mit(|i| i.hochgeladen.len()), uploads);
}

/// Befund 5: Ein unlesbarer oder neuerer entfernter Stand wird nie überschrieben; unbekannte Felder
/// reisen mit.
#[test]
fn unlesbarer_stand_haelt_an_und_unbekannte_felder_reisen_mit() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let key = schluessel_konto();
    let kid = crypto::key_id(&key);
    let ord = a.ordner("Schema");
    let f = a.ablegen("F.txt", "Inhalt F", &ord);
    let g = a.ablegen("G.txt", "Inhalt G", &ord);
    a.sync(&hub);
    b.sync(&hub);
    // F: unlesbar (fremdes Chiffrat). G: neuere Felder. Ordner: neueres Schema.
    hub.faelschen(SAMMLUNG_DATEI, &f, |o| {
        o.payload = crypto::seal(&[9u8; KEY_LEN], b"x", b"{}").unwrap();
    });
    let neu_versiegeln = |coll: &str, id: &str, o: &mut HubObj, f: &dyn Fn(&mut Value)| {
        let aad = crypto::aad_ablage(USER, coll, id, &o.refs, &kid, false);
        let mut v: Value =
            serde_json::from_slice(&crypto::open(&key, &aad, &o.payload).unwrap()).unwrap();
        f(&mut v);
        o.payload = crypto::seal(&key, &aad, collect::canonical(&v).as_bytes()).unwrap();
    };
    hub.faelschen(SAMMLUNG_DATEI, &g, |o| {
        neu_versiegeln(SAMMLUNG_DATEI, &g, o, &|v| {
            v["zukunft"] = serde_json::json!({"feld": 1});
        })
    });
    hub.faelschen(SAMMLUNG_ORDNER, &ord, |o| {
        neu_versiegeln(SAMMLUNG_ORDNER, &ord, o, &|v| {
            v["v"] = serde_json::json!(2);
        })
    });
    let f_vorher = hub.objekt(SAMMLUNG_DATEI, &f).unwrap();
    b.sync(&hub);
    assert_eq!(b.zustand_von(&f), Some(AblageSyncZustand::Angehalten));
    assert_eq!(b.zustand_von(&ord), Some(AblageSyncZustand::Angehalten));
    // Lokale Änderungen auf B überschreiben nichts.
    b.a.datei_umbenennen(&f, "F2.txt").unwrap();
    b.a.ordner_umbenennen(&ord, "Schema2").unwrap();
    b.sync(&hub);
    let f_nachher = hub.objekt(SAMMLUNG_DATEI, &f).unwrap();
    assert_eq!(f_nachher.payload, f_vorher.payload, "F nicht überschrieben");
    assert!(
        hub.mit(|i| i.fertig(&hid("Inhalt F"))),
        "keine Blob-Löschung für ruhende Objekte"
    );
    assert!(hub.mit(|i| !i
        .mitschnitt
        .iter()
        .any(|o| o.object_id == ord && o.base_revision >= 2)));
    // G: das unbekannte Feld bleibt erhalten und reist mit der nächsten Änderung zurück.
    assert_eq!(
        b.datei(&g).unwrap().extra.get("zukunft"),
        Some(&serde_json::json!({"feld": 1}))
    );
    b.a.datei_umbenennen(&g, "G2.txt").unwrap();
    b.sync(&hub);
    let klar = mitschnitt_klartexte(&hub)
        .into_iter()
        .rev()
        .find(|(_, id, _)| *id == g)
        .unwrap()
        .2;
    assert!(
        klar.contains("G2.txt") && klar.contains("zukunft"),
        "{klar}"
    );
}

/// Befund 6: Zwei Geräte mit je eigenem Standardordner haben nach dem ersten Abgleich ein „Allgemein“.
#[test]
fn zwei_geraete_ein_allgemein() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let allgemein = |g: &Geraet| -> Vec<String> {
        g.bestand()
            .ordner
            .values()
            .filter(|o| o.name.wert == crate::ablage::speicher::STANDARD_ORDNER)
            .map(|o| o.id.clone())
            .collect()
    };
    let vorher_a = allgemein(&a);
    assert_eq!(vorher_a.len(), 1);
    assert_ne!(vorher_a, allgemein(&b));
    let dat = a.ablegen("Notiz.txt", "n", &vorher_a[0]);
    a.sync(&hub);
    b.sync(&hub);
    a.sync(&hub);
    let ziel = standard_id(&schluessel_konto());
    assert!(crate::ablage::id_gueltig(
        crate::ablage::PRAEFIX_ORDNER,
        &ziel
    ));
    assert_eq!(allgemein(&a), vec![ziel.clone()]);
    assert_eq!(allgemein(&b), vec![ziel.clone()]);
    assert_eq!(b.datei(&dat).unwrap().ordner.wert, ziel, "Datei umgehängt");
    let am_hub = hub.mit(|i| {
        i.objekte
            .keys()
            .filter(|(c, _)| c == SAMMLUNG_ORDNER)
            .count()
    });
    assert_eq!(am_hub, 1, "ein Ordner am Hub");
}

/// Befund 7: Einzelfehler stellen nur das Objekt zurück; Abweisungen ruhen ohne neue Reservierung;
/// „Zu viele Ablagen“ ist ein eigener Stand.
#[test]
fn einzelfehler_stellen_nur_das_objekt_zurueck() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Fehler");
    let f1 = a.ablegen("Eins.txt", "eins", &ord);
    let f2 = a.ablegen("Zwei.txt", "zwei", &ord);
    hub.mit(|i| {
        i.fehl_reservieren.insert(hid("eins"));
    });
    a.sync(&hub);
    assert!(
        hub.objekt(SAMMLUNG_DATEI, &f1).is_none(),
        "F1 zurückgestellt"
    );
    assert!(
        hub.objekt(SAMMLUNG_DATEI, &f2).is_some(),
        "F2 trotzdem gesendet"
    );
    hub.mit(|i| i.fehl_reservieren.clear());
    a.sync(&hub);
    assert!(hub.objekt(SAMMLUNG_DATEI, &f1).is_some());
    // Abweisung: Rückstellung, beim nächsten Lauf weder Push noch Reservierung.
    let f3 = a.ablegen("Drei.txt", "drei", &ord);
    hub.mit(|i| {
        i.abweisen.insert(f3.clone());
    });
    a.sync(&hub);
    let r = a.zustand().rueck[&format!("{SAMMLUNG_DATEI}/{f3}")];
    assert!(r.bis_ms > crate::ablage::jetzt_ms());
    let (pushes, res) = hub.mit(|i| {
        (
            i.mitschnitt.iter().filter(|o| o.object_id == f3).count(),
            i.reserviert.iter().filter(|b| **b == hid("drei")).count(),
        )
    });
    a.sync(&hub);
    assert_eq!(
        hub.mit(|i| i.mitschnitt.iter().filter(|o| o.object_id == f3).count()),
        pushes
    );
    assert_eq!(
        hub.mit(|i| i.reserviert.iter().filter(|b| **b == hid("drei")).count()),
        res
    );
    assert!(rueck_dauer(30) <= RUECK_MAX_MS);
    // Zu viele Ablagen: eigener Stand, kein „Speicher voll“.
    hub.mit(|i| i.zu_viele = true);
    let f4 = a.ablegen("Vier.txt", "vier", &ord);
    let bericht = a.sync(&hub);
    assert!(bericht.warnung.unwrap_or_default().contains("zu viele"));
    let z = a.zustand();
    assert!(z.zu_viele.is_some() && !z.kontingent_voll);
    assert!(hub.objekt(SAMMLUNG_DATEI, &f4).is_none());
}

/// Befund 8: Eigene Blobs ohne lebende Fassung werden nach 7 Tagen bedingt gelöscht.
#[test]
fn verwaiste_eigene_blobs_werden_nach_sieben_tagen_geloescht() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Waisen");
    let dat = a.ablegen("W.txt", "verwaist", &ord);
    // Upload gelingt, das Objekt wird abgewiesen; danach löscht der Nutzer die Datei endgültig.
    hub.mit(|i| {
        i.abweisen.insert(dat.clone());
    });
    a.sync(&hub);
    let blob = hid("verwaist");
    assert!(hub.mit(|i| i.fertig(&blob)));
    assert!(
        a.bestand().sync_blobs.contains_key(&blob),
        "dauerhaft vermerkt"
    );
    a.a.in_papierkorb(AblageArt::Datei, &dat).unwrap();
    a.a.endgueltig_loeschen(AblageArt::Datei, &dat).unwrap();
    a.sync(&hub);
    assert!(hub.mit(|i| i.fertig(&blob)), "jünger als 7 Tage: bleibt");
    // 8 Tage später.
    let pfad = a.a.dir().join("ablage.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&pfad).unwrap()).unwrap();
    v["sync_blobs"][&blob]["ms"] =
        serde_json::json!(crate::ablage::jetzt_ms() - 8 * 24 * 3600 * 1000);
    std::fs::write(&pfad, serde_json::to_vec(&v).unwrap()).unwrap();
    a.sync(&hub);
    assert!(
        hub.mit(|i| !i.blobs.contains_key(&blob)),
        "verwaister Blob gelöscht"
    );
    assert!(!a.bestand().sync_blobs.contains_key(&blob));
}

/// Hub-Vertrag (wai-portal#25): nur der Fehlercode `too_many_blobs` zählt, nie der Meldungstext.
#[test]
fn zu_viele_ablagen_nur_am_fehlercode() {
    assert!(ist_zu_viele("too_many_blobs: Zu viele Ablagen"));
    assert!(ist_zu_viele("too_many_blobs"));
    assert!(!ist_zu_viele("quota_exceeded: Zu viele Ablagen"));
    assert!(!ist_zu_viele("quota_exceeded: too_many_blobs"));
    assert!(!ist_zu_viele("HTTP 507"));
}

fn rueckweg_aufbau() -> (FakeHub, Geraet, Geraet, String, String, String, String) {
    use crate::ablage::freigabe::KiStufe;
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let x = a.ordner("Akte");
    let u = a.a.ordner_anlegen(Some(&x), "Unterlagen").unwrap();
    let f1 = a.ablegen("Eins.txt", "eins", &x);
    let f2 = a.ablegen("Zwei.txt", "zwei", &u);
    a.a.ki_setzen(&x, Some(KiStufe::An)).unwrap();
    a.sync(&hub);
    b.sync(&hub);
    assert_eq!(b.original(&f2).as_deref(), Some("zwei"));
    (hub, a, b, x, u, f1, f2)
}

/// Spec §9 Tests D4a: Rückweg lokal → sync mit Abbruch nach jedem Journalschritt; der Projektverweis
/// zeigt danach auf die neue Kennung, die alte leitet weiter, und das andere Gerät bekommt den
/// Teilbaum unter den neuen Kennungen.
#[test]
fn rueckweg_lokal_zu_sync_mit_abbruch_nach_jedem_journalschritt() {
    use crate::ablage::freigabe;
    use crate::ablage::speicher::{ProjektVerweise, Schritt, ABBRUCH, TEST_ABBRUCH};
    use crate::projekte::{herkunft::ablage_link, kontext};
    let punkte = [
        None,
        Some(Schritt::UebergangBegonnen),
        Some(Schritt::UebergangDateien),
        Some(Schritt::UebergangUmgezogen),
        Some(Schritt::UebergangFreigaben),
        Some(Schritt::UebergangUmgeschrieben),
        Some(Schritt::UebergangFertig),
    ];
    for punkt in punkte {
        let (hub, a, b, x, u, f1, f2) = rueckweg_aufbau();
        let projekte = a._t.path().join("projekte");
        assert!(kontext::anlegen(&projekte, "prj_test", "mit_1", &ablage_link(&f1), true).unwrap());
        nur_geraet(&a, &x, true);
        a.sync(&hub);
        assert!(hub.objekt(SAMMLUNG_DATEI, &f1).unwrap().deleted);
        assert!(hub.objekt(SAMMLUNG_ORDNER, &x).unwrap().deleted);
        b.sync(&hub);
        assert!(b.datei(&f1).is_none());
        nur_geraet(&a, &x, false);
        let verweise = ProjektVerweise::neu(&projekte);
        if let Some(p) = punkt {
            ABBRUCH.with(|c| c.set(Some(p)));
            let e = a.a.rueckweg(&verweise);
            ABBRUCH.with(|c| c.set(None));
            assert_eq!(e.unwrap_err().code, TEST_ABBRUCH, "{p:?}");
            assert_eq!(a.a.uebergaenge_offen(), 1, "{p:?}: Journal bleibt");
            // Neustart: die Wartung führt den lokalen Teil zu Ende; nichts erscheint als Fund.
            a.a.prozess_neustart();
            assert_eq!(a.a.wartung(), None, "{p:?}");
            let sicht = a.a.ansicht().unwrap();
            assert!(sicht.gefunden.is_empty(), "{p:?}: {:?}", sicht.gefunden);
            assert_eq!(a.original(&f1).as_deref(), Some("eins"), "{p:?}");
        }
        let n = a.a.rueckweg(&verweise).unwrap();
        let wann = format!("{punkt:?}");
        assert_eq!(a.a.uebergaenge_offen(), 0, "{wann}");
        if punkt.is_none() {
            assert_eq!(n, 4, "X, U, zwei Dateien");
        } else {
            assert_eq!(n, 0, "{wann}: fortgesetzt, nicht neu begonnen");
        }
        let [nx, nu, n1, n2] = [&x, &u, &f1, &f2].map(|id| a.a.aufloesen(id).unwrap());
        for (alt, neu) in [(&x, &nx), (&u, &nu), (&f1, &n1), (&f2, &n2)] {
            assert_ne!(alt, neu, "{wann}");
        }
        let bestand = a.bestand();
        assert!(bestand.lokal.is_empty(), "{wann}: {:?}", bestand.lokal);
        assert!(!bestand.dateien.contains_key(&f1) && !bestand.ordner.contains_key(&x));
        assert_eq!(bestand.dateien[&n1].ordner.wert, nx, "{wann}");
        assert_eq!(bestand.dateien[&n2].ordner.wert, nu, "{wann}");
        assert_eq!(
            bestand.ordner[&nu].eltern.wert.as_deref(),
            Some(nx.as_str())
        );
        // Originale umgezogen; die alte Kennung leitet weiter (Deep Link).
        assert_eq!(a.original(&n2).as_deref(), Some("zwei"), "{wann}");
        assert_eq!(a.original(&f1).as_deref(), Some("eins"), "{wann}");
        assert!(!a.a.dir().join("dateien").join(&f1).exists(), "{wann}");
        // KI-Freigabe und Projektverweis folgen.
        match freigabe::laden(a.a.dir()) {
            freigabe::Lage::Gueltig(f) => {
                assert!(
                    f.ordner.contains_key(&nx) && !f.ordner.contains_key(&x),
                    "{wann}"
                )
            }
            freigabe::Lage::Unklar(_) => panic!("{wann}: Freigabe unklar"),
        }
        assert_eq!(
            kontext::link_von(&projekte, "mit_1"),
            Some(ablage_link(&n1)),
            "{wann}"
        );
        assert_eq!(a.zustand_von(&n1), Some(AblageSyncZustand::Ausstehend));
        // Abgleich: neue Kennungen am Hub und auf B, alte bleiben begraben.
        a.sync(&hub);
        b.sync(&hub);
        assert_eq!(b.original(&n1).as_deref(), Some("eins"), "{wann}");
        assert_eq!(b.original(&n2).as_deref(), Some("zwei"), "{wann}");
        assert_eq!(b.datei(&n2).unwrap().ordner.wert, nu);
        assert!(b.datei(&f1).is_none() && !b.bestand().ordner.contains_key(&x));
        assert!(hub.objekt(SAMMLUNG_DATEI, &f1).unwrap().deleted);
        assert!(!hub.objekt(SAMMLUNG_DATEI, &n1).unwrap().deleted);
        assert_eq!(a.zustand_von(&n1), Some(AblageSyncZustand::Synchronisiert));
        // Wiederholt: nichts mehr zu tun.
        assert_eq!(a.a.rueckweg(&verweise).unwrap(), 0, "{wann}");
        assert_eq!(
            kontext::link_von(&projekte, "mit_1"),
            Some(ablage_link(&n1))
        );
    }
}

/// Zurück auf synchron, bevor der Grabstein der alten Kennungen gesendet war: der Lauf begräbt die alten
/// Kennungen am Hub (Dateien vor Ordnern), B bekommt nur die neuen, und ein später eintreffender Stand
/// zur alten Kennung wird nicht wieder aufgenommen.
#[test]
fn rueckweg_vor_dem_abgleich_begraebt_alte_kennungen() {
    use crate::ablage::speicher::ProjektVerweise;
    let (hub, a, b, x, u, f1, f2) = rueckweg_aufbau();
    nur_geraet(&a, &x, true);
    nur_geraet(&a, &x, false);
    let projekte = a._t.path().join("projekte");
    assert_eq!(a.a.rueckweg(&ProjektVerweise::neu(&projekte)).unwrap(), 4);
    let [nx, n1, n2] = [&x, &f1, &f2].map(|id| a.a.aufloesen(id).unwrap());
    a.sync(&hub);
    for (coll, alt) in [
        (SAMMLUNG_ORDNER, &x),
        (SAMMLUNG_ORDNER, &u),
        (SAMMLUNG_DATEI, &f1),
        (SAMMLUNG_DATEI, &f2),
    ] {
        assert!(hub.objekt(coll, alt).unwrap().deleted, "{alt} begraben");
    }
    b.sync(&hub);
    assert_eq!(b.original(&n1).as_deref(), Some("eins"));
    assert_eq!(b.original(&n2).as_deref(), Some("zwei"));
    assert_eq!(b.datei(&n1).unwrap().ordner.wert, nx);
    assert!(b.datei(&f1).is_none() && b.datei(&f2).is_none());
    // Ein Stand zur alten Kennung (z. B. von einem Gerät, das noch offline war) belebt die alte Kennung
    // nicht, sondern wird mit der neuen zusammengeführt.
    let e = a.a.sync_einspielen(&[Eingang::Datei({
        let mut d = b.datei(&n1).unwrap();
        d.id = f1.clone();
        d
    })]);
    assert!(e.unwrap().geaendert.iter().all(|id| *id == n1));
    assert!(a.datei(&f1).is_none() && a.datei(&n1).is_some());
}

/// Fixrunde 3: Ein Gerät, das die alte Kennung noch hatte, benennt die Datei um, bevor A den Rückweg
/// abgleicht. A bildet den gezogenen Stand auf die neue Kennung ab (Name kommt mit, Ordner ebenso),
/// sendet dann den Grabstein der alten; B bekommt die neue Kennung mit dem neuen Namen.
#[test]
fn rueckweg_fuehrt_staende_alter_kennungen_mit_der_neuen_zusammen() {
    use crate::ablage::speicher::ProjektVerweise;
    let (hub, a, b, x, u, f1, _f2) = rueckweg_aufbau();
    nur_geraet(&a, &x, true);
    nur_geraet(&a, &x, false);
    // B (noch mit den alten Kennungen) benennt um und verschiebt in den Unterordner.
    b.a.datei_umbenennen(&f1, "Eins neu.txt").unwrap();
    b.a.datei_verschieben(&f1, &u).unwrap();
    b.sync(&hub);
    assert!(!hub.objekt(SAMMLUNG_DATEI, &f1).unwrap().deleted);
    let projekte = a._t.path().join("projekte");
    a.a.rueckweg(&ProjektVerweise::neu(&projekte)).unwrap();
    let [n1, nu] = [&f1, &u].map(|id| a.a.aufloesen(id).unwrap());
    a.sync(&hub);
    let d = a.datei(&n1).unwrap();
    assert_eq!(d.name.wert, "Eins neu.txt");
    assert_eq!(d.ordner.wert, nu, "Ordnerverweis folgt der Weiterleitung");
    assert!(a.datei(&f1).is_none());
    assert!(hub.objekt(SAMMLUNG_DATEI, &f1).unwrap().deleted);
    b.sync(&hub);
    let d = b.datei(&n1).unwrap();
    assert_eq!(d.name.wert, "Eins neu.txt");
    assert_eq!(d.ordner.wert, nu);
    assert_eq!(b.original(&n1).as_deref(), Some("eins"));
    assert!(b.datei(&f1).is_none());
}

/// Fixrunde 1: Ein Objekt mit fremdem Schlüssel ruht wie ein unlesbares; der Pull läuft weiter (auch
/// `ablage_datei` nach `ablage_ordner`), ist aber nicht vollständig.
#[test]
fn fremder_schluessel_haelt_nur_das_objekt_an() {
    let hub = FakeHub::neu();
    let (a, b) = (geraet("a"), geraet("b"));
    let ord = a.ordner("Schlüssel");
    let f = a.ablegen("F.txt", "Inhalt F", &ord);
    let g = a.ablegen("G.txt", "Inhalt G", &ord);
    a.sync(&hub);
    b.sync(&hub);
    hub.faelschen(SAMMLUNG_ORDNER, &ord, |o| o.key_id = "fremd".into());
    a.a.datei_umbenennen(&g, "G neu.txt").unwrap();
    a.sync(&hub);
    hub.faelschen(SAMMLUNG_DATEI, &f, |o| o.key_id = "fremd".into());
    let bericht = b.sync(&hub);
    assert!(!bericht.vollstaendig);
    assert!(bericht
        .warnung
        .unwrap_or_default()
        .contains("anderem Schlüssel"));
    assert_eq!(
        b.datei(&g).unwrap().name.wert,
        "G neu.txt",
        "Pull ging weiter"
    );
    assert_eq!(b.zustand_von(&f), Some(AblageSyncZustand::Angehalten));
    assert_eq!(b.zustand_von(&ord), Some(AblageSyncZustand::Angehalten));
    let z = b.zustand();
    assert!(z.cursor.get(SAMMLUNG_DATEI).copied().unwrap_or(0) > 0);
    // Nichts überschrieben, keine Blob-Löschung für das ruhende Objekt.
    let vorher = hub.objekt(SAMMLUNG_DATEI, &f).unwrap();
    b.a.datei_umbenennen(&f, "F2.txt").unwrap();
    b.sync(&hub);
    assert_eq!(
        hub.objekt(SAMMLUNG_DATEI, &f).unwrap().payload,
        vorher.payload
    );
    assert!(hub.mit(|i| i.fertig(&hid("Inhalt F"))));
}

/// Fixrunde 2: Wird ein Eintrag während des Hochladens lokal, geht die Kopie sofort wieder weg und
/// wird weder als hochgeladen noch als eigener Blob vermerkt.
#[test]
fn upload_waehrend_wechsel_zu_nur_geraet_wird_sofort_geloescht() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Bald privat");
    let dat = a.ablegen("Geheim.txt", "GEHEIM-upload", &ord);
    let (ablage, ord2) = (a.a.clone(), ord.clone());
    hub.beim_naechsten_hochladen(move || {
        ablage
            .ordner_einstellungen(
                &ord2,
                &AblageEinstellungen {
                    nur_geraet: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();
    });
    a.sync(&hub);
    let h = hid("GEHEIM-upload");
    assert!(hub.mit(|i| i.hochgeladen.contains(&h)), "Upload lief");
    assert!(
        hub.mit(|i| !i.blobs.contains_key(&h)),
        "Kopie sofort gelöscht"
    );
    assert!(!a.zustand().hochgeladen.contains(&h));
    assert!(!a.bestand().sync_blobs.contains_key(&h));
    assert!(hub.mit(|i| !i.mitschnitt.iter().any(|o| o.object_id == dat)));
    assert_eq!(a.zustand_von(&dat), Some(AblageSyncZustand::NurGeraet));
}

/// Fixrunde 7: Nach einem erfolgreichen Upload ruht das Hochladen nicht mehr wegen „zu vieler Ablagen“.
#[test]
fn zu_viele_ablagen_endet_mit_erfolgreichem_upload() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Viele");
    hub.mit(|i| i.zu_viele = true);
    a.ablegen("Eins.txt", "viele-eins", &ord);
    a.sync(&hub);
    assert!(a.zustand().zu_viele.is_some());
    hub.mit(|i| i.zu_viele = false);
    // Rückstellung abgelaufen (Uhr des Zustands zurückgedreht), dann gelingt der Upload.
    let mut z = a.zustand();
    z.zu_viele = z.zu_viele.map(|mut r| {
        r.bis_ms = 0;
        r
    });
    z.speichern(&a.a).unwrap();
    a.sync(&hub);
    assert!(a.zustand().zu_viele.is_none());
}

/// Fixrunde 5: Teilnahme `Halten` erscheint als „angehalten“, nicht als „nur dieses Gerät“; ein
/// Zustand eines anderen Kontos zählt in der Anzeige nicht.
#[test]
fn stand_zeigt_halten_als_angehalten_und_prueft_das_konto() {
    let hub = FakeHub::neu();
    let a = geraet("a");
    let ord = a.ordner("Halten");
    let dat = a.ablegen("H.txt", "halten", &ord);
    a.sync(&hub);
    assert_eq!(a.zustand_von(&dat), Some(AblageSyncZustand::Synchronisiert));
    let anderes = konto_kennung(USER + 1, &schluessel_konto(), HUB_URL);
    let st = stand(&a.a, Some(&anderes)).unwrap();
    let z = st.eintraege.iter().find(|e| e.id == dat).unwrap().zustand;
    assert_eq!(
        z,
        AblageSyncZustand::Ausstehend,
        "fremder Zustand gilt nicht"
    );
    assert!(st.belegt.is_none() && st.fehler.is_none());
    let mut d = a.datei(&dat).unwrap();
    d.klasse_min = zf::Register::neu("zukunft".into(), 999, "g_zukunft");
    let b = SyncBestand {
        dateien: [(dat.clone(), d)].into(),
        ordner: a.bestand().ordner,
        ..Default::default()
    };
    let zst = zustaende(&b, &Zustand::default());
    let z = zst.iter().find(|e| e.id == dat).unwrap().zustand;
    assert_eq!(z, AblageSyncZustand::Angehalten);
}

/// Fixrunde 4: Unklare Freigabe oder defekter Schalterbestand der Projekte halten den Rückweg im
/// Journal an (Fehler statt stillem Erfolg); sind beide wieder lesbar, schreibt der nächste Lauf alles
/// um. Ein fehlender Schalterbestand ist kein Fehler.
#[test]
fn rueckweg_wiederholt_bei_unklarer_freigabe_und_defektem_kontext() {
    use crate::ablage::freigabe;
    use crate::ablage::speicher::ProjektVerweise;
    use crate::projekte::{herkunft::ablage_link, kontext};
    let (_hub, a, _b, x, _u, f1, _f2) = rueckweg_aufbau();
    let projekte = a._t.path().join("projekte");
    assert!(kontext::anlegen(&projekte, "prj_test", "mit_1", &ablage_link(&f1), true).unwrap());
    let verweise = ProjektVerweise::neu(&projekte);
    nur_geraet(&a, &x, true);
    nur_geraet(&a, &x, false);
    // Freigabe von einer neueren Version: unklar.
    let fpfad = a.a.dir().join(freigabe::DATEI);
    let fgut = std::fs::read(&fpfad).unwrap();
    std::fs::write(&fpfad, br#"{"v": 99, "epoche": 1}"#).unwrap();
    assert!(a.a.rueckweg(&verweise).is_err());
    assert_eq!(a.a.uebergaenge_offen(), 1);
    std::fs::write(&fpfad, &fgut).unwrap();
    // Schalterbestand defekt.
    let kpfad = projekte.join(kontext::DATEI);
    let kgut = std::fs::read(&kpfad).unwrap();
    std::fs::write(&kpfad, b"{kaputt").unwrap();
    assert!(a.a.rueckweg(&verweise).is_err());
    assert_eq!(a.a.uebergaenge_offen(), 1);
    std::fs::write(&kpfad, &kgut).unwrap();
    a.a.rueckweg(&verweise).unwrap();
    assert_eq!(a.a.uebergaenge_offen(), 0);
    let [nx, n1] = [&x, &f1].map(|id| a.a.aufloesen(id).unwrap());
    assert_eq!(
        kontext::link_von(&projekte, "mit_1"),
        Some(ablage_link(&n1))
    );
    match freigabe::laden(a.a.dir()) {
        freigabe::Lage::Gueltig(f) => assert!(f.ordner.contains_key(&nx)),
        freigabe::Lage::Unklar(_) => panic!("Freigabe unklar"),
    }
    // Ohne Schalterbestand: nichts umzuschreiben, kein Fehler.
    assert_eq!(
        kontext::links_umschreiben(&a._t.path().join("fehlt"), &[]),
        Ok(0)
    );
}
