//! Assistent-Sync (lva#388, ADR-0028 A1): der Verlauf des Assistenten über den Hub, Tauri-Seite.
//!
//! Die Logik steht in reinen Modulen: `portal_hub` (Hub-Zugang), `conversations::sync` (Abgleich mit
//! `conversations.db`), `assistent_zustand` (Schlüssel, Zustandsdatei). Dieses Modul verdrahtet sie
//! mit Einstellung, Konto und Tauri-Befehlen.
//!
//! Regeln:
//! - **Schalter `assistent_sync_aktiv`** (Standard aus, fehlender Wert gilt als aus). Ohne Schalter
//!   gibt es keinen Netzaufruf und es wird nichts gelesen oder hochgeladen (E3.6). Alle Befehle, die
//!   den Hub ansprechen, prüfen ihn zuerst.
//! - **Token und Weg des Bibliotheks-Syncs:** gleiche Konto-Datei (`sync.json`), gleicher HTTP-Weg
//!   (`HubClient`, unter Windows curl). Das Token trägt `voice.sync` ausdrücklich (Login oder Kopplung).
//! - **Keine Inhalte, Kennungen oder Geräte-IDs im Log.** Fehlertexte sind feste Sätze.
//! - Noch keine Oberfläche: Status, Starten, Zulassen, Widerrufen und Freigeben sind Befehle für die
//!   UI, die später folgt.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};
use wai_assistent::sync::{Abgleich, HubFehler, HubSpeicher, SyncFehler};

use super::account::{self, SyncConfig};
use super::assistent_zustand::{self as zustand, Zustand};
use super::client::HubClient;
use super::portal_hub::{Anfrage, Antwort, HubTransport, Methode, PortalHub, TransportFehler};
use crate::conversations::store::ConversationStore;
use crate::conversations::sync::{self as verlauf, LaufBericht, LaufFehler};

/// Ereignis an die Oberfläche nach jedem Lauf und jeder Änderung des Zustands.
pub const EVENT_STATUS: &str = "assistent-sync-status";

/// Takt des Hintergrundlaufs; zusätzlich stößt jedes lokale Schreiben einen Lauf an.
const TAKT: Duration = Duration::from_secs(300);
/// Wartezeit nach einem Anstoß, damit mehrere Schreibvorgänge einen Lauf ergeben.
const ENTPRELLEN: Duration = Duration::from_secs(5);

/// Zähler des letzten Laufs (nie Inhalte).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, Type)]
pub struct AssistentSyncBericht {
    pub hochgeladen: u32,
    pub heruntergeladen: u32,
    pub geloescht: u32,
    /// Beiträge, die wegen „nur auf diesem Gerät“ oder Vertraulichkeit nicht hochgeladen werden.
    pub nur_lokal: u32,
    /// Objekte, die auf einen Epochenschlüssel warten.
    pub wartend: u32,
    pub verworfen: u32,
}

