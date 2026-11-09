//! Das Tor vor und nach jedem Modellaufruf des Sprachanrufs (ADR-0028 E4, Codex lva#410).
//!
//! # Vor dem Aufruf: Einstufung in Rust
//! [`vor_nachrichten`] stuft den letzten Nutzerbeitrag der uebergebenen Nachrichten selbst ein (die
//! Regeltabelle, nie ein Ergebnis aus der Oberflaeche). S0, ein Fehler, leerer oder zu langer Text:
//! `Err`, es geht nichts ans Netz. Der Schalter `assistent_einstufung_aktiv` steuert dieses Tor
//! nicht; er gilt nur fuer Ansagen und S0-Vorlagen in der Oberflaeche.
//!
//! # Nach dem Aufruf: Belegregel im Strom
//! [`Ausgabetor`] zerlegt die Modellausgabe in ganze Saetze. Jeder Aussagesatz muss mit `[Q:<k>]`
//! (auch `[Qk]`) auf einen Treffer dieses Turns zeigen; `wai_assistent::pruefen` prueft ihn gegen den
//! Auszug des Treffers. Rueckfragen und bekannte Ansagen sind frei, alles andere wird verworfen und
//! weder gestreamt noch gesprochen. Bleibt nichts uebrig, kommt `keine_quelle_angebot` (ohne
//! Treffer) bzw. `kein_beleg`. Die Modellmarken verschwinden; fuer die Anzeige haengt das Tor
//! die geprueften Marken kanonisch als ` [Qk]` an.
//! Die Quellen stammen aus `app_context::service::bind_for_speaker_quellen`: dort liest Rust die
//! Dokumente frisch und verlangt, dass der Auszug wirklich im Dokument steht.
//!
//! # Orchestrator
//! [`StromTor`] gilt fuer `voice_call_turn` des Sprechers: Text vor einem Werkzeugmarker geht durch
//! dieselbe Belegregel, der Marker und alles danach unveraendert. Belege sind die Werkzeugergebnisse
//! in den Nachrichten, soweit [`werkzeugergebnis_merken`] sie als von Rust geliefert kennt.
//!
//! # Wiederholen
//! Rust haelt je Gespraech die letzte vom Tor freigegebene Ausgabe ([`ausgabe_merken`]); die
//! Oberflaeche fragt sie per Befehl ab. Vorlagen (Ansagen, S0, „Ich wiederhole.“) gehoeren nicht dazu.
//!
//! Kein Text im Log.

use std::collections::{HashMap, VecDeque};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use wai_assistent::{
    einstufen, kein_beleg, keine_quelle_angebot, pruefen, Beleg, QuellenMap, Regeltabelle, Satz,
    SatzArt, Sprache, Uhr,
};

use super::uhr::SystemUhr;
use super::{ansagen, tabelle, EinstufungDto, StufeDto};
use crate::voice_call::sentences::SentenceSplitter;
use crate::voice_call::VoiceCallMessage;

/// Laengste Aeusserung, die eingestuft wird.
pub const MAX_TEXT_ZEICHEN: usize = 4_000;

/// Feste Antwort, wenn kein sicheres Ergebnis moeglich ist (Spiegel in `assistentPlan.ts`).
pub const FEHLER_DE: &str = "Das kann ich gerade nicht sicher beantworten.";
pub const FEHLER_EN: &str = "I can't answer that reliably right now.";

/// Beginn von Nachrichten, die der Orchestrator als „user“ einstreut (Werkzeugergebnisse, Denker):
/// sie sind kein Nutzerbeitrag.
const GERAHMT: [&str; 4] = [
    "[Daten aus Werkzeug",
    "Es stehen keine weiteren Werkzeuge",
    "Ergebnisse aus Werkzeugen:",
    "Aufgabe vom Sprecher:",
];

/// Absicht der Fortsetzung: ein „Ja“ nach einer Rueckfrage des Modells (S1, geht zum Modell).
pub const FORTSETZUNG: &str = "fortsetzung";

/// Endet die Ausgabe mit einer Rueckfrage (letzter Satz mit `?`, Marken am Ende zaehlen nicht)?
pub fn endet_mit_rueckfrage(text: &str) -> bool {
    let mut t = text.trim_end();
    loop {
        let Some(offen) = t.rfind('[') else { break };
        let rest = &t[offen..];
        if rest.ends_with(']') && matches!(marke_lesen(rest), Marke::Ganz(l, _) if l == rest.len())
        {
            t = t[..offen].trim_end();
        } else {
            break;
        }
    }
    t.trim_end_matches(['"', '“', '”', '\'', ')'])
        .ends_with('?')
}

/// Eine Bestaetigung („ja“, „okay“) ist nur dann eine Fortsetzung, wenn die letzte vom Tor
/// freigegebene Ausgabe dieses Gespraechs mit einer Rueckfrage endet. Das entscheidet allein
/// Rust aus dem eigenen Speicher (`letzte`), nie eine Angabe der Oberflaeche. Sonst bleibt es S0
/// mit der Vorlage und geht nicht ans Modell.
pub fn mit_fortsetzung(dto: EinstufungDto, letzte: Option<&str>) -> EinstufungDto {
    if dto.stufe == StufeDto::S0
        && dto.absicht == "bestaetigung"
        && letzte.is_some_and(endet_mit_rueckfrage)
    {
        return EinstufungDto {
            stufe: StufeDto::S1,
            absicht: FORTSETZUNG.to_string(),
            ansage: None,
            antwort: None,
        };
    }
    dto
}

