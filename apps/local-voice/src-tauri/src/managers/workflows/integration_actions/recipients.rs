//! Empfaengerregeln der Mail (B5): aus einer festen Regel und den Daten des Laufs bildet der CODE
//! die Empfaenger; Text aus einem Modell oder aus einem Termin bestimmt nie eine Adresse.
//!
//! | Regel          | Empfaenger                                                                    |
//! |----------------|-------------------------------------------------------------------------------|
//! | `me`           | nur ich (die erste eigene Adresse)                                            |
//! | `participants` | die anderen Teilnehmenden des Termins (ohne mich)                             |
//! | `all`          | die Teilnehmenden und ich                                                     |
//! | `internal`     | die Teilnehmenden mit der Domaene meiner Adressen (ohne mich)                 |
//! | `list`         | die feste Liste im Ablauf (`list`)                                            |
//!
//! „Ich“ sind: Teilnehmende, die der Kalender als `is_self` kennzeichnet (Graph kennt die Adresse
//! des Kontos), danach die Adressen der Einstellung „Meine E-Mail-Adressen“; fehlt beides, die
//! Adresse der Integration selbst (SMTP: Absender). Eine unbrauchbare Adresse bricht ab, sie wird
//! nie still weggelassen; mehr als [`MAX_RECIPIENTS`] ebenso (nichts wird gekuerzt).

use std::collections::HashSet;

use serde_json::Value;

use crate::managers::meetings::mail::valid_address;

/// So viele Empfaenger hoechstens je automatischer Mail (An und Kopie zusammen; haelt auch die
/// Vorschau der Freigabe lesbar: dort steht jede Adresse vollstaendig).
pub const MAX_RECIPIENTS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Me,
    Participants,
    All,
    Internal,
    List,
}

impl Rule {
    pub fn parse(s: &str) -> Option<Rule> {
        match s {
            "me" => Some(Rule::Me),
            "participants" => Some(Rule::Participants),
            "all" => Some(Rule::All),
            "internal" => Some(Rule::Internal),
            "list" => Some(Rule::List),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Rule::Me => "me",
            Rule::Participants => "participants",
            Rule::All => "all",
            Rule::Internal => "internal",
            Rule::List => "list",
        }
    }

    /// Kurzname fuer Protokoll und Meldungen.
    pub fn label(self) -> &'static str {
        match self {
            Rule::Me => "nur ich",
            Rule::Participants => "die anderen Teilnehmenden",
            Rule::All => "alle Teilnehmenden",
            Rule::Internal => "interne Teilnehmende",
            Rule::List => "feste Liste",
        }
    }

    /// Geht die Mail (auch) an jemand anderen als mich?
    pub fn reaches_others(self) -> bool {
        self != Rule::Me
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipientError {
    /// Keine eigene Adresse bekannt.
    NoSelf,
    /// Der Ausloeser nennt keine Teilnehmenden mit Adresse.
    NoAttendees,
    /// Keine eigene Domaene bekannt (Regel `internal`).
    NoOwnDomain,
    /// Unter den Teilnehmenden ist niemand aus der eigenen Domaene.
    NoInternal,
    EmptyList,
    BadAddress(String),
    TooMany(usize),
}

impl std::fmt::Display for RecipientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecipientError::NoSelf => write!(
                f,
                "Es ist keine eigene Adresse bekannt. Bitte unter Einstellungen → Besprechungen „Meine E-Mail-Adressen“ eintragen."
            ),
            RecipientError::NoAttendees => write!(
                f,
                "Der Auslöser nennt keine Teilnehmenden mit E-Mail-Adresse; an wen die Mail gehen soll, ist damit offen."
            ),
            RecipientError::NoOwnDomain => write!(
                f,
                "Es ist keine eigene Adresse bekannt, aus der sich die eigene Domäne ergibt (Einstellungen → Besprechungen → „Meine E-Mail-Adressen“)."
            ),
            RecipientError::NoInternal => write!(
                f,
                "Unter den Teilnehmenden ist niemand aus der eigenen Domäne."
            ),
            RecipientError::EmptyList => write!(f, "Die feste Empfängerliste ist leer."),
            RecipientError::BadAddress(a) => {
                write!(f, "Die Adresse „{a}“ ist nicht brauchbar; die Mail wird nicht gesendet.")
            }
            RecipientError::TooMany(n) => write!(
                f,
                "Die Mail ginge an {n} Empfänger (höchstens {MAX_RECIPIENTS}); sie wird nicht gesendet, damit niemand still ausgelassen wird."
            ),
        }
    }
}

impl std::error::Error for RecipientError {}

/// Ein Teilnehmender des Ausloesers (`trigger.attendees[]`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attendee {
    pub email: String,
    pub is_self: bool,
}

