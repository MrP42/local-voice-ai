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
//! - Zweigwahl und Uebernahme sind unveraenderliche, signierte Ereignisse mit eindeutiger ID (E3.8,
//!   Must-fix 5), nicht Teil des ueberschreibbaren Kopfs. **Alle** gueltigen Ereignisse bleiben gespeichert;
//!   die wirksame Wahl wird bei jedem Zusammenfuehren aus der bekannten Menge neu abgeleitet und haengt nie
//!   von der Lieferreihenfolge ab: je Elternteil gewinnt die Wahl mit dem groessten `(takt, autor, ereignis)`;
//!   je Generation gilt die Uebernahme mit dem kleinsten `(basis, ausweis_hash, autor, ereignis)` (kanonische
//!   Gesamtordnung zusammen mit der Generation, keine Hub-Zaehler). Ein Grabstein entfernt ein Ereignis, die
//!   Wahl wird dann aus den uebrigen neu projiziert (Must-fix 6).
//! - **Konkurrierende Inhaber** (E3.3, boesartiger Hub): Haben fuer dieselbe Generation verschiedene Geraete
//!   eine Uebernahme signiert, gilt die Uebernahme nach obiger Ordnung **vorlaeufig**; die Beitraege der
//!   anderen Inhaber dieser Generation bleiben unveraendert erhalten und stehen als Zweig
//!   [`ZweigGrund::KonkurrierenderInhaber`]. Ein spaeter bekanntes, kleineres Ereignis kann den Gewinner
//!   wechseln; dabei geht nichts verloren und nichts wird automatisch reaktiviert oder umgehaengt.
//!   **Kausale Bindung (Runde 3):** Jede Uebernahme traegt signiert die Basis (Hash der Lease-Fassung, auf
//!   der sie aufsetzt) und die eigene Fassung (Hash der Lease-Fassung, die sie schreibt). Ueber die
//!   signierten Lease-Glieder (Hash -> Vorgaenger) wird von der Basis rueckwaerts die erzeugende Uebernahme
//!   einer frueheren Generation gesucht. Haengt eine Uebernahme so an einer unterlegenen, ist sie selbst
//!   unterlegen, und mit ihr alle weiteren Folgeuebernahmen des Asts: sie bleiben gespeichert, werden aber
//!   nicht automatisch wirksam. Ohne bekannte Bindung (Glieder fehlen) gilt nur die Wahl je Generation. Erkannt
//!   wird die Konkurrenz erst, wenn die Ereignisse bekannt sind; ein Hub mit geteilter Sicht kann sie
//!   dauerhaft verbergen.
//! - Eine Antwort behaelt die Generation, unter der sie begonnen wurde (auch ueber Lease-Verlaengerungen),
//!   und verzweigt deshalb nicht.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};

use super::fehler::SyncFehler;
use super::kennung::{aus_hex, hex, hexfeld};

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
#[serde(deny_unknown_fields)]
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
    /// Beitrag eines Inhabers, dessen Uebernahme derselben Generation vorlaeufig unterlegen ist
    /// (konkurrierende Inhaber, nur bei boesartigem Hub oder protokollwidrigem Geraet).
    KonkurrierenderInhaber,
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

/// Herkunft eines Verlaufsereignisses: authentifizierter Autor und eindeutige Ereignis-ID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Herkunft {
    pub autor: String,
    pub ereignis: String,
}

/// Ereignis "Zweig gewaehlt" (Inhalt, unveraenderlich).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Zweigwahl {
    pub eltern: Option<String>,
    pub kind: String,
    /// Lamport-Takt: 1 + groesster bekannter Takt einer Zweigwahl dieser Unterhaltung.
    pub takt: u64,
}

