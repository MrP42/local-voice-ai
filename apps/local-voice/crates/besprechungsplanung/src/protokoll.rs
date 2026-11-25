//! Protokoll und Entwurf zur Planung (Konzept 3.6, P5, lva#736).
//!
//! Reine Funktionen ohne App- und Datenbankzugriff:
//!
//! - [`vorlage`]: die terminbezogene Protokollvorlage aus der Agenda (ein Abschnitt je Agendapunkt,
//!   dazu Ergebnisse, Entscheidungen, Aufgaben). Die App wandelt sie in eine Vorlage ihres
//!   Vorlagensystems um; die Grenzen dieses Systems stehen hier als Konstanten (die App prueft
//!   mit einem Test, dass beide gleich sind).
//! - [`kopf_markdown`]: der Kopf des Protokolls mit Ziel, erwartetem Ergebnis, "Ziel erreicht"
//!   ([`Erreicht`], von Hand gesetzt) und Klassifizierung; [`erreicht_ersetzen`] schreibt das Feld
//!   in ein vorhandenes Protokoll nach.
//! - [`empfaenger`], [`betreff`], [`entwurfstext`]: der Entwurf an die Teilnehmenden. Der Text
//!   entsteht allein aus dem freigegebenen Protokoll; die Funktionen kennen kein Transkript.
//! - [`text_hash`]: Pruefsumme des freigegebenen Protokolls (spaetere Aenderung = nicht mehr
//!   freigegeben).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::consent::{identitaet, person_schluessel};
use crate::invitation::{einzeilig, klassifizierungszeile};
use crate::model::Plan;

/// Hoechstzahl der Abschnitte einer Vorlage des Vorlagensystems der App.
pub const MAX_ABSCHNITTE: usize = 10;
/// Laengster Titel einer Vorlage oder eines Abschnitts (Zeichen).
pub const MAX_TITEL: usize = 60;
/// Laengste Anweisung eines Abschnitts (Zeichen).
pub const MAX_ANWEISUNG: usize = 500;
/// Laengster Kontext einer Vorlage (Zeichen).
pub const MAX_KONTEXT: usize = 1000;

/// Abschnitte, die nicht aus der Agenda stammen: Ergebnisse, Entscheidungen, Aufgaben.
const FESTE_ABSCHNITTE: usize = 3;
/// Laengstes Thema eines Agendapunkts in der Anweisung (der Rest der Anweisung bleibt erhalten).
const MAX_THEMA_IN_ANWEISUNG: usize = 120;

/// Ueberschrift des Abschnitts mit den Redeanteilen im Protokoll (`minutes_to_markdown`). Er
/// geht nicht in den Entwurf an die Teilnehmenden: Redeanteile je Person sind keine Mitteilung an
/// alle (Konzept 4, keine Leistungs- und Verhaltenskontrolle).
pub const REDEANTEILE_UEBERSCHRIFT: &str = "Sprecher & Redeanteile";

/// Beginn der Zeile "Ziel erreicht" im Kopf des Protokolls.
pub const ERREICHT_PRAEFIX: &str = "**Ziel erreicht:**";
/// Wert der Zeile, solange niemand bewertet hat.
pub const NICHT_BEWERTET: &str = "noch nicht bewertet";

/// Bewertung "Ziel erreicht", von Hand gesetzt. Ein Modell darf vorschlagen, entscheiden
/// kann nur ein Mensch; ohne Eintrag gilt das Ziel als nicht bewertet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(rename_all = "snake_case")]
pub enum Erreicht {
    Ja,
    Teilweise,
    Nein,
}

impl Erreicht {
    pub fn as_str(&self) -> &'static str {
        match self {
            Erreicht::Ja => "ja",
            Erreicht::Teilweise => "teilweise",
            Erreicht::Nein => "nein",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "ja" => Erreicht::Ja,
            "teilweise" => Erreicht::Teilweise,
            "nein" => Erreicht::Nein,
            _ => return None,
        })
    }

    /// Wert in der Zeile des Protokolls.
    pub fn label(&self) -> &'static str {
        self.as_str()
    }
}

