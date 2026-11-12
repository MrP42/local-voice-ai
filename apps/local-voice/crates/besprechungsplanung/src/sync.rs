//! Abgleich Planung <-> Kalendertermin (Konzept 3.4).
//!
//! Outlook fuehrt bei Beginn, Ende, Ort, Beitritts-Link, Teilnehmenden und Absage;
//! die App fuehrt bei Ziel, Ergebnis, Agenda, Rollen und Hinweisen und schreibt nur
//! ihren markierten Block. Die Funktion hier ist rein: sie bekommt den gespeicherten
//! Stand und die Beobachtung eines Adapters und liefert den neuen Stand. Speichern,
//! Erinnerungen neu rechnen und Meldungen zeigen macht die App.
//!
//! Vorrang der Zustaende: verwaist > abgesagt > konflikt > verschoben > ok.
//! Absage, Konflikt und unbestaetigte Verschiebung sind zusaetzlich eigene Merker
//! der Verknuepfung, damit sie unter einem hoeheren Zustand nicht verloren gehen.
//!
//! Schreibfreigabe (Konzept 3.4): nur fuer schreibende Adapter (Graph, Outlook lokal),
//! nie fuer Adapter C, und nur bei nachweislich geprueftem Stand: der Termintext liegt
//! vor, eine Pruefsumme der zuletzt geschriebenen Fassung ist gespeichert und der
//! Block im Termin stimmt mit ihr ueberein.

use serde::{Deserialize, Serialize};

use crate::invitation::{block_hash, finde_block, finde_marke};
use crate::model::{AdapterKind, PlanLink, PlanStatus, SyncState};

/// Ein beobachtetes Textfeld: unbekannt (Adapter liefert es nicht), leer (im
/// Termin geloescht) oder ein Wert. Nur `Leer` loescht einen bekannten Wert.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[serde(tag = "art", content = "wert", rename_all = "snake_case")]
pub enum Feld {
    #[default]
    Unbekannt,
    Leer,
    Wert(String),
}

impl Feld {
    /// Wendet die Beobachtung auf einen gespeicherten Wert an.
    fn anwenden(&self, alt: &Option<String>) -> Option<String> {
        match self {
            Feld::Unbekannt => alt.clone(),
            Feld::Leer => None,
            Feld::Wert(v) => Some(v.clone()),
        }
    }
}