impl From<&LaufBericht> for AssistentSyncBericht {
    fn from(b: &LaufBericht) -> Self {
        let z = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Self {
            hochgeladen: z(b.beitraege_hoch + b.koepfe_hoch + b.grabsteine_hoch),
            heruntergeladen: z(b.beitraege_neu + b.unterhaltungen_neu),
            geloescht: z(b.unterhaltungen_geloescht + b.beitraege_geloescht),
            nur_lokal: z(b.nur_lokal),
            wartend: z(b.zurueckgestellt),
            verworfen: z(b.verworfen),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, Type)]
pub struct AssistentSyncStatus {
    /// Schalter `assistent_sync_aktiv`.
    pub aktiv: bool,
    /// Ein Konto des Bibliotheks-Syncs ist angemeldet (Token, Hub-Adresse).
    pub angemeldet: bool,
    /// Dieses Gerät ist am Hub zugelassen.
    pub zugelassen: bool,
    /// Der Hub verlangt eine Freigabe von einem schon zugelassenen Gerät.
    pub braucht_freigabe: bool,
    pub laeuft: bool,
    /// Aktuelle Schlüsselepoche dieses Geräts (ab 1).
    pub epoche: u32,
    /// Ein Widerruf wartet auf die Antwort des Hubs.
    pub widerruf_offen: bool,
    pub letzter_lauf_ms: Option<f64>,
    /// Fester Satz zur Anzeige; nie Inhalte.
    pub letzter_fehler: Option<String>,
    /// Fehlercode des Hubs (`netz`, `kontingent`, `unauthorized`, …) zur Diagnose.
    pub fehler_code: Option<String>,
    /// Lokal gelöschte Gespräche, deren Grabstein der Hub noch nicht bestätigt hat.
    pub grabsteine_offen: u32,
    pub letzter_bericht: Option<AssistentSyncBericht>,
}

/// Ergebnis von `assistent_sync_geraet_widerrufen`.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct AssistentWiderruf {
    /// Neue Schlüsselepoche.
    pub epoche: u32,
    /// Geräte, an die der neue Epochenschlüssel über die Kopplung gehen muss. Der Schlüssel selbst
    /// verlässt dieses Gerät hier nicht.
    pub empfaenger: Vec<String>,
}

/// Ergebnis von `assistent_sync_freigabe_erzeugen`: Einmalcode für ein weiteres Gerät.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct AssistentFreigabe {
    /// Einmalgeheimnis; geht auf einem eigenen Kanal an das neue Gerät (z. B. QR).
    pub code: String,
    pub gueltig_bis: String,
}

impl std::fmt::Debug for AssistentFreigabe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistentFreigabe")
            .field("code", &"***")
            .field("gueltig_bis", &self.gueltig_bis)
            .finish()
    }
}

/// Zustand des Dienstes (von Tauri verwaltet).
pub struct AssistentSyncDienst {
    status: Mutex<AssistentSyncStatus>,
    /// Ein Lauf oder Hub-Befehl zur Zeit.
    sperre: tokio::sync::Mutex<()>,
    notify: tokio::sync::Notify,
}

impl Default for AssistentSyncDienst {
    fn default() -> Self {
        Self {
            status: Mutex::new(AssistentSyncStatus::default()),
            sperre: tokio::sync::Mutex::new(()),
            notify: tokio::sync::Notify::new(),
        }
    }
}

impl AssistentSyncDienst {
    fn status(&self) -> AssistentSyncStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn setzen(&self, f: impl FnOnce(&mut AssistentSyncStatus)) {
        if let Ok(mut s) = self.status.lock() {
            f(&mut s);
        }
    }

    /// Stößt einen Lauf an (entprellt im Takt der Hintergrundschleife).
    pub fn anstossen(&self) {
        self.notify.notify_one();
    }
}

/// Anstoß nach einem lokalen Schreiben im Verlauf. Wirkt nur, wenn der Schalter an ist.
pub fn anstossen(app: &AppHandle) {
    if !aktiv(app) {
        return;
    }
    if let Some(dienst) = app.try_state::<AssistentSyncDienst>() {
        dienst.anstossen();
    }
}

fn aktiv(app: &AppHandle) -> bool {
    crate::settings::get_settings(app).assistent_sync_aktiv
}

// ---------------------------------------------------------------------------------------------
// Transport über den Hub-Client des Bibliotheks-Syncs
// ---------------------------------------------------------------------------------------------

/// `HubTransport` über `HubClient`. Blockiert: nur aus einem Blockier-Thread (`spawn_blocking`)
/// aufrufen, nie auf dem Async-Executor.
struct ClientTransport {
    client: HubClient,
}

