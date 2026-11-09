//! D4a/D4b: Abgleich der Dokumentenablage über den eigenen Hub (Spec
//! `2026-11-08-dokumentenablage.md` §2/§6, wai-sdk ADR-0029 E4/E8/E9), Ende-zu-Ende verschlüsselt.
//!
//! * Ein Objekt je Ordner (`ablage_ordner`) und je Datei (`ablage_datei`); Nutzlast ist der Datensatz aus
//!   §2 als JSON, versiegelt wie alle Sync-Objekte. Bei `ablage_datei` ist das sichtbare Steuerfeld
//!   `blob_refs` (Hub-Kennungen aller lebenden Originale und Texte) in die AAD gebunden
//!   ([`crypto::aad_ablage`]). In der Nutzlast stehen die Inhalts-Hashes (`blob`); die Hub-Kennung
//!   ergibt sich daraus mit dem Kontoschlüssel ([`blobs::blob_id`]).
//! * Nie übertragen: Unterbäume mit „nur dieses Gerät“, Klasse `personal` und `confidential` (Cloud-
//!   Ausnahmen für `confidential` sind noch nicht modelliert, also bleibt `confidential` lokal),
//!   `freigabe.json`, `index.db`, dieser Zustand (`sync.json`).
//! * Reihenfolge je Datei: neue Blobs reservieren → Original (und Text, D4b) hochladen oder vorhandenen
//!   nutzen → Objekt-Push mit `blob_refs`. `blob_missing` → neu hochladen und erneut pushen;
//!   `superseded` → entfernten Stand zusammenführen (nie überschreiben; Konflikte werden Köpfe) und mit
//!   neuer Basis erneut pushen.
//! * Pull je Sammlung mit eigenem Cursor (Ordner vor Dateien), Zusammenführen über
//!   [`zf`](crate::ablage::zusammenfuehren), danach Originale und Texte herunterladen (Texte zuerst),
//!   `textrev` und `original_sha256` prüfen, Index abgleichen.
//! * Grabsteine sind endgültig: lokal endgültig gelöscht → Grabstein; empfangen → Original, Texte, Index
//!   weg. Blob-Löschung nur nach vollständigem Pull, nur ohne lebende Fassung im Bestand, nur bedingt
//!   (409 wird hingenommen).
//! * Wechsel sync → „nur dieses Gerät“ (oder Klasse `personal`/`confidential`): erst wenn alle Originale
//!   hier liegen, Grabstein; die Kennung bleibt lokal (`lokalisiert`) und wird nie wieder gesendet.
//!   Der Rückweg lokal → sync (neue IDs, Übergangsjournal) ist noch offen: solche Einträge bleiben
//!   „nur dieses Gerät“.
//!
//! Datenschutz: ins Log nur Kennungen und Codes, nie Namen oder Inhalte.

use super::blobs::{self, BlobTransport, Source, Upload};
use super::client::{
    BlobLoeschung, BlobVerbrauch, HubClient, HubError, PullResponse, PushObject, PushResult,
};
use super::collect;
use super::crypto::{self, KEY_LEN};
use crate::ablage::speicher::{Ablage, Eingang, SyncBestand};
use crate::ablage::zusammenfuehren::{self as zf, Datei, Eltern, Klasse, Ordner};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const SAMMLUNG_ORDNER: &str = "ablage_ordner";
pub const SAMMLUNG_DATEI: &str = "ablage_datei";
/// Gerätelokaler Zustand des Abgleichs unter `ablage/` (nie synchronisiert).
pub const ZUSTAND: &str = "sync.json";
const ZUSTAND_VERSION: u32 = 1;
const PULL_LIMIT: u32 = 200;
const PULL_MAX_SEITEN: usize = 1000;
const PUSH_STAPEL: usize = 40;
/// Klartext-Bytes je Lauf zum Hoch- bzw. Herunterladen (mindestens eine Datei je Lauf).
pub const UPLOAD_BUDGET: u64 = 256 * 1024 * 1024;
pub const DOWNLOAD_BUDGET: u64 = 256 * 1024 * 1024;
/// Runden je Lauf für `superseded`/`blob_missing`.
const MAX_RUNDEN: usize = 4;

pub fn ist_ablage(collection: &str) -> bool {
    collection == SAMMLUNG_ORDNER || collection == SAMMLUNG_DATEI
}

// ---------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------

/// Die Routen, die der Abgleich braucht. Der echte Hub ist [`HubClient`]; die Tests setzen einen
/// nachgebauten Hub mit derselben Semantik ein (`ablage_tests.rs`).
#[async_trait::async_trait]
pub trait AblageHub: BlobTransport {
    async fn push(
        &self,
        device_id: &str,
        objects: &[PushObject],
    ) -> Result<Vec<PushResult>, HubError>;
    async fn pull_sammlung(
        &self,
        since: i64,
        limit: u32,
        collection: &str,
    ) -> Result<PullResponse, HubError>;
    async fn reservieren(&self, blob_id: &str, device_id: &str) -> Result<(), HubError>;
    async fn blob_loeschen(
        &self,
        blob_id: &str,
        device_id: &str,
    ) -> Result<BlobLoeschung, HubError>;
    async fn verbrauch(&self) -> Result<BlobVerbrauch, HubError>;
}

#[async_trait::async_trait]
impl AblageHub for HubClient {
    async fn push(
        &self,
        device_id: &str,
        objects: &[PushObject],
    ) -> Result<Vec<PushResult>, HubError> {
        HubClient::push(self, device_id, objects).await
    }
    async fn pull_sammlung(
        &self,
        since: i64,
        limit: u32,
        collection: &str,
    ) -> Result<PullResponse, HubError> {
        self.pull_collection(since, limit, collection).await
    }
    async fn reservieren(&self, blob_id: &str, device_id: &str) -> Result<(), HubError> {
        self.reserve_blob(blob_id, device_id).await
    }
    async fn blob_loeschen(
        &self,
        blob_id: &str,
        device_id: &str,
    ) -> Result<BlobLoeschung, HubError> {
        self.delete_blob(blob_id, device_id).await
    }
    async fn verbrauch(&self) -> Result<BlobVerbrauch, HubError> {
        self.blob_usage().await
    }
}

/// Konto und Gerät eines Laufs.
pub struct Konto<'a> {
    pub key: &'a [u8; KEY_LEN],
    pub user_id: i64,
    pub device_id: &'a str,
    /// Adresse des Hubs (Teil der Kontobindung des Zustands).
    pub hub_url: &'a str,
}