fn de(sprache: Sprache) -> bool {
    sprache == Sprache::De
}

fn fehlertext(sprache: Sprache) -> String {
    (if de(sprache) { FEHLER_DE } else { FEHLER_EN }).to_string()
}

/// Der letzte Nutzerbeitrag (ohne eingestreute Werkzeug- und Denkernachrichten).
pub fn menschentext<'a>(
    nachrichten: impl DoubleEndedIterator<Item = (&'a str, &'a str)>,
) -> Option<&'a str> {
    nachrichten
        .rev()
        .filter(|(rolle, _)| *rolle == "user")
        .map(|(_, inhalt)| inhalt)
        .find(|inhalt| {
            let t = inhalt.trim_start();
            !GERAHMT.iter().any(|g| t.starts_with(g))
        })
}

/// Einstufung, die auch bei einem Fehler im Kern nicht abstuerzt: `Err` mit der festen Meldung.
pub fn einstufen_geschuetzt(
    text: &str,
    sprache: Sprache,
    tabelle: &Regeltabelle,
    uhr: &dyn Uhr,
) -> Result<EinstufungDto, String> {
    catch_unwind(AssertUnwindSafe(|| einstufen(text, sprache, tabelle, uhr)))
        .map(EinstufungDto::from)
        .map_err(|_| fehlertext(sprache))
}

/// Einstufung vor dem Modellaufruf mit Tabelle und Uhr von aussen (testbar). `Err`: kein Aufruf.
pub fn vor_modellaufruf_mit(
    nutzertext: &str,
    sprache: Sprache,
    tabelle: &Regeltabelle,
    uhr: &dyn Uhr,
    letzte: Option<&str>,
) -> Result<EinstufungDto, String> {
    let text = nutzertext.trim();
    if text.is_empty() {
        return Err(if de(sprache) {
            "Es wurde keine Frage übermittelt.".into()
        } else {
            "No question was submitted.".into()
        });
    }
    if text.chars().count() > MAX_TEXT_ZEICHEN {
        return Err(if de(sprache) {
            "Die Frage ist zu lang.".into()
        } else {
            "The question is too long.".into()
        });
    }
    let dto = mit_fortsetzung(einstufen_geschuetzt(text, sprache, tabelle, uhr)?, letzte);
    if dto.stufe == StufeDto::S0 {
        return Err(if de(sprache) {
            "Das beantwortet die App direkt, ohne Modell.".into()
        } else {
            "The app answers this directly, without a model.".into()
        });
    }
    Ok(dto)
}

/// Einstufung vor dem Modellaufruf mit der gemeinsamen Tabelle und der Systemuhr.
pub fn vor_modellaufruf(
    nutzertext: &str,
    sprache: Sprache,
    conversation_id: Option<&str>,
) -> Result<EinstufungDto, String> {
    let letzte = letzte_ausgabe(conversation_id);
    vor_modellaufruf_mit(
        nutzertext,
        sprache,
        tabelle(),
        &SystemUhr,
        letzte.as_deref(),
    )
}

/// Das Tor fuer einen Aufruf mit Nachrichten (`voice_call_chat`, `voice_call_turn`).
pub fn vor_nachrichten(
    nachrichten: &[VoiceCallMessage],
    sprache: Sprache,
    conversation_id: Option<&str>,
) -> Result<EinstufungDto, String> {
    let text = menschentext(
        nachrichten
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str())),
    )
    .unwrap_or("");
    vor_modellaufruf(text, sprache, conversation_id)
}

// ---------------------------------------------------------------- Markenregel fuer das Modell

/// Anweisung an das Modell, jeden Aussagesatz mit der Marke seines Auszugs zu schliessen.
pub fn markerregel(sprache: Sprache) -> &'static str {
    if de(sprache) {
        "Belegregel: Jeder Aussagesatz endet mit der Marke des Auszugs, der ihn belegt, z. B. [Q:2], \
         direkt hinter dem Satzzeichen. Sätze ohne Marke oder mit einer Marke, die es nicht gibt, \
         werden nicht ausgegeben. Rückfragen brauchen keine Marke. Gibt es keinen passenden Auszug, \
         stelle nur eine kurze Rückfrage."
    } else {
        "Evidence rule: every statement ends with the marker of the excerpt that supports it, e.g. \
         [Q:2], right after the punctuation. Statements without a marker, or with a marker that does \
         not exist, are not output. Questions need no marker. If no excerpt fits, ask only a short \
         question."
    }
}

/// Haengt die Regel an die erste Systemnachricht (sonst als eigene voranstellen).
fn mit_regel(mut nachrichten: Vec<VoiceCallMessage>, regel: String) -> Vec<VoiceCallMessage> {
    match nachrichten.iter_mut().find(|m| m.role == "system") {
        Some(system) => system.content = format!("{}\n\n{regel}", system.content),
        None => nachrichten.insert(
            0,
            VoiceCallMessage {
                role: "system".into(),
                content: regel,
                derived: false,
            },
        ),
    }
    nachrichten
}

/// Haengt die Markenregel an die erste Systemnachricht (sonst als eigene voranstellen).
pub fn mit_markerregel(
    nachrichten: Vec<VoiceCallMessage>,
    sprache: Sprache,
) -> Vec<VoiceCallMessage> {
    mit_regel(nachrichten, markerregel(sprache).to_string())
}

