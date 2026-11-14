//! Die produktive Lesequelle des Prüfers (lva#604, Teilpaket 2): Einstellungsdatei der App und
//! Besprechungsdatenbank.
//!
//! Sie liest **genau wie der MCP-Server** (`mcp::tools::lauf_lesen`) und benutzt dessen
//! Bausteine, statt sie nachzubauen:
//! - Die Richtlinie kommt über den **strengen Leseweg** aus der Datei `settings_store.json`
//!   ([`read_ki_sperre_lesbar`](crate::mcp::read_ki_sperre_lesbar), Vorbild `mcp/mod.rs:140`),
//!   nicht über `settings::get_settings(&AppHandle)`: das hat Rettungs- und Schreibverhalten
//!   (`settings.rs:2345`, `:2393`) und würde einen Lesefehler durch Standardwerte ersetzen. Ein
//!   Lesefehler ist hier [`grund::LESEFEHLER`], nie „Standard“. Die Quelle schreibt nie.
//! - Revision und Richtlinienspiegel kommen aus **einer** Lesetransaktion
//!   (`MeetingStore::ki_stand`); die Richtlinie gilt nur, wenn sie zum Spiegel passt
//!   (`KiSperre::abgleichen`): fehlender Spiegel ist [`grund::SPIEGEL_FEHLT`], abweichender
//!   [`grund::SPIEGEL_ABWEICHEND`].
//! - Die Sichtbarkeit einer Besprechung ist `KiSperre::sichtbar` (`ki_freigabe.rs:520`); ein
//!   Lesefehler macht sie unsichtbar.
//!
//! Die Reihenfolge ist wie im MCP-Server: erst die Datei, dann die Datenbank. Die Quelle hält
//! keinen Zustand und keine Sperre; jede Lesung öffnet ihre Datenbankverbindung frisch.
//!
//! Hinweis zur Dauer: Eine halb geschriebene Einstellungsdatei liest `read_settings_value`
//! bis zu dreimal im Abstand von 40 ms nach. Die Quelle läuft nie im Audiopfad.

use std::path::PathBuf;
use std::sync::Arc;

use super::versiegelt::Versiegelt;
use super::{FreigabeQuelle, Lesung};
use crate::llm_versand::fehler::{grund, VersandFehler};
use crate::managers::meetings::ki_freigabe::KiSperre;
use crate::managers::meetings::store::MeetingStore;

/// Lesequelle der App: Einstellungsdatei plus Besprechungsspeicher.
pub struct AppFreigabeQuelle {
    einstellungen: PathBuf,
    store: Arc<MeetingStore>,
}

impl AppFreigabeQuelle {
    /// Quelle für eine bestimmte Einstellungsdatei und einen Besprechungsspeicher.
    pub fn neu(einstellungen: PathBuf, store: Arc<MeetingStore>) -> Self {
        Self {
            einstellungen,
            store,
        }
    }

    /// Quelle der laufenden App: die Einstellungsdatei im Datenverzeichnis (wie
    /// `portable::store_path`) und der verwaltete `Arc<MeetingStore>`. Ist eines von beiden nicht
    /// zu bestimmen, gibt es keinen Nachweis und damit [`grund::LESEFEHLER`].
    pub fn aus_app(app: &tauri::AppHandle) -> Result<Self, VersandFehler> {
        let lesefehler = || VersandFehler::gesperrt(grund::LESEFEHLER);
        let einstellungen =
            crate::portable::resolve_app_data(app, crate::settings::SETTINGS_STORE_PATH)
                .map_err(|_| lesefehler())?;
        let store = tauri::Manager::try_state::<Arc<MeetingStore>>(app).ok_or_else(lesefehler)?;
        Ok(Self::neu(einstellungen, Arc::clone(store.inner())))
    }
}

impl Versiegelt for AppFreigabeQuelle {}

impl FreigabeQuelle for AppFreigabeQuelle {
    fn lesen(&self) -> Result<Lesung, VersandFehler> {
        let lesefehler = || VersandFehler::gesperrt(grund::LESEFEHLER);
        let quelle =
            crate::mcp::read_ki_sperre_lesbar(&self.einstellungen).ok_or_else(lesefehler)?;
        let stand = self.store.ki_stand().map_err(|_| lesefehler())?;
        let Some(spiegel) = stand.richtlinie.as_ref() else {
            return Err(VersandFehler::gesperrt(grund::SPIEGEL_FEHLT));
        };
        let sperre: KiSperre = quelle
            .abgleichen(Some(spiegel))
            .ok_or_else(|| VersandFehler::gesperrt(grund::SPIEGEL_ABWEICHEND))?;
        Ok(Lesung {
            revision: stand.rev,
            sperre,
        })
    }

    fn sichtbar(&self, sperre: &KiSperre, besprechung_id: &str) -> bool {
        sperre.sichtbar(&self.store, besprechung_id)
    }
}
