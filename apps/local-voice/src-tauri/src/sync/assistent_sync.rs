//! Assistent-Sync (lva#388, ADR-0028 A1, E3.8): der Verlauf des Assistenten über den Hub, Tauri-Seite.
//!
//! Die Logik steht in reinen Modulen: `portal_hub` (Hub-Zugang), `conversations::sync` (Abgleich mit
//! `conversations.db`), `assistent_zustand` (verschlüsselter Zustandsblock, Transportzulassung,
//! Kopplungs-QR). Dieses Modul verdrahtet sie mit Einstellung, Konto und Tauri-Befehlen.
//!
//! Regeln:
//! - **Schalter `assistent_sync_aktiv`** (Standard aus, fehlender Wert gilt als aus). Ohne Schalter
//!   gibt es keinen Netzaufruf und es wird nichts gelesen oder hochgeladen (E3.6). Alle Befehle, die
//!   den Hub ansprechen, prüfen ihn zuerst.
//! - **Der Hub ist nicht vertrauenswürdig (E3.8).** Zulassung, Widerruf und Epoche stehen im signierten
//!   Geräteverzeichnis, das der Kern selbst prüft. Das erste Gerät **gründet** das Konto (Vertrauensanker),
//!   jedes weitere wird **gekoppelt**: das aufnehmende Gerät zeigt einen QR-Code mit Prüfpunkt des
//!   Verzeichnisses, QR-Geheimnis und Freigabecode; das neue Gerät antwortet über einen Rückkanal mit
//!   seinem Ausweis samt HMAC; das aufnehmende Gerät nimmt es per signiertem Eintrag auf und legt die
//!   Epochenschlüssel als Pakete ab; das neue Gerät schließt nur auf einer Kette ab, die den Prüfpunkt
//!   fortsetzt. Ein Widerruf ist ein signierter Verzeichniseintrag (Vorbereiten, Sichern, Abschließen).
//! - **Zustand:** Der Kern wird vor jedem Lauf aus dem Zustandsblock geladen (`DateiSpeicher`) und sichert
//!   sich selbst, vor jedem Hub-Schreiben (Ausgang) und nach jeder Hub-Runde. Scheitert eine Sicherung,
//!   ist er gesperrt; der nächste Lauf lädt neu.
//! - **Token und Weg des Bibliotheks-Syncs:** gleiche Konto-Datei (`sync.json`), gleicher HTTP-Weg
//!   (`HubClient`, unter Windows curl). Das Token trägt `voice.sync` ausdrücklich (Login oder Kopplung).
//! - **Keine Inhalte, Kennungen oder Geräte-IDs im Log.** Fehlertexte sind feste Sätze.
//! - **Kennung je Kopplungsversuch (Must-fix 1):** Die Identität einer laufenden Kopplung liegt nur im
//!   Arbeitsspeicher (der Kern bietet keine serialisierbare Kopplung). Damit ein Neustart zwischen der
//!   Aufnahme durch das andere Gerät und dem Abschluss die Kennung nicht dauerhaft blockiert, trägt der
//!   verschlüsselte Kennungsstand (`assistent_zustand::Kennungsstand`) eine Generation und den Vermerk
//!   „Kopplung begonnen“, gesichert bevor die Antwort das Gerät verlässt. Ein verlorener Versuch wird beim
//!   nächsten Lauf aufgelöst: hat der Hub die Kennung schon im Verzeichnis, gilt die nächste Generation
//!   (`<Grundkennung>-<n>`), sonst dieselbe. Weil der Hub das Token an genau eine Kennung bindet, braucht die
//!   neue Kennung eine neue Anmeldung. Solange eine Kopplung offen ist, fragt die Hintergrundschleife alle
//!   [`KOPPLUNG_TAKT`] ab.
//! - **Gründen vertraut der Leermeldung des Hubs:** Ob das Konto schon Geräte hat, sagt nur der Hub
//!   (`GET /verzeichnis` leer). Ein Hub, der ein bestehendes Verzeichnis verschweigt, kann ein zweites
//!   Konto gründen lassen (Gabelung, kein Datenverlust; die anderen Geräte erkennen die fremde Kette).
//! - Noch keine Oberfläche: Status, Gründen, Koppeln, Widerrufen, Freigeben und Sitzung sind Befehle
//!   für die UI, die später folgt.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};
use wai_assistent::sync::{
    Abgleich, AuthGeraet, HubFehler, HubSpeicher, Kopplung, Kopplungsangebot, Kopplungsantwort,
    LeaseFehler, SyncFehler,
};
use zeroize::Zeroizing;

use super::account::{self, SyncConfig};
use super::assistent_zustand::{self as zustand, DateiSpeicher, KopplungsQr, Zustand};
use super::client::HubClient;
use super::portal_hub::{Anfrage, Antwort, HubTransport, Methode, PortalHub, TransportFehler};
use crate::conversations::store::ConversationStore;
use crate::conversations::sync::{self as verlauf, LaufBericht, LaufFehler, Sitzungsstand};

/// Ereignis an die Oberfläche nach jedem Lauf und jeder Änderung des Zustands.
pub const EVENT_STATUS: &str = "assistent-sync-status";

/// Fester Code, wenn das Konto oder der Schlüssel gewechselt hat.
pub const CODE_KONTO_GEWECHSELT: &str = "konto_gewechselt";
/// Fester Code, solange dieses Gerät weder gegründet noch gekoppelt ist.
pub const CODE_NICHT_EINGERICHTET: &str = "nicht_eingerichtet";
/// Fester Code, solange eine Kopplung auf die Aufnahme durch das andere Gerät wartet.
pub const CODE_KOPPLUNG_OFFEN: &str = "kopplung_offen";
/// Fester Code, wenn der gesicherte Zustand nicht lesbar ist (nichts wird überschrieben).
pub const CODE_ZUSTAND: &str = "zustand_unlesbar";
/// Fester Code, wenn ein unterbrochener Kopplungsversuch die Kennung verbraucht hat (neue Kennung).
pub const CODE_KENNUNG_NEU: &str = "kennung_neu";