// ---------------------------------------------------------------------------
// Zustand (gerätelokal)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjStand {
    pub revision: i64,
    /// Hash des Stands, den der Hub hat (leer beim Grabstein oder unlesbaren Objekt).
    pub hash: String,
    #[serde(default)]
    pub deleted: bool,
    /// Hub-Kennungen, die dieses Objekt am Hub referenziert.
    #[serde(default)]
    pub refs: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Zustand {
    #[serde(default)]
    pub v: u32,
    /// Benutzer und Schlüsselkennung: ein anderes Konto beginnt mit leerem Zustand.
    #[serde(default)]
    pub konto: String,
    #[serde(default)]
    pub cursor: BTreeMap<String, i64>,
    #[serde(default)]
    pub objekte: BTreeMap<String, ObjStand>,
    /// Kennungen, die dieses Gerät beim Wechsel zu „nur dieses Gerät“ am Hub begraben hat; sie bleiben
    /// lokal und werden nie wieder gesendet (Rückweg mit neuen IDs: offen).
    #[serde(default)]
    pub lokalisiert: BTreeSet<String>,
    /// Blobs, die dieses Gerät abgeschlossen am Hub weiß.
    #[serde(default)]
    pub hochgeladen: BTreeSet<String>,
    /// Kandidaten für die bedingte Blob-Löschung.
    #[serde(default)]
    pub blob_loeschen: BTreeSet<String>,
    #[serde(default)]
    pub verbrauch: Option<BlobVerbrauch>,
    #[serde(default)]
    pub kontingent_voll: bool,
    #[serde(default)]
    pub letzter_lauf_ms: Option<i64>,
    #[serde(default)]
    pub fehler: Option<String>,
}

fn schluessel(collection: &str, id: &str) -> String {
    format!("{collection}/{id}")
}

impl Zustand {
    pub fn laden(ablage: &Ablage) -> Zustand {
        std::fs::read(ablage.dir().join(ZUSTAND))
            .ok()
            .and_then(|r| serde_json::from_slice::<Zustand>(&r).ok())
            .filter(|z| z.v <= ZUSTAND_VERSION)
            .unwrap_or_default()
    }

    pub fn speichern(&self, ablage: &Ablage) -> Result<(), HubError> {
        std::fs::create_dir_all(ablage.dir())
            .map_err(|_| HubError::Other("ablage.speicher: Ablageordner nicht anlegbar.".into()))?;
        let roh = serde_json::to_vec_pretty(self)
            .map_err(|_| HubError::Other("ablage.speicher: Zustand nicht speicherbar.".into()))?;
        crate::ablage::schreibe_atomar(&ablage.dir().join(ZUSTAND), &roh)
            .map_err(|f| HubError::Other(String::from(f)))
    }

    fn objekt(&self, collection: &str, id: &str) -> Option<&ObjStand> {
        self.objekte.get(&schluessel(collection, id))
    }

    /// Lebt das Objekt (nach dem Wissen dieses Geräts) am Hub?
    fn lebt_am_hub(&self, collection: &str, id: &str) -> bool {
        self.objekt(collection, id)
            .is_some_and(|o| o.revision > 0 && !o.deleted)
    }
}

// ---------------------------------------------------------------------------
// Abbildung lokal ⇄ Hub
// ---------------------------------------------------------------------------

/// Ob ein Eintrag am Abgleich teilnimmt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Teilnahme {
    Sync,
    /// „Nur dieses Gerät“, `personal` oder `confidential`: verlässt das Gerät nicht.
    Lokal,
    /// Unbekannte Klasse oder unvollständige Kette: weder senden noch begraben.
    Halten,
}

fn klasse_teilnahme(k: Klasse) -> Teilnahme {
    match k {
        Klasse::Public | Klasse::Internal => Teilnahme::Sync,
        Klasse::Confidential | Klasse::Personal => Teilnahme::Lokal,
        Klasse::Unbekannt => Teilnahme::Halten,
    }
}

pub fn teilnahme_ordner(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    id: &str,
) -> Teilnahme {
    if zf::wirksam_nur_geraet(ordner, eltern, id) {
        return Teilnahme::Lokal;
    }
    klasse_teilnahme(zf::wirksame_klasse(ordner, eltern, id))
}

pub fn teilnahme_datei(
    ordner: &BTreeMap<String, Ordner>,
    eltern: &BTreeMap<String, Eltern>,
    d: &Datei,
) -> Teilnahme {
    if d.nur_geraet_min {
        return Teilnahme::Lokal;
    }
    match teilnahme_ordner(ordner, eltern, &d.ordner.wert) {
        Teilnahme::Sync => klasse_teilnahme(zf::wirksame_klasse_datei(ordner, eltern, d)),
        t => t,
    }
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Hub-Kennung eines Inhalts-Hashes (Hex).
pub fn hub_blob(key: &[u8; KEY_LEN], sha256_hex: &str) -> Option<String> {
    unhex32(sha256_hex).map(|s| blobs::blob_id(key, &s))
}

/// Woher ein Blob einer Datei stammt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Quelle {
    Original { fas: String, size: u64 },
    Text { textrev: String, size: u64 },
}

/// Hub-Kennungen aller lebenden Fassungen und Texte (sortiert, eindeutig) mit ihrer Quelle.
fn blob_quellen(key: &[u8; KEY_LEN], d: &Datei) -> BTreeMap<String, Quelle> {
    let mut m = BTreeMap::new();
    for f in &d.fassungen {
        if let Some(id) = hub_blob(key, &f.blob) {
            m.entry(id).or_insert(Quelle::Original {
                fas: f.fas.clone(),
                size: f.size,
            });
        }
        for t in f.texte.iter().filter(|t| !t.blob.is_empty()) {
            if let Some(id) = hub_blob(key, &t.blob) {
                m.entry(id).or_insert(Quelle::Text {
                    textrev: t.textrev.clone(),
                    size: t.size,
                });
            }
        }
    }
    m
}

pub fn hub_refs(key: &[u8; KEY_LEN], d: &Datei) -> Vec<String> {
    blob_quellen(key, d).into_keys().collect()
}

fn wert_ordner(o: &Ordner) -> Value {
    serde_json::to_value(o).unwrap_or(Value::Null)
}

