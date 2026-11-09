//! Fehlertypen des Sync-Kerns. Keine Variante traegt Inhalte (Text, Titel, Schluessel).

use thiserror::Error;

use super::hub::HubFehler;

/// Fehler im geraeteseitigen Sync-Kern.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SyncFehler {
    #[error("ungueltige Kennung (erwartet: 32 Hexzeichen)")]
    UngueltigeKennung,
    #[error("Zufallsquelle nicht verfuegbar")]
    Zufall,
    #[error("Umschlag verletzt den Vertrag: {0}")]
    Vertrag(&'static str),
    #[error("Schluesselepoche {0} ist auf diesem Geraet unbekannt")]
    EpocheUnbekannt(u64),
    #[error("Epoche {0} ist schon mit einem anderen Schluessel belegt")]
    EpocheKollision(u64),
    #[error("Verschluesselung fehlgeschlagen")]
    Verschluesseln,
    #[error("Oeffnen fehlgeschlagen: Schluessel oder gebundene Steuerfelder passen nicht")]
    Oeffnen,
    #[error("sichtbare Steuerfelder passen nicht zum verschluesselten Klartext")]
    SteuerfelderUngleich,
    #[error("Inhalt nicht lesbar")]
    Inhalt,
    #[error("Assistent-Sync ist nicht freigegeben")]
    NichtFreigegeben,
    #[error("Unterhaltung ist nur auf diesem Geraet (geraet_lokal)")]
    GeraetLokal,
    #[error("Unterhaltung oder Beitrag ist durch einen Grabstein geloescht")]
    Geloescht,
    #[error("Unterhaltung unbekannt")]
    UnbekannteUnterhaltung,
    #[error("Zweigwahl ungueltig: Beitrag ist kein Kind des Elternteils")]
    ZweigUngueltig,
    #[error("Autor ist im Geraeteverzeichnis unbekannt")]
    AutorUnbekannt,
    #[error("Signatur des Autors ungueltig")]
    Signatur,
    #[error("Geraeteverzeichnis: {0}")]
    Verzeichnis(&'static str),
    #[error("Geraeteverzeichnis gegabelt: zwei Fassungen desselben Eintrags")]
    VerzeichnisGabelung,
    #[error("Schluesselpaket: {0}")]
    Paket(&'static str),
    #[error("Zustandsversion aelter als der lokale Hoechststand (Rollback)")]
    Rollback,
    #[error("Widerruf noch nicht abgeschlossen: Schreibzugriffe gesperrt")]
    WiderrufOffen,
    #[error("Zustand konnte nicht dauerhaft gespeichert werden")]
    Speicher,
    #[error("Hub: {0}")]
    Hub(#[from] HubFehler),
}
