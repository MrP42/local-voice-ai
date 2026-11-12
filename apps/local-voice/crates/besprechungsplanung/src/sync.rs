//! Abgleich Planung <-> Kalendertermin (Konzept 3.4).
//!
//! Outlook fuehrt bei Beginn, Ende, Ort, Beitritts-Link, Teilnehmenden und Absage;
//! die App fuehrt bei Ziel, Ergebnis, Agenda, Rollen und Hinweisen und schreibt nur
//! ihren markierten Block. Die Funktion hier ist rein: sie bekommt den gespeicherten
//! Stand und die Beobachtung eines Adapters und liefert den neuen Stand. Speichern,
//! Erinnerungen neu rechnen und Meldungen zeigen macht die App.
//!
//! Vorrang der Zustaende: verwaist > abgesagt > konflikt > verschoben > ok.

use serde::{Deserialize, Serialize};

use crate::invitation::{block_hash, finde_block, finde_marke};
use crate::model::{PlanLink, PlanStatus, SyncState};

/// Was ein Adapter ueber den verknuepften Termin weiss. `None`-Felder kennt der
/// Adapter nicht (z. B. liefert ein ICS-Feed keinen `change_key`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Beobachtung {
    pub remote_id: Option<String>,
    pub ical_uid: Option<String>,
    pub change_key: Option<String>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub location: Option<String>,
    pub join_url: Option<String>,
    pub cancelled: bool,
    /// Termintext als Text. `None`: der Adapter liefert keinen Text, dann werden
    /// Marke und Block nicht geprueft.
    pub description: Option<String>,
    /// Teilnehmende, normalisiert (siehe [`crate::consent::person_schluessel`]).
    pub attendees: Vec<String>,
}

/// Was sich beim Abgleich ergeben hat (fuer Liste, Hinweise und Audit).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Ereignis {
    Verknuepft,
    Verschoben {
        alt_beginn: Option<i64>,
        neu_beginn: i64,
    },
    OrtGeaendert,
    Abgesagt,
    Wiederhergestellt,
    Konflikt,
    KonfliktAufgeloest,
    Verwaist,
}

/// Ergebnis eines Abgleichs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Abgleich {
    pub link: PlanLink,
    /// Neuer Status der Planung, falls er sich aendert.
    pub plan_status: Option<PlanStatus>,
    pub ereignisse: Vec<Ereignis>,
    /// Darf die App ihren Block im Termin (neu) schreiben?
    pub schreiben_erlaubt: bool,
    /// Teilnehmende laut Termin (fuer neue Einwilligungen).
    pub teilnehmende: Vec<String>,
}

/// Gleicht eine Verknuepfung mit der Beobachtung ab. `beobachtung = None` heisst:
/// der Adapter findet den Termin nicht mehr (geloescht).
pub fn abgleichen(
    status: PlanStatus,
    link: &PlanLink,
    beobachtung: Option<&Beobachtung>,
    jetzt: i64,
) -> Abgleich {
    let mut neu = link.clone();
    let mut ereignisse = Vec::new();
    let alt_state = link.sync_state;

    let Some(b) = beobachtung else {
        neu.sync_state = SyncState::Verwaist;
        if alt_state != SyncState::Verwaist {
            neu.last_change_at = Some(jetzt);
            ereignisse.push(Ereignis::Verwaist);
        }
        return Abgleich {
            link: neu,
            plan_status: None,
            ereignisse,
            schreiben_erlaubt: false,
            teilnehmende: Vec::new(),
        };
    };

    neu.last_seen_at = Some(jetzt);
    if b.remote_id.is_some() {
        neu.remote_id = b.remote_id.clone();
    }
    if b.ical_uid.is_some() {
        neu.ical_uid = b.ical_uid.clone();
    }
    if b.change_key.is_some() {
        neu.change_key = b.change_key.clone();
    }

    // Marke weg -> verwaist (der Termin gehoert nicht mehr zu dieser Planung).
    if let Some(d) = &b.description {
        if finde_marke(d).as_deref() != Some(link.plan_id.as_str()) {
            neu.sync_state = SyncState::Verwaist;
            if alt_state != SyncState::Verwaist {
                neu.last_change_at = Some(jetzt);
                ereignisse.push(Ereignis::Verwaist);
            }
            return Abgleich {
                link: neu,
                plan_status: None,
                ereignisse,
                schreiben_erlaubt: false,
                teilnehmende: b.attendees.clone(),
            };
        }
    }

    // Outlook-gefuehrte Felder uebernehmen.
    let zeit_neu = link.starts_at != Some(b.starts_at) || link.ends_at != Some(b.ends_at);
    let schon_bekannt = link.starts_at.is_some();
    if zeit_neu && schon_bekannt {
        ereignisse.push(Ereignis::Verschoben {
            alt_beginn: link.starts_at,
            neu_beginn: b.starts_at,
        });
    }
    let ort_neu = link.location != b.location || link.join_url != b.join_url;
    if ort_neu && schon_bekannt {
        ereignisse.push(Ereignis::OrtGeaendert);
    }
    neu.starts_at = Some(b.starts_at);
    neu.ends_at = Some(b.ends_at);
    neu.location = b.location.clone();
    neu.join_url = b.join_url.clone();

    // Block der App pruefen.
    let konflikt = match (&b.description, &link.body_hash) {
        (Some(d), Some(h)) => match finde_block(d, &link.plan_id) {
            Some(block) => &block_hash(&block) != h,
            None => true,
        },
        _ => false,
    };

    let state = if b.cancelled {
        SyncState::Abgesagt
    } else if konflikt {
        SyncState::Konflikt
    } else if zeit_neu && schon_bekannt {
        SyncState::Verschoben
    } else if alt_state == SyncState::Verschoben {
        // Verschiebung bleibt sichtbar, bis jemand sie bestaetigt.
        SyncState::Verschoben
    } else {
        SyncState::Ok
    };

    if state == SyncState::Abgesagt && alt_state != SyncState::Abgesagt {
        ereignisse.push(Ereignis::Abgesagt);
    }
    if alt_state == SyncState::Abgesagt && !b.cancelled {
        ereignisse.push(Ereignis::Wiederhergestellt);
    }
    if konflikt && alt_state != SyncState::Konflikt {
        ereignisse.push(Ereignis::Konflikt);
    }
    if !konflikt && alt_state == SyncState::Konflikt && !b.cancelled {
        ereignisse.push(Ereignis::KonfliktAufgeloest);
    }
    if !schon_bekannt {
        ereignisse.push(Ereignis::Verknuepft);
    }

    neu.sync_state = state;
    if state != alt_state || zeit_neu || ort_neu {
        neu.last_change_at = Some(jetzt);
    }

    let plan_status = if status.ist_abgeschlossen() {
        None
    } else {
        let ziel = match state {
            SyncState::Abgesagt => PlanStatus::Abgesagt,
            SyncState::Verschoben => PlanStatus::Verschoben,
            _ => PlanStatus::Verknuepft,
        };
        (ziel != status).then_some(ziel)
    };

    Abgleich {
        link: neu,
        plan_status,
        ereignisse,
        schreiben_erlaubt: matches!(state, SyncState::Ok | SyncState::Verschoben),
        teilnehmende: b.attendees.clone(),
    }
}

/// Die Nutzerin bzw. der Nutzer hat die Verschiebung zur Kenntnis genommen.
pub fn verschiebung_bestaetigen(link: &PlanLink) -> PlanLink {
    let mut neu = link.clone();
    if neu.sync_state == SyncState::Verschoben {
        neu.sync_state = SyncState::Ok;
    }
    neu
}
