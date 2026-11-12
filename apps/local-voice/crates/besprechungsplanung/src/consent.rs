//! Einwilligungs-Tor (Konzept 3.5): je Planung (Termin) und Person dokumentiert.
//!
//! Regeln, alle sperrend:
//! - Transkription nur, wenn sie geplant ist und ALLE Anwesenden zugestimmt haben.
//! - Es zaehlen nur Einwilligungen DIESER Planung (`plan_id`).
//! - Eine Zustimmung zaehlt nur mit Nachweis (Zeitpunkt und Kanal).
//! - Widerspruechliche Eintraege derselben Person: eine Ablehnung gewinnt, sonst offen.
//! - Eine anwesende Person ohne Identitaet sperrt (sie kann nicht zugestimmt haben).
//!
//! Identitaet ist NUR eine E-Mail-Adresse (`lokal@domain`) oder eine Personen-Id der
//! App mit Praefix `id:` (z. B. `id:01H...`). Ein Anzeigename wie `Alex` ist keine
//! Identitaet: zwei Personen gleichen Namens duerfen nicht zusammenfallen. Solche
//! Anwesende sperren einzeln und werden nicht zusammengefasst.
//! - Ohne erfasste Anwesende keine Transkription.

use serde::{Deserialize, Serialize};

use crate::model::{Consent, ConsentState};

/// Platzhalter in den Listen fuer eine anwesende Person ohne Identitaet.
pub const UNBEKANNT: &str = "unbekannt";

/// Identitaet einer Person (siehe Moduldoku) oder `None`, wenn keine erkennbar ist.
pub fn identitaet(s: &str) -> Option<String> {
    let t = s.trim();
    if let Some(id) = t.strip_prefix("id:") {
        let id = id.trim();
        let ok = !id.is_empty()
            && id.len() <= 64
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        return ok.then(|| format!("id:{id}"));
    }
    let p = person_schluessel(t);
    let (lokal, domain) = p.split_once('@')?;
    let ok = !lokal.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !p.chars().any(char::is_whitespace)
        && p.matches('@').count() == 1;
    ok.then_some(p)
}

/// Normalisiert eine E-Mail-Angabe: klein, ohne `mailto:`, ohne Leerraum; aus
/// `Name <adresse>` zaehlt die Adresse. Prueft NICHT, ob eine Identitaet vorliegt
/// (dafuer [`identitaet`]).
pub fn person_schluessel(s: &str) -> String {
    let t = s.trim();
    let t = match (t.rfind('<'), t.rfind('>')) {
        (Some(a), Some(b)) if a < b => &t[a + 1..b],
        _ => t,
    };
    let t = t.trim();
    let t = t
        .strip_prefix("mailto:")
        .or_else(|| t.strip_prefix("MAILTO:"))
        .unwrap_or(t);
    t.trim().to_lowercase()
}

/// Hat eine Zustimmung einen Nachweis (Zeitpunkt und Kanal)?
pub fn mit_nachweis(c: &Consent) -> bool {
    c.recorded_at.is_some() && c.channel.is_some()
}

/// Einwilligungen, die fuer neue Teilnehmende der Planung `plan_id` anzulegen sind
/// (Zustand `offen`). Eintraege anderer Planungen zaehlen nicht.
pub fn neue_einwilligungen(
    plan_id: &str,
    bestehend: &[Consent],
    teilnehmende: &[String],
) -> Vec<Consent> {
    let mut out: Vec<Consent> = Vec::new();
    for t in teilnehmende {
        let Some(p) = identitaet(t) else {
            continue;
        };
        if bestehend
            .iter()
            .any(|c| c.plan_id == plan_id && identitaet(&c.person).as_deref() == Some(&p))
            || out.iter().any(|c| c.person == p)
        {
            continue;
        }
        out.push(Consent {
            plan_id: plan_id.to_string(),
            person: p,
            state: ConsentState::Offen,
            channel: None,
            recorded_at: None,
        });
    }
    out
}

/// Wie die Besprechung startet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "modus", rename_all = "snake_case")]
pub enum StartModus {
    MitTranskription,
    NurNotizen {
        /// Transkription war gar nicht geplant.
        nicht_geplant: bool,
        /// Anwesende ohne (nachgewiesene) Zustimmung; [`UNBEKANNT`] fuer Personen
        /// ohne Identitaet.
        offen: Vec<String>,
        /// Anwesende mit Ablehnung.
        abgelehnt: Vec<String>,
    },
}

/// Zusammengefasster Stand einer Person in einer Planung (sperrend).
fn stand(plan_id: &str, person: &str, einwilligungen: &[Consent]) -> ConsentState {
    let mut zugestimmt = false;
    let mut offen = false;
    for c in einwilligungen
        .iter()
        .filter(|c| c.plan_id == plan_id && identitaet(&c.person).as_deref() == Some(person))
    {
        match c.state {
            ConsentState::Abgelehnt => return ConsentState::Abgelehnt,
            ConsentState::Zugestimmt if mit_nachweis(c) => zugestimmt = true,
            _ => offen = true,
        }
    }
    if zugestimmt && !offen {
        ConsentState::Zugestimmt
    } else {
        ConsentState::Offen
    }
}

/// Entscheidet den Startmodus einer Planung aus Anwesenden und Einwilligungen.
pub fn start_modus(
    plan_id: &str,
    transcription_planned: bool,
    anwesende: &[String],
    einwilligungen: &[Consent],
) -> StartModus {
    if !transcription_planned {
        return StartModus::NurNotizen {
            nicht_geplant: true,
            offen: Vec::new(),
            abgelehnt: Vec::new(),
        };
    }
    let mut offen = Vec::new();
    let mut abgelehnt = Vec::new();
    let mut gesehen: Vec<String> = Vec::new();
    for a in anwesende {
        let Some(p) = identitaet(a) else {
            // Jede nicht identifizierte Person sperrt fuer sich.
            offen.push(UNBEKANNT.to_string());
            continue;
        };
        if gesehen.contains(&p) {
            continue;
        }
        gesehen.push(p.clone());
        match stand(plan_id, &p, einwilligungen) {
            ConsentState::Zugestimmt => {}
            ConsentState::Abgelehnt => abgelehnt.push(p),
            ConsentState::Offen => offen.push(p),
        }
    }
    if anwesende.is_empty() || !offen.is_empty() || !abgelehnt.is_empty() {
        return StartModus::NurNotizen {
            nicht_geplant: false,
            offen,
            abgelehnt,
        };
    }
    StartModus::MitTranskription
}