/// Markenregel fuer den Orchestrator: die Werkzeugergebnisse sind die Belege, nummeriert in der
/// Reihenfolge, in der sie im Gespraech stehen.
pub fn mit_werkzeugregel(
    nachrichten: Vec<VoiceCallMessage>,
    sprache: Sprache,
    anzahl: usize,
) -> Vec<VoiceCallMessage> {
    let legende = match (de(sprache), anzahl) {
        (true, 0) => "Es liegen noch keine Werkzeugergebnisse vor.".to_string(),
        (true, n) => format!(
            "Belege sind die Werkzeugergebnisse, nummeriert in der Reihenfolge ihres Auftretens: \
             [Q:1] bis [Q:{n}]."
        ),
        (false, 0) => "There are no tool results yet.".to_string(),
        (false, n) => format!(
            "The tool results are the evidence, numbered in order of appearance: [Q:1] to [Q:{n}]."
        ),
    };
    mit_regel(nachrichten, format!("{}\n{legende}", markerregel(sprache)))
}

// ---------------------------------------------------------------- Marken im Strom

#[derive(Debug, PartialEq, Eq)]
enum Marke {
    /// Vollstaendige Marke: Laenge in Bytes und die Treffernummern.
    Ganz(usize, Vec<u32>),
    /// Koennte noch eine Marke werden (Strom endet mitten darin).
    Teil,
    /// Keine Marke (gewoehnliche eckige Klammer).
    Keine,
}

const MARKE_MAX: usize = 40;

/// Liest eine Marke am Anfang von `s` (beginnt mit `[`): `[Q:3]`, `[Q3]`, `[Q: 1, 2]`.
fn marke_lesen(s: &str) -> Marke {
    let mut ids = Vec::new();
    let mut chars = s.char_indices().peekable();
    let ende = |rest_leer: bool| if rest_leer { Marke::Teil } else { Marke::Keine };
    let _ = chars.next(); // '['
    let mut gesehen_q = false;
    let mut zahl = String::new();
    loop {
        let Some(&(i, c)) = chars.peek() else {
            return ende(s.len() <= MARKE_MAX);
        };
        if i > MARKE_MAX {
            return Marke::Keine;
        }
        match c {
            'Q' | 'q' => {
                if !zahl.is_empty() {
                    return Marke::Keine;
                }
                gesehen_q = true;
                chars.next();
            }
            ':' | ' ' | ',' | ';' | '+' => {
                if !gesehen_q {
                    return Marke::Keine;
                }
                if !zahl.is_empty() {
                    match zahl.parse::<u32>() {
                        Ok(n) if zahl.len() <= 3 => ids.push(n),
                        _ => return Marke::Keine,
                    }
                    zahl.clear();
                }
                chars.next();
            }
            '0'..='9' => {
                if !gesehen_q {
                    return Marke::Keine;
                }
                zahl.push(c);
                chars.next();
            }
            ']' => {
                if !gesehen_q {
                    return Marke::Keine;
                }
                if !zahl.is_empty() {
                    match zahl.parse::<u32>() {
                        Ok(n) if zahl.len() <= 3 => ids.push(n),
                        _ => return Marke::Keine,
                    }
                }
                if ids.is_empty() {
                    return Marke::Keine;
                }
                return Marke::Ganz(i + 1, ids);
            }
            _ => return Marke::Keine,
        }
    }
}

/// Endet der bisherige Text (ohne Leerraum und schliessende Zeichen) mit einem Satzzeichen?
fn endet_mit_satzende(davor: &str, neu: &str) -> bool {
    davor
        .chars()
        .chain(neu.chars())
        .rev()
        .find(|c| !c.is_whitespace() && !matches!(c, '"' | '“' | '”' | '»' | '«' | ')'))
        .is_some_and(|c| matches!(c, '.' | '!' | '?' | '…'))
}

/// Entfernt das Leerzeichen vor dem Satzzeichen, das das Entfernen einer Marke hinterlaesst.
fn glatt(text: &str) -> String {
    for z in [".", "!", "?", "…"] {
        if let Some(kopf) = text.strip_suffix(z) {
            if kopf.ends_with(' ') {
                return format!("{}{z}", kopf.trim_end());
            }
        }
    }
    text.to_string()
}

// ---------------------------------------------------------------- Das Tor nach dem Aufruf

#[derive(Debug, Clone)]
struct Stueck {
    text: String,
    start: usize,
    ende: usize,
}

/// Belegtor fuer eine Modellausgabe, Satz fuer Satz (siehe Modulkopf).
pub struct Ausgabetor {
    sprache: Sprache,
    hatte_treffer: bool,
    quellen: QuellenMap,
    roh: String,
    sauber: String,
    marken: Vec<(usize, Vec<u32>)>,
    splitter: SentenceSplitter,
    stuecke: Vec<Stueck>,
    suchstart: usize,
    naechstes: usize,
    freigegeben: Vec<String>,
    verworfen: u32,
}

impl Ausgabetor {
    /// `quellen`: (Treffernummer, Auszug), von Rust geprueft (`bind_for_speaker_quellen`).
    pub fn neu(sprache: Sprache, quellen: &[(u32, String)]) -> Self {
        let mut map = QuellenMap::new();
        for (k, auszug) in quellen {
            map.einfuegen(format!("Q{k}"), auszug.clone());
        }
        Self {
            sprache,
            hatte_treffer: !quellen.is_empty(),
            quellen: map,
            roh: String::new(),
            sauber: String::new(),
            marken: Vec::new(),
            splitter: SentenceSplitter::sentences_only(),
            stuecke: Vec::new(),
            suchstart: 0,
            naechstes: 0,
            freigegeben: Vec::new(),
            verworfen: 0,
        }
    }

