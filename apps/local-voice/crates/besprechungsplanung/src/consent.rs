//! Einwilligungs-Tor (Konzept 3.5): je Planung und Person dokumentiert.
//!
//! Regel: Ist Transkription geplant, startet sie nur, wenn ALLE Anwesenden
//! zugestimmt haben. Fehlt eine Zustimmung oder liegt eine Ablehnung vor, startet
//! die Besprechung nur mit Notizen. Ohne erfasste Anwesende gibt es ebenfalls keine
//! Transkription (niemand hat zugestimmt).

use serde::{Deserialize, Serialize};

use crate::model::{Consent, ConsentState};

/// Normalisiert eine Person: E-Mail klein, ohne `mailto:`, ohne Leerraum; aus
/// `Name <adresse>` zaehlt die Adresse.
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

/// Einwilligungen, die fuer neue Teilnehmende anzulegen sind (Zustand `offen`).
pub fn neue_einwilligungen(
    plan_id: &str,
    bestehend: &[Consent],
    teilnehmende: &[String],
) -> Vec<Consent> {
    let mut out: Vec<Consent> = Vec::new();
    for t in teilnehmende {
        let p = person_schluessel(t);
        if p.is_empty()
            || bestehend.iter().any(|c| c.person == p)
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
        /// Anwesende ohne Zustimmung.
        offen: Vec<String>,
        /// Anwesende mit Ablehnung.
        abgelehnt: Vec<String>,
    },
}

/// Entscheidet den Startmodus aus Planung, Anwesenden und Einwilligungen.
pub fn start_modus(
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
        let p = person_schluessel(a);
        if p.is_empty() || gesehen.contains(&p) {
            continue;
        }
        gesehen.push(p.clone());
        match einwilligungen
            .iter()
            .find(|c| c.person == p)
            .map(|c| c.state)
        {
            Some(ConsentState::Zugestimmt) => {}
            Some(ConsentState::Abgelehnt) => abgelehnt.push(p),
            _ => offen.push(p),
        }
    }
    if gesehen.is_empty() || !offen.is_empty() || !abgelehnt.is_empty() {
        return StartModus::NurNotizen {
            nicht_geplant: false,
            offen,
            abgelehnt,
        };
    }
    StartModus::MitTranskription
}
