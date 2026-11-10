//! Authentifiziertes Geraeteverzeichnis (ADR-0028 E3.8, Must-fix 2).
//!
//! Ein signiertes, hash-verkettetes Anhaengeprotokoll je Konto. Der Hub speichert und verteilt es nur;
//! jedes Geraet prueft es selbst ab dem gepinnten Vertrauensanker:
//!
//! - Eintrag 0 ist der **Anker**: das Erstgeraet, selbst signiert. Die Konto-ID ist ein Hash seines
//!   Ausweises; ein neues Geraet pinnt sie bei der Kopplung (QR).
//! - **Aufnahme** und **Widerruf** signiert ein zu diesem Zeitpunkt zugelassenes Geraet.
//! - Ein Widerruf ist endgueltig: Weder die Geraete-ID noch einer ihrer Schluessel wird je wieder
//!   aufgenommen. Er hebt die Epoche um genau 1 (die Epochennummer ist damit authentifiziert) und
//!   pinnt den hoechsten gesehenen Schreibzaehler des widerrufenen Geraets (`bis_zaehler`).
//! - Jeder Eintrag bindet Konto, laufende Nummer und den Hash des Vorgaengers. Eine zweite Fassung
//!   derselben Nummer ist eine Gabelung und wird abgewiesen; lokal gespeicherte Eintraege kann der Hub
//!   nicht zuruecknehmen.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::fehler::SyncFehler;
use super::identitaet::{Ausweis, Geraeteschluessel};
use super::kennung::{Hex128, begrenzt_lesen, hash_teile, hexfeld};
use super::umschlag::zaehler;

const SIG_KENNUNG: &[u8] = b"wai-assistent-sync/verzeichnis/v1";
const HASH_KENNUNG: &[u8] = b"wai-assistent-sync/verzeichnis-hash/v1";
const KONTO_KENNUNG: &[u8] = b"wai-assistent-sync/konto/v1";

/// Inhalt eines Verzeichniseintrags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum Aenderung {
    /// Vertrauensanker (nur Eintrag 0).
    Anker { ausweis: Ausweis },
    /// Neues Geraet (Kopplung).
    Aufnahme { ausweis: Ausweis },
    /// Endgueltiger Widerruf; `epoche` ist die neue Schluesselepoche (= bisherige + 1).
    Widerruf {
        geraet_id: String,
        #[serde(with = "zaehler")]
        epoche: u64,
        /// Hoechster Schreibzaehler des Geraets, den der Unterzeichner gesehen hat. Spaetere Objekte
        /// dieses Geraets gelten als nach dem Widerruf entstanden.
        #[serde(with = "zaehler")]
        bis_zaehler: u64,
    },
}

/// Signierter Eintrag (Hub-Transportform).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verzeichniseintrag {
    pub konto: Hex128,
    #[serde(with = "zaehler")]
    pub nummer: u64,
    #[serde(with = "hexfeld")]
    pub vorgaenger: [u8; 32],
    pub aenderung: Aenderung,
    pub unterzeichner: String,
    #[serde(with = "hexfeld")]
    pub signatur: [u8; 64],
}

fn feld(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_be_bytes());
    out.extend_from_slice(b);
}

/// Groesste Transportform eines Verzeichniseintrags in Bytes (vor dem Lesen geprueft).
pub const MAX_VERZEICHNISEINTRAG_JSON: usize = 4096;

impl Verzeichniseintrag {
    /// Liest einen Eintrag aus der Transportform; Rohlaenge vor der Deserialisierung begrenzt (E3.8).
    pub fn aus_json(roh: &[u8]) -> Result<Self, SyncFehler> {
        begrenzt_lesen(roh, MAX_VERZEICHNISEINTRAG_JSON)
    }