/// Die Teilnehmenden mit Adresse aus den Ausloeserdaten (`trigger.attendees`); Eintraege ohne
/// Adresse zaehlen nicht. Ein fehlender oder falsch geformter Wert ergibt eine leere Liste.
pub fn attendees_of(context: &Value) -> Vec<Attendee> {
    context
        .pointer("/trigger/attendees")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let email = a.get("email").and_then(Value::as_str)?.trim();
                    (!email.is_empty()).then(|| Attendee {
                        email: email.to_string(),
                        is_self: a.get("is_self").and_then(Value::as_bool).unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Woraus die Empfaenger entstehen.
pub struct Sources<'a> {
    pub attendees: &'a [Attendee],
    /// Einstellung „Meine E-Mail-Adressen“.
    pub self_emails: &'a [String],
    /// Die Adresse der Integration selbst (SMTP: Absender), Rueckfall fuer „ich“.
    pub own_address: Option<&'a str>,
    /// Die feste Liste des Ablaufs (`list`).
    pub list: &'a [String],
}

fn domain_of(addr: &str) -> Option<String> {
    addr.rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
}

fn push_unique(out: &mut Vec<String>, seen: &mut HashSet<String>, addr: &str) {
    if seen.insert(addr.to_lowercase()) {
        out.push(addr.to_string());
    }
}

/// Bildet die Empfaenger (siehe Moduldoku). Reihenfolge: wie sie genannt sind, „ich“ zuerst.
pub fn resolve(rule: Rule, src: &Sources<'_>) -> Result<Vec<String>, RecipientError> {
    // „Ich“: Kennzeichen des Kalenders, dann die Einstellung.
    let mut mine: Vec<String> = Vec::new();
    let mut mine_seen = HashSet::new();
    for a in src.attendees.iter().filter(|a| a.is_self) {
        if valid_address(&a.email) {
            push_unique(&mut mine, &mut mine_seen, a.email.trim());
        }
    }
    for e in src.self_emails {
        if valid_address(e) {
            push_unique(&mut mine, &mut mine_seen, e.trim());
        }
    }
    let own = src.own_address.map(str::trim).filter(|a| valid_address(a));
    let me = mine.first().cloned().or_else(|| own.map(str::to_string));
    let is_me = |addr: &str| {
        let l = addr.trim().to_lowercase();
        mine_seen.contains(&l) || own.is_some_and(|o| o.eq_ignore_ascii_case(&l))
    };

    let others = || -> Result<Vec<String>, RecipientError> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for a in src.attendees.iter().filter(|a| !a.is_self) {
            let addr = a.email.trim();
            if is_me(addr) {
                continue;
            }
            if !valid_address(addr) {
                return Err(RecipientError::BadAddress(addr.to_string()));
            }
            push_unique(&mut out, &mut seen, addr);
        }
        Ok(out)
    };

    let recipients = match rule {
        Rule::Me => vec![me.ok_or(RecipientError::NoSelf)?],
        Rule::Participants => {
            let list = others()?;
            if list.is_empty() {
                return Err(RecipientError::NoAttendees);
            }
            list
        }
        Rule::All => {
            let list = others()?;
            if list.is_empty() {
                return Err(RecipientError::NoAttendees);
            }
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            if let Some(m) = &me {
                push_unique(&mut out, &mut seen, m);
            }
            for a in &list {
                push_unique(&mut out, &mut seen, a);
            }
            out
        }
        Rule::Internal => {
            let domains: HashSet<String> = mine
                .iter()
                .map(String::as_str)
                .chain(own)
                .filter_map(domain_of)
                .collect();
            if domains.is_empty() {
                return Err(RecipientError::NoOwnDomain);
            }
            let list: Vec<String> = others()?
                .into_iter()
                .filter(|a| domain_of(a).is_some_and(|d| domains.contains(&d)))
                .collect();
            if list.is_empty() {
                return Err(RecipientError::NoInternal);
            }
            list
        }
        Rule::List => {
            let mut out = Vec::new();
            let mut seen = HashSet::new();
            for raw in src.list {
                let addr = raw.trim();
                if addr.is_empty() {
                    continue;
                }
                if !valid_address(addr) {
                    return Err(RecipientError::BadAddress(addr.to_string()));
                }
                push_unique(&mut out, &mut seen, addr);
            }
            if out.is_empty() {
                return Err(RecipientError::EmptyList);
            }
            out
        }
    };
    if recipients.len() > MAX_RECIPIENTS {
        return Err(RecipientError::TooMany(recipients.len()));
    }
    Ok(recipients)
}

/// Kurzes Ziel fuer Audit und Freigabe: erster Empfaenger und Anzahl.
pub fn target_of(to: &[String]) -> String {
    match to {
        [] => "(kein Empfänger)".to_string(),
        [one] => one.clone(),
        [first, rest @ ..] => format!("{first} (+{})", rest.len()),
    }
}

#[cfg(test)]
mod tests;