/// Ereignis "Uebernahme" (Inhalt, unveraenderlich).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Uebernahme {
    pub generation: u64,
    /// Hauptlinienspitze zum Zeitpunkt der Uebernahme (`None` = leer).
    pub spitze: Option<String>,
    /// Signierter Uebernahmekontext: Inhaltshash der Lease-Fassung, auf der die Uebernahme aufbaut
    /// (Nullen fuer die erste Lease). Zwei Uebernahmen derselben Generation auf demselben Stand teilen ihn.
    #[serde(with = "hexfeld")]
    pub basis: [u8; 32],
    /// Inhaltshash der Lease-Fassung, die diese Uebernahme schreibt (kausale Bindung der Folgeuebernahmen).
    #[serde(with = "hexfeld")]
    pub fassung: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Gewaehlt {
    kind: String,
    takt: u64,
    herkunft: Herkunft,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Uebernommen {
    spitze: Option<String>,
    #[serde(with = "hexfeld")]
    basis: [u8; 32],
    #[serde(with = "hexfeld")]
    fassung: [u8; 32],
    /// Hash des gepruften Ausweises des Autors (authentisierter Autor).
    #[serde(with = "hexfeld")]
    ausweis: [u8; 32],
    herkunft: Herkunft,
}

impl Uebernommen {
    /// Kanonischer Rang innerhalb einer Generation: kleiner gewinnt.
    fn rang(&self) -> (&[u8; 32], &[u8; 32], &Herkunft) {
        (&self.basis, &self.ausweis, &self.herkunft)
    }
}

/// Konkurrierende Inhaber einer Generation (vorlaeufige Wahl, siehe Moduldoku).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Konkurrenz {
    /// Geraet mit der vorlaeufig wirksamen Uebernahme; `None`, wenn alle Uebernahmen dieser Generation auf
    /// einem unterlegenen Ast haengen.
    pub gewinner: Option<String>,
    /// Uebrige Geraete mit Uebernahmen derselben Generation (auch Folgeuebernahmen unterlegener Aeste).
    pub unterlegen: BTreeSet<String>,
}

/// Ergebnis der Uebernahme-Auswertung: wirksame je Generation und unterlegene Ereignisse.
struct Auswertung<'a> {
    wirksam: BTreeMap<u64, &'a Uebernommen>,
    unterlegen: BTreeSet<&'a str>,
}

