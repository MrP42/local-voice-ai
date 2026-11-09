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
//! * Grabsteine sind endgültig und versiegelt: die Nutzlast `{id, collection, deleted: true}` ist mit
//!   `deleted` und `key_id` in der AAD deterministisch versiegelt (eine Wiederholung trägt exakt
//!   dasselbe Chiffrat). Ein Grabstein ohne gültiges Siegel wird nie angewendet (Warnung im Stand).
//!   Empfangen → Original und Texte gehen für 30 Tage in den Papierkorb, Index weg. Blob-Löschung nur
//!   nach vollständigem Pull, nur ohne lebende Fassung im Bestand, nur bedingt (409 wird hingenommen).
//! * Wechsel sync → „nur dieses Gerät“ (oder Klasse `personal`/`confidential`): die Ablage hält die
//!   betroffenen Kennungen beim Umschalten im eigenen Stand fest (Write-ahead, `lokal`); nur sie werden
//!   (wenn alle Originale hier liegen) am Hub begraben, nie wieder gesendet, und ein Grabstein wird auf
//!   sie nie angewendet. Später von anderen Geräten eingetroffene Einträge unter einem festgehaltenen
//!   Ordner bleiben synchron („befreit“).
//! * Rückweg lokal → sync (`speicher_uebergang`, im App-Takt vor dem Lauf): der festgehaltene Teilbaum
//!   bekommt neue Kennungen; die alten bleiben lokal als Weiterleitung. Lebt eine alte Kennung am Hub
//!   noch, sendet der Lauf ihren Grabstein (Dateien vor Ordnern); Stände zu alten Kennungen werden nie
//!   wieder aufgenommen.
//! * Unlesbare entfernte Stände (Siegel, JSON, neueres Schema) halten das Objekt an: kein Push, keine
//!   Blob-Löschung. Unbekannte Felder bleiben erhalten und reisen mit.
//!
//! Datenschutz: ins Log nur Kennungen und Codes, nie Namen oder Inhalte.

use super::blobs::{self, BlobTransport, Source, Upload};
use super::client::{
    BlobLoeschung, BlobVerbrauch, HubClient, HubError, PullResponse, PushObject, PushResult,
};
use super::collect;
use super::crypto::{self, KEY_LEN};
use crate::ablage::speicher::{Ablage, Eingang, SyncBestand};
use crate::ablage::zusammenfuehren::Register;
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
/// Schema der Datensätze, das dieses Gerät versteht (ein neueres hält das Objekt an).
const SCHEMA: u64 = 1;
/// Rückstellung nach einer Abweisung: 1 min · 2^Stufe, höchstens 6 h.
const RUECK_MAX_MS: i64 = 6 * 3600 * 1000;
/// Verwaiste eigene Blobs werden erst nach dieser Frist gelöscht.
const WAISE_MS: i64 = 7 * 24 * 3600 * 1000;

fn rueck_dauer(stufe: u32) -> i64 {
    (60_000i64 << stufe.min(20)).min(RUECK_MAX_MS)
}

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
    /// Der entfernte Stand ist hier unlesbar (Siegel, JSON, neueres Schema): kein Push, keine
    /// Blob-Löschung, bis ein lesbarer Stand eintrifft.
    #[serde(default)]
    pub angehalten: bool,
}

/// Rückstellung eines Objekts oder der Ablage (exponentiell).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rueck {
    pub bis_ms: i64,
    pub stufe: u32,
}