/// Nutzlast einer Datei: der Datensatz, `blob_refs` als Inhalts-Hashes (wie lokal).
fn wert_datei(d: &Datei) -> Value {
    let mut d = d.clone();
    d.blob_refs = zf::blob_refs_aus(&d.fassungen);
    serde_json::to_value(&d).unwrap_or(Value::Null)
}

pub fn hash_ordner(o: &Ordner) -> String {
    collect::hash(&wert_ordner(o))
}

pub fn hash_datei(d: &Datei) -> String {
    collect::hash(&wert_datei(d))
}

// ---------------------------------------------------------------------------
// Ein Lauf
// ---------------------------------------------------------------------------

/// Ergebnis eines Laufs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bericht {
    /// Der lokale Bestand hat sich geändert (Seite neu laden, Index abgeglichen).
    pub geaendert: bool,
    /// Objekte, die noch auf ihren Push warten.
    pub ausstehend: u32,
    pub warnung: Option<String>,
    /// Pull vollständig (Bedingung für Blob-Löschungen).
    pub vollstaendig: bool,
}

/// Was ein Push-Kandidat ist.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Art {
    Stand,
    /// Endgültig gelöscht.
    Grab,
    /// Wechsel zu „nur dieses Gerät“: Grabstein, die Kennung bleibt lokal.
    Lokalisieren,
}

#[derive(Debug, Clone)]
struct Kandidat {
    collection: &'static str,
    id: String,
    art: Art,
    plain: String,
    hash: String,
    refs: Vec<String>,
}

struct Lauf<'a, H: AblageHub + ?Sized> {
    hub: &'a H,
    ablage: &'a Ablage,
    konto: &'a Konto<'a>,
    key_id: String,
    z: Zustand,
    bericht: Bericht,
}

fn ablage_fehler(f: crate::ablage::Fehler) -> HubError {
    HubError::Other(String::from(f))
}

/// Ein vollständiger Abgleich der Ablage: Pull, Downloads, Push, bedingte Blob-Löschung. Ist
/// `geaendert` gesetzt, gleicht der Aufrufer danach den Index ab ([`index_nachziehen`]; eigene
/// Laufzeit, daher außerhalb dieses Futures).
pub async fn lauf<H: AblageHub + ?Sized>(
    hub: &H,
    ablage: &Ablage,
    konto: &Konto<'_>,
) -> Result<Bericht, HubError> {
    let mut z = Zustand::laden(ablage);
    let kennung = format!(
        "{}:{}:{}",
        konto.user_id,
        crypto::key_id(konto.key),
        konto.hub_url.trim_end_matches('/')
    );
    if z.konto != kennung {
        z = Zustand {
            v: ZUSTAND_VERSION,
            konto: kennung,
            ..Default::default()
        };
    }
    let mut l = Lauf {
        hub,
        ablage,
        konto,
        key_id: crypto::key_id(konto.key),
        z,
        bericht: Bericht::default(),
    };
    let r = l.ausfuehren().await;
    l.z.letzter_lauf_ms = Some(crate::ablage::jetzt_ms());
    l.z.fehler = match &r {
        Ok(()) => l.bericht.warnung.clone(),
        Err(e) => Some(e.to_string()),
    };
    let gespeichert = l.z.speichern(ablage);
    r?;
    gespeichert?;
    Ok(l.bericht)
}