impl HubTransport for ClientTransport {
    fn senden(&self, a: &Anfrage) -> Result<Antwort, TransportFehler> {
        let query: Vec<(&str, String)> = a.query.iter().map(|(k, v)| (*k, v.clone())).collect();
        tauri::async_runtime::block_on(self.client.assistent_aufruf(
            a.methode == Methode::Post,
            a.pfad,
            &query,
            a.body.as_ref(),
        ))
        .map(|(status, body)| Antwort { status, body })
        .map_err(|_| TransportFehler)
    }
}

// ---------------------------------------------------------------------------------------------
// Konto, Zustandsdatei, Fehlertexte
// ---------------------------------------------------------------------------------------------

/// Fehler mit festem Anzeigetext und optionalem Hub-Code.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Meldung {
    text: String,
    code: Option<String>,
    /// Der Hub verlangt eine Freigabe (403 `nicht_zugelassen`).
    braucht_freigabe: bool,
}

impl Meldung {
    fn neu(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            code: None,
            braucht_freigabe: false,
        }
    }
}

impl From<String> for Meldung {
    fn from(text: String) -> Self {
        Self {
            text,
            code: None,
            braucht_freigabe: false,
        }
    }
}

/// Feste Sätze zu Hub-Fehlern; `code` ist der Code des Hubs (oder `netz`).
fn hub_meldung(f: &HubFehler, code: Option<String>) -> Meldung {
    let text = match (f, code.as_deref()) {
        (_, Some("unauthorized")) => "Die Anmeldung ist abgelaufen. Bitte neu anmelden.",
        (_, Some("forbidden")) => {
            "Dieses Gerät darf den Assistent-Sync nicht nutzen. Bitte neu anmelden."
        }
        (HubFehler::Nichtverfuegbar, _) => {
            "Der Hub ist gerade nicht erreichbar. Es wird später erneut versucht."
        }
        (HubFehler::NichtZugelassen, _) => {
            "Dieses Gerät braucht eine Freigabe von einem Gerät, das schon zugelassen ist."
        }
        (HubFehler::Widerrufen, _) => {
            "Dieses Gerät wurde widerrufen und darf den Assistent-Sync nicht mehr nutzen."
        }
        (HubFehler::GeraetStimmtNichtUeberein, _) => {
            "Das Konto kennt dieses Gerät unter einer anderen Anmeldung."
        }
        (
            HubFehler::EpocheVeraltet { .. }
            | HubFehler::EpocheVoraus { .. }
            | HubFehler::EpocheKonflikt { .. },
            _,
        ) => "Die Schlüsselepoche passt nicht zum Hub. Bitte später erneut abgleichen.",
        (HubFehler::GrabsteinAbgewiesen, _) => {
            "Das Gespräch wurde auf einem anderen Gerät gelöscht."
        }
        (HubFehler::RevisionKonflikt { .. }, _) => {
            "Der Hub hat zwischenzeitlich geändert. Bitte erneut abgleichen."
        }
        (HubFehler::NichtVeraenderlich, _) | (HubFehler::Vertrag(_), _) => {
            "Der Hub hat die Daten abgewiesen."
        }
    };
    Meldung {
        text: text.to_owned(),
        braucht_freigabe: matches!(f, HubFehler::NichtZugelassen)
            && code.as_deref() != Some("forbidden"),
        code,
    }
}

fn lauf_meldung(f: &LaufFehler, code: Option<String>) -> Meldung {
    match f {
        LaufFehler::NichtFreigegeben => Meldung::neu("Der Assistent-Sync ist ausgeschaltet."),
        LaufFehler::Sync(SyncFehler::Hub(h)) => hub_meldung(h, code),
        LaufFehler::Sync(_) => Meldung {
            text: "Der Abgleich ist an einem Objekt gescheitert.".to_owned(),
            code,
            braucht_freigabe: false,
        },
        LaufFehler::Speicher(_) => {
            Meldung::neu("Der Gesprächsverlauf konnte nicht gelesen oder geschrieben werden.")
        }
    }
}

