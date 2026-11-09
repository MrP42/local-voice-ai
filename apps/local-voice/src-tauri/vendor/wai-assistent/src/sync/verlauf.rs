//! Gespraechsbaum (ADR-0028 E3.4) ueber `eltern_id`: Zusammenfuehren ohne Verlust und Doppelung.
//!
//! - Maßgeblich sind `id` und `eltern_id`; Zeitangaben dienen nur der Anzeige.
//! - Zustellung ist idempotent ueber die ID; gleiche ID mit anderem Inhalt ist ein Konflikt, der
//!   protokolliert wird, ohne das Vorhandene zu ueberschreiben.
//! - Fehlt ein Elternteil, wird der Beitrag samt Nachkommen zurueckgehalten und die Luecke gemeldet.
//! - Zwei Kinder desselben Elternteils sind eine sichtbare Verzweigung. Hauptlinie ist standardmaessig das
//!   Kind mit der hoechsten `lease_generation` (Gleichstand: kleinste ID); ein Beitrag mit veralteter
//!   Generation wird nie automatisch zur Hauptlinie. Die Zweigwahl ist eine ausdrueckliche Nutzeraktion
//!   ([`Verlauf::zweig_waehlen`]).
//! - Eine Antwort behaelt die Generation, unter der sie begonnen wurde (auch ueber Lease-Verlaengerungen),
//!   und verzweigt deshalb nicht.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::fehler::SyncFehler;

/// Eingabeweg (Vertrag `conversation-entry.mode`), `unbekannt` fuer migrierte Altdaten (E11).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modus {
    Typed,
    Spoken,
    Dictated,
    Transcribed,
    System,
    Unbekannt,
}

/// Datenklasse eines Beitrags. `confidential` ist die Bezeichnung des Corporate LLM (ADR-0022:
/// entspricht `RESTRICTED`); fehlende Angaben gelten fail-closed als `Restricted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Datenklasse {
    #[serde(alias = "PUBLIC")]
    Public,
    #[serde(alias = "INTERNAL")]
    Internal,
    #[serde(alias = "PERSONAL")]
    Personal,
    #[serde(alias = "SPECIAL")]
    Special,
    #[serde(alias = "RESTRICTED")]
    Restricted,
    Confidential,
}

impl Datenklasse {
    /// `confidential` (und die gleichwertige Klasse `restricted`).
    pub fn ist_vertraulich(self) -> bool {
        matches!(self, Self::Confidential | Self::Restricted)
    }
}

/// Abgeschlossener Beitrag (Inhalt des Objekts `beitrag`, nur verschluesselt im Hub).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Beitrag {
    pub id: String,
    pub eltern_id: Option<String>,
    /// Lease-Generation, unter der der Beitrag begonnen wurde (Fencing-Wert, E3.3).
    pub lease_generation: u64,
    pub autor: String,
    pub modus: Modus,
    pub text: String,
    /// Tatsaechlich verwendetes Modell; `unbekannt`, wenn nicht bekannt (nie erfunden).
    pub modell: String,
    pub quellen: Vec<String>,
    /// Nur zur Anzeige; `unbekannt`, wenn nicht bekannt.
    pub anzeigezeit: String,
    pub datenklasse: Datenklasse,
}

impl Beitrag {
    /// Beitrag mit ehrlichen Standardwerten (`unbekannt`, `internal`, keine Quellen).
    pub fn neu(
        id: &str,
        eltern_id: Option<&str>,
        lease_generation: u64,
        autor: &str,
        modus: Modus,
        text: &str,
    ) -> Self {
        Self {
            id: id.to_owned(),
            eltern_id: eltern_id.map(str::to_owned),
            lease_generation,
            autor: autor.to_owned(),
            modus,
            text: text.to_owned(),
            modell: "unbekannt".to_owned(),
            quellen: Vec::new(),
            anzeigezeit: "unbekannt".to_owned(),
            datenklasse: Datenklasse::Internal,
        }
    }
}

impl fmt::Debug for Beitrag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Beitrag")
            .field("id", &self.id)
            .field("eltern_id", &self.eltern_id)
            .field("lease_generation", &self.lease_generation)
            .field("inhalt", &"<verborgen>")
            .finish()
    }
}