/// Gespraechsbaum einer Unterhaltung.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Verlauf {
    beitraege: BTreeMap<String, Beitrag>,
    /// Geloeschte Beitraege: nur Struktur (Elternlink, Generation), kein Inhalt.
    geloescht: BTreeMap<String, (Option<String>, u64)>,
    konflikte: BTreeSet<String>,
    /// Alle bekannten Zweigwahlen je Elternbeitrag (`""` = Wurzelebene); wirksam ist die groesste.
    wahl: BTreeMap<String, Vec<Gewaehlt>>,
    /// Alle bekannten Uebernahmen je Generation; wirksam ist die kleinste nach [`Uebernommen::rang`]. Ihre
    /// Spitze ist die Hauptlinie zum Zeitpunkt der Uebernahme (`None` = leer); Beitraege mit kleinerer
    /// Generation darunter sind veraltet (E3.3).
    uebernahmen: BTreeMap<u64, Vec<Uebernommen>>,
    /// Authentisierter Schreiber (Geraet) je Beitrag, fuer konkurrierende Inhaber.
    #[serde(default)]
    schreiber: BTreeMap<String, String>,
    /// Signierte Lease-Glieder (Hash -> Vorgaenger, hex) fuer die kausale Bindung der Uebernahmen.
    #[serde(default)]
    lease_glieder: BTreeMap<String, String>,
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
    unterlegen: &[bool],
) -> ZweigGrund {
    if unterlegen[kind] {
        return ZweigGrund::KonkurrierenderInhaber;
    }
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
    unterlegen: &'a [bool],
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
            let grund = if baum.unterlegen[kind] {
                ZweigGrund::KonkurrierenderInhaber
            } else if abgang == 0 {
                ZweigGrund::VeralteteGeneration
            } else {
                zweiggrund(
                    kind,
                    Some(liste[0]),
                    baum.knoten,
                    baum.veraltet,
                    baum.unterlegen,
                )
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
    ///
    /// Erzeugt das Ereignis mit dem naechsten Takt und ordnet es sofort ein; der Aufrufer laedt es hoch.
    pub fn zweig_waehlen(
        &mut self,
        eltern: Option<&str>,
        kind: &str,
        herkunft: Herkunft,
    ) -> Result<Zweigwahl, SyncFehler> {
        let w = self.zweigwahl_bauen(eltern, kind)?;
        self.wahl_einordnen(&w, herkunft);
        Ok(w)
    }

    /// Baut die naechste Zweigwahl (naechster Takt), ohne sie einzuordnen.
    pub fn zweigwahl_bauen(
        &self,
        eltern: Option<&str>,
        kind: &str,
    ) -> Result<Zweigwahl, SyncFehler> {
        let ist_kind = self
            .beitraege
            .get(kind)
            .is_some_and(|b| b.eltern_id.as_deref() == eltern);
        if !ist_kind {
            return Err(SyncFehler::ZweigUngueltig);
        }
        let takt = self
            .wahl
            .values()
            .flatten()
            .map(|w| w.takt)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(SyncFehler::Vertrag("takt ueberlaeuft"))?;
        Ok(Zweigwahl {
            eltern: eltern.map(str::to_owned),
            kind: kind.to_owned(),
            takt,
        })
    }

    /// Ordnet eine Zweigwahl ein (reihenfolgeunabhaengig, alle Ereignisse bleiben). `true`, wenn sie jetzt
    /// die wirksame ist.
    pub fn wahl_einordnen(&mut self, w: &Zweigwahl, herkunft: Herkunft) -> bool {
        let schluessel = w.eltern.clone().unwrap_or_default();
        let liste = self.wahl.entry(schluessel.clone()).or_default();
        if !liste.iter().any(|g| g.herkunft == herkunft) {
            liste.push(Gewaehlt {
                kind: w.kind.clone(),
                takt: w.takt,
                herkunft: herkunft.clone(),
            });
        }
        self.wirksame_wahl(&schluessel)
            .is_some_and(|g| g.herkunft == herkunft)
    }

    fn wirksame_wahl(&self, schluessel: &str) -> Option<&Gewaehlt> {
        self.wahl
            .get(schluessel)?
            .iter()
            .max_by(|a, b| (a.takt, &a.herkunft).cmp(&(b.takt, &b.herkunft)))
    }

    /// Herkunft (Autor, Ereignis-ID) der wirksamen Zweigwahl an `eltern` (`None` = Wurzelebene).
    pub fn wahl_herkunft(&self, eltern: Option<&str>) -> Option<&Herkunft> {
        self.wirksame_wahl(eltern.unwrap_or_default())
            .map(|g| &g.herkunft)
    }

    /// Herkunft der wirksamen Uebernahme einer Generation.
    pub fn uebernahme_herkunft(&self, generation: u64) -> Option<&Herkunft> {
        self.auswertung()
            .wirksam
            .get(&generation)
            .map(|u| &u.herkunft)
    }

    /// Wirksame Wahl: Elternbeitrag (`""` = Wurzelebene) -> gewaehltes Kind.
    pub fn wahl(&self) -> BTreeMap<String, String> {
        self.wahl
            .keys()
            .filter_map(|e| self.wirksame_wahl(e).map(|w| (e.clone(), w.kind.clone())))
            .collect()
    }

    /// Entfernt ein Verlaufsereignis (Zweigwahl oder Uebernahme) nach einem Grabstein; die wirksame Wahl
    /// ergibt sich danach aus den uebrigen Ereignissen (Must-fix 6). `true`, wenn etwas entfernt wurde.
    /// Autor eines bekannten Verlaufsereignisses (Zweigwahl oder Uebernahme), falls gespeichert.
    pub fn ereignis_autor(&self, ereignis: &str) -> Option<&str> {
        self.wahl
            .values()
            .flatten()
            .map(|g| &g.herkunft)
            .chain(self.uebernahmen.values().flatten().map(|u| &u.herkunft))
            .find(|h| h.ereignis == ereignis)
            .map(|h| h.autor.as_str())
    }

    pub fn ereignis_entfernen(&mut self, ereignis: &str) -> bool {
        let mut entfernt = false;
        for liste in self.wahl.values_mut() {
            let vorher = liste.len();
            liste.retain(|g| g.herkunft.ereignis != ereignis);
            entfernt |= liste.len() != vorher;
        }
        for liste in self.uebernahmen.values_mut() {
            let vorher = liste.len();
            liste.retain(|u| u.herkunft.ereignis != ereignis);
            entfernt |= liste.len() != vorher;
        }
        self.wahl.retain(|_, l| !l.is_empty());
        self.uebernahmen.retain(|_, l| !l.is_empty());
        entfernt
    }

    /// Merkt den authentisierten Schreiber (Geraet) eines Beitrags; der erste bekannte bleibt.
    pub fn schreiber_merken(&mut self, beitrag_id: &str, geraet: &str) {
        self.schreiber
            .entry(beitrag_id.to_owned())
            .or_insert_with(|| geraet.to_owned());
    }

    /// Authentisierter Schreiber eines Beitrags, falls bekannt.
    pub fn schreiber(&self, beitrag_id: &str) -> Option<&str> {
        self.schreiber.get(beitrag_id).map(String::as_str)
    }

    /// Hauptlinienspitze unter Beruecksichtigung der bekannten Uebernahmen (Zweige zaehlen nicht).
    pub fn spitze(&self, aktuelle_generation: u64) -> Option<String> {
        self.ansicht(aktuelle_generation)
            .spitze()
            .map(str::to_owned)
    }

    /// Ordnet eine Uebernahme ein: Beitraege mit kleinerer Generation unterhalb von `spitze` sind veraltet.
    ///
    /// Alle Ereignisse bleiben gespeichert. Je Generation gilt vorlaeufig das kleinste nach
    /// `(basis, ausweis, autor, ereignis)`; `ausweis` ist der Hash des gepruften Ausweises des Autors. Die Wahl
    /// haengt nur von der bekannten Menge ab, nie von der Reihenfolge, und kann sich durch ein spaeter
    /// bekanntes Ereignis aendern. `true`, wenn sich die wirksame Uebernahme dadurch geaendert hat.
    pub fn uebernahme_einordnen(
        &mut self,
        u: &Uebernahme,
        herkunft: Herkunft,
        ausweis: [u8; 32],
    ) -> bool {
        let vorher = self.wirksame_herkunft();
        let liste = self.uebernahmen.entry(u.generation).or_default();
        if !liste.iter().any(|x| x.herkunft == herkunft) {
            liste.push(Uebernommen {
                spitze: u.spitze.clone(),
                basis: u.basis,
                fassung: u.fassung,
                ausweis,
                herkunft,
            });
        }
        self.wirksame_herkunft() != vorher
    }

    fn wirksame_herkunft(&self) -> BTreeMap<u64, Herkunft> {
        self.auswertung()
            .wirksam
            .into_iter()
            .map(|(g, u)| (g, u.herkunft.clone()))
            .collect()
    }

    /// Merkt ein signiertes Lease-Glied (Hash der Fassung -> Hash der Vorgaengerfassung). Nur Fakten aus
    /// gepruften Fassungen; ein Glied, das schon bekannt ist, bleibt unveraendert.
    pub fn lease_glied_merken(&mut self, hash: [u8; 32], vorgaenger: [u8; 32]) {
        self.lease_glieder
            .entry(hex(&hash))
            .or_insert_with(|| hex(&vorgaenger));
    }

    /// Erzeugende Uebernahme einer frueheren Generation: von der Basis rueckwaerts ueber die Lease-Glieder
    /// bis zur ersten Fassung, die eine Uebernahme geschrieben hat. `None`, wenn die Kette abbricht.
    fn eltern<'a>(
        &self,
        u: &Uebernommen,
        generation: u64,
        nach_fassung: &BTreeMap<[u8; 32], (u64, &'a Uebernommen)>,
    ) -> Option<&'a Uebernommen> {
        let mut h = u.basis;
        for _ in 0..=self.lease_glieder.len() {
            if h == [0; 32] {
                return None;
            }
            if let Some((g, e)) = nach_fassung.get(&h) {
                return (*g < generation).then_some(*e);
            }
            h = aus_hex::<32>(self.lease_glieder.get(&hex(&h))?)?;
        }
        None
    }

    /// Wertet alle Uebernahmen aus, Generation fuer Generation aufsteigend: Haengt eine Uebernahme kausal an
    /// einer unterlegenen, ist sie unterlegen; unter den uebrigen gewinnt je Generation die kleinste nach
    /// [`Uebernommen::rang`], die anderen sind unterlegen. Haengt nur von der Menge ab.
    fn auswertung(&self) -> Auswertung<'_> {
        let nach_fassung: BTreeMap<[u8; 32], (u64, &Uebernommen)> = self
            .uebernahmen
            .iter()
            .flat_map(|(g, l)| l.iter().map(move |u| (u.fassung, (*g, u))))
            .filter(|(f, _)| *f != [0; 32])
            .collect();
        let mut a = Auswertung {
            wirksam: BTreeMap::new(),
            unterlegen: BTreeSet::new(),
        };
        for (g, liste) in &self.uebernahmen {
            let mut kandidaten = Vec::new();
            for u in liste {
                let ast_verloren = self
                    .eltern(u, *g, &nach_fassung)
                    .is_some_and(|e| a.unterlegen.contains(e.herkunft.ereignis.as_str()));
                if ast_verloren {
                    a.unterlegen.insert(u.herkunft.ereignis.as_str());
                } else {
                    kandidaten.push(u);
                }
            }
            let Some(w) = kandidaten
                .iter()
                .copied()
                .min_by(|x, y| x.rang().cmp(&y.rang()))
            else {
                continue;
            };
            a.wirksam.insert(*g, w);
            for k in kandidaten {
                if k.herkunft != w.herkunft {
                    a.unterlegen.insert(k.herkunft.ereignis.as_str());
                }
            }
        }
        a
    }

    fn hoechste_uebernahme(&self) -> u64 {
        self.auswertung()
            .wirksam
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
    }

    /// Wirksame Uebernahmen: Generation -> Spitze.
    pub fn uebernahmen(&self) -> BTreeMap<u64, Option<String>> {
        self.auswertung()
            .wirksam
            .into_iter()
            .map(|(g, u)| (g, u.spitze.clone()))
            .collect()
    }

    /// Ereignis-IDs unterlegener Uebernahmen (auch Folgeuebernahmen unterlegener Aeste); sie bleiben
    /// gespeichert, werden aber nicht automatisch wirksam.
    pub fn unterlegene_uebernahmen(&self) -> BTreeSet<String> {
        self.auswertung()
            .unterlegen
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    /// Anzahl aller gespeicherten Uebernahme-Ereignisse (auch unterlegener).
    pub fn uebernahme_ereignisse(&self) -> usize {
        self.uebernahmen.values().map(Vec::len).sum()
    }

    /// Generationen mit konkurrierenden Inhabern: mehr als ein Geraet hat eine Uebernahme signiert, oder
    /// eine Uebernahme haengt an einem unterlegenen Ast.
    pub fn konkurrenz(&self) -> BTreeMap<u64, Konkurrenz> {
        let a = self.auswertung();
        self.uebernahmen
            .iter()
            .filter_map(|(g, liste)| {
                let gewinner = a.wirksam.get(g).map(|u| u.herkunft.autor.clone());
                let unterlegen: BTreeSet<String> = liste
                    .iter()
                    .filter(|u| a.unterlegen.contains(u.herkunft.ereignis.as_str()))
                    .map(|u| u.herkunft.autor.clone())
                    .filter(|x| gewinner.as_ref() != Some(x))
                    .collect();
                (!unterlegen.is_empty()).then_some((
                    *g,
                    Konkurrenz {
                        gewinner,
                        unterlegen,
                    },
                ))
            })
            .collect()
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
        let konkurrenz = self.konkurrenz();
        let unterlegen: Vec<bool> = knoten
            .iter()
            .map(|k| {
                konkurrenz.get(&k.generation).is_some_and(|x| {
                    self.schreiber
                        .get(k.id)
                        .is_some_and(|s| x.unterlegen.contains(s))
                })
            })
            .collect();
        for (i, u) in unterlegen.iter().enumerate() {
            veraltet[i] |= *u;
        }
        let auswertung = self.auswertung();
        let wirksam: Vec<(u64, &Uebernommen)> = auswertung
            .wirksam
            .range(..=aktuelle_generation)
            .map(|(g, u)| (*g, *u))
            .collect();
        for (generation, u) in wirksam {
            let wurzel = match &u.spitze {
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

        let wahl = self.wahl();
        let baum = Baum {
            knoten: &knoten,
            kinder: &kinder,
            veraltet: &veraltet,
            unterlegen: &unterlegen,
            wahl: &wahl,
        };
        let mut besucht = vec![false; knoten.len()];
        let mut warteschlange: VecDeque<Start> = VecDeque::new();
        let mut ansicht = Ansicht::default();
        if let Some(wurzeln) = kinder.get(&None) {
            let mut wurzeln = wurzeln.clone();
            let fortsetzen = rangordnen(&mut wurzeln, &knoten, &veraltet, &wahl, "");
            let abgang = if fortsetzen { 1 } else { 0 };
            for &w in &wurzeln[abgang..] {
                warteschlange.push_back(Start {
                    knoten: w,
                    abzweigung: None,
                    grund: if unterlegen[w] {
                        ZweigGrund::KonkurrierenderInhaber
                    } else if abgang == 0 {
                        ZweigGrund::VeralteteGeneration
                    } else {
                        zweiggrund(w, Some(wurzeln[0]), &knoten, &veraltet, &unterlegen)
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

    fn h(autor: &str, ereignis: &str) -> Herkunft {
        Herkunft {
            autor: autor.into(),
            ereignis: ereignis.into(),
        }
    }

    fn merken(v: &mut Verlauf, generation: u64, spitze: Option<String>) -> bool {
        v.uebernahme_einordnen(
            &Uebernahme {
                generation,
                spitze,
                basis: [0; 32],
                fassung: [0; 32],
            },
            h("pc", &format!("u{generation}")),
            [1; 32],
        )
    }

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
        v.zweig_waehlen(Some("p"), "k2", h("pc", "e1")).unwrap();
        let a = v.ansicht(1);
        assert_eq!(a.hauptlinie, vec!["p", "k2"]);
        assert_eq!(a.zweige[0].eintraege, vec!["k1"]);
        assert_eq!(
            v.zweig_waehlen(Some("p"), "p", h("pc", "e2")),
            Err(SyncFehler::ZweigUngueltig)
        );
        assert_eq!(
            v.zweig_waehlen(Some("k1"), "k2", h("pc", "e3")),
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
        let s = v.spitze(1);
        assert!(merken(&mut v, 2, s));
        assert!(!merken(&mut v, 2, None), "gleiche Herkunft aendert nichts");
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
        v.zweig_waehlen(Some("frage"), "alt", h("pc", "e1"))
            .unwrap();
        assert_eq!(v.ansicht(2).hauptlinie, vec!["frage", "alt", "alt2"]);
    }

    #[test]
    fn uebernahme_auf_leerem_verlauf_und_vereinigung() {
        let mut v = Verlauf::new();
        assert!(merken(&mut v, 1, None));
        v.zusammenfuehren(vec![b("a", None, 0), b("b", None, 1)]);
        let a = v.ansicht(1);
        assert_eq!(
            a.hauptlinie,
            vec!["b"],
            "Generation 0 nach der Uebernahme ist veraltet"
        );
        assert_eq!(a.zweige[0].eintraege, vec!["a"]);
    }

    #[test]
    fn ereignisse_sind_reihenfolgeunabhaengig() {
        let wahlen = [
            (
                Zweigwahl {
                    eltern: Some("p".into()),
                    kind: "k1".into(),
                    takt: 1,
                },
                h("pc", "a"),
            ),
            (
                Zweigwahl {
                    eltern: Some("p".into()),
                    kind: "k2".into(),
                    takt: 2,
                },
                h("iphone", "b"),
            ),
            (
                Zweigwahl {
                    eltern: Some("p".into()),
                    kind: "k1".into(),
                    takt: 2,
                },
                h("pc", "c"),
            ),
        ];
        let uebernahmen = [
            (
                Uebernahme {
                    generation: 2,
                    spitze: Some("x".into()),
                    basis: [4; 32],
                    fassung: [0; 32],
                },
                h("pc", "u1"),
            ),
            (
                Uebernahme {
                    generation: 2,
                    spitze: Some("y".into()),
                    basis: [4; 32],
                    fassung: [0; 32],
                },
                h("iphone", "u2"),
            ),
        ];
        let mut ergebnisse = Vec::new();
        for perm in [[0, 1, 2], [2, 1, 0], [1, 0, 2], [1, 2, 0]] {
            let mut v = Verlauf::new();
            for i in perm {
                v.wahl_einordnen(&wahlen[i].0, wahlen[i].1.clone());
                if i < 2 {
                    v.uebernahme_einordnen(&uebernahmen[i].0, uebernahmen[i].1.clone(), [9; 32]);
                    v.uebernahme_einordnen(
                        &uebernahmen[1 - i].0,
                        uebernahmen[1 - i].1.clone(),
                        [9; 32],
                    );
                }
            }
            ergebnisse.push((v.wahl(), v.uebernahmen()));
        }
        assert!(ergebnisse.windows(2).all(|w| w[0] == w[1]));
        // Takt 2 gleichauf: (autor, ereignis) entscheidet; "pc" > "iphone".
        assert_eq!(ergebnisse[0].0["p"], "k1");
        assert_eq!(
            ergebnisse[0].1[&2],
            Some("y".into()),
            "gleiche Basis und Ausweis-Hash: kleinste Herkunft (iphone)"
        );
    }

    fn ue(generation: u64, spitze: &str, basis: u8) -> Uebernahme {
        Uebernahme {
            generation,
            spitze: Some(spitze.into()),
            basis: [basis; 32],
            fassung: [0; 32],
        }
    }

    /// Codex-Bedingung 2: gleiche Menge, beliebige Reihenfolge, gleiche Wahl; auch fuer verschiedene Mengen
    /// wird immer aus der jeweils bekannten Menge gewaehlt.
    #[test]
    fn konkurrenzwahl_haengt_nur_von_der_menge_ab() {
        let ereignisse = [
            (ue(3, "a", 5), h("dev-pc", "e1"), [3; 32]),
            (ue(3, "b", 5), h("dev-iphone", "e2"), [2; 32]),
            (ue(3, "c", 4), h("dev-tablet", "e3"), [9; 32]),
            (ue(2, "d", 0), h("dev-pc", "e4"), [3; 32]),
        ];
        let perms: [[usize; 4]; 6] = [
            [0, 1, 2, 3],
            [3, 2, 1, 0],
            [1, 3, 0, 2],
            [2, 0, 3, 1],
            [0, 2, 1, 3],
            [3, 1, 2, 0],
        ];
        for teilmenge in [vec![0, 1], vec![0, 1, 2], vec![0, 1, 2, 3], vec![1, 3]] {
            let mut ergebnis = Vec::new();
            for p in &perms {
                let mut v = Verlauf::new();
                for i in p.iter().filter(|i| teilmenge.contains(i)) {
                    let (u, hk, a) = &ereignisse[*i];
                    v.uebernahme_einordnen(u, hk.clone(), *a);
                }
                ergebnis.push((v.uebernahmen(), v.konkurrenz()));
            }
            assert!(ergebnis.windows(2).all(|w| w[0] == w[1]), "{teilmenge:?}");
        }
        // Basis vor Ausweis-Hash: e3 (Basis 4) gewinnt gegen e2 (Basis 5, kleinerer Ausweis).
        let mut v = Verlauf::new();
        for (u, hk, a) in &ereignisse {
            v.uebernahme_einordnen(u, hk.clone(), *a);
        }
        assert_eq!(v.uebernahmen()[&3], Some("c".into()));
        assert_eq!(v.konkurrenz()[&3].gewinner.as_deref(), Some("dev-tablet"));
        assert_eq!(
            v.uebernahme_ereignisse(),
            4,
            "unterlegene Ereignisse bleiben gespeichert"
        );
    }

    /// Codex-Bedingung 3/4: ein spaeter bekanntes, kleineres Ereignis wechselt den vorlaeufigen Gewinner;
    /// die Beitraege beider Inhaber bleiben unveraendert, der unterlegene steht als Zweig.
    #[test]
    fn spaeter_bekanntes_ereignis_wechselt_den_gewinner_ohne_verlust() {
        let mut v = Verlauf::new();
        v.einfuegen(b("w", None, 1));
        let mut pc = b("p1", Some("w"), 2);
        pc.text = "vom pc".into();
        v.einfuegen(pc);
        v.schreiber_merken("p1", "dev-pc");
        v.einfuegen(b("i1", Some("w"), 2));
        v.schreiber_merken("i1", "dev-iphone");
        v.schreiber_merken("w", "dev-pc");
        v.uebernahme_einordnen(&ue(2, "w", 7), h("dev-pc", "e-pc"), [5; 32]);
        let a = v.ansicht(2);
        assert_eq!(
            a.hauptlinie,
            ["w", "i1"],
            "ohne Konkurrenz: Gleichstand, kleinste ID"
        );
        assert!(v.uebernahme_einordnen(&ue(2, "w", 7), h("dev-iphone", "e-ip"), [1; 32]));
        let a = v.ansicht(2);
        assert_eq!(a.hauptlinie, ["w", "i1"]);
        let z = a.zweige.iter().find(|z| z.eintraege == ["p1"]).unwrap();
        assert_eq!(z.grund, ZweigGrund::KonkurrierenderInhaber);
        // Ein noch kleineres Ereignis des pc wechselt den Gewinner zurueck.
        assert!(v.uebernahme_einordnen(&ue(2, "w", 3), h("dev-pc", "e-pc2"), [5; 32]));
        let a = v.ansicht(2);
        assert_eq!(a.hauptlinie, ["w", "p1"]);
        let z = a.zweige.iter().find(|z| z.eintraege == ["i1"]).unwrap();
        assert_eq!(z.grund, ZweigGrund::KonkurrierenderInhaber);
        assert_eq!(v.beitrag("p1").unwrap().text, "vom pc");
        assert_eq!(a.alle_ids().len(), 3, "nichts verloren");
        assert_eq!(v.uebernahme_ereignisse(), 3);
    }

    fn uef(generation: u64, spitze: &str, basis: u8, fassung: u8) -> Uebernahme {
        Uebernahme {
            generation,
            spitze: Some(spitze.into()),
            basis: [basis; 32],
            fassung: [fassung; 32],
        }
    }

    /// R3 Rest 5 (MF5): Folgeuebernahmen eines unterlegenen Asts sind ueber die Basis-Hash-Kette an ihn
    /// gebunden und werden ebenfalls unterlegen (auch transitiv und auch dann, wenn sie in ihrer Generation
    /// nach Rang gewinnen wuerden). Sie bleiben gespeichert, werden nicht wirksam, und das Ergebnis haengt nicht
    /// von der Reihenfolge der Ereignisse und Glieder ab.
    #[test]
    fn folgeuebernahmen_eines_unterlegenen_asts_sind_unterlegen() {
        // Generation 1: A (pc) gewinnt gegen B (iphone) nach Ausweis-Hash. Beide schreiben Fassungen, die
        // der jeweilige Inhaber danach verlaengert (A1 -> A2, B1 -> B2).
        // Generation 2: C (tablet) setzt auf B2 auf (kleinere Basis, wuerde nach Rang gewinnen), E (uhr) auf A2.
        // Generation 3: D (uhr) setzt auf C auf.
        let ereignisse = [
            (uef(1, "w", 0, 0xa1), h("dev-pc", "A"), [1; 32]),
            (uef(1, "w", 0, 0x11), h("dev-iphone", "B"), [9; 32]),
            (uef(2, "c", 0x12, 0xc1), h("dev-tablet", "C"), [2; 32]),
            (uef(2, "e", 0xa2, 0xe1), h("dev-uhr", "E"), [3; 32]),
            (uef(3, "d", 0xc1, 0xd1), h("dev-uhr", "D"), [3; 32]),
        ];
        let glieder = [([0xa2; 32], [0xa1; 32]), ([0x12; 32], [0x11; 32])];
        let perms: [[usize; 5]; 4] = [
            [0, 1, 2, 3, 4],
            [4, 3, 2, 1, 0],
            [2, 4, 0, 3, 1],
            [3, 0, 4, 1, 2],
        ];
        let mut ergebnis = Vec::new();
        for (n, p) in perms.iter().enumerate() {
            let mut v = Verlauf::new();
            v.einfuegen(b("w", None, 1));
            v.einfuegen(b("c1", Some("w"), 2));
            v.schreiber_merken("c1", "dev-tablet");
            v.einfuegen(b("e1", Some("w"), 2));
            v.schreiber_merken("e1", "dev-uhr");
            // Glieder vor, zwischen oder nach den Ereignissen.
            if n % 2 == 0 {
                for (x, y) in glieder {
                    v.lease_glied_merken(x, y);
                }
            }
            for i in p {
                let (u, hk, a) = &ereignisse[*i];
                v.uebernahme_einordnen(u, hk.clone(), *a);
            }
            if n % 2 == 1 {
                for (x, y) in glieder {
                    v.lease_glied_merken(x, y);
                }
            }
            ergebnis.push((
                v.uebernahmen(),
                v.konkurrenz(),
                v.unterlegene_uebernahmen(),
                v.ansicht(0),
            ));
        }
        assert!(
            ergebnis
                .windows(2)
                .all(|w| w[0].0 == w[1].0 && w[0].1 == w[1].1 && w[0].2 == w[1].2)
        );
        let (wirksam, konkurrenz, unterlegen, ansicht) = &ergebnis[0];
        assert_eq!(
            wirksam.len(),
            2,
            "Generation 3 haengt nur am unterlegenen Ast"
        );
        assert_eq!(wirksam[&1], Some("w".into()));
        assert_eq!(
            wirksam[&2],
            Some("e".into()),
            "C waere nach Rang vorn, ist aber gebunden"
        );
        assert_eq!(
            unterlegen.iter().map(String::as_str).collect::<Vec<_>>(),
            ["B", "C", "D"]
        );
        assert_eq!(konkurrenz[&2].gewinner.as_deref(), Some("dev-uhr"));
        assert_eq!(konkurrenz[&3].gewinner, None);
        assert!(konkurrenz[&2].unterlegen.contains("dev-tablet"));
        // Der Beitrag des unterlegenen Asts bleibt erhalten und steht als Zweig.
        assert_eq!(ansicht.hauptlinie, ["w", "e1"]);
        let z = ansicht
            .zweige
            .iter()
            .find(|z| z.eintraege == ["c1"])
            .unwrap();
        assert_eq!(z.grund, ZweigGrund::KonkurrierenderInhaber);

        // Ohne bekannte Glieder ist C nicht gebunden: dann gilt nur der Rang je Generation.
        let mut v = Verlauf::new();
        for (u, hk, a) in &ereignisse {
            v.uebernahme_einordnen(u, hk.clone(), *a);
        }
        assert_eq!(v.uebernahmen()[&2], Some("c".into()));
        assert_eq!(v.uebernahme_ereignisse(), 5, "nichts geht verloren");
    }

    #[test]
    fn grabstein_entfernt_ereignis_und_projiziert_neu() {
        let mut v = Verlauf::new();
        v.einfuegen(b("p", None, 1));
        v.einfuegen(b("k1", Some("p"), 1));
        v.einfuegen(b("k2", Some("p"), 1));
        let w1 = Zweigwahl {
            eltern: Some("p".into()),
            kind: "k2".into(),
            takt: 1,
        };
        let w2 = Zweigwahl {
            eltern: Some("p".into()),
            kind: "k1".into(),
            takt: 2,
        };
        v.wahl_einordnen(&w1, h("pc", "a"));
        v.wahl_einordnen(&w2, h("pc", "b"));
        assert_eq!(v.wahl()["p"], "k1");
        assert!(v.ereignis_entfernen("b"));
        assert_eq!(v.wahl()["p"], "k2", "die naechste Wahl wird wirksam");
        assert!(v.ereignis_entfernen("a"));
        assert!(v.wahl().is_empty());
        assert!(!v.ereignis_entfernen("a"));
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
