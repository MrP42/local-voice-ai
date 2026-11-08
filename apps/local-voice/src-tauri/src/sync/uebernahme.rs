//! Lokale Daten bei Anmeldung, Abmeldung und Kontowechsel (lva#400): Tauri-Hülle um
//! `uebernahme_kern`. Spec: `docs/superpowers/specs/2026-11-08-lokale-daten-bei-anmeldung.md`.
//!
//! Ablauf Anmeldung: `sync_login` prüft die Zugangsdaten und legt eine **wartende**
//! Anmeldung (`sync-anmeldung.json`) an, aber kein `sync.json`: Ohne `sync.json` läuft kein
//! Zyklus, kein Relay, keine Kopplung (U1). Die Wahl im Modal (`sync_anmeldung_entscheiden`)
//! oder der Abbruch (`sync_anmeldung_abbrechen`) beendet den Zustand. Alle verändernden
//! Abläufe gehen über das Journal (U11) und setzen nach einem Absturz fort.

use super::account::{self, SyncConfig};
use super::client::{HubClient, RemoteObject};
use super::collect::{self, LocalObject};
use super::engine::{self, SyncEngine, SyncStatus};
use super::ledger::{self, Ledger};
use super::uebernahme_kern::{self as kern, Art, Journal, Phase, Plan, Wahl};
use super::{book_files, books, crypto};
use crate::commands::pages;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, State};

const WARTEND: &str = "sync-anmeldung.json";
const WIDERRUF: &str = "sync-widerruf.json";
/// Lesender Pull der Kontokennungen (U2): Seitengröße und Obergrenze wie im Zyklus.
const PULL_LIMIT: u32 = 100;
const PULL_MAX_PAGES: usize = 500;

// ---------------------------------------------------------------------------
// Ablageorte
// ---------------------------------------------------------------------------

/// Datenordner mit `projects/` und `library/` (dort liegen auch Herkunft, Nur-lokal,
/// Journal und Sicherungen).
pub fn data(app: &AppHandle) -> Result<PathBuf, String> {
    pages::data_root(app)
}

fn konto_datei(app: &AppHandle, name: &str) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app)
        .map(|d| d.join(name))
        .map_err(|e| format!("Kein Datenverzeichnis: {e}"))
}

#[derive(Serialize, Deserialize, Clone)]
struct Wartend {
    cfg: SyncConfig,
    /// Schlüssel (`page/<id>`, `book/<id>`) der nicht gelöschten Kontoobjekte.
    konto: BTreeSet<String>,
    /// Lose Seiten, die im Konto mit gleichem Inhalt liegen (keine Kollision).
    gleiche_seiten: BTreeSet<String>,
}

fn wartend_laden(app: &AppHandle) -> Option<Wartend> {
    let raw = std::fs::read(konto_datei(app, WARTEND).ok()?).ok()?;
    serde_json::from_slice(&raw).ok()
}

fn wartend_speichern(app: &AppHandle, w: &Wartend) -> Result<(), String> {
    let raw = serde_json::to_vec(w).map_err(|e| e.to_string())?;
    account::write_private(&konto_datei(app, WARTEND)?, &raw)
}

fn wartend_loeschen(app: &AppHandle) {
    if let Ok(p) = konto_datei(app, WARTEND) {
        let _ = std::fs::remove_file(p);
    }
}

use kern::Widerruf;

/// Lese-Ändern-Schreiben der Widerrufsliste (nie über ein `await` gehalten).
static WIDERRUF_SPERRE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Ausstehende Widerrufe. Beschädigt oder unlesbar: Fehler (nie leer, U10).
fn widerrufe_laden(app: &AppHandle) -> Result<Vec<Widerruf>, String> {
    kern::widerrufe_laden(&konto_datei(app, WIDERRUF)?)
}

fn widerrufe_speichern(app: &AppHandle, liste: &[Widerruf]) -> Result<(), String> {
    let p = konto_datei(app, WIDERRUF)?;
    if liste.is_empty() {
        return match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        };
    }
    let raw = serde_json::to_vec(liste).map_err(|e| e.to_string())?;
    account::write_private(&p, &raw)
}

/// Widerrufe dauerhaft vormerken, BEVOR die letzte Kopie eines Tokens verschwindet (U10).
/// Bestehende Aufträge bleiben; eine beschädigte Liste und Schreibfehler brechen ab.
fn widerrufe_vormerken(app: &AppHandle, neu: &[Widerruf]) -> Result<(), String> {
    let _sperre = WIDERRUF_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
    kern::widerrufe_vormerken(&konto_datei(app, WIDERRUF)?, neu, account::write_private)
}

fn widerruf_vormerken(app: &AppHandle, url: &str, token: &str) -> Result<(), String> {
    widerrufe_vormerken(
        app,
        &[Widerruf {
            url: url.to_string(),
            token: token.to_string(),
        }],
    )
}

/// Tokens widerrufen: erst vormerken, dann ausführen (scheitert das Netz, beim nächsten
/// Start). Lässt sich nicht vormerken, wird sofort widerrufen; scheitert auch das, ist es
/// ein Fehler: der Aufrufer darf die Tokenquelle dann nicht löschen.
async fn widerrufen_alle(app: &AppHandle, ws: &[Widerruf]) -> Result<(), String> {
    if let Err(e) = widerrufe_vormerken(app, ws) {
        log::warn!("Sync: Widerruf nicht vorgemerkt: {e}");
        for w in ws {
            if let Err(n) = HubClient::new(&w.url, &w.token).logout().await {
                log::warn!("Sync: Widerruf gescheitert: {n}");
                return Err(format!(
                    "Widerruf nicht vorgemerkt ({e}) und nicht erreichbar"
                ));
            }
        }
        return Ok(());
    }
    widerruf_nachholen(app).await;
    Ok(())
}

async fn widerrufen(app: &AppHandle, url: &str, token: &str) -> Result<(), String> {
    widerrufen_alle(
        app,
        &[Widerruf {
            url: url.to_string(),
            token: token.to_string(),
        }],
    )
    .await
}

/// Ein nur im Speicher gehaltenes Token widerrufen (es gibt keine Datei, die bliebe).
async fn widerrufen_fluechtig(app: &AppHandle, url: &str, token: &str) {
    if let Err(e) = widerrufen(app, url, token).await {
        log::warn!("Sync: {e}");
    }
}