/// Ergebnis des Einfuegens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Einfuegen {
    Neu,
    /// Bereits mit identischem Inhalt vorhanden.
    Doppelt,
    /// Gleiche ID, anderer Inhalt: protokolliert, nichts ueberschrieben.
    Konflikt,
    /// Beitrag wurde geloescht (Grabstein): nicht wiederbelebt.
    Geloescht,
}

/// Warum ein Zweig nicht die Hauptlinie ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZweigGrund {
    /// Kleinere `lease_generation` als das gewaehlte Geschwister (z. B. Antwort des alten Inhabers).
    VeralteteGeneration,
    /// Gleiche Generation: gleichwertige Alternative, Wahl liegt beim Nutzer.
    Gleichstand,
}

/// Sichtbar markierter Zweig.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zweig {
    /// Elternbeitrag, an dem der Zweig abgeht (`None` bei einer zweiten Wurzel).
    pub abzweigung: Option<String>,
    /// Beitrags-IDs des Zweigs, vom Abzweig abwaerts (bei Verzweigungen weiter der Rangfolge nach).
    pub eintraege: Vec<String>,
    pub grund: ZweigGrund,
    /// Erster Beitrag hat eine kleinere Generation als die aktuelle Lease.
    pub veraltet: bool,
}

/// Fehlender Elternbeitrag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Luecke {
    pub fehlt: String,
    /// Beitraege, die auf den fehlenden Elternteil zeigen (zurueckgehalten).
    pub kinder: Vec<String>,
}

/// Berechnete Sicht auf den Baum.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ansicht {
    pub hauptlinie: Vec<String>,
    pub zweige: Vec<Zweig>,
    /// Zurueckgehaltene Beitraege (Luecke oder Zyklus); nie verloren, nur noch nicht einsortierbar.
    pub zurueckgehalten: Vec<String>,
    pub luecken: Vec<Luecke>,
    /// IDs mit Konflikt (gleiche ID, anderer Inhalt).
    pub konflikte: Vec<String>,
    /// Geloeschte Beitraege, die als leere Platzhalter im Baum stehen.
    pub geloeschte: Vec<String>,
}

impl Ansicht {
    /// Elternbeitrag fuer die naechste Eingabe: Spitze der Hauptlinie.
    pub fn spitze(&self) -> Option<&str> {
        self.hauptlinie.last().map(String::as_str)
    }

    /// Alle einsortierten oder zurueckgehaltenen IDs (Probe auf Verlustfreiheit).
    pub fn alle_ids(&self) -> BTreeSet<&str> {
        self.hauptlinie
            .iter()
            .chain(self.zweige.iter().flat_map(|z| z.eintraege.iter()))
            .chain(self.zurueckgehalten.iter())
            .map(String::as_str)
            .collect()
    }
}

/// Gespraechsbaum einer Unterhaltung.
#[derive(Clone, Debug, Default)]
pub struct Verlauf {
    beitraege: BTreeMap<String, Beitrag>,
    /// Geloeschte Beitraege: nur Struktur (Elternlink, Generation), kein Inhalt.
    geloescht: BTreeMap<String, (Option<String>, u64)>,
    konflikte: BTreeSet<String>,
    /// Nutzerwahl: Elternbeitrag (`""` = Wurzelebene) -> gewaehltes Kind.
    wahl: BTreeMap<String, String>,
    /// Uebernahmen: Generation -> Spitze der Hauptlinie zum Zeitpunkt der Uebernahme (`None` = leer).
    /// Beitraege mit kleinerer Generation unterhalb dieser Spitze sind veraltet (E3.3).
    uebernahmen: BTreeMap<u64, Option<String>>,
}

struct Knoten<'a> {
    id: &'a str,
    eltern: Option<&'a str>,
    generation: u64,
}

struct Start {
    knoten: usize,
    abzweigung: Option<usize>,
    grund: ZweigGrund,
}

/// Rangfolge der Geschwister: nicht veraltete zuerst, dann hoechste Generation, dann kleinste ID;
/// eine ausdrueckliche Nutzerwahl steht vorn. Gibt zurueck, ob der erste die Hauptlinie fortsetzen darf
/// (nicht veraltet oder vom Nutzer gewaehlt).
fn rangordnen(
    liste: &mut [usize],
    knoten: &[Knoten],
    veraltet: &[bool],
    wahl: &BTreeMap<String, String>,
    wahlschluessel: &str,
) -> bool {
    liste.sort_by(|&a, &b| {
        veraltet[a]
            .cmp(&veraltet[b])
            .then(knoten[b].generation.cmp(&knoten[a].generation))
            .then(knoten[a].id.cmp(knoten[b].id))
    });
    let gewaehlt = wahl.get(wahlschluessel);
    let pos = liste
        .iter()
        .position(|&k| gewaehlt.is_some_and(|w| w.as_str() == knoten[k].id));
    if let Some(pos) = pos {
        liste[..=pos].rotate_right(1);
        return true;
    }
    !veraltet[liste[0]]
}