    /// Kanonische, signierte Bytes.
    fn signaturbytes(&self) -> Vec<u8> {
        let mut m = SIG_KENNUNG.to_vec();
        feld(&mut m, self.konto.as_str().as_bytes());
        feld(&mut m, &self.nummer.to_be_bytes());
        feld(&mut m, &self.vorgaenger);
        match &self.aenderung {
            Aenderung::Anker { ausweis } | Aenderung::Aufnahme { ausweis } => {
                let art: &[u8] = if matches!(self.aenderung, Aenderung::Anker { .. }) {
                    b"anker"
                } else {
                    b"aufnahme"
                };
                feld(&mut m, art);
                feld(&mut m, ausweis.geraet_id.as_bytes());
                feld(&mut m, &ausweis.signatur);
                feld(&mut m, &ausweis.austausch);
            }
            Aenderung::Widerruf {
                geraet_id,
                epoche,
                bis_zaehler,
            } => {
                feld(&mut m, b"widerruf");
                feld(&mut m, geraet_id.as_bytes());
                feld(&mut m, &epoche.to_be_bytes());
                feld(&mut m, &bis_zaehler.to_be_bytes());
            }
        }
        feld(&mut m, self.unterzeichner.as_bytes());
        m
    }

    /// Hash des Eintrags (Vorgaenger des naechsten, Bindung der Schluesselverteilung).
    pub fn hash(&self) -> [u8; 32] {
        hash_teile(HASH_KENNUNG, &[&self.signaturbytes(), &self.signatur])
    }

    fn signiert(
        konto: Hex128,
        nummer: u64,
        vorgaenger: [u8; 32],
        aenderung: Aenderung,
        schluessel: &Geraeteschluessel,
    ) -> Self {
        let mut e = Self {
            konto,
            nummer,
            vorgaenger,
            aenderung,
            unterzeichner: schluessel.geraet_id().to_owned(),
            signatur: [0; 64],
        };
        e.signatur = schluessel.signieren(&e.signaturbytes());
        e
    }
}

/// Konto-ID eines Vertrauensankers (Hash seines Ausweises, 128 Bit).
pub fn konto_aus_anker(anker: &Ausweis) -> Hex128 {
    let h = hash_teile(
        KONTO_KENNUNG,
        &[
            anker.geraet_id.as_bytes(),
            &anker.signatur,
            &anker.austausch,
        ],
    );
    Hex128::aus_bytes(&h[..16])
}

/// Widerruf eines Mitglieds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widerrufen {
    /// Erste Epoche, die das Geraet nicht mehr kennt.
    pub epoche: u64,
    pub bis_zaehler: u64,
    pub nummer: u64,
}

/// Mitglied des Verzeichnisses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mitglied {
    pub ausweis: Ausweis,
    pub aufgenommen: u64,
    pub widerrufen: Option<Widerrufen>,
}

impl Mitglied {
    pub fn zugelassen(&self) -> bool {
        self.widerrufen.is_none()
    }
}

/// Aus den Eintraegen abgeleiteter Stand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stand {
    pub mitglieder: BTreeMap<String, Mitglied>,
    /// Aktuelle Schluesselepoche (0 = leeres Verzeichnis).
    pub epoche: u64,
    /// Hash des letzten Eintrags.
    pub kopf: [u8; 32],
    pub laenge: u64,
    gesperrt: BTreeSet<[u8; 32]>,
}