/// Ausstehende Widerrufe ausführen (Start, Anmeldung, nach Abbrechen/Abmelden).
/// Eine beschädigte Liste bleibt unangetastet (Hinweis „Widerruf offen“ bleibt sichtbar).
pub async fn widerruf_nachholen(app: &AppHandle) {
    let offen = {
        let _sperre = WIDERRUF_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
        widerrufe_laden(app)
    };
    let offen = match offen {
        Ok(o) => o,
        Err(e) => {
            log::warn!("Sync: Widerrufsliste: {e}");
            return;
        }
    };
    let mut erledigt = Vec::new();
    for w in offen {
        if HubClient::new(&w.url, &w.token).logout().await.is_ok() {
            erledigt.push(w);
        }
    }
    if erledigt.is_empty() {
        return;
    }
    let _sperre = WIDERRUF_SPERRE.lock().unwrap_or_else(|e| e.into_inner());
    let rest: Vec<Widerruf> = match widerrufe_laden(app) {
        Ok(l) => l.into_iter().filter(|w| !erledigt.contains(w)).collect(),
        Err(e) => {
            log::warn!("Sync: Widerrufsliste: {e}");
            return;
        }
    };
    if let Err(e) = widerrufe_speichern(app, &rest) {
        log::warn!("Sync: Widerrufsliste nicht gespeichert: {e}");
    }
}

pub fn widerruf_offen(app: &AppHandle) -> bool {
    konto_datei(app, WIDERRUF).is_ok_and(|p| p.is_file())
}

// ---------------------------------------------------------------------------
// Oberflächen-Typen
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct Kollision {
    pub titel: String,
    /// `seite` (wird unter neuer Kennung lokal behalten) oder `buch` (bleibt in der Sicherung).
    pub art: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, specta::Type)]