/// Was ein Adapter ueber den verknuepften Termin weiss. `None`-Felder bzw.
/// [`Feld::Unbekannt`] kennt der Adapter nicht (z. B. liefert ein ICS-Feed keinen
/// `change_key`); unbekannte Werte veraendern den gespeicherten Stand nie.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Beobachtung {
    pub remote_id: Option<String>,
    pub ical_uid: Option<String>,
    pub change_key: Option<String>,
    /// Bei Serienterminen: urspruenglicher Beginn der Instanz (RECURRENCE-ID bzw.
    /// `originalStart`), UTC-ms. Identifiziert die Instanz auch nach Verschiebung.
    pub instanz: Option<i64>,
    pub starts_at: i64,
    pub ends_at: i64,
    pub location: Feld,
    pub join_url: Feld,
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
    /// Ein verwaister Termin ist wieder auffindbar.
    Wiederverknuepft,
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
///
/// Ereignisse entstehen nur bei Aenderungen (Konflikt, Absage usw. je einmal),
/// damit wiederholte Abgleiche keine doppelten Meldungen oder Audit-Zeilen erzeugen.
pub fn abgleichen(
    status: PlanStatus,
    link: &PlanLink,
    beobachtung: Option<&Beobachtung>,
    jetzt: i64,
) -> Abgleich {
    let mut neu = link.clone();
    let mut ereignisse = Vec::new();
    let alt_state = link.sync_state;

    let verwaist = |mut neu: PlanLink, teilnehmende: Vec<String>| {
        let mut ereignisse = Vec::new();
        neu.sync_state = SyncState::Verwaist;
        if alt_state != SyncState::Verwaist {
            neu.last_change_at = Some(jetzt);
            ereignisse.push(Ereignis::Verwaist);
        }
        Abgleich {
            link: neu,
            plan_status: None,
            ereignisse,
            schreiben_erlaubt: false,
            teilnehmende,
        }
    };

    let Some(b) = beobachtung else {
        return verwaist(neu, Vec::new());
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
    if b.instanz.is_some() {
        neu.instanz = b.instanz;
    }

    // Marke weg -> verwaist (der Termin gehoert nicht mehr zu dieser Planung).
    if let Some(d) = &b.description {
        if finde_marke(d).as_deref() != Some(link.plan_id.as_str()) {
            return verwaist(neu, b.attendees.clone());
        }
    }

    let schon_bekannt = link.starts_at.is_some();
    if !schon_bekannt {
        ereignisse.push(Ereignis::Verknuepft);
    } else if alt_state == SyncState::Verwaist {
        ereignisse.push(Ereignis::Wiederverknuepft);
    }

    // Outlook-gefuehrte Felder uebernehmen.
    let zeit_neu = link.starts_at != Some(b.starts_at) || link.ends_at != Some(b.ends_at);
    if zeit_neu && schon_bekannt {
        ereignisse.push(Ereignis::Verschoben {
            alt_beginn: link.starts_at,
            neu_beginn: b.starts_at,
        });
        neu.verschiebung_offen = true;
    }
    neu.location = b.location.anwenden(&link.location);
    neu.join_url = b.join_url.anwenden(&link.join_url);
    let ort_neu = neu.location != link.location || neu.join_url != link.join_url;
    if ort_neu && schon_bekannt {
        ereignisse.push(Ereignis::OrtGeaendert);
    }
    neu.starts_at = Some(b.starts_at);
    neu.ends_at = Some(b.ends_at);

    // Block der App pruefen. Ohne Text oder ohne Pruefsumme bleibt der bekannte
    // Konfliktstand, und `geprueft` ist falsch (keine Schreibfreigabe).
    let (konflikt, geprueft) = match (&b.description, &link.body_hash) {
        (Some(d), Some(h)) => match finde_block(d, &link.plan_id) {
            Some(block) => {
                let gleich = &block_hash(&block) == h;
                (!gleich, gleich)
            }
            None => (true, false),
        },
        _ => (link.block_konflikt, false),
    };
    neu.block_konflikt = konflikt;
    if neu.block_konflikt && !link.block_konflikt {
        ereignisse.push(Ereignis::Konflikt);
    }
    if !neu.block_konflikt && link.block_konflikt {
        ereignisse.push(Ereignis::KonfliktAufgeloest);
    }

    let war_abgesagt = link.abgesagt;
    neu.abgesagt = b.cancelled;
    if b.cancelled && !war_abgesagt {
        ereignisse.push(Ereignis::Abgesagt);
    }
    if war_abgesagt && !b.cancelled {
        ereignisse.push(Ereignis::Wiederhergestellt);
    }

    let state = if b.cancelled {
        SyncState::Abgesagt
    } else if neu.block_konflikt {
        SyncState::Konflikt
    } else if neu.verschiebung_offen {
        SyncState::Verschoben
    } else {
        SyncState::Ok
    };
    neu.sync_state = state;
    if state != alt_state
        || zeit_neu
        || ort_neu
        || neu.block_konflikt != link.block_konflikt
        || neu.abgesagt != link.abgesagt
    {
        neu.last_change_at = Some(jetzt);
    }

    let plan_status = if status.ist_abgeschlossen() {
        None
    } else {
        let ziel = if b.cancelled {
            PlanStatus::Abgesagt
        } else if neu.verschiebung_offen {
            PlanStatus::Verschoben
        } else {
            PlanStatus::Verknuepft
        };
        (ziel != status).then_some(ziel)
    };

    Abgleich {
        link: neu,
        plan_status,
        ereignisse,
        schreiben_erlaubt: schreibender_adapter(link.adapter)
            && geprueft
            && matches!(state, SyncState::Ok | SyncState::Verschoben),
        teilnehmende: b.attendees.clone(),
    }
}

/// Darf dieser Adapter den Block im Termin ueberhaupt schreiben? Adapter C
/// (Zwischenablage + ICS) schreibt nach dem Einfuegen nie wieder.
pub fn schreibender_adapter(a: AdapterKind) -> bool {
    matches!(a, AdapterKind::Graph | AdapterKind::OutlookCom)
}

/// Die Nutzerin bzw. der Nutzer hat die Verschiebung zur Kenntnis genommen.
pub fn verschiebung_bestaetigen(link: &PlanLink) -> PlanLink {
    let mut neu = link.clone();
    neu.verschiebung_offen = false;
    if neu.sync_state == SyncState::Verschoben {
        neu.sync_state = SyncState::Ok;
    }
    neu
}