/// Takt des Hintergrundlaufs; zusätzlich stößt jedes lokale Schreiben einen Lauf an.
const TAKT: Duration = Duration::from_secs(300);
/// Wartezeit nach einem Anstoß, damit mehrere Schreibvorgänge einen Lauf ergeben.
const ENTPRELLEN: Duration = Duration::from_secs(5);
/// Takt, solange eine Kopplung als neues Gerät auf die Aufnahme wartet.
const KOPPLUNG_TAKT: Duration = Duration::from_secs(5);

/// Zähler des letzten Laufs (nie Inhalte).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, Type)]
pub struct AssistentSyncBericht {
    pub hochgeladen: u32,
    pub heruntergeladen: u32,
    pub geloescht: u32,
    /// Beiträge, die wegen „nur auf diesem Gerät“ oder Vertraulichkeit nicht hochgeladen werden.
    pub nur_lokal: u32,
    /// Objekte, die auf einen Epochenschlüssel oder einen Vorgänger warten.
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
    /// Dieses Gerät ist am Hub zugelassen (Transport).
    pub zugelassen: bool,
    /// Der Hub verlangt eine Freigabe von einem schon zugelassenen Gerät.
    pub braucht_freigabe: bool,
    /// Dieses Gerät hat einen gesicherten Sync-Zustand (gegründet oder gekoppelt).
    pub eingerichtet: bool,
    /// Geräte-Kennung dieses Geräts beim Assistent-Hub (für das Kopplungsangebot am anderen Gerät).
    pub geraet_kennung: Option<String>,
    /// Eine Kopplung als neues Gerät wartet auf die Aufnahme.
    pub kopplung_offen: bool,
    pub laeuft: bool,
    /// Aktuelle Schlüsselepoche laut geprüftem Verzeichnis (ab 1; 0 = unbekannt).
    pub epoche: u32,
    /// Ein Widerruf ist vorbereitet, aber noch nicht abgeschlossen.
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
    /// Geräte, die den neuen Epochenschlüssel als Paket bekommen haben (laut geprüftem Verzeichnis).
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

/// Ergebnis von `assistent_sync_kopplung_anbieten`: Inhalt des QR-Codes für das neue Gerät.
#[derive(Serialize, Deserialize, Clone, Type)]
pub struct AssistentKopplungsangebot {
    /// QR-Inhalt (Prüfpunkt, QR-Geheimnis, Freigabecode); nur einmal anzeigen.
    pub qr: String,
    pub gueltig_bis: String,
}

impl std::fmt::Debug for AssistentKopplungsangebot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistentKopplungsangebot")
            .field("qr", &"***")
            .field("gueltig_bis", &self.gueltig_bis)
            .finish()
    }
}

/// Lease-Stand einer Sprachsitzung (E3.3).
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct AssistentSitzung {
    /// Dieses Gerät hält die Lease.
    pub hier: bool,
    /// Fencing-Wert; Beiträge dieser Sitzung tragen ihn.
    pub generation: u32,
    /// Ablauf (ms seit Epoche).
    pub gueltig_bis_ms: f64,
}

/// Zustand des Dienstes (von Tauri verwaltet).
pub struct AssistentSyncDienst {
    status: Mutex<AssistentSyncStatus>,
    /// Ein Lauf oder Hub-Befehl zur Zeit.
    sperre: tokio::sync::Mutex<()>,
    notify: tokio::sync::Notify,
    /// Abbruchschranke aller Hub-Aufrufe (Ausschalten): gesetzt, geht kein Aufruf mehr ans Netz.
    abbruch: Arc<AtomicBool>,
    /// Laufende Kopplung als neues Gerät (nur im Arbeitsspeicher; nach einem Neustart neu koppeln).
    kopplung: Arc<Mutex<Kopplungsablage>>,
    /// Eigenes Kopplungsangebot als aufnehmendes Gerät (Transportform, nur im Arbeitsspeicher).
    angebot: Arc<Mutex<Option<Zeroizing<Vec<u8>>>>>,
}

