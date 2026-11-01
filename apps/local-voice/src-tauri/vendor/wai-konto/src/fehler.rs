use std::io;

/// Fehler des Kontospeichers; enthält nie Profilinhalte.
#[derive(Debug, thiserror::Error)]
pub enum KontoFehler {
    #[error("Ein-/Ausgabefehler: {0}")]
    Io(#[from] io::Error),
    /// Nur Ort und Kategorie; der Text von serde kann Dateiinhalt enthalten.
    #[error("profile.json ist beschädigt (Zeile {zeile}, Spalte {spalte}, {kategorie})")]
    Format {
        zeile: usize,
        spalte: usize,
        kategorie: &'static str,
    },
    #[error("Dateiversion {0} wird nicht unterstützt")]
    Version(u32),
    #[error("Kein Datenordner ermittelbar (Umgebungsvariable fehlt)")]
    KeinDatenordner,
    #[error("Zufallsquelle nicht verfügbar")]
    Zufall,
    #[error("Ungültiger Wert für Feld '{feld}': {grund}")]
    Ungueltig {
        feld: &'static str,
        grund: &'static str,
    },
    #[error("Feld '{0}' stammt vom Identitätsanbieter und ist schreibgeschützt")]
    Schreibgeschuetzt(String),
    #[error("Profil '{0}' nicht gefunden")]
    NichtGefunden(String),
    #[error("Profil nicht entfernbar: {0}")]
    NichtEntfernbar(&'static str),
    #[error("Ungültiges Profilbild: {0}")]
    Bild(&'static str),
}

impl From<serde_json::Error> for KontoFehler {
    fn from(e: serde_json::Error) -> Self {
        use serde_json::error::Category;
        KontoFehler::Format {
            zeile: e.line(),
            spalte: e.column(),
            kategorie: match e.classify() {
                Category::Io => "Ein-/Ausgabe",
                Category::Syntax => "Syntax",
                Category::Data => "Daten",
                Category::Eof => "unerwartetes Ende",
            },
        }
    }
}
