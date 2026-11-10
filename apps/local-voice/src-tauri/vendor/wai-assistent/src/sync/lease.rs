//! Sitzungs-Lease nach ADR-0028 E3.3 (Fassung 4: boesartiger Hub, E3.8).
//!
//! - Vergabestelle ist der Hub: Die Lease (`sitzung`) aendert sich nur per Vergleichsschreiben
//!   ([`HubSpeicher::cas`]).
//!
//! **Zwei Zusagen, streng getrennt (E3.3, E3.8):**
//! 1. *Exklusive Vergabe bei ehrlichem Hub und protokolltreuen Geraeten:* Bei gleichzeitigen Versuchen
//!    gewinnt genau einer; je Generation gibt es genau einen Inhaber.
//! 2. *Vorlaeufige Konfliktwahl bei boesartigem Hub:* Ein Hub kann dieselbe Generation zwei Geraeten
//!    bestaetigen. Das ist ein moeglicher Zustand, keine Ausnahme. Es gibt dann **keine Exklusivitaet, kein
//!    garantiertes Fencing und keine unbedingte Verlustfreiheit**. Zugesagt ist nur: Sobald die
//!    konkurrierenden, signierten Uebernahme-Ereignisse bekannt sind, wird die Konkurrenz erkannt und
//!    reihenfolgeunabhaengig vorlaeufig entschieden ([`super::verlauf::Verlauf::konkurrenz`]); ein spaeter
//!    bekanntes Ereignis kann die Wahl aendern; die Beitraege der unterlegenen Inhaber bleiben dauerhaft
//!    erhalten und als Zweig zugeordnet. Ein Hub mit geteilter Sicht kann die Konkurrenz dauerhaft verbergen.
//!    Eine gegabelte Lease-Kette sperrt die Lease dauerhaft ([`LeaseFehler::Gabelung`]).
//!
//! **Hinweis fuer Aufrufer:** Aussenwirkungen (Werkzeugaufrufe, Nachrichten, Zahlungen) sind durch die Lease
//! **nicht** abgeschirmt. Ein Inhaber, der sich fuer exklusiv haelt, kann bei boesartigem Hub gleichzeitig mit
//! einem anderen handeln; solche Effekte muessen eigene Idempotenz bzw. eigene Bestaetigung haben.
//! - **Fencing-Wert ist allein die `generation`.** Sie steigt nur bei einer Uebernahme um 1; eine
//!   Verlaengerung durch denselben Inhaber schreibt dieselbe `generation` und aendert nur `gueltig_bis`.
//! - Jede Fassung traegt zusaetzlich eine signierte, streng steigende `version` mit Vorgaenger-Hash
//!   ([`Glied`]). Das Geraet merkt sich den Hoechststand dauerhaft; aeltere Fassungen (Rollback) weist es ab.
//! - Die Hub-`revision` dient allein dem Vergleichsschreiben und entscheidet nie, wer Inhaber ist.
//! - Eigentum gilt erst als bestaetigt, wenn ein Vergleichsschreiben gelungen ist: ein veraltetes eigenes
//!   [`Gelesen`] fuehrt nie zum Erfolg ohne Schreiben.
//! - Eine abgelaufene Lease wird nie per Verlaengerung wiederbelebt; danach gibt es nur eine neue Generation.
//! - Laufzeit 30 s, Verlaengerung alle 10 s. Die Uhr liefert der Aufrufer (`jetzt` in Unix-Sekunden).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::abgleich::{Abgleich, Gesendet, Hoechststand, OffenesEreignis};
use super::fehler::SyncFehler;
use super::hub::{HubFehler, HubSpeicher};
use super::kennung::Hex128;
use super::kette::Einordnung;
use super::schluessel::Schluesselbund;
use super::umschlag::{Geoeffnet, Glied, SCHEMA_VERSION, Sammlung, oeffnen, zaehler};
use super::zeit::{MAX_SEKUNDEN, aus_rfc3339, rfc3339};

/// Lease-Laufzeit in Sekunden.
pub const LAUFZEIT_S: u64 = 30;
/// Soll-Abstand der Verlaengerungen in Sekunden.
pub const VERLAENGERUNG_S: u64 = 10;
const VERLAENGERUNG_VERSUCHE: usize = 3;