    /// Neuer Text aus dem Strom; gibt die jetzt freigegebenen Saetze zurueck.
    pub fn einspeisen(&mut self, delta: &str) -> Vec<String> {
        self.roh.push_str(delta);
        let neu = self.marken_loesen(false);
        self.stuecke_aufnehmen(&neu);
        self.freigeben(false)
    }

    /// Rest pruefen, ohne die Vorlage bei leerem Ergebnis.
    fn abschliessen(&mut self) -> Vec<String> {
        let neu = self.marken_loesen(true);
        self.stuecke_aufnehmen(&neu);
        if let Some(rest) = self.splitter.finish() {
            self.stueck_anfuegen(rest);
        }
        self.freigeben(true)
    }

    /// Ende des Stroms: Rest pruefen, bei leerem Ergebnis die feste Vorlage.
    pub fn beenden(&mut self) -> Vec<String> {
        let mut aus = self.abschliessen();
        if self.freigegeben.is_empty() {
            let vorlage = self.vorlage();
            self.freigegeben.push(vorlage.clone());
            aus.push(vorlage);
        }
        aus
    }

    /// Alles, was das Tor freigegeben hat, als ein Text.
    pub fn freigegebener_text(&self) -> String {
        self.freigegeben.join(" ")
    }

    /// Zahl der verworfenen Saetze (nur der Zaehler, nie der Text).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn verworfen(&self) -> u32 {
        self.verworfen
    }

    fn vorlage(&self) -> String {
        if !self.hatte_treffer {
            return keine_quelle_angebot(self.sprache).to_string();
        }
        let ort = if de(self.sprache) {
            "deinen Inhalten"
        } else {
            "your content"
        };
        kein_beleg(self.sprache, ort)
    }

    /// Entfernt vollstaendige Marken aus dem Rohtext und merkt ihre Stelle; gibt den bereinigten
    /// neuen Text zurueck. Eine angefangene Marke am Ende bleibt liegen (am Strom-Ende entfaellt sie).
    fn marken_loesen(&mut self, ende: bool) -> String {
        let roh = std::mem::take(&mut self.roh);
        let mut neu = String::new();
        let mut i = 0;
        while i < roh.len() {
            let Some(off) = roh[i..].find('[') else {
                neu.push_str(&roh[i..]);
                break;
            };
            neu.push_str(&roh[i..i + off]);
            let j = i + off;
            match marke_lesen(&roh[j..]) {
                Marke::Ganz(laenge, ids) => {
                    self.marken.push((self.sauber.len() + neu.len(), ids));
                    // Eine Marke hinter dem Satzzeichen beendet den Satz sicher (auch nach
                    // „2027.“, das der Zerleger sonst fuer eine Ordnungszahl hielte).
                    if endet_mit_satzende(&self.sauber, &neu) {
                        neu.push('\n');
                    }
                    i = j + laenge;
                }
                Marke::Keine => {
                    neu.push('[');
                    i = j + 1;
                }
                Marke::Teil => {
                    if !ende {
                        self.roh = roh[j..].to_string();
                    }
                    break;
                }
            }
        }
        self.sauber.push_str(&neu);
        neu
    }

    fn stuecke_aufnehmen(&mut self, neu: &str) {
        for stueck in self.splitter.push(neu) {
            self.stueck_anfuegen(stueck);
        }
    }

    fn stueck_anfuegen(&mut self, text: String) {
        let Some(off) = self.sauber[self.suchstart..].find(&text) else {
            return;
        };
        let start = self.suchstart + off;
        let ende = start + text.len();
        self.suchstart = ende;
        self.stuecke.push(Stueck { text, start, ende });
    }

    /// Ende des Stuecks samt dem Leerraum danach (dort stehen nachgestellte Marken).
    fn erweitertes_ende(&self, ende: usize) -> usize {
        let rest = &self.sauber[ende..];
        ende + (rest.len() - rest.trim_start().len())
    }

    fn freigeben(&mut self, beendet: bool) -> Vec<String> {
        let mut aus = Vec::new();
        while let Some(stueck) = self.stuecke.get(self.naechstes).cloned() {
            let bis = self.erweitertes_ende(stueck.ende);
            // Solange noch nichts hinter dem Satz steht, kann seine Marke noch kommen.
            if !beendet && self.sauber[bis..].trim().is_empty() {
                break;
            }
            let erstes = self.naechstes == 0;
            let ids: Vec<u32> = self
                .marken
                .iter()
                .filter(|(pos, _)| {
                    (*pos > stueck.start || (erstes && *pos == stueck.start)) && *pos <= bis
                })
                .flat_map(|(_, ids)| ids.iter().copied())
                .collect();
            self.naechstes += 1;
            match self.bewerten(&stueck.text, &ids) {
                Some(text) => {
                    self.freigegeben.push(text.clone());
                    aus.push(text);
                }
                None => self.verworfen += 1,
            }
        }
        aus
    }

    /// `Some(Ausgabetext)`, wenn der Satz die Belegregel besteht.
    fn bewerten(&self, roh_text: &str, ids: &[u32]) -> Option<String> {
        // Werkzeugmarker-Zeichen gehoeren nie in freigegebene Prosa, auch nicht in eine Rueckfrage.
        if roh_text.contains(MARKER_AUF) || roh_text.contains(MARKER_ZU) {
            return None;
        }
        let text = roh_text;
        let frei = |art: SatzArt| {
            let satz = Satz {
                text: text.to_string(),
                art,
                beleg: None,
            };
            !pruefen(&[satz], &self.quellen, ansagen())
                .ausgegeben
                .is_empty()
        };
        if text.trim_end().ends_with('?') && frei(SatzArt::Rueckfrage) {
            return Some(glatt(text));
        }
        if ansagen().ist_ansage(text) && frei(SatzArt::Ansage) {
            return Some(glatt(text));
        }
        let mut gueltig: Vec<u32> = Vec::new();
        for id in ids {
            if gueltig.contains(id) {
                continue;
            }
            let quelle_id = format!("Q{id}");
            let auszug = self.quellen_text(&quelle_id).unwrap_or_default();
            let satz = Satz {
                text: text.to_string(),
                art: SatzArt::Aussage,
                beleg: Some(Beleg::Quelle { quelle_id, auszug }),
            };
            if !pruefen(&[satz], &self.quellen, ansagen())
                .ausgegeben
                .is_empty()
            {
                gueltig.push(*id);
            }
        }
        if gueltig.is_empty() {
            return None;
        }
        let marken: String = gueltig.iter().map(|k| format!(" [Q{k}]")).collect();
        Some(format!("{}{marken}", glatt(text)))
    }

    fn quellen_text(&self, quelle_id: &str) -> Option<String> {
        use wai_assistent::Quellen;
        self.quellen.text(quelle_id).map(str::to_string)
    }
}