fn zweiggrund(
    kind: usize,
    haupt: Option<usize>,
    knoten: &[Knoten],
    veraltet: &[bool],
) -> ZweigGrund {
    let kleiner = haupt.is_some_and(|h| knoten[kind].generation < knoten[h].generation);
    if veraltet[kind] || kleiner {
        ZweigGrund::VeralteteGeneration
    } else {
        ZweigGrund::Gleichstand
    }
}

/// Gemeinsame, nur lesende Sicht fuer das Abwaertslaufen.
struct Baum<'a> {
    knoten: &'a [Knoten<'a>],
    kinder: &'a BTreeMap<Option<usize>, Vec<usize>>,
    veraltet: &'a [bool],
    wahl: &'a BTreeMap<String, String>,
}

/// Laeuft von `start` der Rangfolge nach abwaerts; Geschwister werden als neue Zweige vorgemerkt.
/// Auf der Hauptlinie (`haupt`) endet der Lauf vor einem veralteten ersten Kind: alle Kinder werden Zweige.
fn abwaerts(
    baum: &Baum,
    start: usize,
    haupt: bool,
    besucht: &mut [bool],
    neu: &mut VecDeque<Start>,
) -> Vec<usize> {
    let mut pfad = Vec::new();
    let mut aktuell = start;
    while !besucht[aktuell] {
        besucht[aktuell] = true;
        pfad.push(aktuell);
        let Some(liste) = baum.kinder.get(&Some(aktuell)) else {
            break;
        };
        let mut liste = liste.clone();
        let fortsetzen = rangordnen(
            &mut liste,
            baum.knoten,
            baum.veraltet,
            baum.wahl,
            baum.knoten[aktuell].id,
        );
        // Ein veralteter Knoten steht nur auf der Hauptlinie, wenn der Nutzer ihn gewaehlt hat.
        let abgang = if haupt && !fortsetzen && !baum.veraltet[aktuell] {
            0
        } else {
            1
        };
        for &kind in &liste[abgang..] {
            let grund = if abgang == 0 {
                ZweigGrund::VeralteteGeneration
            } else {
                zweiggrund(kind, Some(liste[0]), baum.knoten, baum.veraltet)
            };
            neu.push_back(Start {
                knoten: kind,
                abzweigung: Some(aktuell),
                grund,
            });
        }
        if abgang == 0 {
            break;
        }
        aktuell = liste[0];
    }
    pfad
}

