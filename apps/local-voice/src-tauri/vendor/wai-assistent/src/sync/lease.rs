//! Sitzungs-Lease nach ADR-0028 E3.3.
//!
//! - Vergabestelle ist der Hub: Die Lease (`sitzung`) aendert sich nur per Vergleichsschreiben
//!   ([`HubSpeicher::cas`]); bei gleichzeitigen Versuchen gewinnt genau einer.
//! - **Fencing-Wert ist die `generation`.** Sie steigt nur bei einer Uebernahme um 1; eine Verlaengerung
//!   durch denselben Inhaber schreibt dieselbe `generation` und aendert nur `gueltig_bis`.
//! - Die Hub-`revision` dient allein dem Vergleichsschreiben und entscheidet nie, wer Inhaber ist.
//! - Laufzeit 30 s, Verlaengerung alle 10 s. Die Uhr liefert der Aufrufer (`jetzt` in Unix-Sekunden).
//!
//! Die Funktionen hier pruefen weder Sync-Freigabe noch `geraet_lokal`; Aufrufer nutzen die Methoden
//! `lease_*` von [`super::abgleich::Abgleich`].

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::fehler::SyncFehler;
use super::hub::{AuthGeraet, HubFehler, HubSpeicher};
use super::kennung::Hex128;
use super::schluessel::Schluesselbund;
use super::umschlag::{
    SCHEMA_VERSION, Sammlung, Steuerfelder, Umschlag, oeffnen, versiegeln, zaehler,
};
use super::zeit::{aus_rfc3339, rfc3339};

/// Lease-Laufzeit in Sekunden.
pub const LAUFZEIT_S: u64 = 30;
/// Soll-Abstand der Verlaengerungen in Sekunden.
pub const VERLAENGERUNG_S: u64 = 10;
const VERLAENGERUNG_VERSUCHE: usize = 3;

/// Fehler der Lease-Vergabe.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseFehler {
    /// Vergleichsschreiben verloren: neu lesen und erneut fragen (E3.3).
    #[error("Lease wurde zwischenzeitlich geaendert, neu lesen")]
    Konflikt,
    /// Ein anderes Geraet (hoehere Generation) ist Inhaber; die laufende Antwort wird Zweig.
    #[error("Lease verloren, aktuelle Generation {aktuelle_generation}")]
    Verloren { aktuelle_generation: u64 },
    #[error("Assistent-Sync ist nicht freigegeben")]
    NichtFreigegeben,
    #[error("Unterhaltung ist nur auf diesem Geraet (geraet_lokal)")]
    GeraetLokal,
    #[error("Lease-Objekt ungueltig: {0}")]
    Ungueltig(&'static str),
    #[error(transparent)]
    Krypto(#[from] SyncFehler),
    #[error(transparent)]
    Hub(HubFehler),
}

/// Klartext der Lease (Vertrag `assistent-lease`).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseKlartext {
    schema_version: String,
    inhaber: String,
    #[serde(with = "zaehler")]
    generation: u64,
    gueltig_bis: String,
    unterhaltung_ref: Hex128,
}

/// Inhalt einer Lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub inhaber: String,
    pub generation: u64,
    /// Ablauf in Unix-Sekunden.
    pub gueltig_bis: u64,
    pub unterhaltung_ref: Hex128,
}

impl Lease {
    pub fn laeuft(&self, jetzt: u64) -> bool {
        jetzt < self.gueltig_bis
    }
}

/// Gelesene Lease mit der Hub-Revision, gegen die ein Vergleichsschreiben laeuft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gelesen {
    pub lease: Lease,
    pub revision: u64,
}

/// Stand des Inhabers nach Uebernahme oder Verlaengerung.
///
/// `generation` ist der Fencing-Wert, den die Beitraege dieser Sitzung als `lease_generation` tragen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Halt {
    pub generation: u64,
    pub revision: u64,
    pub gueltig_bis: u64,
}

/// Feste Objekt-ID der Lease einer Unterhaltung: auf jedem Geraet gleich, ohne Bezug zum Inhalt.
pub fn objekt_id(bund: &Schluesselbund, unterhaltung_ref: &Hex128) -> Hex128 {
    Hex128::abgeleitet(bund.id_salz(), "sitzung", unterhaltung_ref.as_str())
}