struct Sitzung {
    cfg: SyncConfig,
    geraet: String,
    store: Arc<ConversationStore>,
    pfad: PathBuf,
}

fn sitzung(app: &AppHandle) -> Result<Sitzung, Meldung> {
    if !aktiv(app) {
        return Err(Meldung::neu("Der Assistent-Sync ist ausgeschaltet."));
    }
    sitzung_ohne_schalter(app)
}

impl Sitzung {
    fn konto(&self) -> String {
        Zustand::konto_kennung(&self.cfg.url, self.cfg.user_id, &self.geraet)
    }

    fn hub(&self) -> PortalHub<ClientTransport> {
        PortalHub::new(
            ClientTransport {
                client: HubClient::new(&self.cfg.url, &self.cfg.token),
            },
            &self.geraet,
        )
    }

    /// Zustandsdatei lesen (leer, wenn sie fehlt oder zu einem anderen Konto gehört).
    fn zustand(&self) -> Zustand {
        let text = std::fs::read_to_string(&self.pfad).ok();
        Zustand::aus_json(text.as_deref(), &self.konto())
    }

    fn speichern(&self, z: &Zustand) -> Result<(), Meldung> {
        let json = z.zu_json()?;
        account::write_private(&self.pfad, json.as_bytes()).map_err(Meldung::from)
    }

    fn schluessel(&self) -> Result<[u8; zustand::KEY_LEN], Meldung> {
        self.cfg.key().map_err(Meldung::from)
    }

    /// Kontowechsel: Zuordnung und Sync-Marken im Verlauf gehören zum alten Konto.
    fn konto_abgleichen(&self) -> Result<(), Meldung> {
        let konto = self.konto();
        let bekannt = self
            .store
            .sync_meta("konto")
            .map_err(|_| Meldung::neu("Der Gesprächsverlauf konnte nicht gelesen werden."))?;
        if bekannt.as_deref() == Some(konto.as_str()) {
            return Ok(());
        }
        self.store
            .sync_zuruecksetzen()
            .and_then(|_| self.store.sync_meta_setzen("konto", &konto))
            .map_err(|_| Meldung::neu("Der Gesprächsverlauf konnte nicht geschrieben werden."))
    }

    /// Ein vorgemerkter Widerruf wird wiederholt, bevor etwas anderes läuft (verlorene Antwort).
    fn widerruf_nachholen(&self, z: &mut Zustand) -> Result<(), Meldung> {
        let Some(offen) = z.offenen_widerruf()? else {
            return Ok(());
        };
        let hub = self.hub();
        let mut a = Abgleich::new(&self.geraet, z.bund(&self.schluessel()?)?);
        a.freigabe_einschalten();
        a.offenen_widerruf_wiederherstellen(offen);
        let r = a.widerruf_abschliessen(&hub);
        z.aus_bund(a.bund());
        z.offenen_widerruf_merken(a.offener_widerruf());
        self.speichern(z)?;
        match r {
            Ok(_) => Ok(()),
            Err(SyncFehler::Hub(h)) => Err(hub_meldung(&h, hub.letzter_code())),
            Err(_) => Err(Meldung::neu(
                "Der Widerruf konnte nicht abgeschlossen werden.",
            )),
        }
    }