impl Verlauf {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.beitraege.len()
    }

    pub fn is_empty(&self) -> bool {
        self.beitraege.is_empty()
    }

    pub fn beitrag(&self, id: &str) -> Option<&Beitrag> {
        self.beitraege.get(id)
    }

    pub fn beitraege(&self) -> impl Iterator<Item = &Beitrag> {
        self.beitraege.values()
    }

    pub fn konflikte(&self) -> impl Iterator<Item = &str> {
        self.konflikte.iter().map(String::as_str)
    }

    /// Idempotentes Einfuegen ueber die Beitrags-ID.
    pub fn einfuegen(&mut self, beitrag: Beitrag) -> Einfuegen {
        if self.geloescht.contains_key(&beitrag.id) {
            return Einfuegen::Geloescht;
        }
        match self.beitraege.get(&beitrag.id) {
            Some(vorhanden) if vorhanden == &beitrag => Einfuegen::Doppelt,
            Some(_) => {
                self.konflikte.insert(beitrag.id);
                Einfuegen::Konflikt
            }
            None => {
                self.beitraege.insert(beitrag.id.clone(), beitrag);
                Einfuegen::Neu
            }
        }
    }

    /// Fuehrt Beitraege zusammen; gibt die Anzahl neu aufgenommener zurueck.
    pub fn zusammenfuehren(&mut self, beitraege: impl IntoIterator<Item = Beitrag>) -> usize {
        beitraege
            .into_iter()
            .filter(|b| self.einfuegen(b.clone()) == Einfuegen::Neu)
            .count()
    }

    /// Loescht den Inhalt eines Beitrags (Grabstein); die Struktur bleibt als Platzhalter.
    /// Gibt `false` zurueck, wenn er schon geloescht war oder unbekannt ist.
    pub fn loeschen(&mut self, id: &str) -> bool {
        let Some(b) = self.beitraege.remove(id) else {
            return false;
        };
        self.konflikte.remove(id);
        self.geloescht
            .insert(b.id, (b.eltern_id, b.lease_generation));
        true
    }

    /// Ausdrueckliche Zweigwahl: `kind` wird an `eltern` (`None` = Wurzelebene) zur Hauptlinie.
    pub fn zweig_waehlen(&mut self, eltern: Option<&str>, kind: &str) -> Result<(), SyncFehler> {
        let ist_kind = self
            .beitraege
            .get(kind)
            .is_some_and(|b| b.eltern_id.as_deref() == eltern);
        if !ist_kind {
            return Err(SyncFehler::ZweigUngueltig);
        }
        self.wahl
            .insert(eltern.unwrap_or_default().to_owned(), kind.to_owned());
        Ok(())
    }

    pub fn wahl(&self) -> &BTreeMap<String, String> {
        &self.wahl
    }

    pub fn wahl_setzen(&mut self, wahl: BTreeMap<String, String>) {
        self.wahl = wahl;
    }

    /// Hauptlinienspitze unter Beruecksichtigung der bekannten Uebernahmen (Zweige zaehlen nicht).
    pub fn spitze(&self, aktuelle_generation: u64) -> Option<String> {
        self.ansicht(aktuelle_generation)
            .spitze()
            .map(str::to_owned)
    }

    /// Merkt eine Uebernahme: Beitraege mit kleinerer Generation unterhalb von `spitze` sind veraltet.
    /// Eine bekannte Generation wird nicht ueberschrieben. `true`, wenn neu.
    pub fn uebernahme_merken(&mut self, generation: u64, spitze: Option<String>) -> bool {
        if self.uebernahmen.contains_key(&generation) {
            return false;
        }
        self.uebernahmen.insert(generation, spitze);
        true
    }

    fn hoechste_uebernahme(&self) -> u64 {
        self.uebernahmen.keys().next_back().copied().unwrap_or(0)
    }

    pub fn uebernahmen(&self) -> &BTreeMap<u64, Option<String>> {
        &self.uebernahmen
    }

    /// Vereinigt fremde Uebernahmen (die erste bekannte je Generation gilt).
    pub fn uebernahmen_vereinen(&mut self, andere: &BTreeMap<u64, Option<String>>) -> bool {
        let vorher = self.uebernahmen.len();
        for (g, s) in andere {
            self.uebernahmen.entry(*g).or_insert_with(|| s.clone());
        }
        self.uebernahmen.len() != vorher
    }

    /// Baumsicht. `aktuelle_generation` ist die zuletzt bekannte Lease-Generation (nur fuer die
    /// Markierung `veraltet`). Intern gilt mindestens die hoechste bekannte Uebernahme: der Aufrufer
    /// kann sie nicht unterschreiten und so keine veralteten Beitraege als aktuell zeigen.
    pub fn ansicht(&self, aktuelle_generation: u64) -> Ansicht {
        let aktuelle_generation = aktuelle_generation.max(self.hoechste_uebernahme());
        let mut knoten: Vec<Knoten> = self
            .beitraege
            .values()
            .map(|b| Knoten {
                id: &b.id,
                eltern: b.eltern_id.as_deref(),
                generation: b.lease_generation,
            })
            .collect();
        knoten.extend(
            self.geloescht
                .iter()
                .map(|(id, (eltern, generation))| Knoten {
                    id,
                    eltern: eltern.as_deref(),
                    generation: *generation,
                }),
        );
        knoten.sort_by(|a, b| a.id.cmp(b.id));
        let index: BTreeMap<&str, usize> =
            knoten.iter().enumerate().map(|(i, k)| (k.id, i)).collect();

        let mut kinder: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
        let mut fehlend: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for (i, k) in knoten.iter().enumerate() {
            match k.eltern.map(|p| (p, index.get(p))) {
                Some((p, None)) => fehlend.entry(p).or_default().push(k.id.to_owned()),
                Some((_, Some(&p))) => kinder.entry(Some(p)).or_default().push(i),
                None => kinder.entry(None).or_default().push(i),
            }
        }

        let mut veraltet = vec![false; knoten.len()];
        for (&generation, spitze) in self.uebernahmen.range(..=aktuelle_generation) {
            let wurzel = match spitze {
                None => None,
                Some(id) => match index.get(id.as_str()) {
                    Some(&i) => Some(i),
                    None => continue,
                },
            };
            let mut stapel: Vec<usize> = kinder.get(&wurzel).cloned().unwrap_or_default();
            let mut gesehen = vec![false; knoten.len()];
            while let Some(i) = stapel.pop() {
                if std::mem::replace(&mut gesehen[i], true) {
                    continue;
                }
                veraltet[i] |= knoten[i].generation < generation;
                stapel.extend(kinder.get(&Some(i)).into_iter().flatten());
            }
        }

        let baum = Baum {
            knoten: &knoten,
            kinder: &kinder,
            veraltet: &veraltet,
            wahl: &self.wahl,
        };
        let mut besucht = vec![false; knoten.len()];
        let mut warteschlange: VecDeque<Start> = VecDeque::new();
        let mut ansicht = Ansicht::default();
        if let Some(wurzeln) = kinder.get(&None) {
            let mut wurzeln = wurzeln.clone();
            let fortsetzen = rangordnen(&mut wurzeln, &knoten, &veraltet, &self.wahl, "");
            let abgang = if fortsetzen { 1 } else { 0 };
            for &w in &wurzeln[abgang..] {
                warteschlange.push_back(Start {
                    knoten: w,
                    abzweigung: None,
                    grund: if abgang == 0 {
                        ZweigGrund::VeralteteGeneration
                    } else {
                        zweiggrund(w, Some(wurzeln[0]), &knoten, &veraltet)
                    },
                });
            }
            if fortsetzen {
                ansicht.hauptlinie =
                    abwaerts(&baum, wurzeln[0], true, &mut besucht, &mut warteschlange)
                        .into_iter()
                        .map(|i| knoten[i].id.to_owned())
                        .collect();
            }
        }
        while let Some(s) = warteschlange.pop_front() {
            let pfad = abwaerts(&baum, s.knoten, false, &mut besucht, &mut warteschlange);
            if pfad.is_empty() {
                continue;
            }
            ansicht.zweige.push(Zweig {
                abzweigung: s.abzweigung.map(|i| knoten[i].id.to_owned()),
                eintraege: pfad.into_iter().map(|i| knoten[i].id.to_owned()).collect(),
                grund: s.grund,
                veraltet: veraltet[s.knoten] || knoten[s.knoten].generation < aktuelle_generation,
            });
        }

        ansicht.zurueckgehalten = knoten
            .iter()
            .zip(&besucht)
            .filter(|(_, besucht)| !**besucht)
            .map(|(k, _)| k.id.to_owned())
            .collect();
        ansicht.luecken = fehlend
            .into_iter()
            .map(|(fehlt, kinder)| Luecke {
                fehlt: fehlt.to_owned(),
                kinder,
            })
            .collect();
        ansicht.konflikte = self.konflikte.iter().cloned().collect();
        ansicht.geloeschte = knoten
            .iter()
            .zip(&besucht)
            .filter(|(k, besucht)| **besucht && self.geloescht.contains_key(k.id))
            .map(|(k, _)| k.id.to_owned())
            .collect();
        ansicht
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(id: &str, eltern: Option<&str>, generation: u64) -> Beitrag {
        Beitrag::neu(
            id,
            eltern,
            generation,
            "autor",
            Modus::Typed,
            &format!("text {id}"),
        )
    }

    fn kette(n: usize) -> Vec<Beitrag> {
        (0..n)
            .map(|i| {
                let eltern = (i > 0).then(|| format!("m{:03}", i - 1));
                b(&format!("m{i:03}"), eltern.as_deref(), 1)
            })
            .collect()
    }

    #[test]
    fn kette_ist_hauptlinie_ohne_zweige() {
        let mut v = Verlauf::new();
        assert_eq!(v.zusammenfuehren(kette(50)), 50);
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie.len(), 50);
        assert!(a.zweige.is_empty() && a.luecken.is_empty() && a.zurueckgehalten.is_empty());
        assert_eq!(a.spitze(), Some("m049"));
    }

    #[test]
    fn einfuegen_ist_idempotent_und_reihenfolgeunabhaengig() {
        let mut vor = Verlauf::new();
        vor.zusammenfuehren(kette(10));
        let mut rueck = Verlauf::new();
        let mut umgekehrt = kette(10);
        umgekehrt.reverse();
        rueck.zusammenfuehren(umgekehrt.clone());
        rueck.zusammenfuehren(umgekehrt);
        assert_eq!(rueck.len(), 10, "keine Doppelung");
        assert_eq!(vor.ansicht(1), rueck.ansicht(1));
    }

    #[test]
    fn gleiche_id_anderer_inhalt_ist_konflikt_ohne_ueberschreiben() {
        let mut v = Verlauf::new();
        v.einfuegen(b("a", None, 1));
        let mut anders = b("a", None, 1);
        anders.text = "anderer Text".into();
        assert_eq!(v.einfuegen(anders), Einfuegen::Konflikt);
        assert_eq!(v.beitrag("a").unwrap().text, "text a");
        assert_eq!(v.ansicht(1).konflikte, vec!["a".to_owned()]);
    }

    #[test]
    fn luecke_haelt_zurueck_und_zeigt_fehlenden_elternteil() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("a", None, 1),
            b("c", Some("b"), 1),
            b("d", Some("c"), 1),
        ]);
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["a"]);
        assert_eq!(a.zurueckgehalten, vec!["c", "d"]);
        assert_eq!(
            a.luecken,
            vec![Luecke {
                fehlt: "b".into(),
                kinder: vec!["c".into()]
            }]
        );
        // Elternteil trifft nach: Luecke schliesst sich, nichts geht verloren.
        v.einfuegen(b("b", Some("a"), 1));
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["a", "b", "c", "d"]);
        assert!(a.luecken.is_empty() && a.zurueckgehalten.is_empty());
    }

    #[test]
    fn zyklus_wird_zurueckgehalten_statt_endlos_zu_laufen() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("w", None, 1),
            b("x", Some("y"), 1),
            b("y", Some("x"), 1),
            b("s", Some("s"), 1),
        ]);
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["w"]);
        assert_eq!(a.zurueckgehalten, vec!["s", "x", "y"]);
    }

    #[test]
    fn veralteter_beitrag_wird_zweig_nie_hauptlinie() {
        // Uebernahme waehrend laufender Antwort (E3.3 b): Antwort des alten Inhabers (Gen 1)
        // und erste Antwort des neuen (Gen 2) haengen am selben Elternteil.
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("frage", None, 1),
            b("alt", Some("frage"), 1),
            b("neu", Some("frage"), 2),
            b("neu2", Some("neu"), 2),
        ]);
        let a = v.ansicht(2);
        assert_eq!(a.hauptlinie, vec!["frage", "neu", "neu2"]);
        assert_eq!(a.zweige.len(), 1);
        let z = &a.zweige[0];
        assert_eq!(z.eintraege, vec!["alt"]);
        assert_eq!(z.abzweigung.as_deref(), Some("frage"));
        assert_eq!(z.grund, ZweigGrund::VeralteteGeneration);
        assert!(z.veraltet);
        // Reihenfolge des Eintreffens aendert nichts.
        let mut w = Verlauf::new();
        w.zusammenfuehren(vec![
            b("alt", Some("frage"), 1),
            b("neu2", Some("neu"), 2),
            b("neu", Some("frage"), 2),
            b("frage", None, 1),
        ]);
        assert_eq!(w.ansicht(2), a);
    }

    #[test]
    fn gleichstand_ist_zweig_und_nutzerwahl_dreht_um() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("p", None, 1),
            b("k1", Some("p"), 1),
            b("k2", Some("p"), 1),
        ]);
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["p", "k1"]);
        assert_eq!(a.zweige[0].grund, ZweigGrund::Gleichstand);
        assert_eq!(a.zweige[0].eintraege, vec!["k2"]);
        v.zweig_waehlen(Some("p"), "k2").unwrap();
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["p", "k2"]);
        assert_eq!(a.zweige[0].eintraege, vec!["k1"]);
        assert_eq!(
            v.zweig_waehlen(Some("p"), "p"),
            Err(SyncFehler::ZweigUngueltig)
        );
        assert_eq!(
            v.zweig_waehlen(Some("k1"), "k2"),
            Err(SyncFehler::ZweigUngueltig)
        );
    }

    #[test]
    fn zweite_wurzel_ist_zweig_ohne_abzweigung() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("w1", None, 1),
            b("w2", None, 1),
            b("x", Some("w2"), 1),
        ]);
        let a = v.ansicht(1);
        assert_eq!(a.zweige[0].abzweigung, None);
        assert_eq!(a.zweige[0].eintraege, vec!["w2", "x"]);
    }

    #[test]
    fn verlustfreiheit_ueber_alle_arten() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("a", None, 1),
            b("b", Some("a"), 1),
            b("c", Some("a"), 2),
            b("d", Some("c"), 2),
            b("e", Some("zzz"), 1),
            b("f", Some("e"), 1),
        ]);
        let a = v.ansicht(2);
        let erwartet: BTreeSet<&str> = ["a", "b", "c", "d", "e", "f"].into();
        assert_eq!(a.alle_ids(), erwartet);
    }

    #[test]
    fn geloeschter_beitrag_bleibt_platzhalter_und_lebt_nicht_wieder_auf() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![
            b("a", None, 1),
            b("b", Some("a"), 1),
            b("c", Some("b"), 1),
        ]);
        assert!(v.loeschen("b"));
        assert!(!v.loeschen("b"));
        assert_eq!(v.einfuegen(b("b", Some("a"), 1)), Einfuegen::Geloescht);
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["a", "b", "c"]);
        assert_eq!(a.geloeschte, vec!["b"]);
        assert!(a.luecken.is_empty());
        assert!(v.beitrag("b").is_none());
    }

    #[test]
    fn uebernahme_macht_spaetere_alte_beitraege_zu_zweigen_auch_ohne_geschwister() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(vec![b("frage", None, 1)]);
        assert!(v.uebernahme_merken(2, v.spitze(1)));
        assert!(!v.uebernahme_merken(2, None), "bekannte Generation bleibt");
        v.einfuegen(b("alt", Some("frage"), 1));
        let a = v.ansicht(2);
        assert_eq!(a.hauptlinie, vec!["frage"]);
        assert_eq!(a.spitze(), Some("frage"));
        assert_eq!(a.zweige.len(), 1);
        assert_eq!(a.zweige[0].grund, ZweigGrund::VeralteteGeneration);
        // Nachkommen der veralteten Antwort bleiben im Zweig.
        v.einfuegen(b("alt2", Some("alt"), 1));
        let a = v.ansicht(2);
        assert_eq!(a.zweige[0].eintraege, vec!["alt", "alt2"]);
        // Aktuelle Generation setzt die Hauptlinie fort, der Zweig bleibt.
        v.einfuegen(b("neu", Some("frage"), 2));
        let a = v.ansicht(2);
        assert_eq!(a.hauptlinie, vec!["frage", "neu"]);
        assert_eq!(a.zweige[0].eintraege, vec!["alt", "alt2"]);
        // Ausdrueckliche Nutzerwahl darf auch einen veralteten Zweig zur Hauptlinie machen.
        v.zweig_waehlen(Some("frage"), "alt").unwrap();
        assert_eq!(v.ansicht(2).hauptlinie, vec!["frage", "alt", "alt2"]);
    }

    #[test]
    fn uebernahme_auf_leerem_verlauf_und_vereinigung() {
        let mut v = Verlauf::new();
        assert!(v.uebernahme_merken(1, None));
        v.zusammenfuehren(vec![b("a", None, 0), b("b", None, 1)]);
        let a = v.ansicht(1);
        assert_eq!(
            a.hauptlinie,
            vec!["b"],
            "Generation 0 nach der Uebernahme ist veraltet"
        );
        assert_eq!(a.zweige[0].eintraege, vec!["a"]);
        let mut w = Verlauf::new();
        assert!(w.uebernahmen_vereinen(v.uebernahmen()));
        assert!(!w.uebernahmen_vereinen(v.uebernahmen()));
        assert_eq!(w.uebernahmen(), v.uebernahmen());
    }

    #[test]
    fn debug_zeigt_keinen_inhalt() {
        let s = format!("{:?}", b("a", None, 1));
        assert!(!s.contains("text a") && s.contains("verborgen"));
    }

    #[test]
    fn lange_ketten_laufen_ohne_rekursion() {
        let mut v = Verlauf::new();
        v.zusammenfuehren(kette(5000));
        assert_eq!(v.ansicht(1).hauptlinie.len(), 5000);
    }
}