/// Fehler der Lease-Vergabe.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseFehler {
    /// Vergleichsschreiben verloren oder Lesestand veraltet: neu lesen und erneut fragen (E3.3).
    #[error("Lease wurde zwischenzeitlich geaendert, neu lesen")]
    Konflikt,
    /// Ein anderes Geraet (hoehere Generation) ist Inhaber; die laufende Antwort wird Zweig.
    #[error("Lease verloren, aktuelle Generation {aktuelle_generation}")]
    Verloren { aktuelle_generation: u64 },
    /// Die eigene Lease ist abgelaufen; sie wird nicht wiederbelebt, nur neu uebernommen.
    #[error("Lease ist abgelaufen, neu uebernehmen")]
    Abgelaufen,
    /// Der Hub liefert eine aeltere Fassung als bereits gesehen (Rollback/Replay).
    #[error("Lease-Fassung ist aelter als der lokale Hoechststand")]
    Rollback,
    /// Die Lease-Kette hat sich gegabelt (zwei widerspruechliche, gepruefte Fassungen). Die Lease bleibt
    /// dauerhaft gesperrt (Must-fix 1); die Gabelung steht in [`Abgleich::gabelungen`].
    #[error("Lease-Kette gegabelt, Lease gesperrt")]
    Gabelung,
    /// Die gelieferte Fassung ist nicht lueckenlos vom lokalen Stand belegt (Zwischenfassungen fehlen).
    #[error("Lease-Fassung ohne lueckenlosen Beleg")]
    Luecke,
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
    /// Exakt [`SCHEMA_VERSION`], geprueft beim Deserialisieren (Runde 8).
    #[serde(deserialize_with = "super::umschlag::schema_exakt")]
    schema_version: String,
    inhaber: String,
    #[serde(with = "zaehler")]
    generation: u64,
    gueltig_bis: String,
    unterhaltung_ref: Hex128,
}

/// Lease-Inhalt eines geoeffneten Objekts passt: gueltiger Klartext (innere `schema_version` exakt, Runde 8), Inhaber ist der Schreiber, Unterhaltung
/// stimmt mit den Steuerfeldern ueberein (Runde 6, auch im Pull).
pub(super) fn inhalt_passt(g: &Geoeffnet) -> bool {
    serde_json::from_value::<LeaseKlartext>(g.inhalt.clone()).is_ok_and(|k| {
        k.inhaber == g.steuer.geraet_id
            && k.unterhaltung_ref == g.steuer.unterhaltung_ref
            && aus_rfc3339(&k.gueltig_bis).is_some()
    })
}

/// Gueltiger Lease-Klartext fuer Tests (synthetische Hub-Eingriffe).
#[cfg(test)]
pub(super) fn klartext_fuer_test(
    schema: &str,
    inhaber: &str,
    generation: u64,
    gueltig_bis: u64,
    r: &Hex128,
) -> serde_json::Value {
    serde_json::to_value(LeaseKlartext {
        schema_version: schema.to_owned(),
        inhaber: inhaber.to_owned(),
        generation,
        gueltig_bis: rfc3339(gueltig_bis),
        unterhaltung_ref: r.clone(),
    })
    .expect("lease-klartext")
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

/// Gelesene, gepruefte Lease mit signierter Fassung und der Hub-Revision fuer das Vergleichsschreiben.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gelesen {
    pub lease: Lease,
    pub revision: u64,
    /// Signierte Fassung (steigt mit jedem Schreiben, auch bei Verlaengerung).
    pub version: u64,
    pub hash: [u8; 32],
}

/// Stand des Inhabers nach Uebernahme oder Verlaengerung.
///
/// `generation` ist der Fencing-Wert, den die Beitraege dieser Sitzung als `lease_generation` tragen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Halt {
    pub generation: u64,
    pub revision: u64,
    pub gueltig_bis: u64,
    pub version: u64,
    pub hash: [u8; 32],
}