    /// Lässt dieses Gerät zu, falls es das noch nicht ist (Erstgerät, Kopplung oder Freigabecode).
    fn zulassen(&self, z: &mut Zustand, code: Option<&str>) -> Result<(), Meldung> {
        if z.zugelassen && code.is_none() {
            return Ok(());
        }
        let hub = self.hub();
        match hub.zulassen(code) {
            Ok(_) => {
                z.zugelassen = true;
                self.speichern(z)
            }
            Err(h) => Err(hub_meldung(&h, hub.letzter_code())),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Läufe und Befehle (blockierende Teile laufen in `spawn_blocking`)
// ---------------------------------------------------------------------------------------------

struct Ergebnis {
    bericht: LaufBericht,
    epoche: u32,
}

fn lauf_blockierend(s: &Sitzung) -> Result<Ergebnis, Meldung> {
    let mut z = s.zustand();
    s.konto_abgleichen()?;
    s.zulassen(&mut z, None)?;
    s.widerruf_nachholen(&mut z)?;
    let bund = z.bund(&s.schluessel()?)?;
    let epoche = u32::try_from(bund.aktuelle_epoche()).unwrap_or(u32::MAX);
    let hub = s.hub();
    match verlauf::lauf(&s.store, &hub, bund, &s.geraet, true) {
        Ok(bericht) => Ok(Ergebnis { bericht, epoche }),
        Err(f) => Err(lauf_meldung(&f, hub.letzter_code())),
    }
}

async fn blockierend<T, F>(f: F) -> Result<T, Meldung>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, Meldung> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| Meldung::neu("Der Abgleich wurde unterbrochen."))?
}

fn now_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

/// Status aus Schalter, Konto und Zustandsdatei; ohne Netz.
fn status_lokal(app: &AppHandle, dienst: &AssistentSyncDienst) -> AssistentSyncStatus {
    let mut st = dienst.status();
    st.aktiv = aktiv(app);
    let cfg = account::load(app);
    st.angemeldet = cfg.is_some();
    if let Ok(s) = sitzung_ohne_schalter(app) {
        let z = s.zustand();
        st.zugelassen = z.zugelassen;
        st.widerruf_offen = z.offener_widerruf.is_some();
        st.epoche = s
            .schluessel()
            .and_then(|k| z.bund(&k).map_err(Meldung::from))
            .map_or(0, |b| {
                u32::try_from(b.aktuelle_epoche()).unwrap_or(u32::MAX)
            });
        st.grabsteine_offen = s
            .store
            .sync_grabsteine_offen()
            .map_or(0, |g| u32::try_from(g.len()).unwrap_or(u32::MAX));
    } else {
        st.zugelassen = false;
        st.widerruf_offen = false;
    }
    st
}

/// Wie [`sitzung`], aber ohne den Schalter zu verlangen (nur lokale Statusanzeige); ohne Netzaufruf.
fn sitzung_ohne_schalter(app: &AppHandle) -> Result<Sitzung, Meldung> {
    let cfg = account::load(app).ok_or_else(|| Meldung::neu("Nicht angemeldet."))?;
    let geraet = zustand::geraete_kennung(&cfg.device_id)
        .ok_or_else(|| Meldung::neu("Die Geräte-Kennung ist ungültig."))?;
    let store = app
        .try_state::<Arc<ConversationStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| Meldung::neu("Der Gesprächsverlauf ist nicht verfügbar."))?;
    let pfad = crate::portable::app_data_dir(app)
        .map(|d| d.join(zustand::DATEI))
        .map_err(|_| Meldung::neu("Kein Datenverzeichnis."))?;
    Ok(Sitzung {
        cfg,
        geraet,
        store,
        pfad,
    })
}

fn melden(app: &AppHandle, dienst: &AssistentSyncDienst) -> AssistentSyncStatus {
    let st = status_lokal(app, dienst);
    let _ = app.emit(EVENT_STATUS, st.clone());
    st
}

