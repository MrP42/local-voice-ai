//! Kopplung eines neuen Geraets an einen Checkpoint des Geraeteverzeichnisses (ADR-0028 E3.8, Must-fix 2).
//!
//! Ablauf ohne Vertrauen in den Hub (kein Hub-TOFU):
//! 1. Das **aufnehmende** Geraet zeigt ein [`Kopplungsangebot`] als QR-Code (authentischer Kanal): Konto-ID,
//!    Verzeichnisnummer und Kopf-Hash seines gepruften Verzeichnisses sowie ein frisches 32-Byte-Geheimnis.
//! 2. Das **neue** Geraet liest es ([`Kopplung::neu`]) und schickt ueber einen beliebigen, auch vom Hub
//!    vermittelten Rueckkanal eine [`Kopplungsantwort`]: seinen Ausweis, einen MAC (HMAC-SHA256 mit dem
//!    QR-Geheimnis ueber Checkpoint und Ausweis) und eine Signatur mit dem neuen Geraeteschluessel
//!    (Besitznachweis). Der Hub kann den Ausweis nicht austauschen: ohne Geheimnis kein gueltiger MAC.
//! 3. Das aufnehmende Geraet prueft MAC, Signatur und dass sein Verzeichnis den Checkpoint fortsetzt, und
//!    nimmt auf ([`Abgleich::geraet_aufnehmen`]).
//! 4. Das neue Geraet schliesst ab ([`Kopplung::abschliessen`]) nur auf einer Kette, die den Checkpoint
//!    fortsetzt (gleicher Eintrag an der Checkpoint-Stelle), und nur, wenn es dort **zugelassen** ist. Eine
//!    vom Hub gelieferte Gabelung (etwa von einem laengst widerrufenen Geraet signiert) scheitert am
//!    Checkpoint.

use std::collections::BTreeMap;
use std::fmt;

use hmac::{KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::abgleich::{Abgleich, Herunterladebericht};
use super::fehler::SyncFehler;
use super::hub::{AuthGeraet, HubSpeicher};
use super::identitaet::{Ausweis, Geraeteschluessel};
use super::kennung::{GeheimHex, Hex128, begrenzt_lesen, hexfeld, zufall};
use super::paket::{entpacken, wrappen};
use super::schluessel::{Schluessel, Schluesselbund, gleich};
use super::speicher::Zustandsspeicher;
use super::umschlag::zaehler;
use super::verzeichnis::Verzeichnis;

const MAC_KENNUNG: &[u8] = b"wai-assistent-sync/kopplung-mac/v1";
const BESITZ_KENNUNG: &[u8] = b"wai-assistent-sync/kopplung-besitz/v1";

/// Groesste Transportform von Angebot und Antwort in Bytes (vor dem Lesen geprueft).
pub const MAX_KOPPLUNG_JSON: usize = 2048;

/// Inhalt des QR-Codes des aufnehmenden Geraets. `Debug` zeigt das Geheimnis nicht.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kopplungsangebot {
    pub konto: Hex128,
    /// Laenge des gepruften Verzeichnisses beim Anbieten (Checkpoint-Nummer + 1).
    #[serde(with = "zaehler")]
    pub laenge: u64,
    /// Hash des letzten Eintrags (Kopf) an diesem Checkpoint.
    #[serde(with = "hexfeld")]
    pub kopf: [u8; 32],
    geheimnis: GeheimHex,
}

impl fmt::Debug for Kopplungsangebot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kopplungsangebot")
            .field("konto", &self.konto)
            .field("laenge", &self.laenge)
            .field("geheimnis", &"***")
            .finish()
    }
}

impl Kopplungsangebot {
    /// Liest das Angebot aus dem QR-Inhalt; Rohlaenge vor der Deserialisierung begrenzt.
    pub fn aus_json(roh: &[u8]) -> Result<Self, SyncFehler> {
        begrenzt_lesen(roh, MAX_KOPPLUNG_JSON)
    }

    fn kontext(&self, ausweis: &Ausweis) -> Vec<u8> {
        let mut m = Vec::new();
        for t in [
            self.konto.as_str().as_bytes(),
            &self.laenge.to_be_bytes(),
            &self.kopf,
            ausweis.geraet_id.as_bytes(),
            &ausweis.signatur,
            &ausweis.austausch,
        ] {
            m.extend_from_slice(&(t.len() as u32).to_be_bytes());
            m.extend_from_slice(t);
        }
        m
    }

    fn mac(&self) -> Result<hmac::Hmac<Sha256>, SyncFehler> {
        <hmac::Hmac<Sha256> as KeyInit>::new_from_slice(self.geheimnis.0.as_ref())
            .map_err(|_| SyncFehler::Vertrag("mac-schluessel"))
    }