impl Default for AssistentSyncDienst {
    fn default() -> Self {
        Self {
            status: Mutex::new(AssistentSyncStatus::default()),
            sperre: tokio::sync::Mutex::new(()),
            notify: tokio::sync::Notify::new(),
            abbruch: Arc::new(AtomicBool::new(false)),
            kopplung: Arc::new(Mutex::new(Kopplungsablage::default())),
            angebot: Arc::new(Mutex::new(None)),
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

    fn kopplung_offen(&self) -> bool {
        self.kopplung
            .lock()
            .map(|k| k.kopplung.is_some())
            .unwrap_or(false)
    }
}

/// Laufende Kopplung mit Versuchszähler: Ein Abschluss nimmt die Kopplung für die Netzaufrufe heraus
/// (kein Mutex über Hub-Aufrufe) und legt sie nur zurück, wenn inzwischen weder abgebrochen noch neu
/// begonnen wurde.
#[derive(Default)]
struct Kopplungsablage {
    versuch: u64,
    kopplung: Option<Kopplung>,
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
pub(crate) struct ClientTransport {
    pub(crate) client: HubClient,
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
        .map(|(status, roh)| Antwort { status, roh })
    }
}

// ---------------------------------------------------------------------------------------------
// Konto, Zustand, Fehlertexte
// ---------------------------------------------------------------------------------------------

/// Fehler mit festem Anzeigetext und optionalem Code.
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

    fn mit_code(text: &str, code: &str) -> Self {
        Self {
            text: text.to_owned(),
            code: Some(code.to_owned()),
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
        (HubFehler::EpocheVeraltet { .. } | HubFehler::EpocheVoraus { .. }, _) => {
            "Die Schlüsselepoche passt nicht zum Hub. Bitte später erneut abgleichen."
        }
        (HubFehler::VerzeichnisKonflikt { .. }, _) => {
            "Das Geräteverzeichnis hat sich zwischenzeitlich geändert. Bitte erneut abgleichen."
        }
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

/// Feste Sätze zu Fehlern des Kerns.
fn sync_meldung(f: &SyncFehler, code: Option<String>) -> Meldung {
    match f {
        SyncFehler::Hub(h) => hub_meldung(h, code),
        SyncFehler::NichtFreigegeben => Meldung::neu("Der Assistent-Sync ist ausgeschaltet."),
        SyncFehler::Speicher => Meldung::mit_code(
            "Der Zustand des Assistent-Syncs konnte nicht gesichert werden. Es wird beim nächsten Lauf neu geladen.",
            CODE_ZUSTAND,
        ),
        SyncFehler::WiderrufOffen => Meldung::neu(
            "Ein Widerruf ist noch nicht abgeschlossen; er wird beim nächsten Abgleich fortgesetzt.",
        ),
        SyncFehler::Verzeichnis(_) | SyncFehler::VerzeichnisGabelung => Meldung::neu(
            "Das Geräteverzeichnis passt nicht. Es wurde nichts übernommen.",
        ),
        SyncFehler::Vertrag(_) | SyncFehler::Paket(_) => Meldung::neu(
            "Die Kopplungs- oder Schlüsseldaten sind ungültig. Es wurde nichts übernommen.",
        ),
        _ => Meldung {
            text: "Der Abgleich ist an einem Objekt gescheitert.".to_owned(),
            code,
            braucht_freigabe: false,
        },
    }
}

fn lauf_meldung(f: &LaufFehler, code: Option<String>) -> Meldung {
    match f {
        LaufFehler::NichtFreigegeben => Meldung::neu("Der Assistent-Sync ist ausgeschaltet."),
        LaufFehler::Sync(e) => sync_meldung(e, code),
        LaufFehler::Lease(LeaseFehler::Hub(h)) => hub_meldung(h, code),
        LaufFehler::Lease(LeaseFehler::Krypto(e)) => sync_meldung(e, code),
        LaufFehler::Lease(LeaseFehler::Konflikt) => Meldung::neu(
            "Ein anderes Gerät hat die Sitzung gerade übernommen. Bitte neu laden und erneut fragen.",
        ),
        LaufFehler::Lease(LeaseFehler::Gabelung) => Meldung::neu(
            "Die Sitzung ist wegen widersprüchlicher Stände gesperrt.",
        ),
        LaufFehler::Lease(_) => Meldung::neu("Die Sitzung konnte nicht übernommen werden."),
        LaufFehler::Unbekannt => Meldung::neu(
            "Dieses Gespräch ist (noch) nicht mit dem Assistent-Sync abgeglichen.",
        ),
        LaufFehler::Speicher(_) => {
            Meldung::neu("Der Gesprächsverlauf konnte nicht gelesen oder geschrieben werden.")
        }
    }
}

fn nicht_eingerichtet() -> Meldung {
    Meldung::mit_code(
        "Dieses Gerät ist für den Assistent-Sync noch nicht eingerichtet: Konto gründen oder mit einem eingerichteten Gerät koppeln.",
        CODE_NICHT_EINGERICHTET,
    )
}

struct Sitzung {
    cfg: SyncConfig,
    /// Grundkennung aus der Geräte-ID des Bibliotheks-Syncs.
    basis: String,
    /// Kennung beim Assistent-Hub: Grundkennung der aktuellen Generation (Kennungsstand).
    geraet: String,
    store: Arc<ConversationStore>,
    pfad: PathBuf,
    block_verzeichnis: PathBuf,
    sperre: Arc<AtomicBool>,
}

fn sitzung(app: &AppHandle) -> Result<Sitzung, Meldung> {
    if !aktiv(app) {
        return Err(Meldung::neu("Der Assistent-Sync ist ausgeschaltet."));
    }
    sitzung_ohne_schalter(app)
}

impl Sitzung {
    /// Kontokennung: Hub-URL, Konto-ID und Fingerabdruck des Bibliotheks-Schlüssels (nicht die
    /// Geräte-Kennung).
    fn konto(&self) -> Result<String, Meldung> {
        Ok(Zustand::konto_kennung(
            &self.cfg.url,
            self.cfg.user_id,
            &self.schluessel()?,
        ))
    }

    fn hub(&self) -> PortalHub<ClientTransport> {
        PortalHub::new(
            ClientTransport {
                client: HubClient::new(&self.cfg.url, &self.cfg.token),
            },
            &self.geraet,
        )
        .mit_sperre(self.sperre.clone())
    }

    fn auth(&self) -> AuthGeraet {
        AuthGeraet::new(&self.geraet)
    }

    /// Zustandsspeicher des Kerns für dieses Konto und Gerät.
    fn speicher(&self) -> Result<DateiSpeicher, Meldung> {
        DateiSpeicher::neu(
            &self.block_verzeichnis,
            &self.konto()?,
            &self.geraet,
            &self.schluessel()?,
        )
        .map_err(Meldung::from)
    }

    /// Lädt den Kern aus dem Zustandsblock. `None`: dieses Gerät ist noch nicht eingerichtet. Ein
    /// unlesbarer Block ist ein Fehler, nie „leer“ (nichts wird überschrieben).
    fn abgleich(&self) -> Result<Option<Abgleich>, Meldung> {
        Abgleich::wiederherstellen(Box::new(self.speicher()?)).map_err(|_| {
            Meldung::mit_code(
                "Der gesicherte Zustand des Assistent-Syncs ist nicht lesbar. Es wird nichts überschrieben.",
                CODE_ZUSTAND,
            )
        })
    }

    /// Verschlüsselter Kennungsstand dieses Kontos und dieser Grundkennung.
    fn kennungen(&self) -> Result<zustand::KennungsSpeicher, Meldung> {
        zustand::KennungsSpeicher::neu(
            &self.block_verzeichnis,
            &self.konto()?,
            &self.basis,
            &self.schluessel()?,
        )
        .map_err(Meldung::from)
    }

    fn abgleich_noetig(&self) -> Result<Abgleich, Meldung> {
        self.abgleich()?.ok_or_else(nicht_eingerichtet)
    }

    /// Inhalt der Zulassungsdatei. Nur „fehlt“ gilt als leer; jeder andere Lesefehler ist ein Fehler.
    fn datei_lesen(&self) -> Result<Option<String>, Meldung> {
        match std::fs::read_to_string(&self.pfad) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(Meldung::neu(
                "Die Zustandsdatei des Assistent-Syncs ist nicht lesbar. Es wird nichts überschrieben.",
            )),
        }
    }

    /// Transportzulassung dieses Kontos und Geräts (fail-closed bei unlesbarer Datei).
    fn zustand(&self) -> Result<Zustand, Meldung> {
        let text = self.datei_lesen()?;
        Zustand::aus_datei(text.as_deref(), &self.konto()?, &self.geraet).map_err(Meldung::from)
    }

    /// Schreibt den Zustand; Einträge anderer Konten bleiben, vorher entsteht eine `.bak`.
    fn speichern(&self, z: &Zustand) -> Result<(), Meldung> {
        let alt = self.datei_lesen()?;
        let json = z.in_datei(alt.as_deref(), &self.konto()?)?;
        if alt.is_some() {
            let mut name = self
                .pfad
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default();
            name.push(".bak");
            std::fs::copy(&self.pfad, self.pfad.with_file_name(name))
                .map_err(|_| Meldung::neu("Die Sicherung der Zustandsdatei ist gescheitert."))?;
        }
        account::write_private(&self.pfad, json.as_bytes()).map_err(Meldung::from)
    }

    fn schluessel(&self) -> Result<[u8; zustand::KEY_LEN], Meldung> {
        self.cfg.key().map_err(Meldung::from)
    }

    /// Kontowechsel (anderes Konto, anderer Hub oder anderer Bibliotheks-Schlüssel): nichts wird
    /// gelöscht oder zurückgesetzt, der Zustand je Konto bleibt aufbewahrt. Der Lauf hält an
    /// (`konto_gewechselt`), der Aufrufer schaltet den Assistent-Sync aus; erst das erneute
    /// Einschalten durch die Nutzerin oder den Nutzer bestätigt das neue Konto.
    fn konto_abgleichen(&self) -> Result<(), Meldung> {
        let konto = self.konto()?;
        let bekannt = self
            .store
            .sync_meta("konto")
            .map_err(|_| Meldung::neu("Der Gesprächsverlauf konnte nicht gelesen werden."))?;
        match bekannt.as_deref() {
            Some(k) if k == konto => Ok(()),
            None => self.konto_bestaetigen(),
            Some(_) => Err(Meldung::mit_code(
                "Das Konto oder der Schlüssel hat gewechselt. Der Assistent-Sync wurde ausgeschaltet; bitte neu einschalten, um das neue Konto zu bestätigen.",
                CODE_KONTO_GEWECHSELT,
            )),
        }
    }

    /// Merkt das angemeldete Konto als das, mit dem dieser Verlauf abgleicht.
    fn konto_bestaetigen(&self) -> Result<(), Meldung> {
        self.store
            .sync_meta_setzen("konto", &self.konto()?)
            .map_err(|_| Meldung::neu("Der Gesprächsverlauf konnte nicht geschrieben werden."))
    }

    /// Lässt dieses Gerät am Hub zu, falls es das noch nicht ist (Erstgerät, Kopplung oder Freigabecode).
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

    /// Vorbereitung jedes Hub-Befehls: Konto bestätigt, Transport zugelassen.
    fn vorbereiten(&self) -> Result<PortalHub<ClientTransport>, Meldung> {
        self.konto_abgleichen()?;
        let mut z = self.zustand()?;
        self.zulassen(&mut z, None)?;
        Ok(self.hub())
    }
}

// ---------------------------------------------------------------------------------------------
// Läufe und Befehle (blockierende Teile laufen in `spawn_blocking`)
// ---------------------------------------------------------------------------------------------

struct Ergebnis {
    bericht: LaufBericht,
    epoche: u32,
    widerruf_offen: bool,
}

fn epoche_anzeige(a: &Abgleich) -> u32 {
    u32::try_from(a.epoche()).unwrap_or(u32::MAX)
}

fn widerruf_offen(a: &Abgleich) -> bool {
    a.offener_widerruf().is_some() || a.offene_widerrufsabsichten().next().is_some()
}

/// Schließt eine laufende Kopplung ab, wenn das aufnehmende Gerät aufgenommen und die Schlüssel
/// abgelegt hat. `Ok(None)`: noch nicht so weit (die Kopplung bleibt offen). Die Kopplung wird für die
/// Netzaufrufe aus der Ablage genommen; der Mutex wird nie über einen Hub-Aufruf gehalten.
fn kopplung_abschliessen(
    s: &Sitzung,
    hub: &PortalHub<ClientTransport>,
    ablage: &Mutex<Kopplungsablage>,
) -> Result<Option<Abgleich>, Meldung> {
    let nicht_verfuegbar = || Meldung::neu("Die Kopplung ist nicht verfügbar.");
    let speicher = s.speicher()?;
    let (versuch, laufend) = {
        let mut g = ablage.lock().map_err(|_| nicht_verfuegbar())?;
        match g.kopplung.take() {
            Some(k) => (g.versuch, k),
            None => return Ok(None),
        }
    };
    let zuruecklegen = |k: Kopplung| {
        if let Ok(mut g) = ablage.lock() {
            if g.versuch == versuch && g.kopplung.is_none() {
                g.kopplung = Some(k);
            }
        }
    };
    let ergebnis = laufend.abschliessen(hub, Box::new(speicher));
    match ergebnis {
        Ok(Some(a)) => {
            kopplung_vermerk_loeschen(s);
            Ok(Some(a))
        }
        Ok(None) => {
            zuruecklegen(laufend);
            Ok(None)
        }
        Err(SyncFehler::Hub(h)) => {
            zuruecklegen(laufend);
            Err(hub_meldung(&h, hub.letzter_code()))
        }
        // Kette widerspricht dem Prüfpunkt aus dem QR-Code: diese Kopplung ist verbraucht. Der
        // Vermerk bleibt; der nächste Lauf löst den Versuch auf (ggf. mit neuer Kennung).
        Err(e) => Err(sync_meldung(&e, None)),
    }
}

/// Der Kopplungsversuch ist abgeschlossen. Scheitert das Sichern, holt es der nächste Lauf nach
/// (der Zustandsblock liegt dann schon).
fn kopplung_vermerk_loeschen(s: &Sitzung) {
    let r = s.kennungen().and_then(|sp| {
        let mut st = sp.laden()?;
        st.kopplung_begonnen = false;
        sp.sichern(&st).map_err(Meldung::from)
    });
    if r.is_err() {
        log::warn!("Assistent-Sync: Kopplungsvermerk konnte nicht zurückgesetzt werden");
    }
}

/// Löst einen Kopplungsversuch auf, dessen Identität verloren ist (Neustart oder Abbruch nach dem
/// Sichern des Vermerks). Liegt schon ein Zustandsblock, war er abgeschlossen. Sonst entscheidet das
/// (ungeprüfte) Verzeichnis des Hubs, ob die Kennung schon aufgenommen und damit verbraucht ist; dann
/// gilt die nächste Generation und der Aufruf meldet [`CODE_KENNUNG_NEU`]. Ohne Vermerk kein Netzaufruf.
fn verlorene_kopplung_aufloesen(s: &mut Sitzung) -> Result<(), Meldung> {
    let sp = s.kennungen()?;
    let mut st = sp.laden()?;
    if !st.kopplung_begonnen {
        return Ok(());
    }
    if s.speicher()?.vorhanden() {
        st.kopplung_begonnen = false;
        return sp.sichern(&st).map_err(Meldung::from);
    }
    let hub = s.vorbereiten()?;
    let eintraege = hub
        .verzeichnis_lesen(&s.auth(), 0)
        .map_err(|h| hub_meldung(&h, hub.letzter_code()))?;
    let gewechselt =
        st.verlorenen_versuch_aufloesen(zustand::aufgenommen_laut_hub(&eintraege, &s.geraet));
    sp.sichern(&st)?;
    if gewechselt {
        s.geraet = st.kennung(&s.basis);
        return Err(Meldung::mit_code(
            "Die letzte Kopplung wurde unterbrochen, nachdem das andere Gerät dieses Gerät schon aufgenommen hatte. Dieses Gerät koppelt deshalb unter einer neuen Geräte-Kennung: bitte neu anmelden und am anderen Gerät für die neue Kennung erneut anbieten.",
            CODE_KENNUNG_NEU,
        ));
    }
    Ok(())
}

fn lauf_blockierend(
    s: &mut Sitzung,
    kopplung: &Mutex<Kopplungsablage>,
) -> Result<Ergebnis, Meldung> {
    s.konto_abgleichen()?;
    let gespeichert = s.abgleich()?;
    let kopplung_laeuft = kopplung
        .lock()
        .map(|k| k.kopplung.is_some())
        .unwrap_or(false);
    if gespeichert.is_none() && !kopplung_laeuft {
        // Ohne Zustand und ohne Kopplung kein Netzaufruf, außer um einen unterbrochenen
        // Kopplungsversuch aufzulösen.
        verlorene_kopplung_aufloesen(s)?;
        return Err(nicht_eingerichtet());
    }
    let hub = s.vorbereiten()?;
    let mut a = match gespeichert {
        Some(a) => a,
        None => kopplung_abschliessen(s, &hub, kopplung)?.ok_or_else(|| {
            Meldung::mit_code(
                "Die Kopplung wartet darauf, dass das andere Gerät dieses Gerät aufnimmt.",
                CODE_KOPPLUNG_OFFEN,
            )
        })?,
    };
    let r = verlauf::lauf(&s.store, &hub, &mut a, &s.konto()?, true);
    match r {
        Ok(bericht) => Ok(Ergebnis {
            bericht,
            epoche: epoche_anzeige(&a),
            widerruf_offen: widerruf_offen(&a),
        }),
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

/// Status aus Schalter, Konto und Dateien; ohne Netz und ohne den Zustandsblock zu entschlüsseln.
fn status_lokal(app: &AppHandle, dienst: &AssistentSyncDienst) -> AssistentSyncStatus {
    let mut st = dienst.status();
    st.aktiv = aktiv(app);
    let cfg = account::load(app);
    st.angemeldet = cfg.is_some();
    st.kopplung_offen = dienst.kopplung_offen();
    if let Ok(s) = sitzung_ohne_schalter(app) {
        let z = s.zustand().unwrap_or_default();
        st.zugelassen = z.zugelassen;
        st.eingerichtet = s.speicher().is_ok_and(|sp| sp.vorhanden());
        st.geraet_kennung = Some(s.geraet.clone());
        st.grabsteine_offen = s
            .konto()
            .ok()
            .and_then(|k| s.store.sync_grabsteine_offen(&k).ok())
            .map_or(0, |g| u32::try_from(g.len()).unwrap_or(u32::MAX));
    } else {
        st.zugelassen = false;
        st.eingerichtet = false;
    }
    st
}

/// Wie [`sitzung`], aber ohne den Schalter zu verlangen (nur lokale Statusanzeige); ohne Netzaufruf.
fn sitzung_ohne_schalter(app: &AppHandle) -> Result<Sitzung, Meldung> {
    let cfg = account::load(app).ok_or_else(|| Meldung::neu("Nicht angemeldet."))?;
    let basis = zustand::geraete_kennung(&cfg.device_id)
        .ok_or_else(|| Meldung::neu("Die Geräte-Kennung ist ungültig."))?;
    let store = app
        .try_state::<Arc<ConversationStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| Meldung::neu("Der Gesprächsverlauf ist nicht verfügbar."))?;
    let daten =
        crate::portable::app_data_dir(app).map_err(|_| Meldung::neu("Kein Datenverzeichnis."))?;
    let sperre = app
        .try_state::<AssistentSyncDienst>()
        .map(|d| d.abbruch.clone())
        .unwrap_or_default();
    let mut s = Sitzung {
        cfg,
        geraet: basis.clone(),
        basis,
        store,
        pfad: daten.join(zustand::DATEI),
        block_verzeichnis: daten.join(zustand::BLOCK_VERZEICHNIS),
        sperre,
    };
    // Kennung der aktuellen Generation (fail-closed: ein unlesbarer Stand ist ein Fehler).
    s.geraet = s.kennungen()?.laden()?.kennung(&s.basis);
    Ok(s)
}

fn melden(app: &AppHandle, dienst: &AssistentSyncDienst) -> AssistentSyncStatus {
    let st = status_lokal(app, dienst);
    let _ = app.emit(EVENT_STATUS, st.clone());
    st
}

/// Merkt eine Meldung im Status (und schaltet nach einem Kontowechsel aus).
fn fehler_merken(app: &AppHandle, dienst: &AssistentSyncDienst, m: &Meldung) {
    bei_kontowechsel_ausschalten(app, m);
    dienst.setzen(|st| {
        st.braucht_freigabe = m.braucht_freigabe;
        st.letzter_fehler = Some(m.text.clone());
        st.fehler_code = m.code.clone();
    });
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
    let kopplung = dienst.kopplung.clone();
    let r = blockierend(move || {
        let mut s = s;
        lauf_blockierend(&mut s, &kopplung)
    })
    .await;
    dienst.setzen(|st| st.laeuft = false);
    match r {
        Ok(e) => dienst.setzen(|st| {
            st.letzter_lauf_ms = Some(now_ms());
            st.letzter_fehler = None;
            st.fehler_code = None;
            st.braucht_freigabe = false;
            st.epoche = e.epoche;
            st.widerruf_offen = e.widerruf_offen;
            st.letzter_bericht = Some((&e.bericht).into());
        }),
        Err(m) => fehler_merken(app, dienst, &m),
    }
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
            // Wartet eine Kopplung auf die Aufnahme, kurz und häufig abfragen.
            let takt = if dienst.kopplung_offen() {
                KOPPLUNG_TAKT
            } else {
                TAKT
            };
            tokio::select! {
                _ = tokio::time::sleep(takt) => {}
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

/// Führt einen Hub-Befehl unter der Dienstsperre im Blockier-Thread aus und merkt Fehler im Status.
async fn befehl<T, F>(app: &AppHandle, dienst: &AssistentSyncDienst, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(Sitzung) -> Result<T, Meldung> + Send + 'static,
{
    let r = {
        let _sperre = dienst.sperre.lock().await;
        match sitzung(app) {
            Ok(s) => blockierend(move || f(s)).await,
            Err(m) => Err(m),
        }
    };
    if let Err(m) = &r {
        fehler_merken(app, dienst, m);
    } else {
        dienst.setzen(|st| {
            st.letzter_fehler = None;
            st.fehler_code = None;
        });
    }
    melden(app, dienst);
    r.map_err(|m| m.text)
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

/// Sofort abgleichen (schließt auch eine laufende Kopplung ab, sobald das andere Gerät aufgenommen
/// hat). Ohne den Schalter `assistent_sync_aktiv` passiert nichts.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_starten(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> Result<AssistentSyncStatus, String> {
    Ok(lauf_einmal(&app, &dienst).await)
}

/// Gründet das Konto mit diesem Gerät als Vertrauensanker (Erstgerät): Transportzulassung, neue
/// Identität, Epoche 1 und Kontosalz zufällig, Anker ins Verzeichnis; danach ein Abgleich. Hat das
/// Konto am Hub schon ein Verzeichnis, wird nicht gegründet (dann koppeln).
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_gruenden(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> Result<AssistentSyncStatus, String> {
    befehl(&app, &dienst, |s| {
        if s.abgleich()?.is_some() {
            return Err(Meldung::neu("Dieses Gerät ist schon eingerichtet."));
        }
        let hub = s.vorbereiten()?;
        let vorhanden = hub
            .verzeichnis_lesen(&s.auth(), 0)
            .map_err(|h| hub_meldung(&h, hub.letzter_code()))?;
        if !vorhanden.is_empty() {
            return Err(Meldung::neu(
                "Das Konto hat schon eingerichtete Geräte. Bitte dieses Gerät mit einem davon koppeln.",
            ));
        }
        let mut a = Abgleich::gruenden(&s.geraet, Box::new(s.speicher()?))
            .map_err(|e| sync_meldung(&e, None))?;
        a.freigabe_einschalten()
            .map_err(|e| sync_meldung(&e, None))?;
        // Anker ans Verzeichnis (scheitert das, holt der nächste Lauf es nach).
        a.hochladen(&hub)
            .map(|_| ())
            .map_err(|e| sync_meldung(&e, hub.letzter_code()))
    })
    .await?;
    Ok(lauf_einmal(&app, &dienst).await)
}

/// Kopplung, Schritt 1 (aufnehmendes, eingerichtetes Gerät): gibt die Kennung `geraet_id` am Hub frei
/// und liefert den QR-Inhalt mit Prüfpunkt des Verzeichnisses, QR-Geheimnis und Freigabecode. Das
/// Angebot bleibt nur im Arbeitsspeicher und wird für `assistent_sync_kopplung_aufnehmen` gebraucht.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_kopplung_anbieten(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    geraet_id: String,
) -> Result<AssistentKopplungsangebot, String> {
    let neu = geraet_id.trim().to_ascii_lowercase();
    if !zustand::gueltige_kennung(&neu) {
        return Err("Die Geräte-Kennung hat ein ungültiges Format.".into());
    }
    let ablage = dienst.angebot.clone();
    befehl(&app, &dienst, move |s| {
        let mut a = s.abgleich_noetig()?;
        let hub = s.vorbereiten()?;
        // Aktuelles, geprüftes Verzeichnis als Prüfpunkt.
        a.herunterladen(&hub)
            .map_err(|e| sync_meldung(&e, hub.letzter_code()))?;
        let angebot = a.kopplung_anbieten().map_err(|e| sync_meldung(&e, None))?;
        let freigabe = hub
            .freigeben(&neu)
            .map_err(|h| hub_meldung(&h, hub.letzter_code()))?;
        let roh = Zeroizing::new(
            serde_json::to_vec(&angebot)
                .map_err(|_| Meldung::neu("Das Kopplungsangebot ist ungültig."))?,
        );
        let qr = KopplungsQr {
            geraet_id: neu,
            freigabe_code: Zeroizing::new(freigabe.code),
            angebot,
        }
        .als_text()?;
        *ablage
            .lock()
            .map_err(|_| Meldung::neu("Die Kopplung ist nicht verfügbar."))? = Some(roh);
        Ok(AssistentKopplungsangebot {
            qr: qr.to_string(),
            gueltig_bis: freigabe.gueltig_bis,
        })
    })
    .await
}

/// Kopplung, Schritt 2 (neues Gerät): liest den QR-Inhalt, lässt dieses Gerät mit dem Freigabecode am
/// Hub zu, erzeugt die Geräteidentität und liefert die Antwort für den Rückkanal (Ausweis, HMAC mit dem
/// QR-Geheimnis, Besitznachweis). Die Antwort geht auf beliebigem Weg zurück an das aufnehmende Gerät.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_kopplung_beginnen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    qr: String,
) -> Result<String, String> {
    let ablage = dienst.kopplung.clone();
    let qr = Zeroizing::new(qr);
    befehl(&app, &dienst, move |mut s| {
        if s.abgleich()?.is_some() {
            return Err(Meldung::neu("Dieses Gerät ist schon eingerichtet."));
        }
        let nicht_verfuegbar = || Meldung::neu("Die Kopplung ist nicht verfügbar.");
        if ablage
            .lock()
            .map_err(|_| nicht_verfuegbar())?
            .kopplung
            .is_some()
        {
            return Err(Meldung::mit_code(
                "Eine Kopplung läuft bereits. Bitte warten oder sie abbrechen.",
                CODE_KOPPLUNG_OFFEN,
            ));
        }
        // Ein unterbrochener Versuch wird zuerst aufgelöst (ggf. neue Kennung, dann passt der QR-Code
        // nicht mehr und das andere Gerät muss neu anbieten).
        verlorene_kopplung_aufloesen(&mut s)?;
        let qr = KopplungsQr::aus_text(&qr)?;
        if qr.geraet_id != s.geraet {
            return Err(Meldung::neu(
                "Der Kopplungscode gilt für ein anderes Gerät.",
            ));
        }
        s.konto_abgleichen()?;
        let mut z = s.zustand()?;
        s.zulassen(&mut z, Some(qr.freigabe_code.as_str()))?;
        let k = Kopplung::neu(&s.geraet, qr.angebot).map_err(|e| sync_meldung(&e, None))?;
        let antwort = k.antwort().map_err(|e| sync_meldung(&e, None))?;
        let text = serde_json::to_string(&antwort)
            .map_err(|_| Meldung::neu("Die Kopplungsantwort ist ungültig."))?;
        // Vermerk sichern, BEVOR die Antwort das Gerät verlässt: geht die Identität danach verloren,
        // weiß der nächste Lauf davon.
        let sp = s.kennungen()?;
        let mut st = sp.laden()?;
        st.kopplung_begonnen = true;
        sp.sichern(&st)?;
        let mut g = ablage.lock().map_err(|_| nicht_verfuegbar())?;
        g.versuch = g.versuch.wrapping_add(1);
        g.kopplung = Some(k);
        Ok(text)
    })
    .await
}

/// Kopplung, Schritt 3 (aufnehmendes Gerät): prüft die Antwort des neuen Geräts gegen das eigene
/// Angebot (HMAC, Besitznachweis, Prüfpunkt), nimmt es per signiertem Verzeichniseintrag auf und legt
/// die Epochenschlüssel als Pakete für es ab. Das neue Gerät schließt danach mit einem Abgleich ab.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_kopplung_aufnehmen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    antwort: String,
) -> Result<(), String> {
    let ablage = dienst.angebot.clone();
    befehl(&app, &dienst, move |s| {
        let antwort = Kopplungsantwort::aus_json(antwort.trim().as_bytes())
            .map_err(|_| Meldung::neu("Die Kopplungsantwort ist ungültig."))?;
        let mut gemerkt = ablage
            .lock()
            .map_err(|_| Meldung::neu("Die Kopplung ist nicht verfügbar."))?;
        let roh = gemerkt.as_ref().ok_or_else(|| {
            Meldung::neu("Es gibt kein offenes Kopplungsangebot. Bitte neu anbieten.")
        })?;
        let angebot = Kopplungsangebot::aus_json(roh)
            .map_err(|_| Meldung::neu("Das Kopplungsangebot ist ungültig."))?;
        let mut a = s.abgleich_noetig()?;
        let hub = s.vorbereiten()?;
        a.geraet_aufnehmen(&hub, &angebot, &antwort)
            .map_err(|e| sync_meldung(&e, hub.letzter_code()))?;
        *gemerkt = None;
        Ok(())
    })
    .await
}

/// Bricht eine laufende Kopplung als neues Gerät ab (verwirft die Identität). Der gesicherte Vermerk
/// bleibt; der nächste Lauf oder Kopplungsbeginn löst den Versuch auf (neue Kennung, falls das andere
/// Gerät schon aufgenommen hatte).
#[tauri::command]
#[specta::specta]
pub fn assistent_sync_kopplung_abbrechen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> AssistentSyncStatus {
    if let Ok(mut k) = dienst.kopplung.lock() {
        k.versuch = k.versuch.wrapping_add(1);
        k.kopplung = None;
    }
    if let Ok(mut a) = dienst.angebot.lock() {
        *a = None;
    }
    melden(&app, &dienst)
}

/// Lässt dieses Gerät mit dem Freigabecode eines schon zugelassenen Geräts am Hub zu (`POST /geraet`,
/// nur Transport) und gleicht danach ab. Für die Aufnahme ins Konto ist die Kopplung nötig.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraet_zulassen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    freigabe_code: String,
) -> Result<AssistentSyncStatus, String> {
    let code = freigabe_code.trim().to_owned();
    // Einmalcode: base64url, 27 Zeichen; großzügig prüfen, nie in die Meldung übernehmen.
    let form_ok = (16..=128).contains(&code.len())
        && code
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if !form_ok {
        return Err("Der Freigabecode hat ein ungültiges Format.".into());
    }
    befehl(&app, &dienst, move |s| {
        let mut z = s.zustand()?;
        s.konto_abgleichen()?;
        s.zulassen(&mut z, Some(&code))
    })
    .await?;
    dienst.setzen(|st| st.braucht_freigabe = false);
    Ok(lauf_einmal(&app, &dienst).await)
}

/// Widerruft ein Gerät über das Verzeichnis (E3.8): dauerhafte Absicht, signierter Widerrufseintrag
/// mit neuer Epoche, neuer Schlüssel nur als Pakete an die verbleibenden Geräte des geprüften
/// Verzeichnisses; vor jedem Hub-Schritt gesichert, nach Abbruch beim nächsten Lauf fortgesetzt.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraet_widerrufen(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    geraet_id: String,
) -> Result<AssistentWiderruf, String> {
    let ziel = geraet_id.trim().to_ascii_lowercase();
    if !zustand::gueltige_kennung(&ziel) {
        return Err("Die Geräte-Kennung hat ein ungültiges Format.".into());
    }
    befehl(&app, &dienst, move |s| {
        let mut a = s.abgleich_noetig()?;
        let hub = s.vorbereiten()?;
        let epoche = a
            .widerrufen(&hub, &ziel)
            .map_err(|e| sync_meldung(&e, hub.letzter_code()))?;
        Ok(AssistentWiderruf {
            epoche: u32::try_from(epoche).unwrap_or(u32::MAX),
            empfaenger: a.empfaenger(),
        })
    })
    .await
}

/// Gibt eine Kennung für ein weiteres Gerät frei und liefert den Einmalcode (`POST /freigeben`, nur
/// Transport; die Kopplung enthält den Code schon).
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
    befehl(&app, &dienst, move |s| {
        let hub = s.vorbereiten()?;
        hub.freigeben(&neu)
            .map(|f| AssistentFreigabe {
                code: f.code,
                gueltig_bis: f.gueltig_bis,
            })
            .map_err(|h| hub_meldung(&h, hub.letzter_code()))
    })
    .await
}

/// Zugelassene, nicht widerrufene Geräte des Kontos laut **geprüftem** Verzeichnis (Kennungen); vorher
/// wird das Verzeichnis vom Hub geholt und geprüft.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_geraete(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
) -> Result<Vec<String>, String> {
    befehl(&app, &dienst, |s| {
        let mut a = s.abgleich_noetig()?;
        let hub = s.vorbereiten()?;
        a.herunterladen(&hub)
            .map_err(|e| sync_meldung(&e, hub.letzter_code()))?;
        Ok(a.verzeichnis()
            .zugelassene()
            .into_iter()
            .map(|g| g.geraet_id.clone())
            .collect())
    })
    .await
}

/// Lease der Sprachsitzung eines Gesprächs (E3.3): ohne `uebernehmen` nur lesen, sonst „hier
/// übernehmen“ (neue Generation) bzw. die eigene, laufende Lease verlängern. Fencing nur über die
/// Generation; ein Konflikt heißt: neu lesen und erneut fragen.
#[tauri::command]
#[specta::specta]
pub async fn assistent_sync_sitzung(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    conversation_id: String,
    uebernehmen: bool,
) -> Result<Option<AssistentSitzung>, String> {
    befehl(&app, &dienst, move |s| {
        let mut a = s.abgleich_noetig()?;
        let hub = s.vorbereiten()?;
        let jetzt = u64::try_from(chrono::Utc::now().timestamp()).unwrap_or(0);
        let stand = verlauf::sitzung(&s.store, &hub, &mut a, &conversation_id, jetzt, uebernehmen)
            .map_err(|f| lauf_meldung(&f, hub.letzter_code()))?;
        Ok(stand.map(|st: Sitzungsstand| AssistentSitzung {
            hier: st.inhaber == s.geraet,
            generation: u32::try_from(st.generation).unwrap_or(u32::MAX),
            gueltig_bis_ms: st.gueltig_bis as f64 * 1000.0,
        }))
    })
    .await
}

/// Schaltet den Assistent-Sync ein oder aus (Standard aus, E3.6). Erst nach dem Einschalten werden
/// bestehende lokale Gespräche hochgeladen. Ausschalten setzt die Abbruchschranke, wartet, bis ein
/// laufender Lauf geendet ist (danach geht kein Aufruf mehr ans Netz), und gibt die Schranke wieder
/// frei. Einschalten bestätigt das angemeldete Konto (auch nach einem Konto- oder Schlüsselwechsel).
#[tauri::command]
#[specta::specta]
pub async fn change_assistent_sync_aktiv_setting(
    app: AppHandle,
    dienst: State<'_, AssistentSyncDienst>,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        let _sperre = dienst.sperre.lock().await;
        if let Ok(s) = sitzung_ohne_schalter(&app) {
            s.konto_bestaetigen().map_err(|m| m.text)?;
        }
        schalter_schreiben(&app, true);
        drop(_sperre);
        dienst.anstossen();
    } else {
        ausschalten(&app, &dienst).await;
    }
    melden(&app, &dienst);
    Ok(())
}