impl Stand {
    fn anwenden(&mut self, konto: &Hex128, e: &Verzeichniseintrag) -> Result<(), &'static str> {
        if &e.konto != konto {
            return Err("eintrag gehoert zu einem anderen konto");
        }
        if e.nummer != self.laenge {
            return Err("nummer passt nicht");
        }
        if e.vorgaenger != self.kopf {
            return Err("vorgaenger passt nicht");
        }
        let unterzeichner = match &e.aenderung {
            Aenderung::Anker { ausweis } => {
                if e.nummer != 0 {
                    return Err("anker nur als erster eintrag");
                }
                if e.unterzeichner != ausweis.geraet_id || &konto_aus_anker(ausweis) != konto {
                    return Err("anker passt nicht zum konto");
                }
                ausweis.clone()
            }
            _ if e.nummer == 0 => return Err("erster eintrag muss der anker sein"),
            _ => match self.mitglieder.get(&e.unterzeichner) {
                Some(m) if m.zugelassen() => m.ausweis.clone(),
                Some(_) => return Err("unterzeichner ist widerrufen"),
                None => return Err("unterzeichner ist nicht zugelassen"),
            },
        };
        if !unterzeichner.pruefe_signatur(&e.signaturbytes(), &e.signatur) {
            return Err("signatur ungueltig");
        }
        match &e.aenderung {
            Aenderung::Anker { ausweis } | Aenderung::Aufnahme { ausweis } => {
                ausweis.pruefen_form().map_err(|_| "ausweis ungueltig")?;
                if self.mitglieder.contains_key(&ausweis.geraet_id) {
                    return Err("geraet_id war schon aufgenommen");
                }
                if self.gesperrt.contains(&ausweis.signatur)
                    || self.gesperrt.contains(&ausweis.austausch)
                {
                    return Err("schluessel war schon in verwendung");
                }
                self.gesperrt.insert(ausweis.signatur);
                self.gesperrt.insert(ausweis.austausch);
                self.mitglieder.insert(
                    ausweis.geraet_id.clone(),
                    Mitglied {
                        ausweis: ausweis.clone(),
                        aufgenommen: e.nummer,
                        widerrufen: None,
                    },
                );
                if e.nummer == 0 {
                    self.epoche = 1;
                }
            }
            Aenderung::Widerruf {
                geraet_id,
                epoche,
                bis_zaehler,
            } => {
                if geraet_id == &e.unterzeichner {
                    return Err("ein geraet widerruft sich nicht selbst");
                }
                let neu = self.epoche.checked_add(1).ok_or("epoche ueberlaeuft")?;
                if *epoche != neu {
                    return Err("widerruf muss die epoche um genau 1 heben");
                }
                let Some(m) = self.mitglieder.get_mut(geraet_id) else {
                    return Err("widerrufenes geraet unbekannt");
                };
                if m.widerrufen.is_some() {
                    return Err("geraet ist schon widerrufen");
                }
                m.widerrufen = Some(Widerrufen {
                    epoche: neu,
                    bis_zaehler: *bis_zaehler,
                    nummer: e.nummer,
                });
                self.epoche = neu;
            }
        }
        self.kopf = e.hash();
        self.laenge += 1;
        Ok(())
    }

    pub fn mitglied(&self, geraet_id: &str) -> Option<&Mitglied> {
        self.mitglieder.get(geraet_id)
    }

    pub fn zugelassen(&self, geraet_id: &str) -> bool {
        self.mitglied(geraet_id).is_some_and(Mitglied::zugelassen)
    }
}

/// Lokal gepruefte Kopie des Verzeichnisses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Roh", into = "Roh")]
pub struct Verzeichnis {
    konto: Hex128,
    eintraege: Vec<Verzeichniseintrag>,
    stand: Stand,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Roh {
    konto: Hex128,
    eintraege: Vec<Verzeichniseintrag>,
}

impl TryFrom<Roh> for Verzeichnis {
    type Error = SyncFehler;
    fn try_from(r: Roh) -> Result<Self, SyncFehler> {
        let mut v = Verzeichnis::gepinnt(r.konto);
        for e in r.eintraege {
            v.anhaengen(e)?;
        }
        Ok(v)
    }
}

impl From<Verzeichnis> for Roh {
    fn from(v: Verzeichnis) -> Self {
        Roh {
            konto: v.konto,
            eintraege: v.eintraege,
        }
    }
}

impl Verzeichnis {
    /// Leeres Verzeichnis mit gepinnter Konto-ID (neues Geraet nach dem QR-Scan).
    pub fn gepinnt(konto: Hex128) -> Self {
        Self {
            konto,
            eintraege: Vec::new(),
            stand: Stand::default(),
        }
    }

    /// Neues Konto: das Erstgeraet signiert den Anker.
    pub fn gruenden(anker: &Geraeteschluessel) -> Self {
        let ausweis = anker.ausweis();
        let konto = konto_aus_anker(&ausweis);
        let e = Verzeichniseintrag::signiert(
            konto.clone(),
            0,
            [0; 32],
            Aenderung::Anker { ausweis },
            anker,
        );
        let mut v = Self::gepinnt(konto);
        v.anhaengen(e).expect("selbst signierter Anker ist gueltig");
        v
    }

