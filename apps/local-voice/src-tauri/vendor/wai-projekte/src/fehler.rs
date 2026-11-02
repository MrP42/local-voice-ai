//! Fehlertyp des Projektverzeichnisses. Meldungen tragen nie Titel oder Inhalte.

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjektFehler {
    /// Auch fuer fremde Mandanten: Existenz wird nicht offengelegt.
    #[error("nicht gefunden")]
    NichtGefunden,
    #[error("Revisionskonflikt: aktuelle Revision ist {aktuell}")]
    RevisionKonflikt { aktuell: i64 },
    #[error("Datenklasse des Eintrags liegt ueber der des Projekts")]
    KlasseUeberProjekt,
    #[error("Herabstufen gesperrt: Eintraege hoeherer Klasse sind zugeordnet")]
    HerabstufenGesperrt,
    #[error("Plan ist bereits einem anderen Projekt zugeordnet")]
    PlanVergeben,
    #[error("Projekt hat bereits einen Plan")]
    ProjektHatPlan,
    #[error("kein Leserecht an der Herkunft des Eintrags")]
    KeinHerkunftsrecht,
    #[error("Art des Eintrags passt nicht zur Herkunftsapp im Verweis")]
    ArtPasstNichtZurApp,
    #[error("Verweis ungueltig: {0}")]
    LinkUngueltig(String),
    #[error("Eingabe ungueltig: {0}")]
    Ungueltig(String),
    #[error("Eintrag ist diesem Projekt bereits zugeordnet")]
    Doppelt,
    #[error("Projekt ist archiviert")]
    Archiviert,
    #[error(
        "Schemaversion {gefunden} ist neuer als bekannte {bekannt}: App aktualisieren (nur lesend)"
    )]
    SchemaNeuer { gefunden: i64, bekannt: i64 },
    #[error("Speicherfehler: {0}")]
    Speicher(String),
}

impl From<wai_storage::sqlx::Error> for ProjektFehler {
    fn from(e: wai_storage::sqlx::Error) -> Self {
        ProjektFehler::Speicher(e.to_string())
    }
}

impl From<wai_storage::StorageError> for ProjektFehler {
    fn from(e: wai_storage::StorageError) -> Self {
        ProjektFehler::Speicher(e.to_string())
    }
}

pub type Ergebnis<T> = Result<T, ProjektFehler>;