/// Ein Lauf. Ohne Schalter kein Netzaufruf.
async fn lauf_einmal(app: &AppHandle, dienst: &AssistentSyncDienst) -> AssistentSyncStatus {
    let _sperre = dienst.sperre.lock().await;
    let s = match sitzung(app) {
        Ok(s) => s,
        Err(m) => {
            dienst.setzen(|st| {
                st.laeuft = false;
                st.letzter_fehler = Some(m.text);
                st.fehler_code = m.code;
            });
            return melden(app, dienst);
        }
    };
    dienst.setzen(|st| st.laeuft = true);
    let _ = app.emit(EVENT_STATUS, status_lokal(app, dienst));
    let r = blockierend(move || lauf_blockierend(&s)).await;
    dienst.setzen(|st| {
        st.laeuft = false;
        match r {
            Ok(e) => {
                st.letzter_lauf_ms = Some(now_ms());
                st.letzter_fehler = None;
                st.fehler_code = None;
                st.braucht_freigabe = false;
                st.epoche = e.epoche;
                st.letzter_bericht = Some((&e.bericht).into());
            }
            Err(m) => {
                st.braucht_freigabe = m.braucht_freigabe;
                st.letzter_fehler = Some(m.text);
                st.fehler_code = m.code;
            }
        }
    });
    let geaendert = dienst
        .status()
        .letzter_bericht
        .as_ref()
        .is_some_and(|b| b.heruntergeladen + b.geloescht > 0);
    let st = melden(app, dienst);
    if geaendert {
        // Neue Gespräche oder Beiträge anderer Geräte: die Verlaufsliste neu laden.
        let _ = app.emit("conversations-changed", ());
    }
    st
}

/// Hintergrundschleife: Takt oder Anstoß, dann entprellt ein Lauf. Ohne Schalter kein Netzaufruf;
/// wartet das Gerät auf eine Freigabe, versucht erst der Befehl `assistent_sync_starten` es erneut.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let dienst = app.state::<AssistentSyncDienst>();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(TAKT) => {}
                _ = dienst.notify.notified() => {
                    tokio::time::sleep(ENTPRELLEN).await;
                }
            }
            if aktiv(&app) && !dienst.status().braucht_freigabe {
                lauf_einmal(&app, &dienst).await;
            }
        }
    });
}

/// Status des Assistent-Syncs; ohne Netzaufruf.
#[tauri::command]
#[specta::specta]
pub fn assistent_sync_status(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> AssistentSyncStatus {
    status_lokal(&app, &dienst)
}

/// Sofort abgleichen. Lässt das Gerät zu, wenn es das noch nicht ist (Erstgerät oder Kopplung).
/// Ohne den Schalter `assistent_sync_aktiv` passiert nichts.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_starten(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> Result<AssistentSyncStatus, String> {
    Ok(lauf_einmal(&app, &dienst).await)
}

/// Lässt dieses Gerät mit dem Freigabecode eines schon zugelassenen Geräts zu (`POST /geraet`) und
/// gleicht danach ab.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraet_zulassen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    freigabe_code: String,
) -> Result<AssistentSyncStatus, String> {
    let code = freigabe_code.trim().to_owned();
    // Einmalcode: base64url, 27 Zeichen; großzügig prüfen, nie in die Meldung übernehmen.
    let form_ok = (8..=128).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !form_ok {
        return Err("Der Freigabecode hat ein ungültiges Format.".into());
    }
    {
        let _sperre = dienst.sperre.lock().await;
        let s = sitzung(&app).map_err(|m| m.text)?;
        let r = blockierend(move || {
            let mut z = s.zustand();
            s.konto_abgleichen()?;
            s.zulassen(&mut z, Some(&code))
        })
        .await;
        match r {
            Ok(()) => dienst.setzen(|st| {
                st.braucht_freigabe = false;
                st.letzter_fehler = None;
                st.fehler_code = None;
            }),
            Err(m) => {
                dienst.setzen(|st| {
                    st.braucht_freigabe = m.braucht_freigabe;
                    st.letzter_fehler = Some(m.text.clone());
                    st.fehler_code = m.code.clone();
                });
                melden(&app, &dienst);
                return Err(m.text);
            }
        }
    }
    Ok(lauf_einmal(&app, &dienst).await)
}