/// Feste Objekt-ID der Lease einer Unterhaltung: auf jedem Geraet gleich, ohne Bezug zum Inhalt.
pub fn objekt_id(bund: &Schluesselbund, unterhaltung_ref: &Hex128) -> Hex128 {
    Hex128::abgeleitet(bund.id_salz(), "sitzung", unterhaltung_ref.as_str())
}

fn aus_hub_fehler(e: HubFehler) -> LeaseFehler {
    match e {
        HubFehler::RevisionKonflikt { .. } => LeaseFehler::Konflikt,
        andere => LeaseFehler::Hub(andere),
    }
}

fn aus_sync_fehler(e: SyncFehler) -> LeaseFehler {
    match e {
        SyncFehler::Hub(h) => aus_hub_fehler(h),
        andere => LeaseFehler::Krypto(andere),
    }
}

fn ablauf(jetzt: u64) -> Result<u64, LeaseFehler> {
    jetzt
        .checked_add(LAUFZEIT_S)
        .filter(|t| *t <= MAX_SEKUNDEN)
        .ok_or(LeaseFehler::Ungueltig("zeit ausserhalb des Bereichs"))
}

impl Abgleich {
    fn lease_pruefen(&self, r: &Hex128) -> Result<(), LeaseFehler> {
        match self.freigabe_pruefen() {
            Ok(()) => {}
            Err(SyncFehler::NichtFreigegeben) => return Err(LeaseFehler::NichtFreigegeben),
            Err(e) => return Err(LeaseFehler::Krypto(e)),
        }
        if self.gegabelt(Sammlung::Sitzung, &self.lease_objekt_id(r)) {
            return Err(LeaseFehler::Gabelung);
        }
        if self
            .unterhaltung(r)
            .is_some_and(|u| u.einschraenkung().geraet_lokal)
        {
            return Err(LeaseFehler::GeraetLokal);
        }
        if self.grabsteine().unterhaltung_geloescht(r) {
            return Err(LeaseFehler::Krypto(SyncFehler::Geloescht));
        }
        Ok(())
    }

    /// Feste Objekt-ID der Lease dieser Unterhaltung.
    pub fn lease_objekt_id(&self, r: &Hex128) -> Hex128 {
        objekt_id(self.bund(), r)
    }

