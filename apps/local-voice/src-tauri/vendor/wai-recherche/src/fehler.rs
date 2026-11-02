//! Fehlerarten. Meldungen enthalten bewusst keine URLs, Suchtexte oder Seiteninhalte (Protokoll ohne Inhalte).

#[derive(Debug, thiserror::Error)]
pub enum RechercheFehler {
    #[error("Suchanbieter nicht erreichbar")]
    AnbieterNichtErreichbar,
    #[error("Antwort des Suchanbieters ungueltig")]
    AntwortUngueltig,
    #[error("ungueltige URL")]
    UngueltigeUrl,
    #[error("nur http und https sind erlaubt")]
    SchemaNichtErlaubt,
    #[error("Zieladresse nicht erlaubt (privat, Loopback, Link-Local oder reserviert)")]
    AdresseNichtErlaubt,
    #[error("Port nicht erlaubt")]
    PortNichtErlaubt,
    #[error("Suchtext zu lang")]
    AnfrageZuLang,
    #[error("DNS-Aufloesung fehlgeschlagen")]
    DnsFehler,
    #[error("zu viele Weiterleitungen")]
    ZuVieleWeiterleitungen,
    #[error("Antwort ueberschreitet das Groessenlimit")]
    ZuGross,
    #[error("Zeitlimit ueberschritten")]
    Zeitueberschreitung,
    #[error("Inhaltstyp nicht erlaubt")]
    InhaltstypNichtErlaubt,
    #[error("robots.txt verbietet den Abruf")]
    RobotsVerbietet,
    #[error("HTTP-Status {0}")]
    HttpStatus(u16),
    #[error("Netzwerkfehler beim Abruf")]
    Netzwerk,
    #[error("Budget erreicht: {0}")]
    BudgetErreicht(&'static str),
    #[error("Modell nicht verfuegbar oder Aufruf abgelehnt")]
    ModellFehler,
    #[error("Modellantwort ungueltig")]
    ModellAntwortUngueltig,
}