    pub fn konto(&self) -> &Hex128 {
        &self.konto
    }

    pub fn stand(&self) -> &Stand {
        &self.stand
    }

    pub fn eintraege(&self) -> &[Verzeichniseintrag] {
        &self.eintraege
    }

    pub fn laenge(&self) -> u64 {
        self.stand.laenge
    }

    /// Authentifizierte aktuelle Schluesselepoche (zugleich Mindestepoche fuer eigene Schreibvorgaenge).
    pub fn epoche(&self) -> u64 {
        self.stand.epoche
    }

    /// Haengt einen Eintrag an. `Ok(true)`: neu; `Ok(false)`: identisch schon vorhanden.
    ///
    /// Eine abweichende Fassung einer bekannten Nummer ist [`SyncFehler::VerzeichnisGabelung`].
    pub fn anhaengen(&mut self, e: Verzeichniseintrag) -> Result<bool, SyncFehler> {
        if let Some(vorhanden) = usize::try_from(e.nummer)
            .ok()
            .and_then(|i| self.eintraege.get(i))
        {
            return if vorhanden == &e {
                Ok(false)
            } else {
                Err(SyncFehler::VerzeichnisGabelung)
            };
        }
        let mut neu = self.stand.clone();
        neu.anwenden(&self.konto, &e)
            .map_err(SyncFehler::Verzeichnis)?;
        self.stand = neu;
        self.eintraege.push(e);
        Ok(true)
    }

    /// Stand nach Eintrag `nummer` (fuer die Pruefung gewrappter Schluessel).
    pub fn stand_bei(&self, nummer: u64) -> Option<Stand> {
        let bis = usize::try_from(nummer).ok()?;
        if bis >= self.eintraege.len() {
            return None;
        }
        let mut s = Stand::default();
        for e in &self.eintraege[..=bis] {
            s.anwenden(&self.konto, e).ok()?;
        }
        Some(s)
    }

    /// Zugelassene Geraete (nicht widerrufen), nach ID sortiert.
    pub fn zugelassene(&self) -> Vec<&Ausweis> {
        self.stand
            .mitglieder
            .values()
            .filter(|m| m.zugelassen())
            .map(|m| &m.ausweis)
            .collect()
    }

    fn vorschlag(
        &self,
        aenderung: Aenderung,
        unterzeichner: &Geraeteschluessel,
    ) -> Result<Verzeichniseintrag, SyncFehler> {
        let e = Verzeichniseintrag::signiert(
            self.konto.clone(),
            self.stand.laenge,
            self.stand.kopf,
            aenderung,
            unterzeichner,
        );
        let mut probe = self.stand.clone();
        probe
            .anwenden(&self.konto, &e)
            .map_err(SyncFehler::Verzeichnis)?;
        Ok(e)
    }

    /// Signierter Aufnahme-Eintrag fuer `ausweis` (noch nicht angehaengt).
    pub fn aufnahme_vorschlagen(
        &self,
        ausweis: Ausweis,
        unterzeichner: &Geraeteschluessel,
    ) -> Result<Verzeichniseintrag, SyncFehler> {
        self.vorschlag(Aenderung::Aufnahme { ausweis }, unterzeichner)
    }