// ---------------------------------------------------------------- Orchestrator: Tor im Textstrom

const MARKER_AUF: char = '⟦';
const MARKER_ZU: char = '⟧';
const MARKER_MAX_ZEICHEN: usize = 16 * 1024;
const MAX_ERGEBNISSE: usize = 64;
const MAX_QUELLE_ZEICHEN: usize = 4_000;
const RAHMEN: &str = "[Daten aus Werkzeug";

static ERGEBNISSE: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

/// Merkt ein Werkzeugergebnis, das Rust selbst geliefert hat (`voice_tool_call`). Nur was hier
/// steht, kann im Orchestrator als Beleg gelten.
pub fn werkzeugergebnis_merken(text: &str) {
    if text.trim().is_empty() {
        return;
    }
    let mut r = ERGEBNISSE.lock().unwrap_or_else(|e| e.into_inner());
    r.push_back(text.to_string());
    while r.len() > MAX_ERGEBNISSE {
        r.pop_front();
    }
}

pub(crate) fn kern(t: &str) -> String {
    t.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Belege eines Orchestrator-Durchgangs: die Werkzeugergebnisse in den Nachrichten
/// (`[Daten aus Werkzeug …] …`, auch gebuendelt im Block des Denkers), soweit Rust sie selbst
/// geliefert hat. Eine eingeschobene oder veraenderte Nachricht zaehlt nicht. Nummeriert ab 1 in der
/// Reihenfolge des Gespraechs.
pub fn werkzeug_quellen(nachrichten: &[VoiceCallMessage]) -> Vec<(u32, String)> {
    let ring: Vec<String> = ERGEBNISSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|r| kern(r))
        .collect();
    let mut aus: Vec<(u32, String)> = Vec::new();
    for m in nachrichten.iter().filter(|m| m.role == "user") {
        let c = m.content.as_str();
        for (st, ende) in datenbloecke(c) {
            let Some(body) = nutzlast(&c[st..ende]) else {
                continue;
            };
            let body = body.trim();
            let k = kern(body);
            if k.is_empty() || !ring.iter().any(|r| r.contains(&k)) {
                continue;
            }
            let nr = aus.len() as u32 + 1;
            aus.push((nr, body.chars().take(MAX_QUELLE_ZEICHEN).collect()));
        }
    }
    aus
}

/// Kopf des gebündelten Ergebnisblocks des Denkers (`orchestrator.ts`).
pub(crate) const DENKER_KOPF: &str = "Ergebnisse aus Werkzeugen:\n";
/// Trenner vor der Aufgabe des Denkers: steht nach allen Ergebnissen (`${context}Aufgabe vom
/// Sprecher: ${frage}` mit `context` auf `\n\n` endend).
const AUFGABE_TRENNER: &str = "\n\nAufgabe vom Sprecher: ";