impl<H: AblageHub + ?Sized> Lauf<'_, H> {
    async fn ausfuehren(&mut self) -> Result<(), HubError> {
        match self.hub.verbrauch().await {
            Ok(v) => {
                self.z.kontingent_voll = v.quota > 0 && v.used >= v.quota;
                self.z.verbrauch = Some(v);
            }
            Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
            Err(_) => {}
        }
        self.bericht.vollstaendig = self.pull().await?;
        self.aufbewahren()?;
        self.herunterladen().await?;
        self.pushen().await?;
        if self.bericht.vollstaendig {
            self.blobs_loeschen().await?;
        }
        Ok(())
    }

    fn warnen(&mut self, text: impl Into<String>) {
        if self.bericht.warnung.is_none() {
            self.bericht.warnung = Some(text.into());
        }
    }

    // --- Pull --------------------------------------------------------------------------------

    /// Liefert `true`, wenn beide Sammlungen bis zum Ende gezogen sind.
    async fn pull(&mut self) -> Result<bool, HubError> {
        for coll in [SAMMLUNG_ORDNER, SAMMLUNG_DATEI] {
            let mut since = self.z.cursor.get(coll).copied().unwrap_or(0);
            let mut zurueckgesetzt = false;
            let mut seiten = 0usize;
            loop {
                seiten += 1;
                if seiten > PULL_MAX_SEITEN {
                    return Err(HubError::Other("Pull ohne Ende (Ablage)".into()));
                }
                let seite = match self.hub.pull_sammlung(since, PULL_LIMIT, coll).await {
                    Ok(p) => p,
                    Err(HubError::CursorExpired) if since > 0 && !zurueckgesetzt => {
                        zurueckgesetzt = true;
                        since = 0;
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                if seite.more && seite.cursor <= since {
                    return Err(HubError::Other("Pull ohne Fortschritt (Ablage)".into()));
                }
                let (eingaenge, buch, fremder_schluessel) = self.seite_lesen(coll, &seite);
                if !eingaenge.is_empty() {
                    let e = self
                        .ablage
                        .sync_einspielen(&eingaenge)
                        .map_err(ablage_fehler)?;
                    if !e.geaendert.is_empty() || !e.begraben.is_empty() {
                        self.bericht.geaendert = true;
                    }
                    for id in &e.verworfen {
                        log::warn!("ablage-sync: {coll}/{id} ungültig, übersprungen");
                    }
                }
                for (k, o) in buch {
                    self.z.objekte.insert(k, o);
                }
                if fremder_schluessel {
                    self.warnen("Ablage: Objekte mit anderem Schlüssel im Konto");
                    self.z.speichern(self.ablage)?;
                    return Ok(false);
                }
                since = seite.cursor.max(since);
                self.z.cursor.insert(coll.to_string(), since);
                self.z.speichern(self.ablage)?;
                if !seite.more {
                    break;
                }
            }
        }
        Ok(true)
    }

    /// Eine Pull-Seite entschlüsseln und prüfen. Liefert Eingänge, die neuen Objektstände und ob ein
    /// fremder Schlüssel auftauchte (dann endet die Seite dort, der Cursor bleibt stehen).
    fn seite_lesen(
        &self,
        coll: &'static str,
        seite: &PullResponse,
    ) -> (Vec<Eingang>, Vec<(String, ObjStand)>, bool) {
        let mut eingaenge = Vec::new();
        let mut buch = Vec::new();
        for r in &seite.objects {
            if r.collection != coll {
                continue;
            }
            let k = schluessel(coll, &r.object_id);
            if self
                .z
                .objekte
                .get(&k)
                .is_some_and(|o| o.revision >= r.revision)
            {
                continue; // schon bekannt (auch die eigene, bestätigte Änderung)
            }
            if self.z.lokalisiert.contains(&r.object_id) {
                // Eigener Grabstein beim Wechsel zu „nur dieses Gerät“: die lokale Kopie bleibt.
                buch.push((
                    k,
                    ObjStand {
                        revision: r.revision,
                        deleted: r.deleted,
                        ..Default::default()
                    },
                ));
                continue;
            }
            if r.deleted {
                eingaenge.push(Eingang::Grab(r.object_id.clone()));
                buch.push((
                    k,
                    ObjStand {
                        revision: r.revision,
                        deleted: true,
                        ..Default::default()
                    },
                ));
                continue;
            }
            if r.key_id != self.key_id {
                return (eingaenge, buch, true);
            }
            let refs = r.blob_refs.clone().unwrap_or_default();
            let gelesen = self.entschluesseln(coll, &r.object_id, &refs, &r.payload);
            let (eingang, hash) = match gelesen {
                Some(x) => x,
                None => {
                    log::warn!("ablage-sync: {coll}/{} unlesbar, übersprungen", r.object_id);
                    (None, String::new())
                }
            };
            if let Some(e) = eingang {
                eingaenge.push(e);
            }
            buch.push((
                k,
                ObjStand {
                    revision: r.revision,
                    hash,
                    deleted: false,
                    refs,
                },
            ));
        }
        (eingaenge, buch, false)
    }

    /// Öffnet ein Objekt (AAD mit `blob_refs`) und prüft Kennung und Steuerfeld gegen den Inhalt.
    fn entschluesseln(
        &self,
        coll: &str,
        id: &str,
        refs: &[String],
        payload: &str,
    ) -> Option<(Option<Eingang>, String)> {
        let aad = crypto::aad_ablage(self.konto.user_id, coll, id, refs);
        let plain = crypto::open(self.konto.key, &aad, payload).ok()?;
        if coll == SAMMLUNG_ORDNER {
            let o: Ordner = serde_json::from_slice(&plain).ok()?;
            if o.id != id || !refs.is_empty() {
                return None;
            }
            let h = hash_ordner(&o);
            Some((Some(Eingang::Ordner(o)), h))
        } else {
            let d: Datei = serde_json::from_slice(&plain).ok()?;
            if d.id != id || hub_refs(self.konto.key, &d) != refs {
                return None;
            }
            let h = hash_datei(&d);
            Some((Some(Eingang::Datei(d)), h))
        }
    }

    // --- Aufbewahrung ------------------------------------------------------------------------

    /// Aufbewahrungsregel §6 auf synchronisierte Dateien anwenden (`entfernt` wächst, reist mit).
    fn aufbewahren(&mut self) -> Result<(), HubError> {
        let b = self.ablage.sync_bestand().map_err(ablage_fehler)?;
        let jetzt = crate::ablage::jetzt_ms();
        for d in b.dateien.values() {
            if b.begraben.contains(&d.id)
                || crate::ablage::aufbewahrung::zu_entfernen(d, jetzt).is_empty()
            {
                continue;
            }
            match self.ablage.fassungen_aufraeumen(&d.id, jetzt) {
                Ok(n) if n > 0 => self.bericht.geaendert = true,
                Ok(_) => {}
                Err(e) => log::warn!("ablage-sync: Aufbewahrung {}: {}", d.id, e.code),
            }
        }
        Ok(())
    }

    // --- Downloads ---------------------------------------------------------------------------

    async fn herunterladen(&mut self) -> Result<(), HubError> {
        // Reste abgebrochener Downloads (es läuft nie mehr als ein Abgleich zugleich).
        self.ablage.sync_eingang_raeumen();
        let b = self.ablage.sync_bestand().map_err(ablage_fehler)?;
        let eltern = zf::wirksame_eltern(&b.ordner);
        // (Größe, Klasse: 0 Text, 1 Original) – Texte zuerst, dann nach Größe.
        let mut auftraege: Vec<(u8, u64, String, String, String)> = Vec::new();
        for d in b.dateien.values() {
            if b.begraben.contains(&d.id)
                || !self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id)
                || teilnahme_datei(&b.ordner, &eltern, d) == Teilnahme::Halten
            {
                continue;
            }
            for f in &d.fassungen {
                for t in f
                    .texte
                    .iter()
                    .filter(|t| !t.blob.is_empty() && t.stand == "ok")
                {
                    if !b.texte.contains(&(d.id.clone(), t.textrev.clone())) {
                        auftraege.push((0, t.size, d.id.clone(), f.fas.clone(), t.textrev.clone()));
                    }
                }
                if !b.originale.contains(&(d.id.clone(), f.fas.clone())) {
                    auftraege.push((1, f.size, d.id.clone(), f.fas.clone(), String::new()));
                }
            }
        }
        auftraege.sort();
        let mut budget = DOWNLOAD_BUDGET;
        let mut erste = true;
        for (art, size, dat, fas, textrev) in auftraege {
            if size > budget && !erste {
                break;
            }
            let Some(d) = b.dateien.get(&dat) else {
                continue;
            };
            let Some(f) = d.fassungen.iter().find(|f| f.fas == fas) else {
                continue;
            };
            let sha = if art == 0 {
                f.texte
                    .iter()
                    .find(|t| t.textrev == textrev)
                    .map(|t| t.blob.clone())
                    .unwrap_or_default()
            } else {
                f.blob.clone()
            };
            let Some(hid) = hub_blob(self.konto.key, &sha) else {
                continue;
            };
            let ziel = self.ablage.sync_zwischenpfad().map_err(ablage_fehler)?;
            let r = blobs::download(
                self.hub,
                self.konto.key,
                self.konto.user_id,
                &hid,
                size,
                &ziel,
            )
            .await;
            match r {
                Ok(_) => {
                    erste = false;
                    budget = budget.saturating_sub(size);
                    let gelegt = if art == 0 {
                        let bytes = std::fs::read(&ziel).unwrap_or_default();
                        let _ = std::fs::remove_file(&ziel);
                        self.ablage.text_einlegen(&dat, &fas, &textrev, &bytes)
                    } else {
                        self.ablage.original_einlegen(&dat, &fas, &ziel)
                    };
                    match gelegt {
                        Ok(true) => self.bericht.geaendert = true,
                        Ok(false) => {}
                        Err(e) => {
                            log::warn!("ablage-sync: {dat}/{fas} nicht abgelegt: {}", e.code);
                            self.warnen("Ablage: eine Datei vom Hub passte nicht");
                        }
                    }
                }
                Err(HubError::Unauthorized) => {
                    let _ = std::fs::remove_file(&ziel);
                    return Err(HubError::Unauthorized);
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&ziel);
                    log::warn!("ablage-sync: Download für {dat}/{fas} noch nicht möglich");
                }
            }
        }
        Ok(())
    }

    // --- Push --------------------------------------------------------------------------------

    fn kandidaten(&mut self, b: &SyncBestand) -> Vec<Kandidat> {
        let eltern = zf::wirksame_eltern(&b.ordner);
        let mut aus = Vec::new();
        let mut ausstehend = 0u32;
        // Ein Ordner wird erst lokalisiert, wenn keine Datei darunter mehr am Hub lebt (sonst kämen
        // ihre Kopien auf anderen Geräten vorübergehend unter „Wiederhergestellt“).
        let mut noch_belegt: BTreeSet<String> = BTreeSet::new();
        for d in b.dateien.values() {
            if self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id) && !self.z.lokalisiert.contains(&d.id) {
                noch_belegt.extend(zf::kette(&eltern, &d.ordner.wert));
            }
        }
        for o in b.ordner.values() {
            if b.begraben.contains(&o.id) || self.z.lokalisiert.contains(&o.id) {
                continue;
            }
            match teilnahme_ordner(&b.ordner, &eltern, &o.id) {
                Teilnahme::Sync => {
                    let wert = wert_ordner(o);
                    let hash = collect::hash(&wert);
                    if self
                        .z
                        .objekt(SAMMLUNG_ORDNER, &o.id)
                        .is_some_and(|s| s.hash == hash && !s.deleted)
                    {
                        continue;
                    }
                    if self
                        .z
                        .objekt(SAMMLUNG_ORDNER, &o.id)
                        .is_some_and(|s| s.deleted)
                    {
                        continue; // am Hub begraben: der Pull hat den Grabstein schon angewendet
                    }
                    aus.push(Kandidat {
                        collection: SAMMLUNG_ORDNER,
                        id: o.id.clone(),
                        art: Art::Stand,
                        plain: collect::canonical(&wert),
                        hash,
                        refs: Vec::new(),
                    });
                }
                Teilnahme::Lokal if self.z.lebt_am_hub(SAMMLUNG_ORDNER, &o.id) => {
                    if noch_belegt.contains(&o.id) {
                        ausstehend += 1;
                        continue;
                    }
                    aus.push(Kandidat {
                        collection: SAMMLUNG_ORDNER,
                        id: o.id.clone(),
                        art: Art::Lokalisieren,
                        plain: String::new(),
                        hash: String::new(),
                        refs: Vec::new(),
                    });
                }
                _ => {}
            }
        }
        for d in b.dateien.values() {
            if b.begraben.contains(&d.id) || self.z.lokalisiert.contains(&d.id) {
                continue;
            }
            match teilnahme_datei(&b.ordner, &eltern, d) {
                Teilnahme::Sync => {
                    let wert = wert_datei(d);
                    let hash = collect::hash(&wert);
                    let bekannt = self.z.objekt(SAMMLUNG_DATEI, &d.id);
                    if bekannt.is_some_and(|s| s.deleted || s.hash == hash) {
                        continue;
                    }
                    aus.push(Kandidat {
                        collection: SAMMLUNG_DATEI,
                        id: d.id.clone(),
                        art: Art::Stand,
                        plain: collect::canonical(&wert),
                        hash,
                        refs: hub_refs(self.konto.key, d),
                    });
                }
                Teilnahme::Lokal if self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id) => {
                    // Lokale Kopie prüfen: erst wenn jedes Original hier liegt, verschwindet die Cloud-Kopie.
                    let vollstaendig = d
                        .fassungen
                        .iter()
                        .all(|f| b.originale.contains(&(d.id.clone(), f.fas.clone())));
                    if !vollstaendig {
                        ausstehend += 1;
                        continue;
                    }
                    aus.push(Kandidat {
                        collection: SAMMLUNG_DATEI,
                        id: d.id.clone(),
                        art: Art::Lokalisieren,
                        plain: String::new(),
                        hash: String::new(),
                        refs: Vec::new(),
                    });
                }
                _ => {}
            }
        }
        for id in &b.begraben {
            for coll in [SAMMLUNG_ORDNER, SAMMLUNG_DATEI] {
                if self.z.lebt_am_hub(coll, id) && !self.z.lokalisiert.contains(id) {
                    aus.push(Kandidat {
                        collection: coll,
                        id: id.clone(),
                        art: Art::Grab,
                        plain: String::new(),
                        hash: String::new(),
                        refs: Vec::new(),
                    });
                }
            }
        }
        self.bericht.ausstehend += ausstehend;
        aus
    }

    /// Neue Blobs einer Datei reservieren und (falls nötig) hochladen. `false`: noch nicht pushbar.
    async fn blobs_bereitstellen(
        &mut self,
        b: &SyncBestand,
        k: &Kandidat,
        budget: &mut u64,
    ) -> Result<bool, HubError> {
        let Some(d) = b.dateien.get(&k.id) else {
            return Ok(false);
        };
        let alt: BTreeSet<String> = self
            .z
            .objekt(SAMMLUNG_DATEI, &k.id)
            .filter(|o| !o.deleted)
            .map(|o| o.refs.iter().cloned().collect())
            .unwrap_or_default();
        let quellen = blob_quellen(self.konto.key, d);
        for (hid, quelle) in quellen {
            if alt.contains(&hid) && self.z.hochgeladen.contains(&hid) {
                continue; // vom Objekt schon referenziert: am Hub vor dem Löschen geschützt
            }
            // 1. reservieren (auch beim Wiederverwenden eines vorhandenen Blobs)
            self.hub.reservieren(&hid, self.konto.device_id).await?;
            if self.z.hochgeladen.contains(&hid) {
                continue;
            }
            let pfad = match &quelle {
                Quelle::Original { fas, .. } => self
                    .ablage
                    .original_quelle(&k.id, fas)
                    .map_err(ablage_fehler)?,
                Quelle::Text { textrev, .. } => self.ablage.text_quelle(&k.id, textrev),
            };
            let size = match &quelle {
                Quelle::Original { size, .. } | Quelle::Text { size, .. } => *size,
            };
            let Some(pfad) = pfad else {
                // Nicht hier: vorhanden am Hub? Sonst wartet das Objekt (Gerät mit der Datei lädt hoch).
                match self.hub.blob_info(&hid).await? {
                    Some(i) if i.complete => {
                        self.z.hochgeladen.insert(hid);
                        continue;
                    }
                    _ => return Ok(false),
                }
            };
            let ueber = self.z.kontingent_voll
                || self
                    .z
                    .verbrauch
                    .is_some_and(|v| v.quota > 0 && v.used.saturating_add(size) > v.quota);
            if ueber {
                // Schon am Hub (z. B. von einem anderen Gerät): kein neuer Speicher nötig.
                if matches!(self.hub.blob_info(&hid).await?, Some(i) if i.complete) {
                    self.z.hochgeladen.insert(hid);
                    continue;
                }
                self.z.kontingent_voll = true;
                self.warnen("Ablage: Speicher im Konto voll, Hochladen angehalten");
                return Ok(false);
            }
            let r = blobs::upload(
                self.hub,
                self.konto.key,
                self.konto.user_id,
                &hid,
                &Source::File(pfad),
                size,
                None,
                budget,
            )
            .await;
            match r {
                Ok(Upload::Done) => {
                    self.z.hochgeladen.insert(hid);
                    if let Some(v) = self.z.verbrauch.as_mut() {
                        v.used = v.used.saturating_add(size);
                    }
                }
                Ok(Upload::Later) => return Ok(false),
                Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
                Err(HubError::Other(e)) if e.contains("quota_exceeded") => {
                    self.z.kontingent_voll = true;
                    self.warnen("Ablage: Speicher im Konto voll, Hochladen angehalten");
                    return Ok(false);
                }
                Err(_) => {
                    log::warn!("ablage-sync: Hochladen für {} fehlgeschlagen", k.id);
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    async fn pushen(&mut self) -> Result<(), HubError> {
        let mut budget = UPLOAD_BUDGET;
        for runde in 0..MAX_RUNDEN {
            self.bericht.ausstehend = 0;
            let b = self.ablage.sync_bestand().map_err(ablage_fehler)?;
            let alle = self.kandidaten(&b);
            if alle.is_empty() {
                return Ok(());
            }
            // Ordner vor Dateien, damit Dateien ihren Ordner vorfinden.
            let mut bereit: Vec<Kandidat> = Vec::new();
            for k in alle {
                if k.collection == SAMMLUNG_DATEI && k.art == Art::Stand {
                    if !self.blobs_bereitstellen(&b, &k, &mut budget).await? {
                        self.bericht.ausstehend += 1;
                        continue;
                    }
                }
                bereit.push(k);
            }
            bereit.sort_by_key(|k| (k.collection == SAMMLUNG_DATEI, k.id.clone()));
            let mut nochmal = false;
            for stapel in bereit.chunks(PUSH_STAPEL) {
                let objekte: Vec<PushObject> = stapel
                    .iter()
                    .map(|k| self.push_objekt(k))
                    .collect::<Result<_, _>>()?;
                let ergebnisse = self.hub.push(self.konto.device_id, &objekte).await?;
                for r in ergebnisse {
                    let Some(k) = stapel
                        .iter()
                        .find(|k| k.collection == r.collection && k.id == r.object_id)
                    else {
                        continue;
                    };
                    nochmal |= self.ergebnis(k, &r)?;
                }
                self.z.speichern(self.ablage)?;
            }
            if !nochmal {
                return Ok(());
            }
            if runde + 1 == MAX_RUNDEN {
                self.bericht.ausstehend += 1;
            }
        }
        Ok(())
    }

    fn push_objekt(&self, k: &Kandidat) -> Result<PushObject, HubError> {
        let base = self
            .z
            .objekt(k.collection, &k.id)
            .map(|o| o.revision)
            .unwrap_or(0);
        let grab = k.art != Art::Stand;
        let payload = if grab {
            String::new()
        } else {
            crypto::seal(
                self.konto.key,
                &crypto::aad_ablage(self.konto.user_id, k.collection, &k.id, &k.refs),
                k.plain.as_bytes(),
            )
            .map_err(HubError::Other)?
        };
        Ok(PushObject {
            collection: k.collection.to_string(),
            object_id: k.id.clone(),
            base_revision: base,
            payload,
            key_id: self.key_id.clone(),
            deleted: grab,
            blob_refs: (k.collection == SAMMLUNG_DATEI && !grab).then(|| k.refs.clone()),
        })
    }

    /// Ein Push-Ergebnis verbuchen. `true`: in dieser Runde erneut versuchen.
    fn ergebnis(&mut self, k: &Kandidat, r: &PushResult) -> Result<bool, HubError> {
        let schl = schluessel(k.collection, &k.id);
        let alt_refs: Vec<String> = self
            .z
            .objekte
            .get(&schl)
            .map(|o| o.refs.clone())
            .unwrap_or_default();
        match (r.outcome.as_str(), r.reason.as_deref()) {
            ("accepted", _) => {
                let grab = k.art != Art::Stand;
                self.z.objekte.insert(
                    schl,
                    ObjStand {
                        revision: r.revision,
                        hash: k.hash.clone(),
                        deleted: grab,
                        refs: if grab { Vec::new() } else { k.refs.clone() },
                    },
                );
                if k.art == Art::Lokalisieren {
                    self.z.lokalisiert.insert(k.id.clone());
                }
                let neu: BTreeSet<&String> = k.refs.iter().collect();
                for alt in alt_refs.iter().filter(|a| grab || !neu.contains(a)) {
                    self.z.blob_loeschen.insert(alt.clone());
                }
                // Nach einer lokalisierten Datei kann ihr Ordner in der nächsten Runde folgen.
                Ok(k.art == Art::Lokalisieren && k.collection == SAMMLUNG_DATEI)
            }
            ("superseded", _) => {
                match &r.current {
                    Some(cur) if cur.deleted => {
                        // Endgültiger Grabstein am Hub: lokal anwenden (nie wiederbeleben).
                        self.z.objekte.insert(
                            schl,
                            ObjStand {
                                revision: cur.revision,
                                deleted: true,
                                ..Default::default()
                            },
                        );
                        if k.art == Art::Stand {
                            self.ablage
                                .sync_einspielen(&[Eingang::Grab(k.id.clone())])
                                .map_err(ablage_fehler)?;
                            self.bericht.geaendert = true;
                        }
                        Ok(false)
                    }
                    Some(cur) => {
                        if cur.key_id != self.key_id {
                            self.warnen("Ablage: Objekte mit anderem Schlüssel im Konto");
                            return Ok(false);
                        }
                        let refs = cur.blob_refs.clone().unwrap_or_default();
                        let gelesen = self.entschluesseln(k.collection, &k.id, &refs, &cur.payload);
                        let hash = match gelesen {
                            Some((Some(e), h)) => {
                                let ein =
                                    self.ablage.sync_einspielen(&[e]).map_err(ablage_fehler)?;
                                if !ein.geaendert.is_empty() {
                                    self.bericht.geaendert = true;
                                }
                                h
                            }
                            _ => String::new(),
                        };
                        self.z.objekte.insert(
                            schl,
                            ObjStand {
                                revision: cur.revision,
                                hash,
                                deleted: false,
                                refs,
                            },
                        );
                        Ok(true)
                    }
                    None => {
                        // Der Hub kennt das Objekt nicht (mehr): erneut mit Basis 0.
                        self.z.objekte.remove(&schl);
                        Ok(k.art == Art::Stand)
                    }
                }
            }
            ("rejected", Some("blob_missing")) => {
                // Reservierung abgelaufen und Blob gelöscht (oder nie abgeschlossen): neu hochladen.
                for m in &r.missing {
                    self.z.hochgeladen.remove(m);
                }
                if r.missing.is_empty() {
                    for b in &k.refs {
                        self.z.hochgeladen.remove(b);
                    }
                }
                Ok(true)
            }
            ("rejected", Some("tombstoned")) => {
                self.z.objekte.insert(
                    schl,
                    ObjStand {
                        revision: r.revision,
                        deleted: true,
                        ..Default::default()
                    },
                );
                if k.art == Art::Stand {
                    self.ablage
                        .sync_einspielen(&[Eingang::Grab(k.id.clone())])
                        .map_err(ablage_fehler)?;
                    self.bericht.geaendert = true;
                }
                Ok(false)
            }
            ("rejected", grund) => {
                log::warn!(
                    "ablage-sync: {}/{} abgewiesen ({})",
                    k.collection,
                    k.id,
                    grund.unwrap_or("ohne Grund")
                );
                self.warnen("Ablage: ein Eintrag wurde vom Hub abgewiesen");
                Ok(false)
            }
            _ => {
                self.bericht.ausstehend += 1;
                Ok(false)
            }
        }
    }

    // --- Blob-Löschung ------------------------------------------------------------------------

    /// Bedingte Löschung der Kandidaten: nur ohne lebende Fassung im (vollständig gezogenen) Bestand;
    /// 409 (referenziert/reserviert) ist ein Ergebnis, der Kandidat entfällt.
    async fn blobs_loeschen(&mut self) -> Result<(), HubError> {
        if self.z.blob_loeschen.is_empty() {
            return Ok(());
        }
        let b = self.ablage.sync_bestand().map_err(ablage_fehler)?;
        // Lebend am Hub ist, was eine synchronisierte (nicht lokalisierte) Datei nutzt; was nur noch
        // auf diesem Gerät liegt, braucht keine Cloud-Kopie.
        let eltern = zf::wirksame_eltern(&b.ordner);
        let mut lebend: BTreeSet<String> = BTreeSet::new();
        for d in b.dateien.values() {
            if !b.begraben.contains(&d.id)
                && !self.z.lokalisiert.contains(&d.id)
                && teilnahme_datei(&b.ordner, &eltern, d) != Teilnahme::Lokal
            {
                lebend.extend(hub_refs(self.konto.key, d));
            }
        }
        for o in self.z.objekte.values().filter(|o| !o.deleted) {
            lebend.extend(o.refs.iter().cloned());
        }
        for c in std::mem::take(&mut self.z.blob_loeschen) {
            if lebend.contains(&c) {
                continue;
            }
            match self.hub.blob_loeschen(&c, self.konto.device_id).await {
                Ok(BlobLoeschung::Geloescht(_)) => {
                    self.z.hochgeladen.remove(&c);
                }
                Ok(BlobLoeschung::InGebrauch(_)) => {}
                Err(HubError::Unauthorized) => {
                    self.z.blob_loeschen.insert(c);
                    return Err(HubError::Unauthorized);
                }
                Err(_) => {
                    self.z.blob_loeschen.insert(c);
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Anbindung an die App
// ---------------------------------------------------------------------------

/// Die Ablage im Datenordner der App (wie `ablage::befehle`).
pub fn ablage_der_app(app: &tauri::AppHandle) -> Result<Ablage, String> {
    Ok(Ablage::neu(
        crate::portable::app_data_dir(app)
            .map_err(|e| e.to_string())?
            .join("ablage"),
    ))
}

/// Index nach einem Lauf mit Änderungen abgleichen (nur Abweichungen; entfernt begrabene Dateien).
pub fn index_nachziehen(ablage: &Ablage) {
    if let Err(e) = crate::ablage::index::abgleichen(ablage) {
        log::warn!("ablage-sync: Indexabgleich: {}", e.code);
    }
}

/// Lauf aus dem Sync-Takt (nach dem Abgleich der übrigen Sammlungen). Fehler landen im Zustand und in
/// der Oberfläche, nie im Takt der Bibliothek. Danach Index und Textarbeiter wie nach jeder Änderung.
pub async fn lauf_app(
    app: &tauri::AppHandle,
    cfg: &super::account::SyncConfig,
) -> Result<Bericht, HubError> {
    use tauri::Emitter;
    let key = cfg.key().map_err(HubError::Other)?;
    let ablage = ablage_der_app(app).map_err(HubError::Other)?;
    // Eine Ablage, die nicht lesbar ist (defekt, neuer), wird nicht abgeglichen.
    if ablage.wartung().is_some() {
        return Ok(Bericht::default());
    }
    let client = HubClient::new(&cfg.url, &cfg.token);
    let konto = Konto {
        key: &key,
        user_id: cfg.user_id,
        device_id: &cfg.device_id,
        hub_url: &cfg.url,
    };
    let bericht = lauf(&client, &ablage, &konto).await?;
    if bericht.geaendert {
        let a = ablage.clone();
        let _ = tauri::async_runtime::spawn_blocking(move || index_nachziehen(&a)).await;
        crate::ablage::textfassung::wiederanlauf(ablage.dir().to_path_buf());
    }
    let _ = app.emit(crate::ablage::befehle::EREIGNIS_SYNC, ());
    Ok(bericht)
}

// ---------------------------------------------------------------------------
// Zustand je Eintrag (Oberfläche)
// ---------------------------------------------------------------------------

/// Sync-Zustand eines Ordners oder einer Datei.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AblageSyncZustand {
    Synchronisiert,
    Ausstehend,
    Konflikt,
    WartetAufText,
    NurGeraet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct AblageSyncEintrag {
    pub id: String,
    pub zustand: AblageSyncZustand,
}

/// Stand des Abgleichs für die Seite „Ablage“.
#[derive(Debug, Clone, Default, PartialEq, Serialize, specta::Type)]
pub struct AblageSyncStand {
    /// Ein Konto ist angemeldet (sonst zeigt die Seite keine Sync-Zustände).
    pub verbunden: bool,
    pub letzter_lauf_ms: Option<f64>,
    pub fehler: Option<String>,
    pub kontingent_voll: bool,
    /// Belegt und Kontingent in Bytes (vom Hub), falls bekannt.
    pub belegt: Option<f64>,
    pub kontingent: Option<f64>,
    pub eintraege: Vec<AblageSyncEintrag>,
}

/// Zustand je Eintrag aus Bestand und Sync-Zustand (ohne Schlüssel: Hashes der Nutzlast).
pub fn zustaende(b: &SyncBestand, z: &Zustand) -> Vec<AblageSyncEintrag> {
    let eltern = zf::wirksame_eltern(&b.ordner);
    let mut aus = Vec::new();
    for o in b.ordner.values() {
        let zustand = if z.lokalisiert.contains(&o.id) {
            AblageSyncZustand::NurGeraet
        } else {
            match teilnahme_ordner(&b.ordner, &eltern, &o.id) {
                Teilnahme::Lokal | Teilnahme::Halten => AblageSyncZustand::NurGeraet,
                Teilnahme::Sync => {
                    if z.objekt(SAMMLUNG_ORDNER, &o.id)
                        .is_some_and(|s| s.hash == hash_ordner(o))
                    {
                        AblageSyncZustand::Synchronisiert
                    } else {
                        AblageSyncZustand::Ausstehend
                    }
                }
            }
        };
        aus.push(AblageSyncEintrag {
            id: o.id.clone(),
            zustand,
        });
    }
    for d in b.dateien.values() {
        let zustand = if z.lokalisiert.contains(&d.id) {
            AblageSyncZustand::NurGeraet
        } else {
            match teilnahme_datei(&b.ordner, &eltern, d) {
                Teilnahme::Lokal | Teilnahme::Halten => AblageSyncZustand::NurGeraet,
                Teilnahme::Sync if zf::konflikt(d) => AblageSyncZustand::Konflikt,
                Teilnahme::Sync if wartet_auf_text(b, d) => AblageSyncZustand::WartetAufText,
                Teilnahme::Sync => {
                    let originale = d
                        .fassungen
                        .iter()
                        .all(|f| b.originale.contains(&(d.id.clone(), f.fas.clone())));
                    if originale
                        && z.objekt(SAMMLUNG_DATEI, &d.id)
                            .is_some_and(|s| s.hash == hash_datei(d))
                    {
                        AblageSyncZustand::Synchronisiert
                    } else {
                        AblageSyncZustand::Ausstehend
                    }
                }
            }
        };
        aus.push(AblageSyncEintrag {
            id: d.id.clone(),
            zustand,
        });
    }
    aus
}

/// D4b: Die aktuelle Fassung hat (noch) keine nutzbare Textfassung auf diesem Gerät und kann hier auch
/// keine erzeugen (Original fehlt): „wartet auf Text“.
pub fn wartet_auf_text(b: &SyncBestand, d: &Datei) -> bool {
    let Some(fas) = zf::aktuell(d).or_else(|| {
        d.fassungen
            .iter()
            .max_by(|a, b| a.zeit.cmp(&b.zeit).then_with(|| a.fas.cmp(&b.fas)))
            .map(|f| f.fas.clone())
    }) else {
        return false;
    };
    let Some(f) = d.fassungen.iter().find(|f| f.fas == fas) else {
        return false;
    };
    let Some(rev) = crate::ablage::speicher::erwartete_textrev(f) else {
        return false; // Format ohne Textfassung
    };
    let original_hier = b.originale.contains(&(d.id.clone(), f.fas.clone()));
    // Ein vorhandener Text (auch ein älterer Extraktor) wird genutzt, sobald er hier liegt.
    let text_hier = f.texte.iter().any(|t| {
        t.stand == "kein_text"
            || (t.stand == "ok" && b.texte.contains(&(d.id.clone(), t.textrev.clone())))
    });
    if text_hier {
        return false;
    }
    // Ein Fehler der Extraktion ist kein Warten (die Liste zeigt ihn, D2).
    if f.texte
        .iter()
        .any(|t| t.textrev == rev && t.stand == "fehler")
    {
        return false;
    }
    let eingetragen = f.texte.iter().any(|t| t.textrev == rev);
    // Ohne Original kann dieses Gerät den Text nicht selbst erstellen; mit Original übernimmt die
    // eigene Warteschlange (Zustand „ausstehend“ der Textfassung, D2), außer der Text ist schon
    // anderswo erstellt und wird gerade geholt.
    !original_hier || eingetragen
}

/// Stand für die Oberfläche (ohne Hub-Aufruf).
pub fn stand(ablage: &Ablage, verbunden: bool) -> Result<AblageSyncStand, crate::ablage::Fehler> {
    let z = Zustand::laden(ablage);
    if !verbunden {
        return Ok(AblageSyncStand::default());
    }
    let b = ablage.sync_bestand()?;
    Ok(AblageSyncStand {
        verbunden,
        letzter_lauf_ms: z.letzter_lauf_ms.map(|m| m as f64),
        fehler: z.fehler.clone(),
        kontingent_voll: z.kontingent_voll,
        belegt: z.verbrauch.map(|v| v.used as f64),
        kontingent: z.verbrauch.map(|v| v.quota as f64),
        eintraege: zustaende(&b, &z),
    })
}

#[cfg(test)]
#[path = "ablage_tests.rs"]
mod tests;