/// Ein Abschnitt der Protokollvorlage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Abschnitt {
    /// Stabile Kennung `[a-z0-9_]{1,32}`.
    pub id: String,
    pub titel: String,
    /// Anweisung an das Modell.
    pub anweisung: String,
    /// Aufgabenabschnitt (wer, was, bis wann).
    pub aufgaben: bool,
}

/// Die Protokollvorlage einer Planung.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vorlage {
    pub titel: String,
    pub kontext: String,
    pub abschnitte: Vec<Abschnitt>,
}

/// Kuerzt auf `max` Zeichen (nicht Bytes); ein Kuerzen endet mit `…`.
fn kuerzen(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kopf: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kopf.trim_end())
}

fn nicht_leer(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn agendapunkt(nr: usize, topic: &str, minuten: u32, owner: Option<&str>) -> Abschnitt {
    let wer = nicht_leer(owner)
        .map(|o| format!(", verantwortlich: {}", einzeilig(o)))
        .unwrap_or_default();
    let thema = kuerzen(&einzeilig(topic), MAX_THEMA_IN_ANWEISUNG);
    Abschnitt {
        id: format!("agenda_{nr}"),
        titel: kuerzen(&format!("{nr}. {}", einzeilig(topic)), MAX_TITEL),
        anweisung: kuerzen(
            &format!(
                "Agendapunkt {nr}: „{thema}“ ({minuten} Min.{wer}). Halte fest, was zu diesem Punkt \
                 besprochen wurde, mit der Kernaussage je Beitrag. Keine Wiederholungen, nur \
                 Gesagtes, nichts ergänzen."
            ),
            MAX_ANWEISUNG,
        ),
        aufgaben: false,
    }
}

/// Die Protokollvorlage zur Agenda der Planung: je Agendapunkt ein Abschnitt, danach Ergebnisse,
/// Entscheidungen und Aufgaben. Hat die Agenda mehr Punkte, als die Vorlage Abschnitte zulaesst
/// ([`MAX_ABSCHNITTE`]), steht der Rest in einem Abschnitt "Weitere Agendapunkte". Der Kontext
/// nennt Ziel und erwartetes Ergebnis, damit das Modell gliedert; ob das Ziel erreicht wurde,
/// bewertet ein Mensch ([`Erreicht`]), das Modell soll dazu nichts schreiben.
pub fn vorlage(plan: &Plan) -> Vorlage {
    let platz = MAX_ABSCHNITTE - FESTE_ABSCHNITTE;
    let punkte = &plan.agenda;
    let einzeln = if punkte.len() <= platz {
        punkte.len()
    } else {
        platz - 1
    };
    let mut abschnitte: Vec<Abschnitt> = punkte
        .iter()
        .take(einzeln)
        .enumerate()
        .map(|(i, a)| agendapunkt(i + 1, &a.topic, a.minutes, a.owner.as_deref()))
        .collect();
    if punkte.len() > einzeln {
        let rest = punkte
            .iter()
            .enumerate()
            .skip(einzeln)
            .map(|(i, a)| {
                format!(
                    "{}. {} ({} Min.)",
                    i + 1,
                    kuerzen(&einzeilig(&a.topic), 40),
                    a.minutes
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        abschnitte.push(Abschnitt {
            id: "agenda_rest".into(),
            titel: "Weitere Agendapunkte".into(),
            anweisung: kuerzen(
                &format!(
                    "Je Punkt ein Stichpunkt mit der Kernaussage, in der Reihenfolge der Agenda: \
                     {rest}. Nur Gesagtes, nichts ergänzen."
                ),
                MAX_ANWEISUNG,
            ),
            aufgaben: false,
        });
    }

    let erwartet = nicht_leer(plan.expected_result.as_deref()).map(einzeilig);
    let erwartet_satz = erwartet
        .as_deref()
        .map(|e| format!(" Erwartet war: {}.", kuerzen(e, 250)))
        .unwrap_or_default();
    abschnitte.push(Abschnitt {
        id: "ergebnisse".into(),
        titel: "Ergebnisse".into(),
        anweisung: kuerzen(
            &format!(
                "Konkrete Ergebnisse der Besprechung, ein Satz je Ergebnis.{erwartet_satz} Nur \
                 tatsächlich Erreichtes aufführen, Vorläufiges als vorläufig kennzeichnen."
            ),
            MAX_ANWEISUNG,
        ),
        aufgaben: false,
    });
    abschnitte.push(Abschnitt {
        id: "entscheidungen".into(),
        titel: "Entscheidungen".into(),
        anweisung: "Nur tatsächlich Beschlossenes, ein Satz je Entscheidung. Vorschläge, \
                    Vermutungen und offene Diskussionen gehören nicht hierher."
            .into(),
        aufgaben: false,
    });
    abschnitte.push(Abschnitt {
        id: "aufgaben".into(),
        titel: "Aufgaben".into(),
        anweisung: "Aufgaben, die im Gespräch ausdrücklich vergeben oder zugesagt wurden: wer \
                    erledigt was bis wann. Verantwortliche und Termine nur eintragen, wenn sie \
                    genannt wurden."
            .into(),
        aufgaben: true,
    });

    let kontext =
        format!(
        "Geplante Besprechung „{}“. Ziel ({}): {}.{} Das Protokoll folgt der Agenda der Planung: \
         je Agendapunkt ein Abschnitt, danach Ergebnisse, Entscheidungen und Aufgaben. Ob das Ziel \
         erreicht wurde, bewertet ein Mensch; triff dazu keine Aussage.",
        einzeilig(&plan.title),
        plan.goal_type.label(),
        einzeilig(plan.goal_text.trim_end_matches(['.', ' '])),
        erwartet
            .as_deref()
            .map(|e| format!(" Erwartetes Ergebnis: {}.", kuerzen(e.trim_end_matches('.'), 250)))
            .unwrap_or_default(),
    );
    Vorlage {
        titel: kuerzen(&format!("Agenda: {}", einzeilig(&plan.title)), MAX_TITEL),
        kontext: kuerzen(&kontext, MAX_KONTEXT),
        abschnitte,
    }
}

/// Die Zeilen des Protokollkopfs (Markdown, je Feld eine Zeile): Ziel, erwartetes Ergebnis (falls
/// angegeben), "Ziel erreicht" und Klassifizierung.
pub fn kopf_zeilen(plan: &Plan, erreicht: Option<Erreicht>) -> Vec<String> {
    let mut z = vec![format!(
        "**Ziel ({}):** {}",
        plan.goal_type.label(),
        einzeilig(&plan.goal_text)
    )];
    if let Some(e) = nicht_leer(plan.expected_result.as_deref()) {
        z.push(format!("**Erwartetes Ergebnis:** {}", einzeilig(e)));
    }
    z.push(erreicht_zeile(erreicht));
    z.push(format!(
        "**Klassifizierung:** {}",
        einzeilig(&plan.classification)
    ));
    z
}

fn erreicht_zeile(erreicht: Option<Erreicht>) -> String {
    format!(
        "{ERREICHT_PRAEFIX} {}",
        erreicht.map_or(NICHT_BEWERTET, |e| e.label())
    )
}

/// Der Kopf als Markdown-Block zum Anhaengen hinter die Kopfzeile des Protokolls: jede Zeile ein
/// eigener Absatz, mit Leerzeile davor.
pub fn kopf_markdown(plan: &Plan, erreicht: Option<Erreicht>) -> String {
    let mut out = String::new();
    for zeile in kopf_zeilen(plan, erreicht) {
        out.push('\n');
        out.push_str(&zeile);
        out.push('\n');
    }
    out
}

/// Setzt die Zeile "Ziel erreicht" eines vorhandenen Protokolls neu. `None`, wenn das Protokoll
/// keine solche Zeile traegt (von Hand entfernt oder vor P5 erzeugt): dann bleibt der Text, wie er
/// ist. Zeilenenden bleiben erhalten.
pub fn erreicht_ersetzen(markdown: &str, erreicht: Option<Erreicht>) -> Option<String> {
    let neu = erreicht_zeile(erreicht);
    let mut gefunden = false;
    let mut out = String::with_capacity(markdown.len() + 16);
    for zeile in markdown.split_inclusive('\n') {
        let inhalt = zeile.trim_end_matches(['\n', '\r']);
        if !gefunden && inhalt.trim_start().starts_with(ERREICHT_PRAEFIX) {
            gefunden = true;
            out.push_str(&neu);
            out.push_str(&zeile[inhalt.len()..]);
        } else {
            out.push_str(zeile);
        }
    }
    gefunden.then_some(out)
}

/// Der Wert der Zeile "Ziel erreicht" im Protokoll, falls vorhanden und eindeutig.
pub fn erreicht_lesen(markdown: &str) -> Option<Option<Erreicht>> {
    markdown.lines().find_map(|zeile| {
        let rest = zeile.trim().strip_prefix(ERREICHT_PRAEFIX)?.trim();
        Some(Erreicht::parse(rest))
    })
}

/// Das Protokoll ohne den Abschnitt der Redeanteile ([`REDEANTEILE_UEBERSCHRIFT`]); er reicht bis
/// zur naechsten Ueberschrift der zweiten Ebene.
pub fn ohne_redeanteile(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut uebersprungen = false;
    for zeile in markdown.split_inclusive('\n') {
        let kopf = zeile.trim_end_matches(['\n', '\r']).trim();
        if let Some(titel) = kopf.strip_prefix("## ") {
            uebersprungen = titel.trim() == REDEANTEILE_UEBERSCHRIFT;
        }
        if !uebersprungen {
            out.push_str(zeile);
        }
    }
    out
}

/// Die Empfaenger des Entwurfs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Empfaenger {
    /// Normalisierte Adressen (klein, ohne `mailto:`), ohne Dubletten, in der Reihenfolge des Termins.
    pub adressen: Vec<String>,
    /// Teilnehmende ohne brauchbare Adresse (nur ein Name oder eine Personen-Id).
    pub ohne_adresse: usize,
}

/// Die Empfaenger aus den Teilnehmenden des Termins: `None` oder eine Angabe ohne E-Mail-Adresse
/// zaehlt als "ohne Adresse"; Adressen werden normalisiert, Dubletten (auch bei anderer
/// Gross-/Kleinschreibung) fallen weg, die eigenen Adressen (`eigene`) gehoeren nicht dazu.
pub fn empfaenger(teilnehmende: &[Option<&str>], eigene: &[String]) -> Empfaenger {
    let eigene: Vec<String> = eigene.iter().map(|e| person_schluessel(e)).collect();
    let mut out = Empfaenger::default();
    for angabe in teilnehmende {
        let adresse = angabe
            .and_then(identitaet)
            .filter(|a| !a.starts_with("id:"));
        let Some(adresse) = adresse else {
            out.ohne_adresse += 1;
            continue;
        };
        if eigene.contains(&adresse) || out.adressen.contains(&adresse) {
            continue;
        }
        out.adressen.push(adresse);
    }
    out
}

/// Betreff des Entwurfs: `Protokoll: <Titel> (<Datum>)`.
pub fn betreff(titel: &str, datum: &str) -> String {
    format!("Protokoll: {} ({})", einzeilig(titel), datum.trim())
}

/// Text des Entwurfs aus dem freigegebenen Protokoll (bereits als Klartext). Die
/// Klassifizierungszeile der Einladung steht im Text; fehlt sie darin, kommt sie an den Anfang.
/// Mehr als das Protokoll geht nicht in den Text: es gibt keinen Parameter fuer Transkript oder Notizen.
pub fn entwurfstext(klassifizierung: &str, protokoll_text: &str) -> String {
    let text = protokoll_text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim();
    let zeile = klassifizierungszeile(klassifizierung);
    if text.lines().any(|l| l.trim() == zeile) {
        return text.to_string();
    }
    format!("{zeile}\n\n{text}")
}

/// Pruefsumme des Protokolltexts (SHA-256, hex). Zeilenenden und Leerraum am Ende zaehlen nicht.
pub fn text_hash(text: &str) -> String {
    let norm = text.replace("\r\n", "\n").replace('\r', "\n");
    let digest = Sha256::digest(norm.trim_end().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}