/// Widerruft ein Gerät (Vergleichsschreiben auf die Epoche; das Gerät verliert auch sein Token).
/// Der neue Epochenschlüssel bleibt auf diesem Gerät; die Antwort nennt die Geräte, an die er über
/// die Kopplung gehen muss.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraet_widerrufen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    geraet_id: String,
) -> Result<AssistentWiderruf, String> {
    let ziel = geraet_id.trim().to_owned();
    if !zustand::gueltige_kennung(&ziel) {
        return Err("Die Geräte-Kennung hat ein ungültiges Format.".into());
    }
    let _sperre = dienst.sperre.lock().await;
    let s = sitzung(&app).map_err(|m| m.text)?;
    let r = blockierend(move || {
        let mut z = s.zustand();
        s.konto_abgleichen()?;
        if !z.zugelassen {
            return Err(Meldung::neu(
                "Dieses Gerät ist am Hub noch nicht zugelassen.",
            ));
        }
        let hub = s.hub();
        let mut a = Abgleich::new(&s.geraet, z.bund(&s.schluessel()?)?);
        a.freigabe_einschalten();
        if let Some(offen) = z.offenen_widerruf()? {
            a.offenen_widerruf_wiederherstellen(offen);
        }
        let ergebnis = a.widerrufen(&hub, &ziel);
        // Schlüssel und Vormerkung sofort sichern, auch bei Fehler (verlorene Antwort).
        z.aus_bund(a.bund());
        z.offenen_widerruf_merken(a.offener_widerruf());
        s.speichern(&z)?;
        match ergebnis {
            Ok(epoche) => {
                let empfaenger = a.empfaenger_fuer_epoche(&hub).unwrap_or_default();
                Ok(AssistentWiderruf {
                    epoche: u32::try_from(epoche).unwrap_or(u32::MAX),
                    empfaenger,
                })
            }
            Err(SyncFehler::Hub(h)) => Err(hub_meldung(&h, hub.letzter_code())),
            Err(_) => Err(Meldung::neu("Der Widerruf ist gescheitert.")),
        }
    })
    .await;
    melden(&app, &dienst);
    r.map_err(|m| m.text)
}

/// Gibt eine Kennung für ein weiteres Gerät frei und liefert den Einmalcode (`POST /freigeben`).
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_freigabe_erzeugen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    geraet_id: String,
) -> Result<AssistentFreigabe, String> {
    let neu = geraet_id.trim().to_ascii_lowercase();
    if !zustand::gueltige_kennung(&neu) {
        return Err("Die Geräte-Kennung hat ein ungültiges Format.".into());
    }
    let _sperre = dienst.sperre.lock().await;
    let s = sitzung(&app).map_err(|m| m.text)?;
    blockierend(move || {
        let hub = s.hub();
        hub.freigeben(&neu)
            .map(|f| AssistentFreigabe {
                code: f.code,
                gueltig_bis: f.gueltig_bis,
            })
            .map_err(|h| hub_meldung(&h, hub.letzter_code()))
    })
    .await
    .map_err(|m| m.text)
}

/// Zugelassene, nicht widerrufene Geräte des Kontos (Kennungen).
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraete(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> Result<Vec<String>, String> {
    let _sperre = dienst.sperre.lock().await;
    let s = sitzung(&app).map_err(|m| m.text)?;
    blockierend(move || {
        let hub = s.hub();
        let auth = wai_assistent::sync::AuthGeraet::new(&s.geraet);
        hub.zugelassene(&auth)
            .map_err(|h| hub_meldung(&h, hub.letzter_code()))
    })
    .await
    .map_err(|m| m.text)
}

/// Schaltet den Assistent-Sync ein oder aus (Standard aus, E3.6). Erst nach dem Einschalten werden
/// bestehende lokale Gespräche hochgeladen; beim Ausschalten bleibt alles, wie es ist.
#[tauri::command]
#[specta::specta]
pub fn change_assistent_sync_aktiv_setting(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.assistent_sync_aktiv = enabled;
    crate::settings::write_settings(&app, settings);
    if enabled {
        dienst.anstossen();
    }
    melden(&app, &dienst);
    Ok(())
}