    /// Liest und prueft die Lease (Signatur, Autor zugelassen, Inhaber = Schreiber, Kette): Die Fassung muss
    /// lueckenlos vom lokalen Hoechststand bzw. der Genesis abstammen (Must-fix 1); ein Sprung wird mit
    /// Zwischenbelegen des Hubs geprueft, sonst [`LeaseFehler::Luecke`]. Eine aeltere Fassung ergibt
    /// [`LeaseFehler::Rollback`], eine widerspruechliche Kette sperrt die Lease dauerhaft
    /// ([`LeaseFehler::Gabelung`]). `None`, wenn der Hub keine liefert. Ein neuer Hoechststand wird gesichert.
    pub fn lease_lesen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
    ) -> Result<Option<Gelesen>, LeaseFehler> {
        self.lease_pruefen(r)?;
        let id = self.lease_objekt_id(r);
        let Some(u) = hub
            .holen(&self.auth, Sammlung::Sitzung, &id)
            .map_err(LeaseFehler::Hub)?
        else {
            return Ok(None);
        };
        let g = oeffnen(&self.z.bund, &self.z.verzeichnis, &u)?;
        if g.steuer.sammlung != Sammlung::Sitzung || g.steuer.objekt_id != id {
            return Err(LeaseFehler::Ungueltig("objekt_id"));
        }
        if self.ueber_pin(&g) {
            return Err(LeaseFehler::Ungueltig("autor widerrufen"));
        }
        // Gemeinsame Pruefung wie im Abruf und fuer Zwischenbelege (Runde 7).
        if !self.fassung_gueltig(&g) {
            return Err(LeaseFehler::Ungueltig("fassung"));
        }
        let revision = g.revision.ok_or(LeaseFehler::Ungueltig("revision fehlt"))?;
        let glied = g.kette.ok_or(LeaseFehler::Ungueltig("version fehlt"))?;
        let k: LeaseKlartext = serde_json::from_value(g.inhalt.clone())
            .map_err(|_| LeaseFehler::Ungueltig("inhalt"))?;
        if &k.unterhaltung_ref != r || k.unterhaltung_ref != g.steuer.unterhaltung_ref {
            return Err(LeaseFehler::Ungueltig("unterhaltung_ref"));
        }
        if k.inhaber != g.steuer.geraet_id {
            return Err(LeaseFehler::Ungueltig("inhaber ist nicht der Schreiber"));
        }
        let gueltig_bis =
            aus_rfc3339(&k.gueltig_bis).ok_or(LeaseFehler::Ungueltig("gueltig_bis"))?;
        let einordnung = self.fassung_einordnen(hub, Sammlung::Sitzung, &id, r, glied, g.hash);
        match einordnung {
            Einordnung::Gleich | Einordnung::Nachfolger => {}
            Einordnung::Rollback => return Err(LeaseFehler::Rollback),
            Einordnung::Gabelung { .. } => {
                self.sichern()?;
                return Err(LeaseFehler::Gabelung);
            }
            Einordnung::Luecke => return Err(LeaseFehler::Luecke),
            Einordnung::Ungueltig => return Err(LeaseFehler::Ungueltig("kette")),
        }
        let neu = Hoechststand {
            version: glied.version,
            hash: g.hash,
            revision: Some(revision),
        };
        if self.z.hoechst.get(Sammlung::Sitzung, &id) != Some(neu) {
            // Nur eine neu uebernommene Fassung wird wirksam, Hoechststand und Herkunft gemeinsam (Runde 5/6).
            if einordnung == Einordnung::Nachfolger {
                self.uebernehmen(&g, |a| {
                    a.z.hoechst.setzen(Sammlung::Sitzung, &id, neu);
                    true
                });
            } else {
                self.z.hoechst.setzen(Sammlung::Sitzung, &id, neu);
            }
            self.sichern()?;
        }
        Ok(Some(Gelesen {
            lease: Lease {
                inhaber: k.inhaber,
                generation: k.generation,
                gueltig_bis,
                unterhaltung_ref: k.unterhaltung_ref,
            },
            revision,
            version: glied.version,
            hash: g.hash,
        }))
    }

    /// Versiegelt eine Lease-Fassung und schreibt sie per Vergleichsschreiben (Must-fix 4): Zaehler,
    /// signierter Umschlag und ein etwaiges Uebernahme-Ereignis (`folge`) liegen als Schreibabsicht gesichert
    /// vor, bevor der Hub sie sieht. Gelandet heisst: Hoechststand gehoben und Ereignis wirksam, in einer
    /// Sicherung. Eine noch offene Absicht derselben Lease (Absturz, verlorene Antwort) wird zuerst
    /// byte-identisch nachgezogen; danach kommt [`LeaseFehler::Konflikt`] (neu lesen).
    fn lease_schreiben<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        lease: &Lease,
        glied: Glied,
        erwartet: Option<u64>,
        folge: Option<OffenesEreignis>,
    ) -> Result<Halt, LeaseFehler> {
        let r = &lease.unterhaltung_ref;
        let id = self.lease_objekt_id(r);
        // Sperre unmittelbar vor dem Vergleichsschreiben erneut pruefen (Runde 3): ein vorheriges Herunterladen
        // kann die Gabelung eben erst erkannt haben.
        if self.gegabelt(Sammlung::Sitzung, &id) {
            return Err(LeaseFehler::Gabelung);
        }
        if self.ausgang_index(Sammlung::Sitzung, &id).is_some() {
            self.ausgang_senden(hub, Sammlung::Sitzung, &id)
                .map_err(aus_sync_fehler)?;
            return Err(LeaseFehler::Konflikt);
        }
        let klartext = LeaseKlartext {
            schema_version: SCHEMA_VERSION.to_owned(),
            inhaber: lease.inhaber.clone(),
            generation: lease.generation,
            gueltig_bis: rfc3339(lease.gueltig_bis),
            unterhaltung_ref: r.clone(),
        };
        let inhalt = serde_json::to_value(klartext).map_err(|_| SyncFehler::Inhalt)?;
        self.ausgang_anlegen(Sammlung::Sitzung, &id, |a| {
            let steuer = a.steuer(Sammlung::Sitzung, id.clone(), r)?;
            let u = a.siegeln(&steuer, Some(glied), &inhalt)?;
            let hash = oeffnen(&a.z.bund, &a.z.verzeichnis, &u)?.hash;
            // Kausale Bindung (Runde 3): das Uebernahme-Ereignis nennt die Fassung, die es schreibt, und das
            // eigene Glied steht im Verlauf (Folgeuebernahmen werden darueber ihrem Ast zugeordnet).
            let folge = folge.map(|mut e| {
                a.uebernahme_vollenden(&mut e, r, hash);
                (r.clone(), e)
            });
            a.lease_glied_merken(r, hash, glied.vorgaenger);
            Ok(Abgleich::cas_ausgang(
                u,
                erwartet,
                glied.version,
                hash,
                folge,
            ))
        })?;
        match self
            .ausgang_senden(hub, Sammlung::Sitzung, &id)
            .map_err(aus_sync_fehler)?
        {
            Some(Gesendet::Neu(revision) | Gesendet::Vorhanden(revision)) => {
                let h = self
                    .z
                    .hoechst
                    .get(Sammlung::Sitzung, &id)
                    .ok_or(LeaseFehler::Ungueltig("hoechststand"))?;
                Ok(Halt {
                    generation: lease.generation,
                    revision,
                    gueltig_bis: lease.gueltig_bis,
                    version: h.version,
                    hash: h.hash,
                })
            }
            Some(Gesendet::Grabstein) => Err(LeaseFehler::Krypto(SyncFehler::Geloescht)),
            Some(Gesendet::Verloren) | None => Err(LeaseFehler::Konflikt),
        }
    }

    /// "Hier uebernehmen" (E3.3): Vergleichsschreiben gegen die zuletzt gelesene Revision.
    ///
    /// - `gelesen = None`: es gab noch keine Lease (Generation 1, Objekt muss fehlen).
    /// - Eigene, laufende Lease: Eigentum wird per Vergleichsschreiben bestaetigt (gleiche Generation,
    ///   neue Fassung). Ist der Lesestand veraltet, kommt [`LeaseFehler::Konflikt`], nie ein Erfolg.
    /// - Sonst `generation + 1`. Der Verlauf merkt die Uebernahme als signiertes Ereignis (Generation und
    ///   Hauptlinienspitze), das mit dem naechsten Hochladen auf die anderen Geraete geht.
    ///
    /// Vorher wird heruntergeladen, damit Spitze und Hoechststand den bekannten Stand wiedergeben.
    pub fn lease_uebernehmen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        jetzt: u64,
        gelesen: Option<&Gelesen>,
    ) -> Result<Halt, LeaseFehler> {
        self.lease_pruefen(r)?;
        let id = self.lease_objekt_id(r);
        // Eine gesicherte, noch offene Absicht (Absturz, verlorene Antwort) zuerst byte-identisch aufloesen:
        // ist sie gelandet, wird ihr Uebernahme-Ereignis jetzt wirksam.
        if self.ausgang_index(Sammlung::Sitzung, &id).is_some() {
            self.ausgang_senden(hub, Sammlung::Sitzung, &id)
                .map_err(aus_sync_fehler)?;
        }
        self.herunterladen(hub)?;
        let hoechst = self.z.hoechst.get(Sammlung::Sitzung, &id);
        let gueltig_bis = ablauf(jetzt)?;
        let (generation, glied, erwartet) = match gelesen {
            None => {
                if hoechst.is_some() {
                    return Err(LeaseFehler::Konflikt);
                }
                (
                    1,
                    Glied {
                        version: 1,
                        vorgaenger: [0; 32],
                    },
                    None,
                )
            }
            Some(g) => {
                if g.lease.unterhaltung_ref != *r {
                    return Err(LeaseFehler::Ungueltig("unterhaltung_ref"));
                }
                if hoechst.is_some_and(|h| h.version != g.version || h.hash != g.hash) {
                    return Err(LeaseFehler::Konflikt);
                }
                let eigen = g.lease.inhaber == self.auth.id() && g.lease.laeuft(jetzt);
                let generation = if eigen {
                    g.lease.generation
                } else {
                    g.lease
                        .generation
                        .checked_add(1)
                        .ok_or(LeaseFehler::Ungueltig("generation ueberlaeuft"))?
                };
                let version = g
                    .version
                    .checked_add(1)
                    .ok_or(LeaseFehler::Ungueltig("version ueberlaeuft"))?;
                (
                    generation,
                    Glied {
                        version,
                        vorgaenger: g.hash,
                    },
                    Some(g.revision),
                )
            }
        };
        let lease = Lease {
            inhaber: self.auth.id().to_owned(),
            generation,
            gueltig_bis,
            unterhaltung_ref: r.clone(),
        };
        // Das Uebernahme-Ereignis traegt den signierten Kontext (Hash der Fassung, auf der es aufbaut) und
        // wird mit dem Vergleichsschreiben gesichert; es gilt genau dann, wenn die Fassung gelandet ist.
        let folge = if gelesen.is_none_or(|g| g.lease.generation != generation) {
            Some(self.uebernahme_ereignis(r, generation, gelesen.map_or([0; 32], |g| g.hash))?)
        } else {
            None
        };
        let halt = self.lease_schreiben(hub, &lease, glied, erwartet, folge)?;
        self.sichern()?;
        Ok(halt)
    }

    /// Verlaengerung durch den Inhaber: dieselbe `generation`, neue Fassung, neues `gueltig_bis`.
    ///
    /// Ist die Lease schon abgelaufen, kommt [`LeaseFehler::Abgelaufen`] (keine Wiederbelebung; neu
    /// uebernehmen gibt eine neue Generation). Hat ein anderes Geraet uebernommen, kommt
    /// [`LeaseFehler::Verloren`]; die laufende Antwort behaelt ihre Generation und wird Zweig.
    pub fn lease_verlaengern<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        jetzt: u64,
        halt: &Halt,
    ) -> Result<Halt, LeaseFehler> {
        self.lease_pruefen(r)?;
        self.offenen_widerruf_nachziehen(hub)?;
        if jetzt >= halt.gueltig_bis {
            return Err(LeaseFehler::Abgelaufen);
        }
        let gueltig_bis = ablauf(jetzt)?;
        let mut stand = *halt;
        for _ in 0..VERLAENGERUNG_VERSUCHE {
            let lease = Lease {
                inhaber: self.auth.id().to_owned(),
                generation: halt.generation,
                gueltig_bis,
                unterhaltung_ref: r.clone(),
            };
            let glied = Glied {
                version: stand
                    .version
                    .checked_add(1)
                    .ok_or(LeaseFehler::Ungueltig("version ueberlaeuft"))?,
                vorgaenger: stand.hash,
            };
            let ergebnis = self.lease_schreiben(hub, &lease, glied, Some(stand.revision), None);
            let ergebnis = ergebnis.and_then(|h| self.sichern().map(|()| h).map_err(Into::into));
            match ergebnis {
                Ok(h) => return Ok(h),
                Err(LeaseFehler::Konflikt) => match self.lease_lesen(hub, r)? {
                    Some(g)
                        if g.lease.inhaber == self.auth.id()
                            && g.lease.generation == halt.generation =>
                    {
                        if !g.lease.laeuft(jetzt) {
                            return Err(LeaseFehler::Abgelaufen);
                        }
                        stand = Halt {
                            generation: g.lease.generation,
                            revision: g.revision,
                            gueltig_bis: g.lease.gueltig_bis,
                            version: g.version,
                            hash: g.hash,
                        };
                    }
                    Some(g) => {
                        return Err(LeaseFehler::Verloren {
                            aktuelle_generation: g.lease.generation,
                        });
                    }
                    None => return Err(LeaseFehler::Konflikt),
                },
                Err(e) => return Err(e),
            }
        }
        Err(LeaseFehler::Konflikt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::hub::AuthGeraet;
    use crate::sync::memhub::{Boese, MemHub};
    use crate::sync::speicher::MemSpeicher;
    use crate::sync::testkonto::Testkonto;

    struct Aufbau {
        hub: MemHub,
        pc: Abgleich,
        iphone: Abgleich,
        r: Hex128,
    }

    fn aufbau() -> Aufbau {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "iphone"], &hub);
        Aufbau {
            pc: k.geraet("pc"),
            iphone: k.geraet("iphone"),
            hub,
            r: Hex128::zufall().unwrap(),
        }
    }

    #[test]
    fn erste_uebernahme_ist_generation_1() {
        let Aufbau {
            hub,
            mut pc,
            mut iphone,
            r,
        } = aufbau();
        let h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        assert_eq!((h.generation, h.version, h.gueltig_bis), (1, 1, 1030));
        let g = iphone.lease_lesen(&hub, &r).unwrap().unwrap();
        assert_eq!(g.lease.inhaber, "pc");
        assert_eq!(g.lease.generation, 1);
        assert_eq!(g.revision, h.revision);
    }

    #[test]
    fn verlaengern_aendert_generation_nicht() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        let mut h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        for i in 1..=5u64 {
            let neu = pc.lease_verlaengern(&hub, &r, 1000 + 10 * i, &h).unwrap();
            assert_eq!(neu.generation, 1);
            assert!(neu.revision > h.revision && neu.version == h.version + 1);
            assert_eq!(neu.gueltig_bis, 1000 + 10 * i + 30);
            h = neu;
        }
    }

    #[test]
    fn uebernahme_erhoeht_um_eins_und_verdraengt() {
        let Aufbau {
            hub,
            mut pc,
            mut iphone,
            r,
        } = aufbau();
        let alt = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let g = iphone.lease_lesen(&hub, &r).unwrap();
        let neu = iphone
            .lease_uebernehmen(&hub, &r, 1005, g.as_ref())
            .unwrap();
        assert_eq!(neu.generation, 2);
        assert_eq!(
            pc.lease_verlaengern(&hub, &r, 1010, &alt),
            Err(LeaseFehler::Verloren {
                aktuelle_generation: 2
            })
        );
    }

    #[test]
    fn veraltete_revision_ist_konflikt() {
        let Aufbau {
            hub,
            mut pc,
            mut iphone,
            r,
        } = aufbau();
        let h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let alt_gelesen = iphone.lease_lesen(&hub, &r).unwrap();
        pc.lease_verlaengern(&hub, &r, 1010, &h).unwrap();
        assert_eq!(
            iphone.lease_uebernehmen(&hub, &r, 1011, alt_gelesen.as_ref()),
            Err(LeaseFehler::Konflikt)
        );
    }

    #[test]
    fn eigene_laufende_lease_wird_per_cas_bestaetigt() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        let h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let g = pc.lease_lesen(&hub, &r).unwrap();
        let h2 = pc.lease_uebernehmen(&hub, &r, 1001, g.as_ref()).unwrap();
        assert_eq!(h2.generation, h.generation);
        assert!(h2.revision > h.revision, "Bestaetigung schreibt per CAS");
    }

    #[test]
    fn veraltetes_eigenes_gelesen_ist_kein_erfolg() {
        // Must-fix 4: Gelesen ist eigen und laeuft, aber inzwischen hat iphone uebernommen.
        let Aufbau {
            hub,
            mut pc,
            mut iphone,
            r,
        } = aufbau();
        pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let eigen_alt = pc.lease_lesen(&hub, &r).unwrap();
        let g = iphone.lease_lesen(&hub, &r).unwrap();
        iphone
            .lease_uebernehmen(&hub, &r, 1002, g.as_ref())
            .unwrap();
        assert_eq!(
            pc.lease_uebernehmen(&hub, &r, 1003, eigen_alt.as_ref()),
            Err(LeaseFehler::Konflikt)
        );
    }

    #[test]
    fn abgelaufene_lease_wird_nicht_wiederbelebt() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        let h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        assert_eq!(
            pc.lease_verlaengern(&hub, &r, 1030, &h),
            Err(LeaseFehler::Abgelaufen)
        );
        let g = pc.lease_lesen(&hub, &r).unwrap();
        let neu = pc.lease_uebernehmen(&hub, &r, 2000, g.as_ref()).unwrap();
        assert_eq!(neu.generation, 2);
    }

    #[test]
    fn rollback_der_lease_wird_erkannt_auch_nach_neustart() {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "iphone"], &hub);
        let mut pc = k.geraet("pc");
        let speicher = MemSpeicher::new();
        let mut iphone = k.geraet_mit_speicher("iphone", speicher.clone());
        let r = Hex128::zufall().unwrap();
        let h = pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let alt = hub.schnappschuss();
        pc.lease_verlaengern(&hub, &r, 1010, &h).unwrap();
        iphone.lease_lesen(&hub, &r).unwrap();
        hub.zuruecksetzen(&alt);
        assert_eq!(iphone.lease_lesen(&hub, &r), Err(LeaseFehler::Rollback));
        drop(iphone);
        let mut neu = Abgleich::wiederherstellen(Box::new(speicher))
            .unwrap()
            .unwrap();
        assert_eq!(neu.lease_lesen(&hub, &r), Err(LeaseFehler::Rollback));
    }

    #[test]
    fn hub_ohne_cas_durchsetzung_kann_keinen_gleichstand_erzwingen() {
        let Aufbau {
            hub,
            mut pc,
            mut iphone,
            r,
        } = aufbau();
        pc.lease_uebernehmen(&hub, &r, 1000, None).unwrap();
        let g = iphone.lease_lesen(&hub, &r).unwrap();
        iphone
            .lease_uebernehmen(&hub, &r, 1001, g.as_ref())
            .unwrap();
        hub.boese(Boese {
            ohne_durchsetzung: true,
            ..Boese::default()
        });
        // pc kennt nur die alte Fassung; sein Hoechststand ist kleiner, der Hub nimmt alles an.
        let pc_alt = Gelesen {
            lease: Lease {
                inhaber: "iphone".into(),
                generation: 2,
                gueltig_bis: 1031,
                unterhaltung_ref: r.clone(),
            },
            revision: 1,
            version: 1,
            hash: [0; 32],
        };
        assert_eq!(
            pc.lease_uebernehmen(&hub, &r, 1002, Some(&pc_alt)),
            Err(LeaseFehler::Konflikt)
        );
    }

    #[test]
    fn generation_ueberlauf_ist_fehler() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        let voll = Gelesen {
            lease: Lease {
                inhaber: "iphone".into(),
                generation: u64::MAX,
                gueltig_bis: 0,
                unterhaltung_ref: r.clone(),
            },
            revision: 1,
            version: 1,
            hash: [0; 32],
        };
        let vorher = hub.schreibversuche();
        assert_eq!(
            pc.lease_uebernehmen(&hub, &r, 10, Some(&voll)),
            Err(LeaseFehler::Ungueltig("generation ueberlaeuft"))
        );
        assert_eq!(hub.schreibversuche(), vorher);
    }

    #[test]
    fn zeit_am_bereichsende_ist_fehler() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        assert_eq!(
            pc.lease_uebernehmen(&hub, &r, u64::MAX - 5, None),
            Err(LeaseFehler::Ungueltig("zeit ausserhalb des Bereichs"))
        );
    }

    #[test]
    fn lease_klartext_entspricht_dem_vertrag() {
        let Aufbau { hub, mut pc, r, .. } = aufbau();
        pc.lease_uebernehmen(&hub, &r, 1_794_081_600, None).unwrap();
        let u = hub
            .holen(
                &AuthGeraet::new("pc"),
                Sammlung::Sitzung,
                &pc.lease_objekt_id(&r),
            )
            .unwrap()
            .unwrap();
        let g = oeffnen(pc.bund(), pc.verzeichnis(), &u).unwrap();
        assert_eq!(g.inhalt["generation"], "1");
        assert_eq!(g.inhalt["inhaber"], "pc");
        assert_eq!(g.inhalt["gueltig_bis"], "2026-11-07T20:00:30Z");
        assert_eq!(g.inhalt["unterhaltung_ref"], r.as_str());
    }

    #[test]
    fn fremdes_konto_hat_andere_lease_id() {
        let Aufbau { pc, r, .. } = aufbau();
        let fremd = Testkonto::neu(&["x"]);
        assert_ne!(
            fremd.geraet("x").lease_objekt_id(&r),
            pc.lease_objekt_id(&r)
        );
    }
}