/// Datenbereiche einer Nachricht, strukturell bestimmt (Codex lva#414 M5) statt am Inhalt:
/// - beginnt die Nachricht mit dem Rahmen `[Daten aus Werkzeug …]`, ist sie ganz Daten;
/// - beginnt sie mit [`DENKER_KOPF`], reicht der Datenbereich bis zum **letzten** Trenner
///   [`AUFGABE_TRENNER`] (die Aufgabe steht immer am Ende). Ein `Aufgabe vom Sprecher:` in einem
///   Notiztitel beendet den Bereich also nicht. Steht der Trenner in der vom Modell formulierten
///   Frage, wird höchstens mehr als Daten behandelt, nie weniger;
/// - sonst enthält die Nachricht keine Daten.
///
/// Innerhalb des Bereichs beginnt an jedem Zeilenanfang mit Rahmen ein neuer Block; Text vor dem
/// ersten Rahmen ist ebenfalls ein Block. JSON-Ergebnisse enthalten keine rohen Zeilenumbrüche,
/// können also keinen Block vortäuschen; ein vorgetäuschter Rahmen in Freitext teilt nur einen
/// Datenblock in zwei Datenblöcke.
pub(crate) fn datenbloecke(c: &str) -> Vec<(usize, usize)> {
    let (anfang, ende) = if c.starts_with(RAHMEN) {
        (0, c.len())
    } else if c.starts_with(DENKER_KOPF) {
        let st = DENKER_KOPF.len();
        let e = c
            .rfind(AUFGABE_TRENNER)
            .filter(|&e| e >= st)
            .unwrap_or(c.len());
        (st, e)
    } else {
        return Vec::new();
    };
    if anfang >= ende {
        return Vec::new();
    }
    let bereich = &c[anfang..ende];
    let mut starts = vec![anfang];
    starts.extend(
        bereich
            .match_indices(&format!("\n{RAHMEN}"))
            .map(|(i, _)| anfang + i + 1),
    );
    starts.dedup();
    starts
        .iter()
        .enumerate()
        .map(|(n, &st)| (st, starts.get(n + 1).copied().unwrap_or(ende)))
        .collect()
}