fn schalter_schreiben(app: &AppHandle, an: bool) {
    let mut settings = crate::settings::get_settings(app);
    settings.assistent_sync_aktiv = an;
    crate::settings::write_settings(app, settings);
}

/// Schalter aus, Schranke zu, auf das Ende eines laufenden Laufs warten, Schranke wieder auf.
async fn ausschalten(app: &AppHandle, dienst: &AssistentSyncDienst) {
    schalter_schreiben(app, false);
    dienst.abbruch.store(true, Ordering::SeqCst);
    let _lauf_zu_ende = dienst.sperre.lock().await;
    dienst.abbruch.store(false, Ordering::SeqCst);
}

/// Nach einem Konto- oder Schlüsselwechsel: Schalter aus (ohne auf `sperre` zu warten, die der
/// Aufrufer hält).
fn bei_kontowechsel_ausschalten(app: &AppHandle, m: &Meldung) {
    if m.code.as_deref() == Some(CODE_KONTO_GEWECHSELT) {
        schalter_schreiben(app, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meldungen_tragen_feste_codes_und_keine_kennungen() {
        let m = nicht_eingerichtet();
        assert_eq!(m.code.as_deref(), Some(CODE_NICHT_EINGERICHTET));
        assert!(!m.braucht_freigabe);
        let m = sync_meldung(&SyncFehler::Speicher, None);
        assert_eq!(m.code.as_deref(), Some(CODE_ZUSTAND));
        let m = hub_meldung(&HubFehler::NichtZugelassen, Some("nicht_zugelassen".into()));
        assert!(m.braucht_freigabe);
        let m = hub_meldung(&HubFehler::NichtZugelassen, Some("forbidden".into()));
        assert!(!m.braucht_freigabe);
        let m = hub_meldung(&HubFehler::VerzeichnisKonflikt { laenge: 3 }, None);
        assert!(!m.text.contains('3'));
    }
}