    /// Prueft eine Antwort: MAC (konstante Zeit) und Besitznachweis des neuen Geraeteschluessels.
    pub(super) fn pruefen(&self, antwort: &Kopplungsantwort) -> Result<(), SyncFehler> {
        antwort.ausweis.pruefen_form()?;
        let kontext = self.kontext(&antwort.ausweis);
        let mut mac = self.mac()?;
        mac.update(MAC_KENNUNG);
        mac.update(&kontext);
        mac.verify_slice(&antwort.mac)
            .map_err(|_| SyncFehler::Vertrag("kopplungs-mac ungueltig"))?;
        let mut besitz = BESITZ_KENNUNG.to_vec();
        besitz.extend_from_slice(&kontext);
        if !antwort.ausweis.pruefe_signatur(&besitz, &antwort.besitz) {
            return Err(SyncFehler::Vertrag("besitznachweis ungueltig"));
        }
        Ok(())
    }

    /// Setzt `v` den Checkpoint fort? `None`: noch zu kurz.
    fn fortgesetzt(&self, v: &Verzeichnis) -> Option<bool> {
        let i = usize::try_from(self.laenge.checked_sub(1)?).ok()?;
        let e = v.eintraege().get(i)?;
        Some(v.konto() == &self.konto && e.hash() == self.kopf)
    }
}

/// Rueckkanal des neuen Geraets an das aufnehmende (darf ueber den Hub laufen).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kopplungsantwort {
    pub ausweis: Ausweis,
    #[serde(with = "hexfeld")]
    pub mac: [u8; 32],
    #[serde(with = "hexfeld")]
    pub besitz: [u8; 64],
}

impl Kopplungsantwort {
    /// Antwort eines Geraeteschluessels auf ein Angebot: MAC mit dem QR-Geheimnis ueber Checkpoint und
    /// Ausweis, dazu die Signatur des Geraeteschluessels ueber denselben Kontext.
    pub fn erstellen(
        angebot: &Kopplungsangebot,
        identitaet: &Geraeteschluessel,
    ) -> Result<Self, SyncFehler> {
        let ausweis = identitaet.ausweis();
        let kontext = angebot.kontext(&ausweis);
        let mut mac = angebot.mac()?;
        mac.update(MAC_KENNUNG);
        mac.update(&kontext);
        let mut besitz = BESITZ_KENNUNG.to_vec();
        besitz.extend_from_slice(&kontext);
        Ok(Self {
            mac: mac.finalize().into_bytes().into(),
            besitz: identitaet.signieren(&besitz),
            ausweis,
        })
    }

    /// Liest die Antwort aus der Transportform; Rohlaenge vor der Deserialisierung begrenzt.
    pub fn aus_json(roh: &[u8]) -> Result<Self, SyncFehler> {
        begrenzt_lesen(roh, MAX_KOPPLUNG_JSON)
    }
}

/// Neues Geraet vor dem Abschluss der Kopplung: eigene Identitaet und der Checkpoint aus dem QR-Code.
pub struct Kopplung {
    identitaet: Geraeteschluessel,
    angebot: Kopplungsangebot,
}

impl fmt::Debug for Kopplung {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kopplung")
            .field("identitaet", &self.identitaet)
            .field("angebot", &self.angebot)
            .finish()
    }
}

impl Kopplung {
    /// Erzeugt die Geraeteidentitaet. `angebot` stammt aus dem QR-Code des aufnehmenden Geraets.
    pub fn neu(geraet_id: &str, angebot: Kopplungsangebot) -> Result<Self, SyncFehler> {
        Ok(Self {
            identitaet: Geraeteschluessel::erzeugen(geraet_id)?,
            angebot,
        })
    }

    /// Oeffentlicher Ausweis.
    pub fn ausweis(&self) -> Ausweis {
        self.identitaet.ausweis()
    }

    /// Authentisierte Antwort fuer den Rueckkanal (MAC mit dem QR-Geheimnis, Besitznachweis).
    pub fn antwort(&self) -> Result<Kopplungsantwort, SyncFehler> {
        Kopplungsantwort::erstellen(&self.angebot, &self.identitaet)
    }