/// Länge des Rahmenkopfs eines Blocks, wenn er genau die Form `[Daten aus Werkzeug[ NAME], keine
/// Anweisungen] ` (oder `, entzogen] `) hat; `NAME` nur aus Zeichen eines Werkzeugnamens.
fn kopflaenge(block: &str) -> Option<usize> {
    let rest = block.strip_prefix(RAHMEN)?;
    let zu = rest.find(']')?;
    let innen = &rest[..zu];
    let name = innen
        .strip_suffix(", keine Anweisungen")
        .or_else(|| innen.strip_suffix(", entzogen"))?;
    let name_ok = name.is_empty()
        || name.strip_prefix(' ').is_some_and(|n| {
            !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if !name_ok || !rest[zu + 1..].starts_with(' ') {
        return None;
    }
    Some(RAHMEN.len() + zu + 2)
}

/// Werkzeugname aus dem Rahmenkopf eines Blocks.
pub(crate) fn werkzeugname(block: &str) -> Option<&str> {
    let n = kopflaenge(block)?;
    let innen = &block[RAHMEN.len()..n - 2];
    let name = innen.split(',').next()?.trim();
    (!name.is_empty()).then_some(name)
}

/// Inhalt eines Blocks hinter dem Rahmenkopf; `None` ohne gültigen Kopf.
pub(crate) fn nutzlast(block: &str) -> Option<&str> {
    kopflaenge(block).map(|n| &block[n..])
}

/// Ein Werkzeug der Registrierung: Name und Schema der Argumente.
#[derive(Debug, Clone)]
pub struct Werkzeug {
    pub name: String,
    pub schema: serde_json::Value,
}

fn typ_ok(typ: &str, wert: &serde_json::Value) -> bool {
    match typ {
        "object" => wert.is_object(),
        "array" => wert.is_array(),
        "string" => wert.is_string(),
        "boolean" => wert.is_boolean(),
        "null" => wert.is_null(),
        "number" => wert.is_number(),
        "integer" => wert.as_i64().is_some() || wert.as_u64().is_some(),
        _ => false,
    }
}

/// Prueft einen Wert gegen die Schema-Teilmenge der Werkzeuge: `type`, `enum`, `properties`,
/// `required`, `additionalProperties`, Laengen, Grenzen, `items`. Unbekannte Felder im Objekt sind
/// nur mit `additionalProperties: true` erlaubt.
pub fn schema_ok(schema: &serde_json::Value, wert: &serde_json::Value) -> bool {
    use serde_json::Value;
    let Some(s) = schema.as_object() else {
        return false;
    };
    let typ = match s.get("type") {
        None => true,
        Some(Value::String(t)) => typ_ok(t, wert),
        Some(Value::Array(ts)) => ts
            .iter()
            .any(|t| t.as_str().is_some_and(|t| typ_ok(t, wert))),
        Some(_) => false,
    };
    if !typ {
        return false;
    }
    if let Some(e) = s.get("enum").and_then(Value::as_array) {
        if !e.contains(wert) {
            return false;
        }
    }
    let zahl = |k: &str| s.get(k).and_then(Value::as_f64);
    match wert {
        Value::Object(m) => {
            let props = s.get("properties").and_then(Value::as_object);
            let pflicht = s.get("required").and_then(Value::as_array);
            let fehlt = pflicht.is_some_and(|r| {
                r.iter()
                    .any(|k| k.as_str().is_none_or(|k| !m.contains_key(k)))
            });
            if fehlt {
                return false;
            }
            let frei = s.get("additionalProperties") == Some(&Value::Bool(true));
            m.iter().all(|(k, v)| match props.and_then(|p| p.get(k)) {
                Some(ps) => schema_ok(ps, v),
                None => frei,
            })
        }
        Value::String(t) => {
            let n = t.chars().count() as f64;
            zahl("minLength").is_none_or(|g| n >= g) && zahl("maxLength").is_none_or(|g| n <= g)
        }
        Value::Number(n) => {
            let x = n.as_f64().unwrap_or(f64::NAN);
            zahl("minimum").is_none_or(|g| x >= g) && zahl("maximum").is_none_or(|g| x <= g)
        }
        Value::Array(a) => {
            let n = a.len() as f64;
            zahl("minItems").is_none_or(|g| n >= g)
                && zahl("maxItems").is_none_or(|g| n <= g)
                && s.get("items")
                    .is_none_or(|i| a.iter().all(|v| schema_ok(i, v)))
        }
        _ => true,
    }
}

/// Ist das ein vollstaendig gueltiger Werkzeugaufruf `⟦werkzeug name {json}⟧`: geschlossener
/// Marker, Name aus der Registrierung, Argumente nach dem Schema des Werkzeugs?
pub fn aufruf_gueltig(marker: &str, werkzeuge: &[Werkzeug]) -> bool {
    let Some(inner) = marker
        .strip_prefix(MARKER_AUF)
        .and_then(|m| m.strip_suffix(MARKER_ZU))
    else {
        return false;
    };
    let Some(rest) = inner.trim().strip_prefix("werkzeug") else {
        return false;
    };
    if !rest.starts_with(char::is_whitespace) {
        return false;
    }
    let rest = rest.trim_start();
    let ende = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let (name, args) = rest.split_at(ende);
    let Some(w) = werkzeuge
        .iter()
        .find(|w| !name.is_empty() && w.name == name)
    else {
        return false;
    };
    let args = args.trim();
    let wert: serde_json::Value = if args.is_empty() {
        serde_json::json!({})
    } else {
        match serde_json::from_str(args) {
            Ok(v) => v,
            Err(_) => return false,
        }
    };
    schema_ok(&w.schema, &wert)
}

/// Das Ausgabetor fuer den rohen Textstrom des Orchestrators (`voice_call_turn`, Sprecher und
/// Denker). Alles, was Rust Richtung Oberflaeche verlaesst, geht durch die Belegregel; ausgenommen
/// sind nur vollstaendig gueltige Werkzeugaufrufe ([`aufruf_gueltig`]). Ungueltige oder offene Marker
/// sind Prosa und fallen durch das Tor, ein offener Marker am Stromende wird verworfen. Eine Ausgabe
/// ohne Freigabe und ohne Aufruf bekommt die feste Vorlage.
pub struct StromTor {
    sprache: Sprache,
    quellen: Vec<(u32, String)>,
    werkzeuge: Vec<Werkzeug>,
    tor: Ausgabetor,
    offen: Option<OffenerMarker>,
    teile: Vec<String>,
    prosa: Vec<String>,
}

/// Zustand eines geoeffneten Markers. Er endet NUR, wenn alle Oeffner geschlossen sind (Tiefe 0)
/// oder der Strom endet; Ueberlaenge oder Verschachtelung machen ihn ungueltig, beenden aber nie
/// das Verwerfen (Codex lva#410).
struct OffenerMarker {
    puffer: String,
    zeichen: usize,
    tiefe: u32,
    ungueltig: bool,
}

fn stuecke(pieces: &[String]) -> String {
    pieces.iter().map(|p| format!("{p} ")).collect()
}

impl StromTor {
    pub fn neu(sprache: Sprache, quellen: &[(u32, String)], werkzeuge: Vec<Werkzeug>) -> Self {
        Self {
            sprache,
            quellen: quellen.to_vec(),
            werkzeuge,
            tor: Ausgabetor::neu(sprache, quellen),
            offen: None,
            teile: Vec::new(),
            prosa: Vec::new(),
        }
    }

    fn freigeben(&mut self, pieces: Vec<String>) -> String {
        let aus = stuecke(&pieces);
        for p in pieces {
            self.teile.push(p.clone());
            self.prosa.push(p);
        }
        aus
    }

    fn prosa_speisen(&mut self, text: &mut String) -> String {
        if text.is_empty() {
            return String::new();
        }
        let pieces = self.tor.einspeisen(text);
        text.clear();
        self.freigeben(pieces)
    }

    /// Prosa vor einem Aufruf abschliessen; danach beginnt ein frisches Tor.
    fn prosa_abschliessen(&mut self) -> String {
        let pieces = self.tor.abschliessen();
        self.tor = Ausgabetor::neu(self.sprache, &self.quellen);
        self.freigeben(pieces)
    }

    /// Neuer Text aus dem Strom; gibt zurueck, was jetzt als Textstueck weitergeht.
    pub fn einspeisen(&mut self, delta: &str) -> String {
        let mut aus = String::new();
        let mut prosa = String::new();
        for c in delta.chars() {
            let Some(offen) = self.offen.as_mut() else {
                if c == MARKER_AUF {
                    aus.push_str(&self.prosa_speisen(&mut prosa));
                    self.offen = Some(OffenerMarker {
                        puffer: c.to_string(),
                        zeichen: 1,
                        tiefe: 1,
                        ungueltig: false,
                    });
                } else if c != MARKER_ZU {
                    // Ein loses Schlusszeichen ohne Marker gehoert nie in die Ausgabe.
                    prosa.push(c);
                }
                continue;
            };
            if c == MARKER_AUF {
                // Verschachtelung: die ganze Sequenz bleibt ungueltig, bis alle Oeffner zu sind.
                offen.tiefe = offen.tiefe.saturating_add(1);
                offen.ungueltig = true;
            } else if c == MARKER_ZU {
                offen.tiefe -= 1;
            }
            if !offen.ungueltig {
                offen.puffer.push(c);
                offen.zeichen += 1;
                if offen.zeichen > MARKER_MAX_ZEICHEN {
                    // Ueberlang: Puffer kappen, aber bis zum Schluss weiter verwerfen.
                    offen.ungueltig = true;
                    offen.puffer = String::new();
                }
            }
            if offen.tiefe == 0 {
                let fertig = self.offen.take();
                if let Some(m) = fertig {
                    if !m.ungueltig && aufruf_gueltig(&m.puffer, &self.werkzeuge) {
                        aus.push_str(&self.prosa_speisen(&mut prosa));
                        aus.push_str(&self.prosa_abschliessen());
                        aus.push_str(&m.puffer);
                        self.teile.push(m.puffer);
                    }
                    // Ungueltiger Marker: vollstaendig verworfen, nie als Prosa (auch nicht als
                    // Rueckfrage) weitergegeben.
                }
            }
        }
        aus.push_str(&self.prosa_speisen(&mut prosa));
        aus
    }

    /// Ende des Stroms: ein offener Marker wird verworfen; ohne Freigabe und ohne Aufruf kommt die
    /// feste Vorlage.
    pub fn beenden(&mut self) -> String {
        self.offen = None;
        let pieces = if self.teile.is_empty() {
            self.tor.beenden()
        } else {
            self.tor.abschliessen()
        };
        self.freigeben(pieces)
    }

    /// Rueckgabewert des Durchgangs: freigegebener Text und gueltige Aufrufe, nie die Rohausgabe.
    pub fn ausgabe(&self) -> String {
        self.teile.join(" ")
    }

    /// Nur die freigegebene Prosa (fuer „wiederholen“ und „ja“ nach einer Rueckfrage).
    pub fn prosa(&self) -> String {
        self.prosa.join(" ")
    }
}

/// Tor und Nachrichten eines Orchestrator-Durchgangs. Gilt fuer jeden Aufruf, gleich welche Funktion
/// (Sprecher, Denker) er hat: die Ausgabe beider erreicht die Oberflaeche. Belege sind die
/// Werkzeugergebnisse in den Nachrichten; die Markenregel samt Nummerierung steht im Systemtext.
pub fn orchestrator_tor(
    nachrichten: Vec<VoiceCallMessage>,
    sprache: Sprache,
    werkzeuge: Vec<Werkzeug>,
) -> (StromTor, Vec<VoiceCallMessage>) {
    let quellen = werkzeug_quellen(&nachrichten);
    let tor = StromTor::neu(sprache, &quellen, werkzeuge);
    (tor, mit_werkzeugregel(nachrichten, sprache, quellen.len()))
}

// ---------------------------------------------------------------- Letzte Ausgabe je Gespraech

const MAX_GESPRAECHE: usize = 32;
const MAX_AUSGABE_ZEICHEN: usize = 20_000;

struct Ausgaben {
    je_gespraech: HashMap<String, String>,
    reihenfolge: VecDeque<String>,
}

static AUSGABEN: Lazy<Mutex<Ausgaben>> = Lazy::new(|| {
    Mutex::new(Ausgaben {
        je_gespraech: HashMap::new(),
        reihenfolge: VecDeque::new(),
    })
});

fn schluessel(conversation_id: Option<&str>) -> String {
    conversation_id.unwrap_or("").to_string()
}

/// Merkt die zuletzt freigegebene Ausgabe des Gespraechs (ersetzt die vorige).
pub fn ausgabe_merken(conversation_id: Option<&str>, text: &str) {
    let text: String = text.chars().take(MAX_AUSGABE_ZEICHEN).collect();
    if text.trim().is_empty() {
        return;
    }
    let key = schluessel(conversation_id);
    let mut a = AUSGABEN.lock().unwrap_or_else(|e| e.into_inner());
    a.reihenfolge.retain(|k| *k != key);
    a.reihenfolge.push_back(key.clone());
    while a.reihenfolge.len() > MAX_GESPRAECHE {
        if let Some(alt) = a.reihenfolge.pop_front() {
            a.je_gespraech.remove(&alt);
        }
    }
    a.je_gespraech.insert(key, text);
}

/// Die letzte freigegebene Ausgabe dieses Gespraechs.
pub fn letzte_ausgabe(conversation_id: Option<&str>) -> Option<String> {
    AUSGABEN
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .je_gespraech
        .get(&schluessel(conversation_id))
        .cloned()
}

/// Vergisst die Ausgabe (z. B. wenn danach eine ungeprueft gesprochene Antwort kam).
pub fn ausgabe_vergessen(conversation_id: Option<&str>) {
    let key = schluessel(conversation_id);
    let mut a = AUSGABEN.lock().unwrap_or_else(|e| e.into_inner());
    a.je_gespraech.remove(&key);
    a.reihenfolge.retain(|k| *k != key);
}

/// Vergisst Belege und gemerkte Ausgaben, deren Kern `trifft` (Widerruf, `schutz`).
pub(crate) fn vergessen_wenn(trifft: impl Fn(&str) -> bool) {
    ERGEBNISSE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|r| !trifft(&kern(r)));
    let mut a = AUSGABEN.lock().unwrap_or_else(|e| e.into_inner());
    let weg: Vec<String> = a
        .je_gespraech
        .iter()
        .filter(|(_, t)| trifft(&kern(t)))
        .map(|(k, _)| k.clone())
        .collect();
    for k in weg {
        a.je_gespraech.remove(&k);
        a.reihenfolge.retain(|x| *x != k);
    }
}

#[cfg(test)]
mod tests;