pub struct Entscheidung {
    pub konto: String,
    /// Konto, aus dem die lokalen Daten zuletzt kamen, wenn es ein anderes ist.
    pub vorher_konto: Option<String>,
    pub seiten: u32,
    pub buecher: u32,
    pub fortschritte: u32,
    pub dateien: u32,
    pub einstellungen: bool,
    /// Größe der Sicherung in Bytes (geschätzt aus dem lokalen Bestand).
    pub sicherung_bytes: f64,
    pub kollisionen: Vec<Kollision>,
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct AnmeldeErgebnis {
    pub status: SyncStatus,
    /// `Some`: das Modal zeigen; `None`: ohne Rückfrage freigegeben.
    pub entscheidung: Option<Entscheidung>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AnmeldeWahl {
    Zusammenfuehren,
    GetrenntHalten,
    Entfernen,
}

impl From<AnmeldeWahl> for Wahl {
    fn from(w: AnmeldeWahl) -> Self {
        match w {
            AnmeldeWahl::Zusammenfuehren => Wahl::Zusammenfuehren,
            AnmeldeWahl::GetrenntHalten => Wahl::GetrenntHalten,
            AnmeldeWahl::Entfernen => Wahl::Entfernen,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct Sicherung {
    pub name: String,
    pub erstellt_ms: f64,
    pub anlass: String,
    pub konto: Option<String>,
    pub seiten: u32,
    pub buecher: u32,
    pub dateien: u32,
    pub bytes: f64,
    /// Voller Pfad des Sicherungsordners (für „Ordner öffnen“).
    pub ordner: String,
}

impl Sicherung {
    fn aus(data: &Path, s: kern::SicherungInfo) -> Self {
        Self {
            ordner: data
                .join(kern::SICHERUNGEN)
                .join(&s.name)
                .to_string_lossy()
                .into_owned(),
            name: s.name,
            erstellt_ms: s.erstellt_ms as f64,
            anlass: s.anlass,
            konto: s.konto,
            seiten: s.seiten,
            buecher: s.buecher,
            dateien: s.dateien,
            bytes: s.bytes as f64,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, specta::Type)]
pub struct WiederherstellBericht {
    pub zurueck: u32,
    pub zusaetzlich: u32,
    pub gleich: u32,
    /// Abweichende Bücher, die in der Sicherung bleiben (Titel oder Kennung).
    pub in_sicherung: Vec<String>,
    pub fortschritte_ergaenzt: u32,
    pub fortschritte_abweichend: u32,
    pub ordner: String,
}

// ---------------------------------------------------------------------------
// Bestand
// ---------------------------------------------------------------------------

struct Erfasst {
    objekte: Vec<kern::Objekt>,
    /// Seite → Buch (Seiten im Buch-Ordner).
    seite_im_buch: HashMap<String, String>,
    titel: HashMap<String, String>,
    seiten_werte: HashMap<String, Value>,
}

/// Alle lokalen Sync-Objekte mit Hash, mit denselben Sammelfunktionen wie der Zyklus.
fn erfassen(app: &AppHandle, cfg: &SyncConfig) -> Result<Erfasst, String> {
    let key = cfg.key()?;
    let mut led = ledger::load(app);
    let (locals, held_pages) = collect::collect(app)?;
    let mut budget = u64::MAX;
    let lib = books::collect_library(app, &key, &mut led.files, &mut budget)?;
    let extra = book_files::collect(app, &key, &cfg.device_id, &cfg.device_name, &mut budget)?;
    let index = pages::load_index(app)?;
    let mut e = Erfasst {
        objekte: Vec::new(),
        seite_im_buch: HashMap::new(),
        titel: HashMap::new(),
        seiten_werte: HashMap::new(),
    };
    for p in &index.pages {
        e.titel.insert(p.id.clone(), p.title.clone());
        if let Some(b) = p.buch.as_ref().filter(|b| kern::gueltiges_buch(b)) {
            e.seite_im_buch.insert(p.id.clone(), b.clone());
        }
    }
    let mut add = |o: &LocalObject, hash: Option<String>| {
        e.objekte.push(kern::Objekt {
            sammlung: o.collection.to_string(),
            id: o.id.clone(),
            hash,
            einheit: kern::einheit_von(o.collection, &o.id, &o.value),
            map_schluessel: kern::map_schluessel(o.collection, &o.value),
        });
    };
    for o in locals
        .iter()
        .chain(lib.objects.iter())
        .chain(extra.objects.iter())
    {
        let held = o.collection == books::COLLECTION_BOOK && lib.held.contains(&o.id);
        add(o, (!held).then(|| collect::hash(&o.value)));
        if o.collection == collect::COLLECTION_PAGE {
            e.seiten_werte.insert(o.id.clone(), o.value.clone());
        }
    }
    for id in held_pages {
        e.objekte.push(kern::Objekt {
            sammlung: collect::COLLECTION_PAGE.into(),
            einheit: Some(id.clone()),
            id,
            hash: None,
            map_schluessel: None,
        });
    }
    for id in &lib.held {
        if !e
            .objekte
            .iter()
            .any(|o| o.sammlung == "book" && &o.id == id)
        {
            e.objekte.push(kern::Objekt {
                sammlung: books::COLLECTION_BOOK.into(),
                id: id.clone(),
                hash: None,
                einheit: Some(id.clone()),
                map_schluessel: None,
            });
        }
    }
    // Titel der Bücher für die Anzeige.
    if let Ok(root) = crate::library::sync_root(app) {
        for id in kern::buecher(root.parent().unwrap_or(&root)) {
            if let Some(b) = crate::library::sync_read_book(&root.join(&id)) {
                e.titel.insert(id, b.title);
            }
        }
    }
    Ok(e)
}

/// Seiten im Buch-Ordner gehören zu ihrem Buch; sonst die Einheit selbst.
fn effektiv(e: &Erfasst, einheit: &str) -> String {
    e.seite_im_buch
        .get(einheit)
        .cloned()
        .unwrap_or_else(|| einheit.to_string())
}

fn ist_einheitsobjekt(o: &kern::Objekt) -> bool {
    matches!(o.sammlung.as_str(), "page" | "book")
}

fn konto_hat(w: &Wartend, einheit: &str) -> bool {
    let s = if kern::gueltiges_buch(einheit) {
        "book"
    } else {
        "page"
    };
    w.konto.contains(&kern::schluessel(s, einheit))
}

fn herkunft_fuer(app: &AppHandle, cfg: &SyncConfig) -> Option<kern::Herkunft> {
    let d = data(app).ok()?;
    kern::herkunft_laden(&d).filter(|h| h.gleiches_konto(cfg.user_id, &cfg.url))
}

/// Bestand berechnen (Kopien der Objekte, damit `Erfasst` weiterverwendbar bleibt).
fn bestand_von(
    app: &AppHandle,
    cfg: &SyncConfig,
    e: &Erfasst,
) -> Result<Vec<kern::Objekt>, String> {
    let herkunft = herkunft_fuer(app, cfg);
    let nur = kern::nur_lokal_laden(&data(app)?)?;
    Ok(kern::bestand(&e.objekte, herkunft.as_ref(), &nur)
        .into_iter()
        .cloned()
        .collect())
}

/// Der Bestand verlangt eine Wahl, wenn mehr als die Einstellungen betroffen ist. Nur
/// abweichende Einstellungen: Sicherung und Zusammenführen ohne Modal (Kontowerte gelten,
/// die lokalen liegen in der Sicherung).
fn braucht_wahl(bestand: &[kern::Objekt]) -> bool {
    bestand
        .iter()
        .any(|o| o.sammlung != collect::COLLECTION_SETTINGS)
}

fn entscheidung(
    app: &AppHandle,
    w: &Wartend,
    e: &Erfasst,
    bestand: &[kern::Objekt],
) -> Entscheidung {
    let d = data(app).ok();
    let vorher = d
        .as_deref()
        .and_then(kern::herkunft_laden)
        .filter(|h| !h.gleiches_konto(w.cfg.user_id, &w.cfg.url))
        .map(|h| h.email);
    let mut einheiten = BTreeSet::new();
    let mut out = Entscheidung {
        konto: w.cfg.user_email.clone(),
        vorher_konto: vorher,
        ..Default::default()
    };
    for o in bestand {
        match o.sammlung.as_str() {
            "progress" | "bookmark" => out.fortschritte += 1,
            "book_file" => out.dateien += 1,
            "settings" => out.einstellungen = true,
            _ => {}
        }
        if let Some(u) = &o.einheit {
            einheiten.insert(effektiv(e, u));
        }
    }
    for u in &einheiten {
        if kern::gueltiges_buch(u) {
            out.buecher += 1;
        } else {
            out.seiten += 1;
        }
        let kollidiert = konto_hat(w, u)
            && bestand.iter().any(|o| {
                ist_einheitsobjekt(o)
                    && o.einheit.as_deref().map(|x| effektiv(e, x)).as_deref() == Some(u)
            })
            && !w.gleiche_seiten.contains(u);
        if kollidiert {
            out.kollisionen.push(Kollision {
                titel: e.titel.get(u).cloned().unwrap_or_else(|| u.clone()),
                art: if kern::gueltiges_buch(u) {
                    "buch"
                } else {
                    "seite"
                }
                .into(),
            });
        }
    }
    if let Some(d) = d {
        out.sicherung_bytes = kern::umfang(&d)
            .map(|f| kern::platz_bedarf(&d, &f) as f64)
            .unwrap_or(0.0);
    }
    out
}

/// Plan nach U3/U5 aus Bestand, Kontokennungen und Wahl.
fn plan_bauen(d: &Path, wahl: Wahl, w: &Wartend, e: &Erfasst, bestand: &[kern::Objekt]) -> Plan {
    let mut plan = Plan::default();
    if wahl == Wahl::Zusammenfuehren {
        return plan;
    }
    // 1. Einheiten, deren eigenes Objekt (Seite, Buch, Seite im Buch-Ordner) im Bestand ist.
    let mut eigene: BTreeSet<String> = BTreeSet::new();
    for o in bestand.iter().filter(|o| ist_einheitsobjekt(o)) {
        if let Some(u) = &o.einheit {
            eigene.insert(effektiv(e, u));
        }
    }
    let index_ids: BTreeSet<String> = e.titel.keys().cloned().collect();
    let mut probe_index = serde_json::json!({ "pages": index_ids.iter().map(|i| serde_json::json!({"id": i})).collect::<Vec<_>>() });
    for u in &eigene {
        let im_konto = konto_hat(w, u);
        match wahl {
            Wahl::GetrenntHalten if !im_konto => {
                plan.nur_lokal.einheiten.insert(u.clone());
            }
            Wahl::GetrenntHalten if kern::gueltige_seite(u) && w.gleiche_seiten.contains(u) => {}
            Wahl::GetrenntHalten if kern::gueltige_seite(u) => {
                let neu = kern::neue_seiten_kennung(d, &probe_index);
                if let Some(p) = probe_index["pages"].as_array_mut() {
                    p.push(serde_json::json!({ "id": neu }));
                }
                plan.kopien.insert(u.clone(), neu.clone());
                plan.nur_lokal.einheiten.insert(neu);
                plan.entfernen.insert(u.clone());
                plan.freigeben.insert(u.clone());
            }
            Wahl::GetrenntHalten => {
                // Buch im Konto mit abweichendem lokalem Stand: lokal nur in der Sicherung.
                plan.entfernen.insert(u.clone());
                plan.freigeben.insert(u.clone());
            }
            Wahl::Entfernen => {
                plan.entfernen.insert(u.clone());
            }
            Wahl::Zusammenfuehren => {}
        }
    }
    // 2. Abhängige Objekte und Objekte ohne Einheit.
    for o in bestand.iter().filter(|o| !ist_einheitsobjekt(o)) {
        let einheit = o.einheit.as_deref().map(|u| effektiv(e, u));
        if let Some(u) = &einheit {
            if eigene.contains(u) {
                continue; // folgt seiner Einheit
            }
        }
        let k = kern::schluessel(&o.sammlung, &o.id);
        match (wahl, o.sammlung.as_str(), &einheit) {
            (Wahl::GetrenntHalten, _, Some(u)) if !konto_hat(w, u) => {
                plan.nur_lokal.einheiten.insert(u.clone());
            }
            (Wahl::GetrenntHalten, _, _) => {
                plan.nur_lokal.schluessel.insert(k);
            }
            (Wahl::Entfernen, "progress" | "bookmark", _) => {
                if let Some(m) = &o.map_schluessel {
                    plan.fortschritt.insert(m.clone());
                }
            }
            (Wahl::Entfernen, "book_file", Some(u)) => {
                // Neue Datei in einem Buch des Kontos: Buch lokal entfernen, es kommt
                // aus dem Konto zurück; die Datei liegt in der Sicherung.
                plan.entfernen.insert(u.clone());
            }
            _ => {} // Einstellungen beim Entfernen: Kontowerte gelten (Sicherung hat die lokalen)
        }
    }
    plan
}

// ---------------------------------------------------------------------------
// Kontokennungen lesend holen (U2)
// ---------------------------------------------------------------------------

async fn konto_lesen(
    cfg: &SyncConfig,
    seiten: &HashMap<String, Value>,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    let key = cfg.key()?;
    let my_key_id = crypto::key_id(&key);
    let client = HubClient::new(&cfg.url, &cfg.token);
    let mut letzter: BTreeMap<String, RemoteObject> = BTreeMap::new();
    let mut since = 0;
    for _ in 0..PULL_MAX_PAGES {
        let page = client
            .pull(since, PULL_LIMIT)
            .await
            .map_err(|e| e.to_string())?;
        for o in page.objects {
            if matches!(o.collection.as_str(), "page" | "book") {
                letzter.insert(kern::schluessel(&o.collection, &o.object_id), o);
            }
        }
        if !page.more || page.cursor <= since {
            break;
        }
        since = page.cursor;
    }
    let mut konto = BTreeSet::new();
    let mut gleich = BTreeSet::new();
    for (k, o) in letzter {
        if o.deleted {
            continue;
        }
        if o.collection == collect::COLLECTION_PAGE && o.key_id == my_key_id {
            if let (Some(lokal), Ok(fern)) =
                (seiten.get(&o.object_id), engine::decrypt(&key, cfg, &o))
            {
                if !collect::pages_differ(lokal, &fern) {
                    gleich.insert(o.object_id.clone());
                }
            }
        }
        konto.insert(k);
    }
    Ok((konto, gleich))
}

// ---------------------------------------------------------------------------
// Übergang ausführen (U11)
// ---------------------------------------------------------------------------

fn einstellungen_jetzt(app: &AppHandle) -> Value {
    collect::settings_subset(
        &serde_json::to_value(crate::settings::get_settings(app)).unwrap_or(Value::Null),
    )
}

fn anlass_von(art: Art) -> &'static str {
    match art {
        Art::Anmeldung => "anmeldung",
        Art::Abmeldung => "abmeldung",
        Art::Kollision => "kollision",
        Art::Uebernahme => "freigabe",
        Art::Wiederherstellen => "wiederherstellen",
    }
}

/// Sichern (Phase `entscheidung` → `gesichert`) und umsetzen (`gesichert` → `umgesetzt`),
/// unter Seiten- und Bibliothekssperre. Gibt die entfernten Fortschrittsschlüssel zurück.
fn sichern_und_umsetzen(app: &AppHandle, j: &mut Journal) -> Result<Vec<String>, String> {
    let d = data(app)?;
    let _bibliothek = crate::library::sync_lock();
    let _seiten = pages::lock();
    if j.phase == Phase::Entscheidung {
        let einst = einstellungen_jetzt(app);
        let anlass = anlass_von(j.art);
        let info = kern::sichern(
            &d,
            &kern::Auftrag {
                anlass,
                konto: Some(&j.email),
                einstellungen: &einst,
                platz_frei: crate::commands::library_layout::freier_platz(&d),
            },
        )?;
        j.sicherung = Some(info.name);
        j.phase = Phase::Gesichert;
        kern::journal_speichern(&d, j)?;
    }
    let mut entfernt = Vec::new();
    if j.phase == Phase::Gesichert {
        let name = j.sicherung.clone().ok_or("Journal ohne Sicherung")?;
        let dir = kern::sicherungs_ordner(&d, &name)?;
        if let Err(e) = kern::pruefen(&dir) {
            // Blockieren: die Journal-Sicherung bleibt die einzige Grundlage (U11).
            // Zusätzlich den Restbestand schützen, ohne das Journal zu ändern.
            let einst = einstellungen_jetzt(app);
            let _ = kern::sichern(
                &d,
                &kern::Auftrag {
                    anlass: "schutz",
                    konto: Some(&j.email),
                    einstellungen: &einst,
                    platz_frei: crate::commands::library_layout::freier_platz(&d),
                },
            );
            return Err(format!(
                "Die Sicherung {name} ist nicht lesbar ({e}). Der Übergang ist angehalten; es wurde nichts entfernt."
            ));
        }
        let u = match kern::umsetzen(&d, &name, &j.plan) {
            Err(e) if kern::ist_abweichung(&e) => {
                // Lokal seit der Sicherung geändert (nichts entfernt): den jetzigen Stand
                // zusätzlich sichern und gegen diese Sicherung umsetzen. Die frühere
                // bleibt erhalten (Sicherungen werden nie automatisch gelöscht).
                let einst = einstellungen_jetzt(app);
                let info = kern::sichern(
                    &d,
                    &kern::Auftrag {
                        anlass: anlass_von(j.art),
                        konto: Some(&j.email),
                        einstellungen: &einst,
                        platz_frei: crate::commands::library_layout::freier_platz(&d),
                    },
                )?;
                j.sicherung = Some(info.name.clone());
                kern::journal_speichern(&d, j)?;
                kern::umsetzen(&d, &info.name, &j.plan)?
            }
            r => r?,
        };
        entfernt = u.entfernte_fortschritte;
        j.phase = Phase::Umgesetzt;
        kern::journal_speichern(&d, j)?;
    }
    Ok(entfernt)
}

fn neues_ledger(cfg: &SyncConfig) -> Ledger {
    Ledger {
        konto: Some(kern::konto_kennung(cfg.user_id, &cfg.url)),
        ..Default::default()
    }
}

/// Phase `umgesetzt` abschließen (je Art) und Journal löschen.
fn abschliessen(app: &AppHandle, j: &Journal) -> Result<(), String> {
    let d = data(app)?;
    match j.art {
        Art::Anmeldung => {
            match wartend_laden(app) {
                Some(w) if kern::wartend_passt(j, w.cfg.user_id, &w.cfg.url) => {
                    account::save(app, &w.cfg)?;
                    ledger::save(app, &neues_ledger(&w.cfg))?;
                    wartend_loeschen(app);
                }
                Some(_) => {
                    return Err("Die wartende Anmeldung gehört zu einem anderen Konto.".into())
                }
                // Absturz nach dem Löschen der wartenden Anmeldung: nur abgeschlossen, wenn
                // Konto UND Ledger zum Journal gehören.
                None => {
                    let cfg = account::load(app);
                    let led = ledger::load(app);
                    if !kern::anmeldung_gebunden(
                        j,
                        cfg.as_ref().map(|c| (c.user_id, c.url.as_str())),
                        led.konto.as_deref(),
                    ) {
                        return Err("Anmeldung unvollständig: Konto oder Ledger fehlt.".into());
                    }
                }
            }
        }
        Art::Abmeldung | Art::Wiederherstellen => {}
        Art::Kollision | Art::Uebernahme => {
            // Ledger bereinigen: markierte und freigegebene Objekte ohne Basisrevision;
            // freigegebene Kennungen erneut einlesen, damit die Kontofassung ankommt.
            let mut led = ledger::load(app);
            let nur = kern::nur_lokal_laden(&d)?;
            led.objects.retain(|k, _| {
                let (s, id) = k.split_once('/').unwrap_or(("", k));
                !nur.schluessel.contains(k)
                    && !(matches!(s, "page" | "book") && nur.einheiten.contains(id))
                    && !j.plan.freigeben.contains(id)
                    && !j.plan.entfernen.contains(id)
                    && !j.plan.frei_schluessel.contains(k)
                    && !j.plan.ledger_weg.contains(k)
            });
            // Freigegebenes neu einlesen: Kontoobjekte, die gesperrt waren, kommen jetzt an.
            if !j.plan.freigeben.is_empty() || !j.plan.frei_schluessel.is_empty() {
                led.rescan = 0;
            }
            ledger::save(app, &led)?;
        }
    }
    kern::journal_loeschen(&d)
}

/// Herkunft aus Konto und Ledger schreiben (U6). Nur, wenn ein Konto da ist.
pub fn herkunft_schreiben(app: &AppHandle, cfg: &SyncConfig) -> Result<(), String> {
    let led = ledger::load(app);
    let kennung = kern::konto_kennung(cfg.user_id, &cfg.url);
    let eintraege = if led.konto.as_deref().is_none_or(|k| k == kennung) {
        led.objects
            .iter()
            .filter(|(_, e)| !e.deleted)
            .map(|(k, e)| (k.clone(), e.hash.clone()))
            .collect()
    } else {
        BTreeMap::new()
    };
    kern::herkunft_speichern(
        &data(app)?,
        &kern::Herkunft {
            user_id: cfg.user_id,
            hub: cfg.url.clone(),
            email: cfg.user_email.clone(),
            eintraege,
        },
    )
}

fn emit_entfernt(app: &AppHandle, keys: Vec<String>) {
    if !keys.is_empty() {
        let _ = app.emit(engine::EVENT_PROGRESS_REMOVED, keys);
    }
    let _ = app.emit(engine::EVENT_CHANGED, ());
}

/// Offenen Übergang fortsetzen (Start, vor jedem Sync-Befehl). `Ok(true)`: es gibt noch
/// eine offene Entscheidung (Modal), `Ok(false)`: nichts offen.
pub fn wiederanlauf(app: &AppHandle) -> Result<bool, String> {
    let d = data(app)?;
    let Some(mut j) = kern::journal_laden(&d)? else {
        // Wartende Anmeldung ohne Journal (Absturz zwischen den beiden Dateien).
        return Ok(wartend_laden(app).is_some());
    };
    if j.phase == Phase::Entscheidung && j.art == Art::Anmeldung {
        // Nichts verändert (auch nach einer gescheiterten Sicherung): Modal erneut.
        return Ok(true);
    }
    if j.art == Art::Wiederherstellen {
        wiederherstellen_ausfuehren(app, &mut j)?;
        let _ = app.emit(engine::EVENT_CHANGED, ());
        return Ok(false);
    }
    if j.phase == Phase::Entscheidung && j.art == Art::Abmeldung {
        if let Some(cfg) = account::load(app) {
            if cfg.user_id == j.user_id && kern::hub_gleich(&cfg.url, &j.hub) {
                herkunft_schreiben(app, &cfg)?;
            }
            widerruf_vormerken(app, &cfg.url, &cfg.token)?;
            account::remove(app);
        }
        ledger::remove(app);
    }
    let entfernt = sichern_und_umsetzen(app, &mut j)?;
    abschliessen(app, &j)?;
    emit_entfernt(app, entfernt);
    Ok(false)
}

// ---------------------------------------------------------------------------
// Befehle
// ---------------------------------------------------------------------------

fn status_mit(engine: &SyncEngine, app: &AppHandle) -> SyncStatus {
    engine.refresh_from_config(app);
    engine.status()
}

/// Anmelden (U1/U2): prüft Zugangsdaten, legt die Anmeldung wartend an und entscheidet,
/// ob ein Modal nötig ist.
pub async fn anmelden(
    app: &AppHandle,
    engine: &SyncEngine,
    cfg: SyncConfig,
) -> Result<AnmeldeErgebnis, String> {
    widerruf_nachholen(app).await;
    let _guard = engine.account_guard().await;
    let (url, token) = (cfg.url.clone(), cfg.token.clone());
    let ergebnis = anmelden_unter_sperre(app, engine, cfg).await;
    if ergebnis.is_err() {
        // Gescheitert, bevor eine Wahl getroffen war: Anmeldezustand bereinigen (U10).
        if let Ok(d) = data(app) {
            let offen = kern::journal_laden(&d).is_ok_and(|j| {
                j.is_some_and(|j| j.art == Art::Anmeldung && j.phase == Phase::Entscheidung)
            });
            if offen {
                // Erst alle Tokens (neu und wartend) dauerhaft vormerken bzw. widerrufen,
                // dann die Tokenquellen löschen. Scheitert beides, bleibt alles liegen.
                let mut ws = vec![Widerruf { url, token }];
                if let Some(w) = wartend_laden(app) {
                    let alt = Widerruf {
                        url: w.cfg.url,
                        token: w.cfg.token,
                    };
                    if !ws.contains(&alt) {
                        ws.push(alt);
                    }
                }
                let gesichert = widerrufen_alle(app, &ws).await;
                if let Err(e) = konto_datei(app, WARTEND)
                    .and_then(|wp| kern::anmeldung_verwerfen(|| gesichert, &wp, &d))
                {
                    log::warn!("Sync: Anmeldung nicht verworfen: {e}");
                }
            }
        }
    }
    ergebnis
}

async fn anmelden_unter_sperre(
    app: &AppHandle,
    engine: &SyncEngine,
    cfg: SyncConfig,
) -> Result<AnmeldeErgebnis, String> {
    let d = data(app)?;
    if account::load(app).is_some() {
        widerrufen_fluechtig(app, &cfg.url, &cfg.token).await;
        return Err("Dieses Gerät ist schon angemeldet. Bitte zuerst abmelden.".into());
    }
    if kern::journal_vorhanden(&d) {
        let offen_anmeldung = kern::journal_laden(&d).is_ok_and(|j| {
            j.is_some_and(|j| j.art == Art::Anmeldung && j.phase == Phase::Entscheidung)
        });
        if !offen_anmeldung {
            widerrufen_fluechtig(app, &cfg.url, &cfg.token).await;
            return Err(
                "Ein vorheriger Übergang ist noch nicht abgeschlossen. Bitte die App neu starten."
                    .into(),
            );
        }
    }
    // Eine ältere wartende Anmeldung ersetzen: deren Token widerrufen.
    // Scheitert das, bleibt die alte Datei (Tokenquelle) stehen und die Anmeldung bricht ab.
    if let Some(alt) = wartend_laden(app) {
        widerrufen(app, &alt.cfg.url, &alt.cfg.token).await?;
    }
    let mut w = Wartend {
        cfg: cfg.clone(),
        konto: BTreeSet::new(),
        gleiche_seiten: BTreeSet::new(),
    };
    wartend_speichern(app, &w)?;
    kern::journal_speichern(
        &d,
        &Journal {
            art: Art::Anmeldung,
            hub: cfg.url.clone(),
            user_id: cfg.user_id,
            email: cfg.user_email.clone(),
            wahl: None,
            phase: Phase::Entscheidung,
            sicherung: None,
            plan: Plan::default(),
        },
    )?;
    let app2 = app.clone();
    let cfg2 = cfg.clone();
    let erfasst = tauri::async_runtime::spawn_blocking(move || erfassen(&app2, &cfg2))
        .await
        .map_err(|e| e.to_string())??;
    let (konto, gleich) = konto_lesen(&cfg, &erfasst.seiten_werte).await?;
    w.konto = konto;
    w.gleiche_seiten = gleich;
    wartend_speichern(app, &w)?;
    let bestand = bestand_von(app, &cfg, &erfasst)?;
    if braucht_wahl(&bestand) {
        let e = entscheidung(app, &w, &erfasst, &bestand);
        return Ok(AnmeldeErgebnis {
            status: status_mit(engine, app),
            entscheidung: Some(e),
        });
    }
    // Nichts zu entscheiden: Zusammenführen ohne Modal; abweichende Einstellungen vorher
    // gesichert.
    let mut j = kern::journal_laden(&d)?.ok_or("Journal fehlt")?;
    j.wahl = Some(Wahl::Zusammenfuehren);
    if bestand.is_empty() {
        j.phase = Phase::Umgesetzt;
        kern::journal_speichern(&d, &j)?;
    } else {
        kern::journal_speichern(&d, &j)?;
        let app2 = app.clone();
        let mut j2 = j.clone();
        j = tauri::async_runtime::spawn_blocking(move || {
            sichern_und_umsetzen(&app2, &mut j2).map(|_| j2)
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    abschliessen(app, &j)?;
    Ok(AnmeldeErgebnis {
        status: status_mit(engine, app),
        entscheidung: None,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn sync_anmeldung_offen(app: AppHandle) -> Result<Option<Entscheidung>, String> {
    let Some(w) = wartend_laden(&app) else {
        return Ok(None);
    };
    let app2 = app.clone();
    let cfg = w.cfg.clone();
    let erfasst = tauri::async_runtime::spawn_blocking(move || erfassen(&app2, &cfg))
        .await
        .map_err(|e| e.to_string())??;
    let bestand = bestand_von(&app, &w.cfg, &erfasst)?;
    Ok(Some(entscheidung(&app, &w, &erfasst, &bestand)))
}

/// Die Wahl im Modal umsetzen (U3, U11).
#[tauri::command]
#[specta::specta]
pub async fn sync_anmeldung_entscheiden(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    wahl: AnmeldeWahl,
    kopieren_bestaetigt: bool,
) -> Result<SyncStatus, String> {
    let entfernt = {
        let _guard = engine.account_guard().await;
        let d = data(&app)?;
        let w = wartend_laden(&app).ok_or("Keine Anmeldung wartet auf eine Entscheidung.")?;
        let kontowechsel =
            kern::herkunft_laden(&d).is_some_and(|h| !h.gleiches_konto(w.cfg.user_id, &w.cfg.url));
        if kontowechsel && matches!(wahl, AnmeldeWahl::Zusammenfuehren) && !kopieren_bestaetigt {
            return Err("Bitte bestätigen, dass die Daten in dieses Konto kopiert werden.".into());
        }
        let app2 = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
            let mut j = kern::journal_laden(&d)?.unwrap_or(Journal {
                art: Art::Anmeldung,
                hub: w.cfg.url.clone(),
                user_id: w.cfg.user_id,
                email: w.cfg.user_email.clone(),
                wahl: None,
                phase: Phase::Entscheidung,
                sicherung: None,
                plan: Plan::default(),
            });
            if j.phase == Phase::Entscheidung {
                // In `entscheidung` ist noch nichts verändert: Plan stets neu aus dem
                // frischen Bestand mit der aktuellen Wahl (auch nach einer gescheiterten
                // Sicherung, z. B. zu wenig Platz). Was zwischen Erfassen und Sicherung
                // entsteht, ist danach gewöhnliche Arbeit im Konto.
                let erfasst = erfassen(&app2, &w.cfg)?;
                let bestand = bestand_von(&app2, &w.cfg, &erfasst)?;
                let wahl: Wahl = wahl.into();
                j.plan = plan_bauen(&d, wahl, &w, &erfasst, &bestand);
                j.wahl = Some(wahl);
                kern::journal_speichern(&d, &j)?;
            }
            let entfernt = sichern_und_umsetzen(&app2, &mut j)?;
            abschliessen(&app2, &j)?;
            Ok(entfernt)
        })
        .await
        .map_err(|e| e.to_string())??
    };
    emit_entfernt(&app, entfernt);
    Ok(engine.run_once(&app).await)
}

/// Abbrechen (U10): Nutzdaten unverändert, wartende Anmeldung weg, Token widerrufen.
#[tauri::command]
#[specta::specta]
pub async fn sync_anmeldung_abbrechen(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
) -> Result<SyncStatus, String> {
    let _guard = engine.account_guard().await;
    let d = data(&app)?;
    if let Some(j) = kern::journal_laden(&d)? {
        // Solange die Phase `entscheidung` ist, wurde nichts verändert (U11).
        if j.art != Art::Anmeldung || j.phase != Phase::Entscheidung {
            return Err(
                "Die Wahl wird schon umgesetzt und lässt sich nicht mehr abbrechen.".into(),
            );
        }
    }
    let w = wartend_laden(&app);
    if let Some(w) = &w {
        // Erst vormerken: danach gibt es keine andere Kopie des Tokens mehr.
        widerruf_vormerken(&app, &w.cfg.url, &w.cfg.token)?;
    }
    wartend_loeschen(&app);
    kern::journal_loeschen(&d)?;
    widerruf_nachholen(&app).await;
    Ok(status_mit(&engine, &app))
}

/// Abmelden (U7). `entfernen`: Bibliothek und Seiten nach geprüfter Sicherung vom Gerät.
pub async fn abmelden(app: &AppHandle, engine: &SyncEngine, entfernen: bool) -> Result<(), String> {
    let cfg = account::load(app);
    let _guard = engine.account_guard().await;
    let d = data(app)?;
    if kern::journal_vorhanden(&d) {
        return Err("Ein Übergang ist noch offen. Bitte zuerst „Erneut prüfen“.".into());
    }
    if entfernen && cfg.is_none() {
        return Err("Nicht angemeldet.".into());
    }
    if let Some(cfg) = &cfg {
        // Herkunft zuerst (verändert keine Nutzdaten); scheitert sie, bleibt alles wie es war.
        herkunft_schreiben(app, cfg)?;
        if entfernen {
            let index = pages::load_index(app)?;
            let mut plan = Plan::default();
            plan.entfernen.extend(
                index
                    .pages
                    .iter()
                    .filter(|p| p.buch.is_none())
                    .map(|p| p.id.clone()),
            );
            plan.entfernen.extend(kern::buecher(&d));
            if let Some(map) = std::fs::read(d.join(kern::BIBLIOTHEK).join(kern::FORTSCHRITT))
                .ok()
                .and_then(|r| serde_json::from_slice::<BTreeMap<String, String>>(&r).ok())
            {
                plan.fortschritt.extend(map.into_keys());
            }
            kern::journal_speichern(
                &d,
                &Journal {
                    art: Art::Abmeldung,
                    hub: cfg.url.clone(),
                    user_id: cfg.user_id,
                    email: cfg.user_email.clone(),
                    wahl: Some(Wahl::Entfernen),
                    phase: Phase::Entscheidung,
                    sicherung: None,
                    plan,
                },
            )?;
        }
    }
    if let Some(cfg) = &cfg {
        widerruf_vormerken(app, &cfg.url, &cfg.token)?;
    }
    account::remove(app);
    ledger::remove(app);
    widerruf_nachholen(app).await;
    if entfernen && kern::journal_vorhanden(&d) {
        let app2 = app.clone();
        let entfernt =
            tauri::async_runtime::spawn_blocking(move || -> Result<Vec<String>, String> {
                let d = data(&app2)?;
                let mut j = kern::journal_laden(&d)?.ok_or("Journal fehlt")?;
                let entfernt = sichern_und_umsetzen(&app2, &mut j)?;
                abschliessen(&app2, &j)?;
                Ok(entfernt)
            })
            .await
            .map_err(|e| e.to_string())??;
        emit_entfernt(app, entfernt);
    }
    Ok(())
}

/// Laufzeit-Kollision (U5): eine markierte Kennung ist im Konto aufgetaucht.
pub fn kollision_aufloesen(
    app: &AppHandle,
    cfg: &SyncConfig,
    einheiten: &BTreeSet<String>,
) -> Result<(), String> {
    let d = data(app)?;
    if kern::journal_vorhanden(&d) {
        return Ok(()); // ein anderer Übergang läuft; der nächste Lauf versucht es erneut
    }
    let mut plan = Plan::default();
    let index = serde_json::to_value(
        pages::load_index(app)?
            .pages
            .iter()
            .map(|p| serde_json::json!({"id": p.id}))
            .collect::<Vec<_>>(),
    )
    .map(|p| serde_json::json!({ "pages": p }))
    .map_err(|e| e.to_string())?;
    let mut probe = index;
    for u in einheiten {
        if kern::gueltige_seite(u) && d.join(kern::PROJEKTE).join(u).is_dir() {
            let neu = kern::neue_seiten_kennung(&d, &probe);
            if let Some(p) = probe["pages"].as_array_mut() {
                p.push(serde_json::json!({ "id": neu }));
            }
            plan.kopien.insert(u.clone(), neu.clone());
            plan.nur_lokal.einheiten.insert(neu);
        }
        plan.entfernen.insert(u.clone());
        plan.freigeben.insert(u.clone());
    }
    let mut j = Journal {
        art: Art::Kollision,
        hub: cfg.url.clone(),
        user_id: cfg.user_id,
        email: cfg.user_email.clone(),
        wahl: Some(Wahl::GetrenntHalten),
        phase: Phase::Entscheidung,
        sicherung: None,
        plan,
    };
    kern::journal_speichern(&d, &j)?;
    let entfernt = sichern_und_umsetzen(app, &mut j)?;
    abschliessen(app, &j)?;
    emit_entfernt(app, entfernt);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn sync_sicherungen(app: AppHandle) -> Result<Vec<Sicherung>, String> {
    let d = data(&app)?;
    Ok(kern::auflisten(&d)
        .into_iter()
        .map(|s| Sicherung::aus(&d, s))
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn sync_sicherung_wiederherstellen(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    name: String,
) -> Result<WiederherstellBericht, String> {
    let _guard = engine.account_guard().await;
    let app2 = app.clone();
    let bericht =
        tauri::async_runtime::spawn_blocking(move || -> Result<WiederherstellBericht, String> {
            let d = data(&app2)?;
            if kern::journal_vorhanden(&d) {
                return Err("Ein Übergang ist noch offen. Bitte erst abschließen.".into());
            }
            let cfg = account::load(&app2);
            let plan = {
                let _bibliothek = crate::library::sync_lock();
                let _seiten = pages::lock();
                wiederherstellen_planen(&d, &name)?
            };
            let mut j = Journal {
                art: Art::Wiederherstellen,
                hub: cfg.as_ref().map(|c| c.url.clone()).unwrap_or_default(),
                user_id: cfg.as_ref().map(|c| c.user_id).unwrap_or_default(),
                email: cfg
                    .as_ref()
                    .map(|c| c.user_email.clone())
                    .unwrap_or_default(),
                wahl: None,
                phase: Phase::Entscheidung,
                sicherung: Some(name.clone()),
                plan,
            };
            // Ab hier läuft kein Zyklus, bis alles markiert und geschrieben ist (U8, U11).
            kern::journal_speichern(&d, &j)?;
            let w = wiederherstellen_ausfuehren(&app2, &mut j)?;
            Ok(WiederherstellBericht {
                zurueck: w.zurueck.len() as u32,
                zusaetzlich: w.zusaetzlich.len() as u32,
                gleich: w.gleich.len() as u32,
                in_sicherung: w.in_sicherung.clone(),
                fortschritte_ergaenzt: w.fortschritte_ergaenzt,
                fortschritte_abweichend: w.fortschritte_abweichend,
                ordner: kern::sicherungs_ordner(&d, &name)?
                    .to_string_lossy()
                    .into_owned(),
            })
        })
        .await
        .map_err(|e| e.to_string())??;
    let _ = app.emit(engine::EVENT_CHANGED, ());
    Ok(bericht)
}

/// Wiederherstellung planen: Kennungen stehen fest, bevor etwas geschrieben wird; alles
/// Zurückkommende ist nur lokal, auch ergänzte Fortschritte und Lesezeichen.
fn wiederherstellen_planen(d: &Path, name: &str) -> Result<Plan, String> {
    let mut plan = kern::wiederherstellen_plan(d, name)?;
    for k in plan.fortschritt.clone() {
        if let Some(s) = fortschritt_sync_schluessel(&k) {
            plan.nur_lokal.schluessel.insert(s.clone());
            plan.ledger_weg.insert(s);
        }
    }
    for u in &plan.nur_lokal.einheiten {
        plan.ledger_weg.insert(kern::schluessel("page", u));
        plan.ledger_weg.insert(kern::schluessel("book", u));
    }
    Ok(plan)
}

/// Sync-Schlüssel eines Eintrags aus `fortschritt.json`.
fn fortschritt_sync_schluessel(k: &str) -> Option<String> {
    if let Some(sub) = k.strip_prefix("progress.") {
        return Some(kern::schluessel(
            "progress",
            &books::progress_object_id(sub),
        ));
    }
    books::is_bookmark_key(k).then(|| kern::schluessel("bookmark", &books::bookmark_object_id(k)))
}

/// Ledger-Einträge entfernen (nur mit Konto; ohne Konto gibt es kein Ledger).
fn ledger_bereinigen(app: &AppHandle, weg: &BTreeSet<String>) -> Result<(), String> {
    if weg.is_empty() || account::load(app).is_none() {
        return Ok(());
    }
    let mut led = ledger::load(app);
    let vorher = led.objects.len();
    led.objects.retain(|k, _| !weg.contains(k));
    if led.objects.len() != vorher {
        ledger::save(app, &led)?;
    }
    Ok(())
}

/// Wiederherstellung nach Journal: erst markieren und Ledger bereinigen (Phase
/// `gesichert`), dann schreiben, dann Journal löschen. Wiederholbar.
fn wiederherstellen_ausfuehren(
    app: &AppHandle,
    j: &mut Journal,
) -> Result<kern::Wiederhergestellt, String> {
    let d = data(app)?;
    let name = j.sicherung.clone().ok_or("Journal ohne Sicherung")?;
    let _bibliothek = crate::library::sync_lock();
    let _seiten = pages::lock();
    if j.phase == Phase::Entscheidung {
        let mut nur = kern::nur_lokal_laden(&d)?;
        nur.vereinigen(&j.plan.nur_lokal);
        kern::nur_lokal_speichern(&d, &nur)?;
        ledger_bereinigen(app, &j.plan.ledger_weg)?;
        j.phase = Phase::Gesichert;
        kern::journal_speichern(&d, j)?;
    }
    let w = kern::wiederherstellen(&d, &name, &j.plan)?;
    kern::journal_loeschen(&d)?;
    Ok(w)
}

/// Einstellungen einer Sicherung zurückspielen (vorher Sicherung der aktuellen).
#[tauri::command]
#[specta::specta]
pub async fn sync_sicherung_einstellungen(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
    name: String,
) -> Result<(), String> {
    let _guard = engine.account_guard().await;
    let d = data(&app)?;
    let werte = kern::einstellungen_der_sicherung(&d, &name)?;
    let einst = einstellungen_jetzt(&app);
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let _bibliothek = crate::library::sync_lock();
        let _seiten = pages::lock();
        kern::sichern(
            &d,
            &kern::Auftrag {
                anlass: "vor_wiederherstellen",
                konto: None,
                einstellungen: &einst,
                platz_frei: crate::commands::library_layout::freier_platz(&d),
            },
        )?;
        collect::apply_settings(&app2, &werte)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn sync_sicherung_loeschen(app: AppHandle, name: String) -> Result<(), String> {
    kern::loeschen(&data(&app)?, &name)
}

/// Nur lokal gehaltene Einträge ins angemeldete Konto übernehmen (U5, Art `uebernahme`).
#[tauri::command]
#[specta::specta]
pub async fn sync_nur_lokal_uebernehmen(
    app: AppHandle,
    engine: State<'_, SyncEngine>,
) -> Result<SyncStatus, String> {
    {
        let _guard = engine.account_guard().await;
        let cfg = account::load(&app).ok_or("Nicht angemeldet")?;
        let app2 = app.clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
            let d = data(&app2)?;
            if kern::journal_vorhanden(&d) {
                return Err("Ein Übergang ist noch offen.".into());
            }
            let nur = kern::nur_lokal_laden(&d)?;
            if nur.ist_leer() {
                return Ok(());
            }
            // Ledger-Schlüssel aller betroffenen lokalen Objekte (auch Fortschritte,
            // Lesezeichen, Buchdateien): Push mit Basis 0 (U5).
            let erfasst = erfassen(&app2, &cfg)?;
            let mut ledger_weg: BTreeSet<String> = erfasst
                .objekte
                .iter()
                .filter(|o| {
                    let k = kern::schluessel(&o.sammlung, &o.id);
                    nur.schluessel.contains(&k)
                        || o.einheit.as_ref().is_some_and(|u| {
                            nur.einheiten.contains(u)
                                || nur.einheiten.contains(&effektiv(&erfasst, u))
                        })
                })
                .map(|o| kern::schluessel(&o.sammlung, &o.id))
                .collect();
            ledger_weg.extend(nur.schluessel.iter().cloned());
            for u in &nur.einheiten {
                ledger_weg.insert(kern::schluessel("page", u));
                ledger_weg.insert(kern::schluessel("book", u));
            }
            let mut j = Journal {
                art: Art::Uebernahme,
                hub: cfg.url.clone(),
                user_id: cfg.user_id,
                email: cfg.user_email.clone(),
                wahl: Some(Wahl::Zusammenfuehren),
                phase: Phase::Entscheidung,
                sicherung: None,
                plan: Plan {
                    freigeben: nur.einheiten.clone(),
                    frei_schluessel: nur.schluessel.clone(),
                    ledger_weg,
                    ..Default::default()
                },
            };
            kern::journal_speichern(&d, &j)?;
            // Derselbe wiederholbare Weg wie beim Wiederanlauf.
            sichern_und_umsetzen(&app2, &mut j)?;
            abschliessen(&app2, &j)
        })
        .await
        .map_err(|e| e.to_string())??;
    }
    Ok(engine.run_once(&app).await)
}

/// Zahl der nur lokal gehaltenen Einträge (für den Status).
pub fn nur_lokal_anzahl(app: &AppHandle) -> u32 {
    data(app)
        .ok()
        .and_then(|d| kern::nur_lokal_laden(&d).ok())
        .map(|n| n.anzahl())
        .unwrap_or(0)
}

pub fn anmeldung_offen(app: &AppHandle) -> bool {
    wartend_laden(app).is_some()
}

pub fn uebergang_offen(app: &AppHandle) -> bool {
    data(app).is_ok_and(|d| d.join(kern::JOURNAL).is_file())
}