    /// Signierter Widerruf (noch nicht angehaengt); hebt die Epoche um 1.
    pub fn widerruf_vorschlagen(
        &self,
        geraet_id: &str,
        bis_zaehler: u64,
        unterzeichner: &Geraeteschluessel,
    ) -> Result<Verzeichniseintrag, SyncFehler> {
        let epoche = self
            .stand
            .epoche
            .checked_add(1)
            .ok_or(SyncFehler::Vertrag("epoche ueberlaeuft"))?;
        self.vorschlag(
            Aenderung::Widerruf {
                geraet_id: geraet_id.to_owned(),
                epoche,
                bis_zaehler,
            },
            unterzeichner,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(n: &str) -> Geraeteschluessel {
        Geraeteschluessel::erzeugen(n).unwrap()
    }

    #[test]
    fn anker_aufnahme_widerruf_und_epoche() {
        let pc = k("pc");
        let ip = k("iphone");
        let tab = k("tablet");
        let mut v = Verzeichnis::gruenden(&pc);
        assert_eq!(v.epoche(), 1);
        let e = v.aufnahme_vorschlagen(ip.ausweis(), &pc).unwrap();
        assert!(v.anhaengen(e.clone()).unwrap());
        assert!(!v.anhaengen(e).unwrap(), "identisch ist idempotent");
        v.anhaengen(v.aufnahme_vorschlagen(tab.ausweis(), &ip).unwrap())
            .unwrap();
        v.anhaengen(v.widerruf_vorschlagen("tablet", 7, &ip).unwrap())
            .unwrap();
        assert_eq!(v.epoche(), 2);
        let w = v.stand().mitglied("tablet").unwrap().widerrufen.unwrap();
        assert_eq!((w.epoche, w.bis_zaehler), (2, 7));
        assert_eq!(v.zugelassene().len(), 2);
        // Rundlauf ueber die Sicherung prueft erneut.
        let j = serde_json::to_string(&v).unwrap();
        assert_eq!(serde_json::from_str::<Verzeichnis>(&j).unwrap(), v);
    }

    #[test]
    fn widerrufene_unterschreiben_nicht_und_kommen_nie_zurueck() {
        let pc = k("pc");
        let tab = k("tablet");
        let mut v = Verzeichnis::gruenden(&pc);
        v.anhaengen(v.aufnahme_vorschlagen(tab.ausweis(), &pc).unwrap())
            .unwrap();
        v.anhaengen(v.widerruf_vorschlagen("tablet", 0, &pc).unwrap())
            .unwrap();
        assert_eq!(
            v.aufnahme_vorschlagen(k("neu").ausweis(), &tab),
            Err(SyncFehler::Verzeichnis("unterzeichner ist widerrufen"))
        );
        assert!(v.aufnahme_vorschlagen(tab.ausweis(), &pc).is_err());
        let mut gleicher_schluessel = tab.ausweis();
        gleicher_schluessel.geraet_id = "tablet-2".into();
        assert_eq!(
            v.aufnahme_vorschlagen(gleicher_schluessel, &pc),
            Err(SyncFehler::Verzeichnis(
                "schluessel war schon in verwendung"
            ))
        );
    }

    #[test]
    fn faelschung_gabelung_und_fremdes_konto() {
        let pc = k("pc");
        let mut v = Verzeichnis::gruenden(&pc);
        let fremd = k("hub");
        // Hub unterschreibt mit eigenem Schluessel unter fremdem Namen.
        let mut e = v.aufnahme_vorschlagen(k("boese").ausweis(), &pc).unwrap();
        e.signatur = fremd.signieren(b"x");
        assert_eq!(
            v.clone().anhaengen(e),
            Err(SyncFehler::Verzeichnis("signatur ungueltig"))
        );
        let a = v.aufnahme_vorschlagen(k("a").ausweis(), &pc).unwrap();
        let b = v.aufnahme_vorschlagen(k("b").ausweis(), &pc).unwrap();
        v.anhaengen(a).unwrap();
        assert_eq!(v.anhaengen(b), Err(SyncFehler::VerzeichnisGabelung));
        let anderes = Verzeichnis::gruenden(&fremd);
        let mut x = Verzeichnis::gepinnt(v.konto().clone());
        assert!(x.anhaengen(anderes.eintraege()[0].clone()).is_err());
    }

    #[test]
    fn stand_bei_einer_nummer() {
        let pc = k("pc");
        let mut v = Verzeichnis::gruenden(&pc);
        v.anhaengen(v.aufnahme_vorschlagen(k("x").ausweis(), &pc).unwrap())
            .unwrap();
        assert!(!v.stand_bei(0).unwrap().zugelassen("x"));
        assert!(v.stand_bei(1).unwrap().zugelassen("x"));
        assert!(v.stand_bei(2).is_none());
    }
}