fn umschlag(bund: &Schluesselbund, geraet: &str, lease: &Lease) -> Result<Umschlag, LeaseFehler> {
    let steuer = Steuerfelder::inhalt(
        Sammlung::Sitzung,
        objekt_id(bund, &lease.unterhaltung_ref),
        lease.unterhaltung_ref.clone(),
        geraet,
        bund.aktuelle_epoche(),
    );
    let klartext = LeaseKlartext {
        schema_version: SCHEMA_VERSION.to_owned(),
        inhaber: lease.inhaber.clone(),
        generation: lease.generation,
        gueltig_bis: rfc3339(lease.gueltig_bis),
        unterhaltung_ref: lease.unterhaltung_ref.clone(),
    };
    let inhalt = serde_json::to_value(klartext).map_err(|_| SyncFehler::Inhalt)?;
    Ok(versiegeln(bund, &steuer, &inhalt)?)
}

fn aus_hub_fehler(e: HubFehler) -> LeaseFehler {
    match e {
        HubFehler::RevisionKonflikt { .. } => LeaseFehler::Konflikt,
        andere => LeaseFehler::Hub(andere),
    }
}

/// Liest und prueft die Lease. `None`, wenn noch keine existiert.
pub(crate) fn lesen<H: HubSpeicher + ?Sized>(
    hub: &H,
    bund: &Schluesselbund,
    auth: &AuthGeraet,
    unterhaltung_ref: &Hex128,
) -> Result<Option<Gelesen>, LeaseFehler> {
    let id = objekt_id(bund, unterhaltung_ref);
    let Some(u) = hub
        .holen(auth, Sammlung::Sitzung, &id)
        .map_err(LeaseFehler::Hub)?
    else {
        return Ok(None);
    };
    let g = oeffnen(bund, &u)?;
    let revision = g.revision.ok_or(LeaseFehler::Ungueltig("revision fehlt"))?;
    let k: LeaseKlartext =
        serde_json::from_value(g.inhalt).map_err(|_| LeaseFehler::Ungueltig("inhalt"))?;
    if &k.unterhaltung_ref != unterhaltung_ref || k.unterhaltung_ref != g.steuer.unterhaltung_ref {
        return Err(LeaseFehler::Ungueltig("unterhaltung_ref"));
    }
    if k.inhaber != g.steuer.geraet_id {
        return Err(LeaseFehler::Ungueltig("inhaber ist nicht der Schreiber"));
    }
    let gueltig_bis = aus_rfc3339(&k.gueltig_bis).ok_or(LeaseFehler::Ungueltig("gueltig_bis"))?;
    Ok(Some(Gelesen {
        lease: Lease {
            inhaber: k.inhaber,
            generation: k.generation,
            gueltig_bis,
            unterhaltung_ref: k.unterhaltung_ref,
        },
        revision,
    }))
}

/// "Hier uebernehmen": Vergleichsschreiben mit `generation + 1` gegen die zuletzt gelesene Revision.
///
/// `gelesen = None` heisst, es gab noch keine Lease (Generation 1, Objekt muss noch fehlen). Ist das
/// Geraet bereits Inhaber einer noch laufenden Lease, wird nichts geschrieben. Scheitert das
/// Vergleichsschreiben, kommt [`LeaseFehler::Konflikt`]; dann neu lesen und fragen.
pub(crate) fn uebernehmen<H: HubSpeicher + ?Sized>(
    hub: &H,
    bund: &Schluesselbund,
    auth: &AuthGeraet,
    unterhaltung_ref: &Hex128,
    jetzt: u64,
    gelesen: Option<&Gelesen>,
) -> Result<Halt, LeaseFehler> {
    if let Some(g) = gelesen
        && g.lease.inhaber == auth.id()
        && g.lease.laeuft(jetzt)
    {
        return Ok(Halt {
            generation: g.lease.generation,
            revision: g.revision,
            gueltig_bis: g.lease.gueltig_bis,
        });
    }
    let (generation, erwartet) = match gelesen {
        None => (1, None),
        Some(g) => (
            g.lease
                .generation
                .checked_add(1)
                .ok_or(LeaseFehler::Ungueltig("generation ueberlaeuft"))?,
            Some(g.revision),
        ),
    };
    let lease = Lease {
        inhaber: auth.id().to_owned(),
        generation,
        gueltig_bis: jetzt + LAUFZEIT_S,
        unterhaltung_ref: unterhaltung_ref.clone(),
    };
    let u = umschlag(bund, auth.id(), &lease)?;
    let revision = hub
        .cas(auth, Sammlung::Sitzung, &u.objekt_id, erwartet, &u)
        .map_err(aus_hub_fehler)?;
    Ok(Halt {
        generation,
        revision,
        gueltig_bis: lease.gueltig_bis,
    })
}