    /// Liest Verzeichnis und eigene Schluesselpakete. Gelingt nur auf einer Kette, die den Checkpoint aus
    /// dem QR-Code fortsetzt, und nur, wenn dieses Geraet dort zugelassen ist (nicht nur aufgenommen).
    /// `Ok(None)`, solange Kette, Aufnahme oder Schluessel noch fehlen; eine Kette, die dem Checkpoint
    /// widerspricht, ergibt einen Fehler. Das Ergebnis ist ein freigegebenes, gesichertes Geraet.
    pub fn abschliessen<H: HubSpeicher + ?Sized>(
        &self,
        hub: &H,
        speicher: Box<dyn Zustandsspeicher>,
    ) -> Result<Option<Abgleich>, SyncFehler> {
        let auth = AuthGeraet::new(self.identitaet.geraet_id());
        let mut v = Verzeichnis::gepinnt(self.angebot.konto.clone());
        let mut eintraege = hub.verzeichnis_lesen(&auth, 0)?;
        eintraege.sort_by_key(|e| e.nummer);
        for e in eintraege {
            let _ = v.anhaengen(e);
        }
        match self.angebot.fortgesetzt(&v) {
            None => return Ok(None),
            Some(false) => {
                return Err(SyncFehler::Verzeichnis(
                    "kette setzt den kopplungs-checkpoint nicht fort",
                ));
            }
            Some(true) => {}
        }
        let ich = self.identitaet.ausweis();
        let zugelassen = v
            .stand()
            .mitglied(self.identitaet.geraet_id())
            .is_some_and(|m| m.zugelassen() && m.ausweis == ich);
        if !zugelassen {
            return Ok(None);
        }
        let mut salz: Option<Zeroizing<[u8; 32]>> = None;
        let mut schluessel: BTreeMap<u64, Schluessel> = BTreeMap::new();
        for p in hub.pakete_lesen(&auth)? {
            let Ok(Some(e)) = entpacken(&v, &self.identitaet, &p) else {
                continue;
            };
            if salz.as_ref().is_some_and(|s| !gleich(s, &e.salz)) {
                return Err(SyncFehler::Paket("kontosalz widerspruechlich"));
            }
            salz.get_or_insert(e.salz);
            if let Some(alt) = schluessel.get(&e.epoche)
                && !gleich(alt.bytes(), e.schluessel.bytes())
            {
                return Err(SyncFehler::EpocheKollision(e.epoche));
            }
            schluessel.insert(e.epoche, e.schluessel);
        }
        let (Some(salz), Some(aktuell)) = (salz, schluessel.remove(&v.epoche())) else {
            return Ok(None);
        };
        let mut bund = Schluesselbund::neu(Schluessel::aus_bytes(*salz), v.epoche(), aktuell);
        for (e, k) in schluessel {
            bund.epoche_aufnehmen(e, k)?;
        }
        let mut a = Abgleich::gekoppelt(self.identitaet.clone(), bund, v, speicher)?;
        a.freigabe_einschalten()?;
        Ok(Some(a))
    }
}

impl Abgleich {
    /// Kopplung, Schritt 1 des aufnehmenden Geraets: Angebot fuer den QR-Code mit dem Checkpoint des
    /// gepruften Verzeichnisses und einem frischen Geheimnis. Das Angebot bleibt beim Aufrufer (nur im
    /// Arbeitsspeicher) und wird fuer [`Abgleich::geraet_aufnehmen`] gebraucht.
    pub fn kopplung_anbieten(&self) -> Result<Kopplungsangebot, SyncFehler> {
        self.freigabe_pruefen()?;
        let v = &self.z.verzeichnis;
        let kopf = v
            .eintraege()
            .last()
            .map(super::verzeichnis::Verzeichniseintrag::hash)
            .ok_or(SyncFehler::Verzeichnis("verzeichnis leer"))?;
        Ok(Kopplungsangebot {
            konto: v.konto().clone(),
            laenge: v.laenge(),
            kopf,
            geheimnis: GeheimHex(Zeroizing::new(zufall::<32>()?)),
        })
    }

    /// Kopplung, Schritt 3 des aufnehmenden Geraets: prueft die Antwort gegen das eigene Angebot (MAC,
    /// Besitznachweis, eigenes Verzeichnis setzt den Checkpoint fort), nimmt per signiertem
    /// Verzeichniseintrag auf und legt je bekannter Epoche einen gewrappten Schluessel ab. Idempotent: ein
    /// schon aufgenommener, identischer Ausweis bekommt nur die Pakete erneut. Ein einmal widerrufenes
    /// Geraet oder ein schon verwendeter Schluessel wird nie aufgenommen.
    pub fn geraet_aufnehmen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        angebot: &Kopplungsangebot,
        antwort: &Kopplungsantwort,
    ) -> Result<(), SyncFehler> {
        self.freigabe_pruefen()?;
        angebot.pruefen(antwort)?;
        let ausweis = &antwort.ausweis;
        self.gesichert(|a| {
            let mut leer = Herunterladebericht::default();
            a.verzeichnis_abgleichen(hub, &mut leer)?;
            a.offenen_widerruf_nachziehen(hub)?;
            if angebot.fortgesetzt(&a.z.verzeichnis) != Some(true) {
                return Err(SyncFehler::Verzeichnis(
                    "angebot passt nicht zum verzeichnis",
                ));
            }
            let bekannt =
                a.z.verzeichnis
                    .stand()
                    .mitglied(&ausweis.geraet_id)
                    .map(|m| (m.zugelassen(), &m.ausweis == ausweis));
            match bekannt {
                Some((true, true)) => {}
                Some(_) => return Err(SyncFehler::Verzeichnis("geraet_id war schon aufgenommen")),
                None => {
                    let e =
                        a.z.verzeichnis
                            .aufnahme_vorschlagen(ausweis.clone(), &a.z.identitaet)?;
                    hub.verzeichnis_anhaengen(&a.auth, &e)?;
                    a.z.verzeichnis.anhaengen(e)?;
                }
            }
            let mut pakete = Vec::new();
            for epoche in a.z.bund.epochen().collect::<Vec<_>>() {
                if let Some(k) = a.z.bund.schluessel(epoche) {
                    pakete.push(wrappen(
                        &a.z.verzeichnis,
                        &a.z.identitaet,
                        ausweis,
                        epoche,
                        k,
                        a.z.bund.salz(),
                    )?);
                }
            }
            hub.pakete_ablegen(&a.auth, &pakete)?;
            Ok(())
        })
    }
}