impl Rueck {
    fn naechste(alt: Option<Rueck>, jetzt: i64) -> Rueck {
        let stufe = alt.map(|r| r.stufe + 1).unwrap_or(0);
        Rueck {
            bis_ms: jetzt + rueck_dauer(stufe),
            stufe,
        }
    }
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
    /// Abgewiesene Objekte: bis wann sie ruhen (ohne erneute Reservierung).
    #[serde(default)]
    pub rueck: BTreeMap<String, Rueck>,
    /// „Zu viele Ablagen“ (Blob-Anzahl am Hub): Hochladen ruht bis dahin.
    #[serde(default)]
    pub zu_viele: Option<Rueck>,
    /// Geprüfte Ordnergräber, die erst nach dem Pull der Dateien angewendet werden (sonst läge eine
    /// gleichzeitig begrabene Datei noch darunter und hielte den Ordner).
    #[serde(default)]
    pub ordnergraeber: BTreeSet<String>,
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

/// Ordner ohne eigene Regeln (für fehlende Elternordner und – aus Sicht befreiter Einträge –
/// festgehaltene Ordner): wie „Wiederhergestellt“.
fn neutraler_ordner(id: &str) -> Ordner {
    Ordner {
        id: id.to_string(),
        eltern: Register::neu(None, 0, ""),
        name: Register::neu(String::new(), 0, ""),
        datenklasse: Register::neu("erbt".into(), 0, ""),
        nur_geraet: Register::neu(false, 0, ""),
        zielordner_ki: Register::neu(false, 0, ""),
        papierkorb: Register::neu(None, 0, ""),
        extra: BTreeMap::new(),
    }
}

/// Teilnahme am Abgleich je Kennung (Ordner und Dateien), maßgeblich für Push, Grabsteine und
/// Momentaufnahme der Ablage:
/// * festgehalten (`lokal`) → `Lokal`, für immer;
/// * ein fehlender Elternordner zählt neutral (wie „Wiederhergestellt“), nicht als unbekannt;
/// * befreite Einträge sehen festgehaltene Ordner ebenfalls neutral (sie bleiben synchron);
/// * sonst Klasse und „nur dieses Gerät“ entlang der Kette.
pub fn teilnahmen(
    ordner: &BTreeMap<String, Ordner>,
    dateien: &BTreeMap<String, Datei>,
    lokal: &BTreeSet<String>,
    befreit: &BTreeSet<String>,
) -> BTreeMap<String, Teilnahme> {
    let mut voll = ordner.clone();
    let fehlend: BTreeSet<&String> = dateien
        .values()
        .map(|d| &d.ordner.wert)
        .chain(ordner.values().filter_map(|o| o.eltern.wert.as_ref()))
        .filter(|id| !ordner.contains_key(*id))
        .collect();
    for id in fehlend {
        voll.insert(id.clone(), neutraler_ordner(id));
    }
    let e_voll = zf::wirksame_eltern(&voll);
    let frei = (!befreit.is_empty()).then(|| {
        let mut f = voll.clone();
        for id in lokal {
            if f.contains_key(id) {
                f.insert(id.clone(), neutraler_ordner(id));
            }
        }
        let e = zf::wirksame_eltern(&f);
        (f, e)
    });
    let sicht = |id: &str| match &frei {
        Some((f, e)) if befreit.contains(id) => (f, e),
        _ => (&voll, &e_voll),
    };
    let mut aus = BTreeMap::new();
    for id in ordner.keys() {
        let t = if lokal.contains(id) {
            Teilnahme::Lokal
        } else {
            let (o, e) = sicht(id);
            teilnahme_ordner(o, e, id)
        };
        aus.insert(id.clone(), t);
    }
    for d in dateien.values() {
        let t = if lokal.contains(&d.id) {
            Teilnahme::Lokal
        } else {
            let (o, e) = sicht(&d.id);
            teilnahme_datei(o, e, d)
        };
        aus.insert(d.id.clone(), t);
    }
    aus
}

/// Kontoweit gleiche Kennung des Standardordners „Allgemein“: `ord_` + ULID aus SHA-256(Schlüssel ‖
/// "lv-ablage-allgemein") (gekürzt auf 128 Bit).
pub fn standard_id(key: &[u8; KEY_LEN]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(key);
    h.update(b"lv-ablage-allgemein");
    let d = h.finalize();
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    format!(
        "{}{}",
        crate::ablage::PRAEFIX_ORDNER,
        ulid::Ulid::from_bytes(b)
    )
}

/// Klartext eines Grabsteins (deterministisch).
fn grab_klartext(collection: &str, id: &str) -> String {
    collect::canonical(&serde_json::json!({
        "collection": collection,
        "deleted": true,
        "id": id,
    }))
}

/// Versiegelte Nutzlast eines Grabsteins: deterministisch, damit eine Wiederholung exakt dasselbe
/// Chiffrat trägt (der Hub nimmt eine Wiederholung nur bei gleicher Nutzlast und `key_id` an).
pub fn grab_nutzlast(
    key: &[u8; KEY_LEN],
    user_id: i64,
    collection: &str,
    id: &str,
) -> Result<String, String> {
    let kid = crypto::key_id(key);
    crypto::seal_deterministisch(
        key,
        &crypto::aad_ablage(user_id, collection, id, &[], &kid, true),
        grab_klartext(collection, id).as_bytes(),
    )
}

/// Ist der Grabstein vom Konto selbst versiegelt (Kennung, Sammlung, `deleted` bestätigt)?
fn grab_echt(
    key: &[u8; KEY_LEN],
    user_id: i64,
    collection: &str,
    id: &str,
    key_id: &str,
    payload: &str,
) -> bool {
    if payload.is_empty() || key_id != crypto::key_id(key) {
        return false;
    }
    let aad = crypto::aad_ablage(user_id, collection, id, &[], key_id, true);
    let Ok(plain) = crypto::open(key, &aad, payload) else {
        return false;
    };
    serde_json::from_slice::<Value>(&plain).is_ok_and(|v| {
        v.get("id").and_then(Value::as_str) == Some(id)
            && v.get("collection").and_then(Value::as_str) == Some(collection)
            && v.get("deleted").and_then(Value::as_bool) == Some(true)
    })
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
    /// Kontokennung (Benutzer, Schlüsselkennung, Hub) für den Zustand und die Blob-Liste.
    kennung: String,
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
            konto: kennung.clone(),
            ..Default::default()
        };
    }
    let mut l = Lauf {
        hub,
        ablage,
        konto,
        key_id: crypto::key_id(konto.key),
        kennung,
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
            // Ohne gelesenen Verbrauch bleibt `kontingent_voll`, wie es war.
            Err(_) => {}
        }
        // Ein unveränderter Standardordner zieht vor dem ersten Push auf die kontoweite Kennung um.
        let ziel = standard_id(self.konto.key);
        let z = &self.z;
        let am_hub = |id: &str| z.objekt(SAMMLUNG_ORDNER, id).is_some();
        match self.ablage.standard_umziehen(&ziel, &am_hub) {
            Ok(true) => self.bericht.geaendert = true,
            Ok(false) => {}
            Err(e) => log::warn!("ablage-sync: Standardordner nicht umgezogen: {}", e.code),
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
                let (mut eingaenge, buch, fremder_schluessel) = self.seite_lesen(coll, &seite);
                if coll == SAMMLUNG_ORDNER {
                    eingaenge.retain(|e| match e {
                        Eingang::Grab(id) => {
                            self.z.ordnergraeber.insert(id.clone());
                            false
                        }
                        _ => true,
                    });
                }
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
        if !self.z.ordnergraeber.is_empty() {
            let graeber: Vec<Eingang> = self
                .z
                .ordnergraeber
                .iter()
                .map(|id| Eingang::Grab(id.clone()))
                .collect();
            let e = self
                .ablage
                .sync_einspielen(&graeber)
                .map_err(ablage_fehler)?;
            if !e.begraben.is_empty() {
                self.bericht.geaendert = true;
            }
            self.z.ordnergraeber.clear();
            self.z.speichern(self.ablage)?;
        }
        Ok(true)
    }

    /// Eine Pull-Seite entschlüsseln und prüfen. Liefert Eingänge, die neuen Objektstände und ob ein
    /// fremder Schlüssel auftauchte (dann endet die Seite dort, der Cursor bleibt stehen).
    fn seite_lesen(
        &mut self,
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
            if r.deleted {
                // Nur ein vom Konto versiegelter Grabstein gilt; die Ablage wendet ihn außerdem nie auf
                // festgehaltene oder lokale Einträge an (`sync_einspielen`).
                if grab_echt(
                    self.konto.key,
                    self.konto.user_id,
                    coll,
                    &r.object_id,
                    &r.key_id,
                    &r.payload,
                ) {
                    eingaenge.push(Eingang::Grab(r.object_id.clone()));
                } else {
                    log::warn!(
                        "ablage-sync: {coll}/{} Grabstein ohne Siegel, ignoriert",
                        r.object_id
                    );
                    self.warnen("Ablage: ein Grabstein ohne gültiges Siegel wurde ignoriert");
                }
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
            let angehalten = gelesen.is_none();
            let hash = match gelesen {
                Some((e, h)) => {
                    eingaenge.push(e);
                    h
                }
                None => {
                    log::warn!("ablage-sync: {coll}/{} unlesbar, angehalten", r.object_id);
                    self.warnen("Ablage: ein Eintrag vom Hub ist hier nicht lesbar und ruht");
                    String::new()
                }
            };
            buch.push((
                k,
                ObjStand {
                    revision: r.revision,
                    hash,
                    deleted: false,
                    refs,
                    angehalten,
                },
            ));
        }
        (eingaenge, buch, false)
    }

    /// Öffnet ein Objekt (AAD mit `blob_refs`, `key_id`, `deleted`) und prüft Kennung, Schema und
    /// Steuerfeld gegen den Inhalt. `None`: unlesbar (das Objekt ruht).
    fn entschluesseln(
        &self,
        coll: &str,
        id: &str,
        refs: &[String],
        payload: &str,
    ) -> Option<(Eingang, String)> {
        let aad = crypto::aad_ablage(self.konto.user_id, coll, id, refs, &self.key_id, false);
        let plain = crypto::open(self.konto.key, &aad, payload).ok()?;
        let roh: Value = serde_json::from_slice(&plain).ok()?;
        if roh.get("v").and_then(Value::as_u64).unwrap_or(1) > SCHEMA {
            return None; // neueres Schema: nicht verstehen heißt nicht überschreiben
        }
        if coll == SAMMLUNG_ORDNER {
            let o: Ordner = serde_json::from_value(roh).ok()?;
            if o.id != id || !refs.is_empty() {
                return None;
            }
            let h = hash_ordner(&o);
            Some((Eingang::Ordner(o), h))
        } else {
            let d: Datei = serde_json::from_value(roh).ok()?;
            if d.id != id || hub_refs(self.konto.key, &d) != refs {
                return None;
            }
            let h = hash_datei(&d);
            Some((Eingang::Datei(d), h))
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
        let t = b.teilnahmen();
        // (Größe, Klasse: 0 Text, 1 Original) – Texte zuerst, dann nach Größe.
        let mut auftraege: Vec<(u8, u64, String, String, String)> = Vec::new();
        for d in b.dateien.values() {
            if b.begraben.contains(&d.id)
                || !self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id)
                || t.get(&d.id) == Some(&Teilnahme::Halten)
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

    fn ruht(&self, collection: &str, id: &str, jetzt: i64) -> bool {
        self.z
            .rueck
            .get(&schluessel(collection, id))
            .is_some_and(|r| r.bis_ms > jetzt)
    }

    fn angehalten(&self, collection: &str, id: &str) -> bool {
        self.z.objekt(collection, id).is_some_and(|o| o.angehalten)
    }

    fn kandidaten(&mut self, b: &SyncBestand) -> Vec<Kandidat> {
        let t = b.teilnahmen();
        let jetzt = crate::ablage::jetzt_ms();
        let eltern = zf::wirksame_eltern(&b.ordner);
        let mut aus = Vec::new();
        let mut ausstehend = 0u32;
        // Ein festgehaltener Ordner wird erst begraben, wenn keine festgehaltene Datei darunter mehr am
        // Hub lebt (sonst kämen ihre Kopien anderswo vorübergehend unter „Wiederhergestellt“).
        let mut noch_belegt: BTreeSet<String> = BTreeSet::new();
        for d in b.dateien.values() {
            if b.lokal.contains(&d.id) && self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id) {
                noch_belegt.extend(zf::kette(&eltern, &d.ordner.wert));
            }
        }
        let lokalisieren = |collection: &'static str, id: &str| Kandidat {
            collection,
            id: id.to_string(),
            art: Art::Lokalisieren,
            plain: String::new(),
            hash: String::new(),
            refs: Vec::new(),
        };
        for o in b.ordner.values() {
            if b.begraben.contains(&o.id) || self.ruht(SAMMLUNG_ORDNER, &o.id, jetzt) {
                continue;
            }
            if self.angehalten(SAMMLUNG_ORDNER, &o.id) {
                ausstehend += 1;
                continue;
            }
            if b.lokal.contains(&o.id) {
                if self.z.lebt_am_hub(SAMMLUNG_ORDNER, &o.id) {
                    if noch_belegt.contains(&o.id) {
                        ausstehend += 1;
                    } else {
                        aus.push(lokalisieren(SAMMLUNG_ORDNER, &o.id));
                    }
                }
                continue;
            }
            if t.get(&o.id) != Some(&Teilnahme::Sync) {
                continue;
            }
            let wert = wert_ordner(o);
            let hash = collect::hash(&wert);
            if self
                .z
                .objekt(SAMMLUNG_ORDNER, &o.id)
                .is_some_and(|s| s.deleted || s.hash == hash)
            {
                continue; // unverändert, oder am Hub begraben (der Pull hat entschieden)
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
        for d in b.dateien.values() {
            if b.begraben.contains(&d.id) || self.ruht(SAMMLUNG_DATEI, &d.id, jetzt) {
                continue;
            }
            if self.angehalten(SAMMLUNG_DATEI, &d.id) {
                ausstehend += 1;
                continue;
            }
            if b.lokal.contains(&d.id) {
                if self.z.lebt_am_hub(SAMMLUNG_DATEI, &d.id) {
                    // Lokale Kopie prüfen: erst wenn jedes Original hier liegt, verschwindet die Cloud-Kopie.
                    let vollstaendig = d
                        .fassungen
                        .iter()
                        .all(|f| b.originale.contains(&(d.id.clone(), f.fas.clone())));
                    if vollstaendig {
                        aus.push(lokalisieren(SAMMLUNG_DATEI, &d.id));
                    } else {
                        ausstehend += 1;
                    }
                }
                continue;
            }
            if t.get(&d.id) != Some(&Teilnahme::Sync) {
                continue;
            }
            let wert = wert_datei(d);
            let hash = collect::hash(&wert);
            if self
                .z
                .objekt(SAMMLUNG_DATEI, &d.id)
                .is_some_and(|s| s.deleted || s.hash == hash)
            {
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
        // Alte Kennungen nach dem Rückweg lokal → sync: am Hub begraben, Ordner erst nach ihren Dateien.
        let alte_dateien_leben = b.weiterleitung.keys().any(|id| {
            id.starts_with(crate::ablage::PRAEFIX_DATEI) && self.z.lebt_am_hub(SAMMLUNG_DATEI, id)
        });
        for id in b.weiterleitung.keys() {
            let coll = if id.starts_with(crate::ablage::PRAEFIX_DATEI) {
                SAMMLUNG_DATEI
            } else {
                SAMMLUNG_ORDNER
            };
            if !self.z.lebt_am_hub(coll, id)
                || self.angehalten(coll, id)
                || self.ruht(coll, id, jetzt)
            {
                continue;
            }
            if coll == SAMMLUNG_ORDNER && alte_dateien_leben {
                ausstehend += 1;
                continue;
            }
            aus.push(lokalisieren(coll, id));
        }
        for id in &b.begraben {
            for coll in [SAMMLUNG_ORDNER, SAMMLUNG_DATEI] {
                // Ein unlesbarer, neuerer Stand am Hub wird nicht blind begraben.
                if self.z.lebt_am_hub(coll, id)
                    && !self.angehalten(coll, id)
                    && !self.ruht(coll, id, jetzt)
                {
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

    /// Fehler einer einzelnen Hub-Anfrage: Anmeldung abgelaufen bricht den Lauf ab, alles andere stellt
    /// nur dieses Objekt zurück (`Ok(false)`).
    fn einzelfehler(&mut self, id: &str, was: &str, e: HubError) -> Result<bool, HubError> {
        match e {
            HubError::Unauthorized => Err(HubError::Unauthorized),
            HubError::Other(t) if ist_zu_viele(&t) => {
                self.zu_viele_merken();
                Ok(false)
            }
            _ => {
                log::warn!("ablage-sync: {was} für {id} noch nicht möglich");
                Ok(false)
            }
        }
    }

    fn zu_viele_merken(&mut self) {
        self.z.zu_viele = Some(Rueck::naechste(self.z.zu_viele, crate::ablage::jetzt_ms()));
        self.warnen("Ablage: zu viele Dateien im Konto, Hochladen ruht");
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
            // Unmittelbar vor jeder Übertragung frisch prüfen: noch synchron? (Klasse, „nur dieses
            // Gerät“ oder Ordner können sich seit der Momentaufnahme geändert haben.)
            if !self
                .ablage
                .sync_noch_synchron(std::slice::from_ref(&k.id))
                .map_err(ablage_fehler)?
                .contains(&k.id)
            {
                return Ok(false);
            }
            // 1. reservieren (auch beim Wiederverwenden eines vorhandenen Blobs)
            if let Err(e) = self.hub.reservieren(&hid, self.konto.device_id).await {
                return self.einzelfehler(&k.id, "Reservierung", e);
            }
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
                match self.hub.blob_info(&hid).await {
                    Ok(Some(i)) if i.complete => {
                        self.z.hochgeladen.insert(hid);
                        continue;
                    }
                    Ok(_) => return Ok(false),
                    Err(e) => return self.einzelfehler(&k.id, "Blob-Abfrage", e),
                }
            };
            let ueber = self.z.kontingent_voll
                || self
                    .z
                    .verbrauch
                    .is_some_and(|v| v.quota > 0 && v.used.saturating_add(size) > v.quota);
            if ueber {
                // Schon am Hub (z. B. von einem anderen Gerät): kein neuer Speicher nötig.
                match self.hub.blob_info(&hid).await {
                    Ok(Some(i)) if i.complete => {
                        self.z.hochgeladen.insert(hid);
                        continue;
                    }
                    Ok(_) => {}
                    Err(e) => return self.einzelfehler(&k.id, "Blob-Abfrage", e),
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
                    // Dauerhaft im Ablagestand (Wartung „Blobs abgleichen“).
                    if let Err(e) = self.ablage.sync_blob_vermerken(&hid, &self.kennung) {
                        log::warn!("ablage-sync: Blob nicht vermerkt: {}", e.code);
                    }
                    self.z.hochgeladen.insert(hid);
                    if let Some(v) = self.z.verbrauch.as_mut() {
                        v.used = v.used.saturating_add(size);
                    }
                }
                Ok(Upload::Later) => return Ok(false),
                Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
                Err(HubError::Other(e)) if ist_zu_viele(&e) => {
                    self.zu_viele_merken();
                    return Ok(false);
                }
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
            let jetzt = crate::ablage::jetzt_ms();
            for k in alle {
                if k.collection == SAMMLUNG_DATEI && k.art == Art::Stand {
                    if self.z.zu_viele.is_some_and(|r| r.bis_ms > jetzt) {
                        self.bericht.ausstehend += 1;
                        continue;
                    }
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
                // Unmittelbar vor dem Push frisch prüfen (unter der Ablage-Sperre): was nicht mehr
                // synchron ist, fällt heraus; die Momentaufnahme der Ablage hat es festgehalten.
                let staende: Vec<String> = stapel
                    .iter()
                    .filter(|k| k.art == Art::Stand)
                    .map(|k| k.id.clone())
                    .collect();
                let noch = self
                    .ablage
                    .sync_noch_synchron(&staende)
                    .map_err(ablage_fehler)?;
                let stapel: Vec<Kandidat> = stapel
                    .iter()
                    .filter(|k| k.art != Art::Stand || noch.contains(&k.id))
                    .cloned()
                    .collect();
                if stapel.is_empty() {
                    continue;
                }
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
            grab_nutzlast(self.konto.key, self.konto.user_id, k.collection, &k.id)
                .map_err(HubError::Other)?
        } else {
            crypto::seal(
                self.konto.key,
                &crypto::aad_ablage(
                    self.konto.user_id,
                    k.collection,
                    &k.id,
                    &k.refs,
                    &self.key_id,
                    false,
                ),
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
                self.z.rueck.remove(&schl);
                self.z.objekte.insert(
                    schl,
                    ObjStand {
                        revision: r.revision,
                        hash: k.hash.clone(),
                        deleted: grab,
                        refs: if grab { Vec::new() } else { k.refs.clone() },
                        angehalten: false,
                    },
                );
                let neu: BTreeSet<&String> = k.refs.iter().collect();
                for alt in alt_refs.iter().filter(|a| grab || !neu.contains(a)) {
                    self.z.blob_loeschen.insert(alt.clone());
                }
                // Nach einer festgehaltenen Datei kann ihr Ordner in der nächsten Runde folgen.
                Ok(k.art == Art::Lokalisieren && k.collection == SAMMLUNG_DATEI)
            }
            ("superseded", _) => {
                match &r.current {
                    Some(cur) if cur.deleted => {
                        // Endgültiger Grabstein am Hub: nur mit gültigem Siegel lokal anwenden (die
                        // Ablage hält ihn außerdem von festgehaltenen Einträgen fern).
                        let echt = grab_echt(
                            self.konto.key,
                            self.konto.user_id,
                            k.collection,
                            &k.id,
                            &cur.key_id,
                            &cur.payload,
                        );
                        self.z.objekte.insert(
                            schl,
                            ObjStand {
                                revision: cur.revision,
                                deleted: true,
                                ..Default::default()
                            },
                        );
                        if !echt {
                            self.warnen(
                                "Ablage: ein Grabstein ohne gültiges Siegel wurde ignoriert",
                            );
                        } else if k.art == Art::Stand {
                            let e = self
                                .ablage
                                .sync_einspielen(&[Eingang::Grab(k.id.clone())])
                                .map_err(ablage_fehler)?;
                            if !e.begraben.is_empty() {
                                self.bericht.geaendert = true;
                            }
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
                        let angehalten = gelesen.is_none();
                        let hash = match gelesen {
                            Some((e, h)) => {
                                let ein =
                                    self.ablage.sync_einspielen(&[e]).map_err(ablage_fehler)?;
                                if !ein.geaendert.is_empty() {
                                    self.bericht.geaendert = true;
                                }
                                h
                            }
                            None => {
                                // Unlesbar: nie überschreiben, das Objekt ruht.
                                self.warnen(
                                    "Ablage: ein Eintrag vom Hub ist hier nicht lesbar und ruht",
                                );
                                String::new()
                            }
                        };
                        self.z.objekte.insert(
                            schl,
                            ObjStand {
                                revision: cur.revision,
                                hash,
                                deleted: false,
                                refs,
                                angehalten,
                            },
                        );
                        Ok(!angehalten)
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
                // Grabstein am Hub, den dieser Lauf nicht gezogen hat: nichts lokal anwenden (das
                // tut nur ein Pull mit geprüftem Siegel); Revision 0, damit der Pull ihn noch liest.
                self.z.objekte.insert(
                    schl,
                    ObjStand {
                        revision: 0,
                        deleted: true,
                        ..Default::default()
                    },
                );
                Ok(false)
            }
            ("rejected", grund) => {
                log::warn!(
                    "ablage-sync: {}/{} abgewiesen ({})",
                    k.collection,
                    k.id,
                    grund.unwrap_or("ohne Grund")
                );
                // Rückstellung mit wachsendem Abstand; bis dahin keine neue Reservierung.
                let alt = self.z.rueck.get(&schl).copied();
                self.z
                    .rueck
                    .insert(schl, Rueck::naechste(alt, crate::ablage::jetzt_ms()));
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

    /// Hub-Kennungen, die noch gebraucht werden: Fassungen synchroner (oder unklarer) Dateien und
    /// alles, was ein lebendes oder ruhendes Objekt am Hub referenziert.
    fn lebende_blobs(&self, b: &SyncBestand) -> BTreeSet<String> {
        let t = b.teilnahmen();
        let mut lebend: BTreeSet<String> = BTreeSet::new();
        for d in b.dateien.values() {
            if !b.begraben.contains(&d.id) && t.get(&d.id) != Some(&Teilnahme::Lokal) {
                lebend.extend(hub_refs(self.konto.key, d));
            }
        }
        for o in self.z.objekte.values().filter(|o| !o.deleted) {
            lebend.extend(o.refs.iter().cloned());
        }
        lebend
    }

    /// Bedingte Löschung der Kandidaten: nur ohne lebende Fassung im (vollständig gezogenen) Bestand;
    /// 409 (referenziert/reserviert) ist ein Ergebnis, der Kandidat entfällt. Danach die Wartung
    /// „Blobs abgleichen“: eigene, länger als 7 Tage unbenutzte Blobs ebenso bedingt löschen.
    async fn blobs_loeschen(&mut self) -> Result<(), HubError> {
        let b = self.ablage.sync_bestand().map_err(ablage_fehler)?;
        let lebend = self.lebende_blobs(&b);
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
        let jetzt = crate::ablage::jetzt_ms();
        let mut weg: Vec<String> = Vec::new();
        for (blob, info) in &b.sync_blobs {
            if info.konto != self.kennung || jetzt - info.ms < WAISE_MS || lebend.contains(blob) {
                continue;
            }
            match self.hub.blob_loeschen(blob, self.konto.device_id).await {
                Ok(BlobLoeschung::Geloescht(_)) => {
                    self.z.hochgeladen.remove(blob);
                    weg.push(blob.clone());
                }
                Ok(BlobLoeschung::InGebrauch(_)) => {}
                Err(HubError::Unauthorized) => return Err(HubError::Unauthorized),
                Err(_) => {}
            }
        }
        // Was der Abgleich noch braucht, bleibt in der Liste (es kann später verwaisen).
        self.ablage
            .sync_blobs_vergessen(&weg)
            .map_err(ablage_fehler)?;
        Ok(())
    }
}

/// „Zu viele Ablagen“ (Anzahl der Blobs am Hub, 507 `too_many_blobs`), getrennt von „Speicher voll“
/// (`quota_exceeded`). Nur der Fehlercode zählt (`code` oder `code: Meldung`, siehe
/// `sync::client`), nie der Meldungstext.
fn ist_zu_viele(text: &str) -> bool {
    text.split_once(": ").map_or(text, |(c, _)| c) == "too_many_blobs"
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
    // Rückweg lokal → sync (D4): neue Kennungen vor dem Lauf, damit er sie gleich hochlädt.
    let projekte = crate::portable::app_data_dir(app)
        .map_err(|e| HubError::Other(e.to_string()))?
        .join("projekte");
    let a = ablage.clone();
    let rueck = tauri::async_runtime::spawn_blocking(move || {
        a.rueckweg(&crate::ablage::speicher::ProjektVerweise::neu(projekte))
    })
    .await;
    if let Ok(Err(e)) = rueck {
        log::warn!("ablage-sync: Rückweg lokal → sync: {}", e.code);
    }
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
    Angehalten,
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
    let t = b.teilnahmen();
    let mut aus = Vec::new();
    for o in b.ordner.values() {
        let zustand = match t.get(&o.id) {
            _ if z
                .objekt(SAMMLUNG_ORDNER, &o.id)
                .is_some_and(|s| s.angehalten) =>
            {
                AblageSyncZustand::Angehalten
            }
            Some(Teilnahme::Sync) => {
                if z.objekt(SAMMLUNG_ORDNER, &o.id)
                    .is_some_and(|s| s.hash == hash_ordner(o))
                {
                    AblageSyncZustand::Synchronisiert
                } else {
                    AblageSyncZustand::Ausstehend
                }
            }
            _ => AblageSyncZustand::NurGeraet,
        };
        aus.push(AblageSyncEintrag {
            id: o.id.clone(),
            zustand,
        });
    }
    for d in b.dateien.values() {
        let zustand = match t.get(&d.id) {
            _ if z
                .objekt(SAMMLUNG_DATEI, &d.id)
                .is_some_and(|s| s.angehalten) =>
            {
                AblageSyncZustand::Angehalten
            }
            Some(Teilnahme::Sync) if zf::konflikt(d) => AblageSyncZustand::Konflikt,
            Some(Teilnahme::Sync) if wartet_auf_text(b, d) => AblageSyncZustand::WartetAufText,
            Some(Teilnahme::Sync) => {
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
            _ => AblageSyncZustand::NurGeraet,
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