/// Verlaengerung durch den Inhaber: dieselbe `generation`, neues `gueltig_bis`.
///
/// Hat ein anderes Geraet inzwischen uebernommen (andere Generation oder anderer Inhaber), kommt
/// [`LeaseFehler::Verloren`]; die laufende Antwort behaelt ihre Generation und wird beim
/// Zusammenfuehren zum Zweig.
pub(crate) fn verlaengern<H: HubSpeicher + ?Sized>(
    hub: &H,
    bund: &Schluesselbund,
    auth: &AuthGeraet,
    unterhaltung_ref: &Hex128,
    jetzt: u64,
    halt: &Halt,
) -> Result<Halt, LeaseFehler> {
    let mut revision = halt.revision;
    for _ in 0..VERLAENGERUNG_VERSUCHE {
        let lease = Lease {
            inhaber: auth.id().to_owned(),
            generation: halt.generation,
            gueltig_bis: jetzt + LAUFZEIT_S,
            unterhaltung_ref: unterhaltung_ref.clone(),
        };
        let u = umschlag(bund, auth.id(), &lease)?;
        match hub.cas(auth, Sammlung::Sitzung, &u.objekt_id, Some(revision), &u) {
            Ok(neu) => {
                return Ok(Halt {
                    generation: halt.generation,
                    revision: neu,
                    gueltig_bis: lease.gueltig_bis,
                });
            }
            Err(HubFehler::RevisionKonflikt { .. }) => {
                let aktuell = lesen(hub, bund, auth, unterhaltung_ref)?;
                match aktuell {
                    Some(g)
                        if g.lease.inhaber == auth.id()
                            && g.lease.generation == halt.generation =>
                    {
                        revision = g.revision;
                    }
                    Some(g) => {
                        return Err(LeaseFehler::Verloren {
                            aktuelle_generation: g.lease.generation,
                        });
                    }
                    None => return Err(LeaseFehler::Konflikt),
                }
            }
            Err(e) => return Err(LeaseFehler::Hub(e)),
        }
    }
    Err(LeaseFehler::Konflikt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::memhub::MemHub;

    fn a(n: &str) -> AuthGeraet {
        AuthGeraet::new(n)
    }

    fn aufbau() -> (MemHub, Schluesselbund, Hex128) {
        let hub = MemHub::new();
        for n in ["pc", "iphone", "x"] {
            hub.zulassen(n);
        }
        (
            hub,
            Schluesselbund::zufaellig().unwrap(),
            Hex128::zufall().unwrap(),
        )
    }

    #[test]
    fn erste_uebernahme_ist_generation_1() {
        let (hub, bund, r) = aufbau();
        let h = uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        assert_eq!(h.generation, 1);
        assert_eq!(h.gueltig_bis, 1030);
        let g = lesen(&hub, &bund, &a("iphone"), &r).unwrap().unwrap();
        assert_eq!(g.lease.inhaber, "pc");
        assert_eq!(g.lease.generation, 1);
        assert_eq!(g.revision, h.revision);
    }

    #[test]
    fn verlaengern_aendert_generation_nicht() {
        let (hub, bund, r) = aufbau();
        let mut h = uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        for i in 1..=5u64 {
            let neu = verlaengern(&hub, &bund, &a("pc"), &r, 1000 + 10 * i, &h).unwrap();
            assert_eq!(neu.generation, 1);
            assert!(
                neu.revision > h.revision,
                "Revision steigt, Generation nicht"
            );
            assert_eq!(neu.gueltig_bis, 1000 + 10 * i + 30);
            h = neu;
        }
    }

    #[test]
    fn uebernahme_erhoeht_um_eins_und_verdraengt() {
        let (hub, bund, r) = aufbau();
        let alt = uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        let g = lesen(&hub, &bund, &a("iphone"), &r).unwrap();
        let neu = uebernehmen(&hub, &bund, &a("iphone"), &r, 1005, g.as_ref()).unwrap();
        assert_eq!(neu.generation, 2);
        assert_eq!(
            verlaengern(&hub, &bund, &a("pc"), &r, 1010, &alt),
            Err(LeaseFehler::Verloren {
                aktuelle_generation: 2
            })
        );
    }

    #[test]
    fn veraltete_revision_ist_konflikt() {
        let (hub, bund, r) = aufbau();
        uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        let alt_gelesen = lesen(&hub, &bund, &a("iphone"), &r).unwrap();
        let g = lesen(&hub, &bund, &a("pc"), &r).unwrap().unwrap();
        let h = Halt {
            generation: 1,
            revision: g.revision,
            gueltig_bis: g.lease.gueltig_bis,
        };
        verlaengern(&hub, &bund, &a("pc"), &r, 1010, &h).unwrap();
        assert_eq!(
            uebernehmen(&hub, &bund, &a("iphone"), &r, 1011, alt_gelesen.as_ref()),
            Err(LeaseFehler::Konflikt)
        );
    }

    #[test]
    fn eigene_laufende_lease_wird_nicht_neu_geschrieben() {
        let (hub, bund, r) = aufbau();
        let h = uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        let versuche = hub.schreibversuche();
        let g = lesen(&hub, &bund, &a("pc"), &r).unwrap();
        let h2 = uebernehmen(&hub, &bund, &a("pc"), &r, 1001, g.as_ref()).unwrap();
        assert_eq!(h, h2);
        assert_eq!(hub.schreibversuche(), versuche);
    }

    #[test]
    fn abgelaufene_eigene_lease_wird_neue_generation() {
        let (hub, bund, r) = aufbau();
        uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        let g = lesen(&hub, &bund, &a("pc"), &r).unwrap();
        let h = uebernehmen(&hub, &bund, &a("pc"), &r, 2000, g.as_ref()).unwrap();
        assert_eq!(h.generation, 2);
    }

    #[test]
    fn generation_ueberlauf_ist_fehler() {
        let (hub, bund, r) = aufbau();
        let voll = Gelesen {
            lease: Lease {
                inhaber: "pc".into(),
                generation: u64::MAX,
                gueltig_bis: 0,
                unterhaltung_ref: r.clone(),
            },
            revision: 1,
        };
        assert_eq!(
            uebernehmen(&hub, &bund, &a("iphone"), &r, 10, Some(&voll)),
            Err(LeaseFehler::Ungueltig("generation ueberlaeuft"))
        );
        assert_eq!(hub.schreibversuche(), 0);
    }

    #[test]
    fn lease_klartext_entspricht_dem_vertrag() {
        let (hub, bund, r) = aufbau();
        uebernehmen(&hub, &bund, &a("pc"), &r, 1_794_081_600, None).unwrap();
        let u = hub
            .holen(&a("pc"), Sammlung::Sitzung, &objekt_id(&bund, &r))
            .unwrap()
            .unwrap();
        let g = oeffnen(&bund, &u).unwrap();
        assert_eq!(g.inhalt["generation"], "1");
        assert_eq!(g.inhalt["inhaber"], "pc");
        assert_eq!(g.inhalt["gueltig_bis"], "2026-11-07T20:00:30Z");
        assert_eq!(g.inhalt["unterhaltung_ref"], r.as_str());
    }

    #[test]
    fn fremdes_salz_sieht_keine_lease() {
        let (hub, bund, r) = aufbau();
        uebernehmen(&hub, &bund, &a("pc"), &r, 1000, None).unwrap();
        let fremd = Schluesselbund::zufaellig().unwrap();
        assert_eq!(lesen(&hub, &fremd, &a("x"), &r).unwrap(), None);
    }
}
