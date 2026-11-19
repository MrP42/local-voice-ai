//! Feste Codes und Zähler der KI-Sprachstufe (lva#712). Es gibt hier bewusst keine Texte:
//! weder Dokumenttext noch Prompt noch Modellantwort gelangen in Fehler, Zähler oder Berichte.

use std::collections::BTreeMap;

/// Warum ein Modellvorschlag verworfen wurde. Der Bericht nennt nur diese Codes und Zähler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrundCode {
    /// Zeile nicht im Format `n|text` (auch Mehrzeiler, Leerzeilen, Fließtext).
    FormatFremd,
    /// Zeilen-ID gehört nicht zum Dokument.
    IdUnbekannt,
    /// Zeilen-ID kam mehrfach vor.
    IdDoppelt,
    /// Zeilen-ID gehört zu einem anderen Abschnitt.
    IdAbschnittsfremd,
    /// Antwort über dem Längenlimit.
    AntwortZuLang,
    /// Zeilenlänge weicht über der absoluten/relativen Grenze ab.
    LaengeAbweichung,
    /// Zahl, Datum, Einheit geändert.
    ZahlGeaendert,
    /// URL oder E-Mail-Adresse geändert.
    UrlGeaendert,
    /// Text in Anführung (Eigenname, wörtliche Rede) geändert.
    AnfuehrungGeaendert,
    /// Negation oder Modalwort geändert.
    NegationGeaendert,
    /// Platzhalter einer Maske verändert, fehlend, doppelt oder erfunden.
    MaskeBeschaedigt,
    /// Vorschlag berührt einen geschützten Bereich.
    GeschuetztBeruehrt,
    /// Ganze KI-Gruppe überschneidet sich mit einem Regelvorschlag.
    RegelKonflikt,
    /// Antwort war leer oder kein Modelltext.
    AntwortLeer,
}

impl GrundCode {
    /// Stabiler Code für Bericht und Telemetrie.
    pub fn code(self) -> &'static str {
        match self {
            Self::FormatFremd => "format_fremd",
            Self::IdUnbekannt => "id_unbekannt",
            Self::IdDoppelt => "id_doppelt",
            Self::IdAbschnittsfremd => "id_abschnittsfremd",
            Self::AntwortZuLang => "antwort_zu_lang",
            Self::LaengeAbweichung => "laenge_abweichung",
            Self::ZahlGeaendert => "zahl_geaendert",
            Self::UrlGeaendert => "url_geaendert",
            Self::AnfuehrungGeaendert => "anfuehrung_geaendert",
            Self::NegationGeaendert => "negation_geaendert",
            Self::MaskeBeschaedigt => "maske_beschaedigt",
            Self::GeschuetztBeruehrt => "geschuetzt_beruehrt",
            Self::RegelKonflikt => "regel_konflikt",
            Self::AntwortLeer => "antwort_leer",
        }
    }
}

/// Zähler je Grund-Code, ohne jeden Inhalt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Zaehler(BTreeMap<GrundCode, u64>);

impl Zaehler {
    pub fn erhoehen(&mut self, grund: GrundCode, um: u64) {
        if um > 0 {
            *self.0.entry(grund).or_insert(0) += um;
        }
    }

    pub fn get(&self, grund: GrundCode) -> u64 {
        self.0.get(&grund).copied().unwrap_or(0)
    }

    pub fn summe(&self) -> u64 {
        self.0.values().sum()
    }

    pub fn iter(&self) -> impl Iterator<Item = (GrundCode, u64)> + '_ {
        self.0.iter().map(|(g, n)| (*g, *n))
    }

    pub fn addieren(&mut self, anderer: &Zaehler) {
        for (grund, n) in anderer.iter() {
            self.erhoehen(grund, n);
        }
    }
}

/// Fehler der Sprachstufe. Nur Codes; `Display` und `Debug` tragen nie Text von außen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprachFehler {
    /// Kein lokales Modell verfügbar und kein Funktionsmodell gewählt.
    KeinLokalesModell,
    /// Das für die Funktion gewählte Modell ist nicht mehr nutzbar (kein stiller Rückfall).
    FunktionsmodellUnbrauchbar,
    /// Das Ziel beim Versand weicht vom angezeigten Ziel ab.
    ZielGewechselt,
    /// Das Ziel ließ sich nicht auflösen (lokaler Server nicht erreichbar).
    ZielAufloesung,
    /// Lokales Modell ausgefallen (Fehler, Kapazität); nie Cloud-Rückfall.
    LokalAusgefallen,
    /// Der Versandvertrag (`llm_versand`) hat gesperrt.
    VersandGesperrt,
    /// Zeitlimit des Modellaufrufs.
    Zeitlimit,
    /// Sonstiger Modellfehler ohne Text.
    ModellFehler,
    /// Abbruch durch den Nutzer.
    Abgebrochen,
    /// Das Dokument hat sich seit der Basisrevision geändert.
    BasisGeaendert,
    /// Das Dokument wurde gelöscht.
    DokumentGeloescht,
    /// Cache nicht les- oder schreibbar.
    Cache,
}

impl SprachFehler {
    pub fn code(self) -> &'static str {
        match self {
            Self::KeinLokalesModell => "kein_lokales_modell",
            Self::FunktionsmodellUnbrauchbar => "funktionsmodell_unbrauchbar",
            Self::ZielGewechselt => "ziel_gewechselt",
            Self::ZielAufloesung => "ziel_aufloesung",
            Self::LokalAusgefallen => "lokal_ausgefallen",
            Self::VersandGesperrt => "versand_gesperrt",
            Self::Zeitlimit => "zeitlimit",
            Self::ModellFehler => "modell_fehler",
            Self::Abgebrochen => "abgebrochen",
            Self::BasisGeaendert => "basis_geaendert",
            Self::DokumentGeloescht => "dokument_geloescht",
            Self::Cache => "cache",
        }
    }
}

impl std::fmt::Display for SprachFehler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sprachstufe: {}", self.code())
    }
}

impl std::error::Error for SprachFehler {}
