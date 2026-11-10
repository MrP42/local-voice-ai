//! Geraeteseitiger Abgleich: lokaler Stand, Hochladen, Herunterladen, Loeschen, Widerruf, Kopplung und
//! Lease-Zugriff.
//!
//! Der Abgleich haelt je Unterhaltung Kopf, Verlauf und Einschraenkung und spricht ueber
//! [`HubSpeicher`] mit dem Hub. Es gelten durchgaengig:
//! - Vor der Sync-Freigabe geht nichts an den Hub und es wird nichts vom Hub gelesen (E3.6).
//! - `geraet_lokal`-Unterhaltungen werden nie hochgeladen; vertrauliche Beitraege darin erst recht nicht (E3.5, E10).
//! - Inhalte werden erst beim Hochladen mit der **aktuellen** Epoche versiegelt; nach einem Widerruf
//!   ueberleben so auch offline entstandene Beitraege.
//! - Der Hub ist nicht vertrauenswuerdig (E3.8): Epoche und Mitgliedschaft kommen aus dem lokal geprueften
//!   Geraeteverzeichnis, jedes Objekt ist vom Autor signiert, veraenderliche Objekte tragen eine
//!   authentifizierte Version, lokale Hoechststaende verhindern Rollback. Der Zustand wird ueber
//!   [`Zustandsspeicher`] dauerhaft gesichert.
//! - Fehler und Berichte enthalten keine Inhalte.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::Zeroizing;

use super::fehler::SyncFehler;
use super::freigabe::{Einschraenkung, SyncFreigabe, Verbot, hochladen_erlaubt};
use super::grabstein::Grabsteine;
use super::hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
use super::identitaet::{Ausweis, Geraeteschluessel};
use super::kennung::{Hex128, hash_teile, hex, hexfeld, json_geheim};
use super::kette::{self, Beleg, Einordnung};
use super::migration::Migrationsmarke;
use super::paket::{Schluesselpaket, entpacken, wrappen};
use super::schluessel::{Schluessel, Schluesselbund, gleich};
use super::speicher::Zustandsspeicher;
use super::umschlag::{
    Geoeffnet, Glied, Sammlung, Steuerfelder, Umschlag, Ziel, ZielTyp, oeffnen, versiegeln,
};
use super::verlauf::{Beitrag, Einfuegen, Herkunft, Uebernahme, Verlauf, Zweigwahl};
use super::verzeichnis::{Verzeichnis, Verzeichniseintrag, Widerrufen};

/// Version des gesicherten Zustandsformats.
const ZUSTANDSFORMAT: u32 = 6;

/// Hoechstzahl der Runden, in denen ein Widerruf gegen gleichzeitige, gepruefte Verzeichniseintraege
/// neu vorbereitet wird.
const WIDERRUF_RUNDEN: usize = 4;

/// Revisionen ab hier gelten als Cursor-Anomalie (E3.8, Must-fix 5): kein ehrlicher Hub erreicht sie.
const REVISION_GRENZE: u64 = 1 << 62;

/// Kopf einer Unterhaltung (Inhalt des Objekts `unterhaltung`).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kopf {
    pub titel: String,
    pub bereich: String,
    pub modellwunsch: String,
    pub angeheftet: bool,
}

impl fmt::Debug for Kopf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kopf")
            .field("inhalt", &"<verborgen>")
            .finish()
    }
}

/// Inhalt des Kopfobjekts. Zweigwahl und Uebernahmen stehen nicht mehr hier, sondern sind eigene,
/// unveraenderliche Ereignisse (E3.8, Must-fix 5).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KopfInhalt {
    kopf: Kopf,
}

/// Inhalt passt zur Sammlung (Runde 7): Kopf und Lease muessen ihr Schema erfuellen; Beitraege, Ereignisse
/// und Grabsteine prueft schon die kanonische ID (sie wird aus dem Inhalt bzw. Ziel abgeleitet).
fn inhalt_gueltig(g: &Geoeffnet) -> bool {
    match g.steuer.sammlung {
        Sammlung::Unterhaltung => serde_json::from_value::<KopfInhalt>(g.inhalt.clone()).is_ok(),
        Sammlung::Sitzung => super::lease::inhalt_passt(g),
        Sammlung::Beitrag
        | Sammlung::Auftragsereignis
        | Sammlung::Hinweis
        | Sammlung::Loeschmarke => true,
    }
}

/// Inhalt eines Objekts der Sammlung `beitrag`: ein Beitrag oder ein Verlaufsereignis. Fuer den Hub sind
/// beide nicht unterscheidbar.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "typ", rename_all = "lowercase")]
enum Verlaufsobjekt {
    Beitrag(Beitrag),
    Zweigwahl(Zweigwahl),
    Uebernahme(Uebernahme),
}

/// Obergrenze der zurueckgestellten (noch nicht pruefbaren) Objekte.
pub const MAX_ZURUECKGESTELLT: usize = 1000;

/// Objekt, das der Kern nur transportiert (`auftragsereignis`, `hinweis`; Paket A3).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Weiteres {
    pub sammlung: Sammlung,
    pub inhalt: Value,
}

impl fmt::Debug for Weiteres {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Weiteres")
            .field("sammlung", &self.sammlung)
            .field("inhalt", &"<verborgen>")
            .finish()
    }
}

/// Noch nicht hochgeladenes eigenes Verlaufsereignis.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct OffenesEreignis {
    objekt_id: Hex128,
    objekt: Verlaufsobjekt,
}

impl OffenesEreignis {
    /// Setzt bei einer Uebernahme den Hash der Lease-Fassung, die sie schreibt (kausale Bindung).
    pub(super) fn fassung_setzen(&mut self, hash: [u8; 32]) {
        if let Verlaufsobjekt::Uebernahme(x) = &mut self.objekt {
            x.fassung = hash;
        }
    }
}

/// Dauerhafte Schreibabsicht (E3.8, Must-fix 4): der versiegelte Umschlag (mit Zaehler und Signatur) liegt
/// gesichert vor, **bevor** er den Hub erreicht. Jede Wiederholung sendet genau diese Bytes.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Ausgang {
    umschlag: Umschlag,
    art: Ausgangsart,
}

#[derive(Clone, Serialize, Deserialize)]
enum Ausgangsart {
    /// Erstschreiben (unveraenderliche Sammlungen).
    Schreiben,
    /// Vergleichsschreiben einer Kettenfassung. `folge`: Uebernahme-Ereignis, das genau dann gilt, wenn
    /// diese Fassung gelandet ist (Lease-Uebernahme, ohne Luecke zwischen Vergleichsschreiben und Ereignis).
    Cas {
        erwartet: Option<u64>,
        version: u64,
        #[serde(with = "hexfeld")]
        hash: [u8; 32],
        folge: Option<Box<Folge>>,
    },
}

/// Uebernahme-Ereignis, das mit dem Landen eines Vergleichsschreibens wirksam wird.
#[derive(Clone, Serialize, Deserialize)]
struct Folge {
    r: Hex128,
    e: OffenesEreignis,
}

/// Serialisiert den Zustand in einen genau bemessenen, beim Freigeben ueberschriebenen Puffer: kein
/// Umkopieren beim Wachsen, also keine unueberschriebenen Teilkopien der Geheimnisse im freigegebenen
/// Speicher (Must-fix 7).
fn sicherungsblock(z: &Zustand) -> Result<Zeroizing<Vec<u8>>, SyncFehler> {
    json_geheim(z).map_err(|_| SyncFehler::Speicher)
}

/// Gleiche gesendete Bytes (Steuerfelder, Nonce, Chiffrat), unabhaengig von der Hub-Revision.
fn gleiche_bytes(a: &Umschlag, b: &Umschlag) -> bool {
    a.steuerfelder() == b.steuerfelder() && a.nonce == b.nonce && a.ciphertext == b.ciphertext
}

/// Ergebnis eines gesendeten Ausgangs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Gesendet {
    /// Erstschreiben neu bzw. schon vorhanden; Vergleichsschreiben gelandet (Revision).
    Neu(u64),
    Vorhanden(u64),
    /// Vergleichsschreiben verloren (eine andere Fassung liegt dort).
    Verloren,
    /// Vom Hub wegen eines Grabsteins abgewiesen.
    Grabstein,
}

/// Dauerhaft festgehaltene Gabelung eines veraenderlichen Objekts (E3.8, Must-fix 1).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Gabelungsbeleg {
    pub sammlung: Sammlung,
    pub objekt_id: Hex128,
    /// Inhaltshash der widersprechenden Fassung.
    #[serde(with = "hexfeld")]
    pub hash: [u8; 32],
}

/// Lokaler Stand einer Unterhaltung.
#[derive(Clone, Serialize, Deserialize)]
pub struct Unterhaltung {
    pub(super) kopf: Kopf,
    pub(super) einschraenkung: Einschraenkung,
    pub(super) verlauf: Verlauf,
    pub(super) weitere: BTreeMap<Hex128, Weiteres>,
    pub(super) kopf_geaendert: bool,
    /// Die Unterhaltung war je nicht `geraet_lokal` oder es gab einen Upload-Versuch. Nur dann
    /// geht ein Grabstein an den Hub (E10: sonst Metadatenabfluss).
    pub(super) jemals_synchron: bool,
    offene_ereignisse: Vec<OffenesEreignis>,
}

impl fmt::Debug for Unterhaltung {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Unterhaltung")
            .field("kopf", &self.kopf)
            .field("einschraenkung", &self.einschraenkung)
            .field("verlauf", &self.verlauf)
            .field("weitere", &self.weitere)
            .field("kopf_geaendert", &self.kopf_geaendert)
            .field("jemals_synchron", &self.jemals_synchron)
            .field("offene_ereignisse", &self.offene_ereignisse.len())
            .finish()
    }
}

impl Unterhaltung {
    pub(super) fn neu(kopf: Kopf, einschraenkung: Einschraenkung) -> Self {
        Self {
            kopf,
            einschraenkung,
            verlauf: Verlauf::new(),
            weitere: BTreeMap::new(),
            kopf_geaendert: true,
            jemals_synchron: !einschraenkung.geraet_lokal,
            offene_ereignisse: Vec::new(),
        }
    }

    pub fn kopf(&self) -> &Kopf {
        &self.kopf
    }

    pub fn einschraenkung(&self) -> &Einschraenkung {
        &self.einschraenkung
    }

    pub fn jemals_synchron(&self) -> bool {
        self.jemals_synchron
    }

    pub fn verlauf(&self) -> &Verlauf {
        &self.verlauf
    }

    pub fn weitere(&self) -> &BTreeMap<Hex128, Weiteres> {
        &self.weitere
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct OffenesGrab {
    ziel: Ziel,
    unterhaltung_ref: Hex128,
    /// Die Unterhaltung war je synchron (`jemals_synchron`); nur dann geht der Grabstein hoch.
    jemals_synchron: bool,
}

/// Lokaler Hoechststand eines veraenderlichen Objekts (Rollback-Schutz, E3.8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hoechststand {
    pub version: u64,
    #[serde(with = "hexfeld")]
    pub hash: [u8; 32],
    /// Hub-Revision der Fassung (nur fuer das Vergleichsschreiben).
    pub revision: Option<u64>,
}

/// Hoechststaende je `(sammlung, objekt_id)`, als Liste gesichert.
#[derive(Clone, Debug, Default)]
pub(super) struct Hoechststaende(BTreeMap<(Sammlung, Hex128), Hoechststand>);

impl Serialize for Hoechststaende {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v: Vec<(Sammlung, &Hex128, &Hoechststand)> =
            self.0.iter().map(|((a, b), c)| (*a, b, c)).collect();
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Hoechststaende {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Vec::<(Sammlung, Hex128, Hoechststand)>::deserialize(d)?;
        Ok(Self(v.into_iter().map(|(a, b, c)| ((a, b), c)).collect()))
    }
}

impl Hoechststaende {
    pub(super) fn get(&self, s: Sammlung, id: &Hex128) -> Option<Hoechststand> {
        self.0.get(&(s, id.clone())).copied()
    }

    pub(super) fn setzen(&mut self, s: Sammlung, id: &Hex128, h: Hoechststand) {
        self.0.insert((s, id.clone()), h);
    }

    fn entfernen(&mut self, s: Sammlung, id: &Hex128) {
        self.0.remove(&(s, id.clone()));
    }
}

/// Strengster je gesehener Widerrufs-Pin eines Geraets (Runde 5).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Pin {
    epoche: u64,
    bis_zaehler: u64,
}

/// Herkunft der wirksamen Fassung eines Objekts (Autor, Zaehler, Epoche, Hash), fuer das rueckwirkende
/// Verwerfen nach einem Widerruf (Runde 4) und die Invariante aus Runde 6.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Angenommen {
    autor: String,
    zaehler: u64,
    epoche: u64,
    r: Hex128,
    #[serde(with = "hexfeld")]
    hash: [u8; 32],
}

impl Angenommen {
    fn aus(g: &Geoeffnet) -> Self {
        Self {
            autor: g.steuer.geraet_id.clone(),
            zaehler: g.zaehler,
            epoche: g.steuer.key_epoch,
            r: g.steuer.unterhaltung_ref.clone(),
            hash: g.hash,
        }
    }
}

/// Herkunft je `(sammlung, objekt_id)` (Runde 6: nie nur nach Objekt-ID), als Liste gesichert.
#[derive(Clone, Debug, Default)]
struct Herkunftsregister(BTreeMap<(Sammlung, Hex128), Angenommen>);

impl Serialize for Herkunftsregister {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let v: Vec<(Sammlung, &Hex128, &Angenommen)> =
            self.0.iter().map(|((a, b), c)| (*a, b, c)).collect();
        v.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Herkunftsregister {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = Vec::<(Sammlung, Hex128, Angenommen)>::deserialize(d)?;
        Ok(Self(v.into_iter().map(|(a, b, c)| ((a, b), c)).collect()))
    }
}

/// Ergebnis des Einordnens eines geoeffneten Objekts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Eingeordnet {
    /// Uebernommen (wirksam): die Herkunft wird gemerkt.
    Ja,
    /// Verarbeitet, aber nichts uebernommen (Rollback, Wiederholung, unterlegene Gabelungsseite, Luecke bei
    /// der Lease): nichts merken (Runde 5).
    Gesehen,
    /// Verwerfen (Vertrag, Signatur, Kette ungueltig).
    Nein,
    /// Noch nicht belegbar (Kettenluecke): zurueckstellen.
    Spaeter,
}

impl Eingeordnet {
    /// Gueltig verarbeitet: uebernommen (`Ja`) oder nur gesehen (Doppel, Konflikt, schon bekannt).
    fn from_uebernahme(uebernommen: bool) -> Self {
        if uebernommen { Self::Ja } else { Self::Gesehen }
    }
}

/// Ergebnis von [`Abgleich::hochladen`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Hochladebericht {
    pub beitraege: usize,
    pub koepfe: usize,
    pub grabsteine: usize,
    pub ereignisse: usize,
    pub bereits_beim_hub: usize,
    /// Wegen `geraet_lokal` nicht hochgeladene Beitraege.
    pub geraet_lokal: usize,
    /// Davon vertraulich (`confidential`).
    pub vertraulich_geraet_lokal: usize,
    /// Vom Hub wegen eines Grabsteins abgewiesen (der Grabstein kommt beim naechsten Herunterladen).
    pub grabstein_abgewiesen: usize,
    pub kopf_konflikte: usize,
}

/// Ergebnis von [`Abgleich::herunterladen`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Herunterladebericht {
    pub beitraege_neu: usize,
    pub beitraege_doppelt: usize,
    pub koepfe: usize,
    pub grabsteine: usize,
    pub ereignisse: usize,
    /// Durch einen bekannten Grabstein unterdrueckt (keine Wiederbelebung).
    pub unterdrueckt: usize,
    /// Verworfen: Steuerfelder, Schluessel, Signatur oder Klartext passen nicht.
    pub verworfen: usize,
    /// Aelter als der lokale Hoechststand (Rollback oder Wiederholung einer alten Fassung).
    pub rollback: usize,
    /// Zwei Fassungen derselben Version oder Fassung, die nicht auf dem bekannten Stand aufbaut.
    pub gabelungen: usize,
    /// Von einem widerrufenen Geraet nach seinem Widerruf (Zaehler ueber dem Pin oder neue Epoche).
    pub nach_widerruf: usize,
    /// Epoche oder Autor noch unbekannt: zurueckgestellt bis Schluessel bzw. Verzeichnis ankommen.
    pub zurueckgestellt: usize,
    pub weitere: usize,
    /// Neu aufgenommene Verzeichniseintraege.
    pub verzeichnis_neu: usize,
    /// Abgewiesene Verzeichniseintraege (Faelschung, Gabelung, Luecke).
    pub verzeichnis_abgewiesen: usize,
    /// Neu aufgenommene Epochenschluessel.
    pub schluessel_neu: usize,
    /// Abgewiesene Schluesselpakete.
    pub pakete_abgewiesen: usize,
    /// Kettenfassungen ohne lueckenlosen Beleg (zurueckgestellt, Must-fix 1).
    pub luecken: usize,
    /// Cursor-Anomalie erkannt: in diesem Lauf wurde alles gelesen (Must-fix 5).
    pub vollabgleich: bool,
}

/// Vorbereiteter, noch nicht abgeschlossener Geraetewiderruf (E3.8, Must-fix 3).
///
/// Ablauf: **Vorbereiten** (signierter Verzeichniseintrag, neuer Epochenschluessel, gewrappte Pakete an
/// die verbleibenden Geraete), **Kandidat sichern** (dauerhaft, bevor irgendetwas den Hub erreicht),
/// **Abschliessen** (Eintrag anhaengen, Pakete ablegen, dann Verzeichnis, Schluessel und das Ende des
/// Vorgangs in einer Sicherung). Solange ein Kandidat offen ist, sind Schreibzugriffe gesperrt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OffenerWiderruf {
    eintrag: Verzeichniseintrag,
    schluessel: Schluessel,
    pakete: Vec<Schluesselpaket>,
}

impl OffenerWiderruf {
    pub fn geraet(&self) -> &str {
        match &self.eintrag.aenderung {
            super::verzeichnis::Aenderung::Widerruf { geraet_id, .. } => geraet_id,
            _ => "",
        }
    }

    pub fn epoche(&self) -> u64 {
        match &self.eintrag.aenderung {
            super::verzeichnis::Aenderung::Widerruf { epoche, .. } => *epoche,
            _ => 0,
        }
    }

    fn pin(&self) -> Widerrufen {
        match &self.eintrag.aenderung {
            super::verzeichnis::Aenderung::Widerruf {
                epoche,
                bis_zaehler,
                ..
            } => Widerrufen {
                epoche: *epoche,
                bis_zaehler: *bis_zaehler,
                nummer: self.eintrag.nummer,
            },
            _ => Widerrufen {
                epoche: 0,
                bis_zaehler: 0,
                nummer: 0,
            },
        }
    }
}

/// Gesicherter Gesamtzustand eines Geraets.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Zustand {
    format: u32,
    pub(super) identitaet: Geraeteschluessel,
    pub(super) bund: Schluesselbund,
    pub(super) verzeichnis: Verzeichnis,
    pub(super) freigabe: bool,
    pub(super) grabsteine: Grabsteine,
    pub(super) unterhaltungen: BTreeMap<Hex128, Unterhaltung>,
    hochgeladen: BTreeSet<Hex128>,
    hochgeladene_refs: BTreeSet<Hex128>,
    offene_grabsteine: Vec<OffenesGrab>,
    cursor: u64,
    zurueckgestellt: BTreeMap<Hex128, Umschlag>,
    verworfen: u64,
    pub(super) marke: Migrationsmarke,
    offener_widerruf: Option<OffenerWiderruf>,
    pub(super) hoechst: Hoechststaende,
    /// Eigener Schreibzaehler (streng steigend, in jedem Objekt signiert).
    zaehler: u64,
    /// Hoechster gesehener Schreibzaehler je Autor (Pin fuer den Widerruf).
    autoren: BTreeMap<String, u64>,
    /// Gesicherte Schreibabsichten, in Sendereihenfolge (Must-fix 4).
    ausgang: Vec<Ausgang>,
    /// Erkannte Gabelungen veraenderlicher Objekte; eine Lease mit Gabelung ist gesperrt (Must-fix 1).
    gabelungen: BTreeSet<Gabelungsbeleg>,
    /// Dauerhafte Widerrufsabsichten (Must-fix 3): Geraet und urspruenglicher Pin (`bis_zaehler`, beim ersten
    /// Vorbereiten festgelegt, `None` davor). Sie bleiben, bis die geprufte Kette den Widerruf enthaelt; der
    /// Pin bleibt ueber Verdraengungen hinweg unveraendert (Runde 3).
    widerrufsabsichten: BTreeMap<String, Option<u64>>,
    /// Angenommene Objekte mit Autor und Zaehler (rueckwirkendes Verwerfen nach einem Widerruf, Runde 4).
    angenommen: Herkunftsregister,
    /// Strengster je gesehener Pin je Geraet; wird nie gelockert (Runde 5).
    pins: BTreeMap<String, Pin>,
}

struct Halter(Box<dyn Zustandsspeicher>);

impl fmt::Debug for Halter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Zustandsspeicher")
    }
}

/// Geraeteseitiger Sync-Kern.
pub struct Abgleich {
    pub(super) z: Zustand,
    pub(super) auth: AuthGeraet,
    speicher: Halter,
    /// Eine Sicherung ist gescheitert: bis zum Neustart (`wiederherstellen`) keine sicherheitsrelevante
    /// Nutzung mehr (Must-fix 4). Der Arbeitsspeicher kann dann weiter sein als der gesicherte Stand.
    gesperrt: bool,
}

impl fmt::Debug for Abgleich {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Abgleich")
            .field("geraet_id", &self.auth.id())
            .field("identitaet", &self.z.identitaet)
            .field("bund", &self.z.bund)
            .field("verzeichnis_laenge", &self.z.verzeichnis.laenge())
            .field("unterhaltungen", &self.z.unterhaltungen)
            .field("grabsteine", &self.z.grabsteine.len())
            .field("cursor", &self.z.cursor)
            .field("speicher", &self.speicher)
            .finish()
    }
}

impl Abgleich {
    fn aus_zustand(z: Zustand, speicher: Box<dyn Zustandsspeicher>) -> Self {
        Self {
            auth: AuthGeraet::new(z.identitaet.geraet_id()),
            z,
            speicher: Halter(speicher),
            gesperrt: false,
        }
    }

    fn leerer_zustand(
        identitaet: Geraeteschluessel,
        bund: Schluesselbund,
        verzeichnis: Verzeichnis,
    ) -> Zustand {
        Zustand {
            format: ZUSTANDSFORMAT,
            identitaet,
            bund,
            verzeichnis,
            freigabe: false,
            grabsteine: Grabsteine::default(),
            unterhaltungen: BTreeMap::new(),
            hochgeladen: BTreeSet::new(),
            hochgeladene_refs: BTreeSet::new(),
            offene_grabsteine: Vec::new(),
            cursor: 0,
            zurueckgestellt: BTreeMap::new(),
            verworfen: 0,
            marke: Migrationsmarke::default(),
            offener_widerruf: None,
            hoechst: Hoechststaende::default(),
            zaehler: 0,
            autoren: BTreeMap::new(),
            ausgang: Vec::new(),
            gabelungen: BTreeSet::new(),
            widerrufsabsichten: BTreeMap::new(),
            angenommen: Herkunftsregister::default(),
            pins: BTreeMap::new(),
        }
    }

    /// Erstgeraet eines neuen Kontos (Vertrauensanker): Identitaet, Epoche 1 und Kontosalz zufaellig.
    pub fn gruenden(
        geraet_id: &str,
        speicher: Box<dyn Zustandsspeicher>,
    ) -> Result<Self, SyncFehler> {
        let identitaet = Geraeteschluessel::erzeugen(geraet_id)?;
        let verzeichnis = Verzeichnis::gruenden(&identitaet);
        let bund = Schluesselbund::zufaellig()?;
        let mut a = Self::aus_zustand(
            Self::leerer_zustand(identitaet, bund, verzeichnis),
            speicher,
        );
        a.sichern()?;
        Ok(a)
    }

    /// Geraet mit abgeschlossener Kopplung (Identitaet, Schluessel und gepruftes Verzeichnis liegen vor).
    pub fn gekoppelt(
        identitaet: Geraeteschluessel,
        bund: Schluesselbund,
        verzeichnis: Verzeichnis,
        speicher: Box<dyn Zustandsspeicher>,
    ) -> Result<Self, SyncFehler> {
        let zugelassen = verzeichnis
            .stand()
            .mitglied(identitaet.geraet_id())
            .is_some_and(|m| m.zugelassen() && m.ausweis == identitaet.ausweis());
        if !zugelassen {
            return Err(SyncFehler::Verzeichnis("geraet ist nicht zugelassen"));
        }
        let mut a = Self::aus_zustand(
            Self::leerer_zustand(identitaet, bund, verzeichnis),
            speicher,
        );
        a.sichern()?;
        Ok(a)
    }

    /// Stellt den zuletzt gesicherten Zustand wieder her (Neustart). `Ok(None)`: nichts gesichert.
    pub fn wiederherstellen(
        speicher: Box<dyn Zustandsspeicher>,
    ) -> Result<Option<Self>, SyncFehler> {
        let Some(block) = speicher.laden().map_err(|_| SyncFehler::Speicher)? else {
            return Ok(None);
        };
        let z: Zustand = serde_json::from_slice(&block).map_err(|_| SyncFehler::Speicher)?;
        if z.format != ZUSTANDSFORMAT {
            return Err(SyncFehler::Speicher);
        }
        Ok(Some(Self::aus_zustand(z, speicher)))
    }

    /// Sichert den ganzen Zustand atomar ueber den Host-Speicher.
    ///
    /// Scheitert die Sicherung, ist die Instanz gesperrt: jeder weitere Hub-Zugriff, jedes Versiegeln und jede
    /// Lease-Operation endet mit [`SyncFehler::Speicher`], bis der Zustand per [`Abgleich::wiederherstellen`]
    /// neu geladen ist (Must-fix 4).
    pub fn sichern(&mut self) -> Result<(), SyncFehler> {
        let block = sicherungsblock(&self.z);
        let r = block.and_then(|b| {
            self.speicher
                .0
                .sichern(&b)
                .map_err(|_| SyncFehler::Speicher)
        });
        if r.is_err() {
            self.gesperrt = true;
        }
        r
    }

    /// `true`, nachdem eine Sicherung gescheitert ist (bis zum Neustart).
    pub fn gesperrt(&self) -> bool {
        self.gesperrt
    }

    /// Fuehrt `f` aus und sichert danach in jedem Fall; der Fehler von `f` hat Vorrang.
    pub(super) fn gesichert<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<T, SyncFehler>,
    ) -> Result<T, SyncFehler> {
        let r = f(self);
        let s = self.sichern();
        let wert = r?;
        s?;
        Ok(wert)
    }

    pub fn geraet_id(&self) -> &str {
        self.auth.id()
    }

    pub fn ausweis(&self) -> Ausweis {
        self.z.identitaet.ausweis()
    }

    /// Konto-ID fuer den QR-Code der Kopplung.
    pub fn konto(&self) -> &Hex128 {
        self.z.verzeichnis.konto()
    }

    pub fn bund(&self) -> &Schluesselbund {
        &self.z.bund
    }

    pub fn verzeichnis(&self) -> &Verzeichnis {
        &self.z.verzeichnis
    }

    /// Authentifizierte aktuelle Epoche (zugleich dauerhafte Mindestepoche fuer eigene Schreibvorgaenge).
    pub fn epoche(&self) -> u64 {
        self.z.verzeichnis.epoche()
    }

    pub fn freigabe(&self) -> SyncFreigabe {
        let mut f = SyncFreigabe::default();
        if self.z.freigabe {
            f.einschalten();
        }
        f
    }

    /// Schaltet den Assistent-Sync ein (E3.6). Erst danach wird hochgeladen.
    pub fn freigabe_einschalten(&mut self) -> Result<(), SyncFehler> {
        self.z.freigabe = true;
        self.sichern()
    }

    pub fn freigabe_ausschalten(&mut self) -> Result<(), SyncFehler> {
        self.z.freigabe = false;
        self.sichern()
    }

    pub fn grabsteine(&self) -> &Grabsteine {
        &self.z.grabsteine
    }

    pub fn unterhaltung(&self, r: &Hex128) -> Option<&Unterhaltung> {
        self.z.unterhaltungen.get(r)
    }

    pub fn unterhaltungen(&self) -> impl Iterator<Item = (&Hex128, &Unterhaltung)> {
        self.z.unterhaltungen.iter()
    }

    /// Anzahl insgesamt verworfener Objekte (kein Inhalt, nur Zaehler).
    pub fn verworfen(&self) -> u64 {
        self.z.verworfen
    }

    /// Hub-Revision, bis zu der alles verarbeitet ist. Nur eine Leseoptimierung ohne Sicherheitsbedeutung.
    pub fn cursor(&self) -> u64 {
        self.z.cursor
    }

    /// Setzt den Cursor zurueck: der naechste Abgleich liest alles (Vollabgleich, idempotent).
    pub fn cursor_zuruecksetzen(&mut self) {
        self.z.cursor = 0;
    }

    pub fn zurueckgestellt(&self) -> usize {
        self.z.zurueckgestellt.len()
    }

    /// Lokaler Hoechststand eines veraenderlichen Objekts.
    pub fn hoechststand(&self, sammlung: Sammlung, objekt_id: &Hex128) -> Option<Hoechststand> {
        self.z.hoechst.get(sammlung, objekt_id)
    }

    /// Stabile Hub-Objekt-ID eines Beitrags, abgeleitet aus `(unterhaltung_ref, beitrag_id)`:
    /// gleiche Beitrags-IDs in verschiedenen Unterhaltungen kollidieren nicht.
    pub fn beitrag_objekt_id(&self, r: &Hex128, beitrag_id: &str) -> Hex128 {
        Hex128::abgeleitet(
            self.z.bund.id_salz(),
            "beitrag",
            &format!("{r}/{beitrag_id}"),
        )
    }

    pub fn kopf_objekt_id(&self, r: &Hex128) -> Hex128 {
        Hex128::abgeleitet(self.z.bund.id_salz(), "kopf", r.as_str())
    }

    /// Stabile `unterhaltung_ref` aus `(Datenbestand-ID, lokale ID)` (Migration): lokale IDs
    /// verschiedener Datenbestaende kollidieren nicht, Wiederholung bleibt auch unter neuer Geraete-ID stabil.
    pub(super) fn ref_aus_lokaler_id(&self, datenbestand_id: &str, lokale_id: &str) -> Hex128 {
        let eingabe = format!("{}/{}", datenbestand_id.len(), datenbestand_id) + "/" + lokale_id;
        Hex128::abgeleitet(self.z.bund.id_salz(), "ref", &eingabe)
    }

    fn grab_objekt_id(&self, ziel: &Ziel) -> Hex128 {
        let eingabe = format!("{:?}/{}", ziel.typ, ziel.wert);
        Hex128::abgeleitet(self.z.bund.id_salz(), "loeschmarke", &eingabe)
    }

    pub(super) fn freigabe_pruefen(&self) -> Result<(), SyncFehler> {
        if self.gesperrt {
            return Err(SyncFehler::Speicher);
        }
        if self.z.freigabe {
            Ok(())
        } else {
            Err(SyncFehler::NichtFreigegeben)
        }
    }

    /// Legt eine neue Unterhaltung mit zufaelliger `unterhaltung_ref` an.
    pub fn unterhaltung_anlegen(
        &mut self,
        kopf: Kopf,
        einschraenkung: Einschraenkung,
    ) -> Result<Hex128, SyncFehler> {
        let r = Hex128::zufall()?;
        self.z
            .unterhaltungen
            .insert(r.clone(), Unterhaltung::neu(kopf, einschraenkung));
        Ok(r)
    }

    pub fn kopf_aendern(&mut self, r: &Hex128, kopf: Kopf) -> Result<(), SyncFehler> {
        let u = self
            .z
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        if u.kopf != kopf {
            u.kopf = kopf;
            u.kopf_geaendert = true;
        }
        Ok(())
    }

    pub fn einschraenkung_setzen(
        &mut self,
        r: &Hex128,
        einschraenkung: Einschraenkung,
    ) -> Result<(), SyncFehler> {
        let u = self
            .z
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        if !einschraenkung.geraet_lokal {
            u.jemals_synchron = true;
        }
        u.einschraenkung = einschraenkung;
        Ok(())
    }

    /// Fuegt einen lokal entstandenen Beitrag ein. Eine geloeschte Unterhaltung wird nicht wiederbelebt.
    pub fn beitrag_anlegen(
        &mut self,
        r: &Hex128,
        beitrag: Beitrag,
    ) -> Result<Einfuegen, SyncFehler> {
        let oid = self.beitrag_objekt_id(r, &beitrag.id);
        if self.z.grabsteine.schlaegt(r, &oid) {
            return Err(SyncFehler::Geloescht);
        }
        let ich = self.auth.id().to_owned();
        let u = self
            .z
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        let id = beitrag.id.clone();
        let e = u.verlauf.einfuegen(beitrag);
        if e == Einfuegen::Neu {
            u.verlauf.schreiber_merken(&id, &ich);
        }
        Ok(e)
    }

    /// Ausdrueckliche Zweigwahl (E3.4) als unveraenderliches Ereignis (E3.8); geht mit `hochladen` hoch.
    pub fn zweig_waehlen(
        &mut self,
        r: &Hex128,
        eltern: Option<&str>,
        kind: &str,
    ) -> Result<(), SyncFehler> {
        let w = self
            .z
            .unterhaltungen
            .get(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?
            .verlauf
            .zweigwahl_bauen(eltern, kind)?;
        // Kanonische Ereignis-ID aus Unterhaltung, Autor und Inhalt (Runde 6).
        let oid = self
            .ereignis_objekt_id(r, self.auth.id(), &Verlaufsobjekt::Zweigwahl(w.clone()))
            .ok_or(SyncFehler::Inhalt)?;
        let herkunft = Herkunft {
            autor: self.auth.id().to_owned(),
            ereignis: oid.to_string(),
        };
        let u = self
            .z
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        u.verlauf.wahl_einordnen(&w, herkunft);
        u.offene_ereignisse.push(OffenesEreignis {
            objekt_id: oid,
            objekt: Verlaufsobjekt::Zweigwahl(w),
        });
        Ok(())
    }

    fn war_synchron(&self, r: &Hex128) -> bool {
        self.z.unterhaltungen.get(r).map_or_else(
            || self.z.hochgeladene_refs.contains(r),
            |u| u.jemals_synchron,
        )
    }

    fn grab_vormerken(&mut self, ziel: Ziel, r: &Hex128, jemals_synchron: bool) {
        if self.z.grabsteine.merken(&ziel) {
            self.z.offene_grabsteine.push(OffenesGrab {
                ziel,
                unterhaltung_ref: r.clone(),
                jemals_synchron,
            });
        }
    }

    /// Loescht eine Unterhaltung lokal und merkt einen Grabstein fuer den Hub vor (sofort gesichert).
    pub fn unterhaltung_loeschen(&mut self, r: &Hex128) -> Result<(), SyncFehler> {
        let ziel = Ziel {
            typ: ZielTyp::Unterhaltung,
            wert: r.clone(),
        };
        let jemals_synchron = self.war_synchron(r);
        self.grab_vormerken(ziel.clone(), r, jemals_synchron);
        self.grab_projizieren(&ziel);
        self.sichern()
    }

    /// Loescht einen Beitrag lokal (Platzhalter im Baum) und merkt einen Grabstein vor (sofort gesichert).
    pub fn beitrag_loeschen(&mut self, r: &Hex128, beitrag_id: &str) -> Result<(), SyncFehler> {
        let oid = self.beitrag_objekt_id(r, beitrag_id);
        self.objekt_loeschen(r, &oid)
    }

    /// Loescht ein transportiertes Objekt (`auftragsereignis`, `hinweis`) lokal und merkt einen Grabstein vor.
    pub fn weiteres_loeschen(&mut self, r: &Hex128, objekt_id: &Hex128) -> Result<(), SyncFehler> {
        self.objekt_loeschen(r, objekt_id)
    }

    /// Loescht ein beliebiges Objekt der Unterhaltung (Beitrag, Verlaufsereignis, transportiertes Objekt oder
    /// den Kopf ueber [`Abgleich::kopf_objekt_id`]) lokal und rueckwirkend (Must-fix 6) und merkt einen
    /// Grabstein vor (sofort gesichert).
    pub fn objekt_loeschen(&mut self, r: &Hex128, objekt_id: &Hex128) -> Result<(), SyncFehler> {
        let jemals_synchron = self
            .z
            .unterhaltungen
            .get(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?
            .jemals_synchron;
        let ziel = Ziel {
            typ: ZielTyp::Objekt,
            wert: objekt_id.clone(),
        };
        self.grab_vormerken(ziel.clone(), r, jemals_synchron);
        self.grab_projizieren(&ziel);
        self.sichern()
    }

    /// Epoche fuer neue Objekte: die des Verzeichnisses. Fehlt ihr Schluessel noch, wird nicht geschrieben.
    fn schreibepoche(&self) -> Result<u64, SyncFehler> {
        if !self.z.verzeichnis.stand().zugelassen(self.auth.id()) {
            return Err(SyncFehler::Verzeichnis("geraet ist nicht zugelassen"));
        }
        let e = self.z.verzeichnis.epoche();
        if self.z.bund.schluessel(e).is_none() {
            return Err(SyncFehler::EpocheUnbekannt(e));
        }
        Ok(e)
    }

    pub(super) fn siegeln(
        &mut self,
        steuer: &Steuerfelder,
        kette: Option<Glied>,
        inhalt: &Value,
    ) -> Result<Umschlag, SyncFehler> {
        if self.gesperrt {
            return Err(SyncFehler::Speicher);
        }
        let zaehler = self
            .z
            .zaehler
            .checked_add(1)
            .ok_or(SyncFehler::Vertrag("zaehler ueberlaeuft"))?;
        let u = versiegeln(
            &self.z.bund,
            &self.z.identitaet,
            zaehler,
            kette,
            steuer,
            inhalt,
        )?;
        self.z.zaehler = zaehler;
        Ok(u)
    }

    pub(super) fn steuer(
        &self,
        sammlung: Sammlung,
        objekt_id: Hex128,
        r: &Hex128,
    ) -> Result<Steuerfelder, SyncFehler> {
        Ok(Steuerfelder::inhalt(
            sammlung,
            objekt_id,
            r.clone(),
            self.auth.id(),
            self.schreibepoche()?,
        ))
    }

    fn versiegeln_inhalt<T: Serialize>(
        &mut self,
        sammlung: Sammlung,
        objekt_id: Hex128,
        r: &Hex128,
        kette: Option<Glied>,
        inhalt: &T,
    ) -> Result<Umschlag, SyncFehler> {
        let steuer = self.steuer(sammlung, objekt_id, r)?;
        let wert = serde_json::to_value(inhalt).map_err(|_| SyncFehler::Inhalt)?;
        self.siegeln(&steuer, kette, &wert)
    }

    fn grab_umschlag(&mut self, g: &OffenesGrab) -> Result<Umschlag, SyncFehler> {
        let steuer = Steuerfelder::loeschmarke(
            self.grab_objekt_id(&g.ziel),
            g.unterhaltung_ref.clone(),
            self.auth.id(),
            self.schreibepoche()?,
            g.ziel.clone(),
        );
        self.siegeln(&steuer, None, &Value::Object(serde_json::Map::new()))
    }

    /// Schliesst offene Widerrufe und Absichten ab; jeder Fehler wird weitergegeben und sperrt damit den
    /// folgenden Schreibzugriff.
    pub(super) fn offenen_widerruf_nachziehen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<(), SyncFehler> {
        self.absichten_nachziehen(hub)
    }

    /// Laedt Loeschmarken, Ereignisse, Beitraege und Koepfe hoch. Ohne Freigabe: Fehler, kein Hub-Zugriff.
    ///
    /// Vorher wird das Verzeichnis abgeglichen und ein offener Widerruf abgeschlossen; scheitert das,
    /// wird nichts geschrieben.
    pub fn hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Hochladebericht, SyncFehler> {
        self.freigabe_pruefen()?;
        self.gesichert(|a| {
            let mut leer = Herunterladebericht::default();
            a.verzeichnis_abgleichen(hub, &mut leer)?;
            a.verzeichnis_nachreichen(hub)?;
            a.absichten_nachziehen(hub)?;
            a.pakete_abholen(hub, &mut leer)?;
            a.schreibepoche()?;
            let mut bericht = Hochladebericht::default();
            a.ausgang_nachziehen(hub, &mut bericht)?;
            a.grabsteine_hochladen(hub, &mut bericht)?;
            let refs: Vec<Hex128> = a.z.unterhaltungen.keys().cloned().collect();
            for r in refs {
                a.unterhaltung_hochladen(hub, &r, &mut bericht)?;
            }
            Ok(bericht)
        })
    }

    /// Reicht lokal bekannte Verzeichniseintraege nach, die dem Hub fehlen (Rollback oder Verlust am Hub).
    fn verzeichnis_nachreichen<H: HubSpeicher + ?Sized>(&self, hub: &H) -> Result<(), SyncFehler> {
        let am_hub = hub
            .verzeichnis_lesen(&self.auth, 0)?
            .iter()
            .map(|e| e.nummer)
            .max()
            .map_or(0, |n| n + 1);
        for e in self.z.verzeichnis.eintraege().iter().skip(am_hub as usize) {
            match hub.verzeichnis_anhaengen(&self.auth, e) {
                Ok(()) | Err(HubFehler::VerzeichnisKonflikt { .. }) => {}
                Err(f) => return Err(f.into()),
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // Ausgang: dauerhafte Schreibabsichten (Must-fix 4)
    // -----------------------------------------------------------------------------------------

    pub(super) fn ausgang_index(&self, s: Sammlung, oid: &Hex128) -> Option<usize> {
        self.z
            .ausgang
            .iter()
            .position(|a| a.umschlag.sammlung == s && &a.umschlag.objekt_id == oid)
    }

    fn ausgang_entfernen(&mut self, s: Sammlung, oid: &Hex128) {
        self.z
            .ausgang
            .retain(|a| !(a.umschlag.sammlung == s && &a.umschlag.objekt_id == oid));
    }

    /// Legt eine Schreibabsicht an und sichert sie, bevor irgendetwas den Hub erreicht. Liegt fuer das
    /// Objekt schon eine vor, bleibt sie unveraendert (byte-identische Wiederholung).
    pub(super) fn ausgang_anlegen(
        &mut self,
        s: Sammlung,
        oid: &Hex128,
        bauen: impl FnOnce(&mut Self) -> Result<Ausgang, SyncFehler>,
    ) -> Result<(), SyncFehler> {
        if self.gesperrt {
            return Err(SyncFehler::Speicher);
        }
        if self.ausgang_index(s, oid).is_some() {
            return Ok(());
        }
        let a = bauen(self)?;
        self.z.ausgang.push(a);
        self.sichern()
    }

    /// Vergleichsschreiben-Absicht fuer eine Kettenfassung.
    pub(super) fn cas_ausgang(
        umschlag: Umschlag,
        erwartet: Option<u64>,
        version: u64,
        hash: [u8; 32],
        folge: Option<(Hex128, OffenesEreignis)>,
    ) -> Ausgang {
        Ausgang {
            umschlag,
            art: Ausgangsart::Cas {
                erwartet,
                version,
                hash,
                folge: folge.map(|(r, e)| Box::new(Folge { r, e })),
            },
        }
    }

    /// Sendet eine gesicherte Schreibabsicht. Ein endgueltiges Ergebnis (geschrieben, vorhanden, verloren,
    /// Grabstein) loescht sie und wird mit seinen Folgen in einer Sicherung festgehalten; ein Hub- oder
    /// Transportfehler laesst sie stehen (naechster Versuch sendet dieselben Bytes). Eine Absicht unter einer
    /// inzwischen ueberholten Epoche wird nie gesendet (ein widerrufenes Geraet koennte sie lesen), sondern
    /// verworfen: `Ok(None)`, der Aufrufer versiegelt neu.
    pub(super) fn ausgang_senden<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        s: Sammlung,
        oid: &Hex128,
    ) -> Result<Option<Gesendet>, SyncFehler> {
        if self.gesperrt {
            return Err(SyncFehler::Speicher);
        }
        let Some(i) = self.ausgang_index(s, oid) else {
            return Err(SyncFehler::Vertrag("keine schreibabsicht"));
        };
        let a = self.z.ausgang[i].clone();
        let veraltet = a.umschlag.key_epoch < self.z.verzeichnis.epoche();
        // Eine gegabelte Lease ist gesperrt: auch eine gesicherte Absicht geht nicht mehr an den Hub.
        let gesperrt = s == Sammlung::Sitzung && self.gegabelt(s, oid);
        let ergebnis = match &a.art {
            Ausgangsart::Schreiben if veraltet => None,
            Ausgangsart::Schreiben => match hub.schreiben(&self.auth, &a.umschlag) {
                Ok(Schreiben::Neu(r)) => Some(Gesendet::Neu(r)),
                Ok(Schreiben::Vorhanden(r)) => Some(Gesendet::Vorhanden(r)),
                Err(HubFehler::GrabsteinAbgewiesen) => Some(Gesendet::Grabstein),
                Err(HubFehler::EpocheVeraltet { .. }) => None,
                Err(e) => return Err(e.into()),
            },
            Ausgangsart::Cas { .. } if gesperrt => None,
            Ausgangsart::Cas { erwartet, .. } => {
                let r = if veraltet {
                    Err(HubFehler::EpocheVeraltet { mindest: 0 })
                } else {
                    hub.cas(&self.auth, s, oid, *erwartet, &a.umschlag)
                };
                match r {
                    Ok(rev) => Some(Gesendet::Neu(rev)),
                    Err(HubFehler::GrabsteinAbgewiesen) => Some(Gesendet::Grabstein),
                    // Vielleicht schon gelandet (Antwort verloren): nur die eigenen Bytes zaehlen.
                    // Ist die eigene Fassung inzwischen fortgeschrieben, steht sie nur noch unter den
                    // aufbewahrten Fassungen (Runde 3): auch dort nach den eigenen Bytes suchen.
                    Err(HubFehler::RevisionKonflikt { .. } | HubFehler::EpocheVeraltet { .. }) => {
                        let aktuell = hub
                            .holen(&self.auth, s, oid)?
                            .filter(|x| gleiche_bytes(x, &a.umschlag));
                        let gelandet = match aktuell {
                            Some(x) => Some(x),
                            None => hub
                                .fassungen_lesen(&self.auth, s, oid)?
                                .into_iter()
                                .find(|x| gleiche_bytes(x, &a.umschlag)),
                        };
                        match gelandet {
                            Some(x) => Some(Gesendet::Neu(
                                x.revision.ok_or(SyncFehler::Vertrag("revision fehlt"))?,
                            )),
                            None if veraltet => None,
                            None => Some(Gesendet::Verloren),
                        }
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        };
        self.ausgang_entfernen(s, oid);
        self.ausgang_folgen(&a, ergebnis);
        self.sichern()?;
        Ok(ergebnis)
    }

    /// Wendet die Folgen eines endgueltigen Ergebnisses an (im selben Sicherungsschritt wie das Loeschen der
    /// Absicht).
    fn ausgang_folgen(&mut self, a: &Ausgang, ergebnis: Option<Gesendet>) {
        let u = &a.umschlag;
        let gelandet = matches!(ergebnis, Some(Gesendet::Neu(_) | Gesendet::Vorhanden(_)));
        match u.sammlung {
            Sammlung::Beitrag | Sammlung::Auftragsereignis | Sammlung::Hinweis => {
                if gelandet {
                    self.z.hochgeladen.insert(u.objekt_id.clone());
                    self.z.hochgeladene_refs.insert(u.unterhaltung_ref.clone());
                }
                if ergebnis.is_some()
                    && let Some(x) = self.z.unterhaltungen.get_mut(&u.unterhaltung_ref)
                {
                    x.offene_ereignisse.retain(|e| e.objekt_id != u.objekt_id);
                }
            }
            Sammlung::Loeschmarke => {
                if ergebnis.is_some() {
                    let ziel = u.ziel.clone();
                    self.z
                        .offene_grabsteine
                        .retain(|g| Some(&g.ziel) != ziel.as_ref());
                }
            }
            Sammlung::Unterhaltung | Sammlung::Sitzung => {}
        }
        let Ausgangsart::Cas {
            version,
            hash,
            folge,
            ..
        } = &a.art
        else {
            return;
        };
        match ergebnis {
            Some(Gesendet::Neu(rev)) => {
                let h = self.z.hoechst.get(u.sammlung, &u.objekt_id);
                if h.is_none_or(|h| {
                    h.version < *version || (h.version == *version && h.hash == *hash)
                }) {
                    // Eigene Fassung wird wirksam: Hoechststand und Herkunft gemeinsam (Runde 6).
                    match oeffnen(&self.z.bund, &self.z.verzeichnis, u) {
                        Ok(g) => {
                            self.z
                                .angenommen
                                .0
                                .insert((u.sammlung, u.objekt_id.clone()), Angenommen::aus(&g));
                        }
                        Err(_) => {
                            self.z
                                .angenommen
                                .0
                                .remove(&(u.sammlung, u.objekt_id.clone()));
                        }
                    }
                    self.z.hoechst.setzen(
                        u.sammlung,
                        &u.objekt_id,
                        Hoechststand {
                            version: *version,
                            hash: *hash,
                            revision: Some(rev),
                        },
                    );
                }
                if u.sammlung == Sammlung::Unterhaltung {
                    self.z.hochgeladene_refs.insert(u.unterhaltung_ref.clone());
                }
                if let Some(f) = folge {
                    self.ereignis_wirksam(&f.r, f.e.clone());
                }
            }
            _ if u.sammlung == Sammlung::Unterhaltung => {
                if let Some(x) = self.z.unterhaltungen.get_mut(&u.unterhaltung_ref) {
                    x.kopf_geaendert = true;
                }
            }
            _ => {}
        }
    }

    /// Ein eigenes Verlaufsereignis wird wirksam: einordnen und zum Hochladen vormerken.
    fn ereignis_wirksam(&mut self, r: &Hex128, e: OffenesEreignis) {
        let herkunft = Herkunft {
            autor: self.auth.id().to_owned(),
            ereignis: e.objekt_id.to_string(),
        };
        let ausweis = self.z.identitaet.ausweis().hash();
        let Some(u) = self.z.unterhaltungen.get_mut(r) else {
            return;
        };
        match &e.objekt {
            Verlaufsobjekt::Uebernahme(x) => {
                u.verlauf.uebernahme_einordnen(x, herkunft, ausweis);
            }
            Verlaufsobjekt::Zweigwahl(w) => {
                u.verlauf.wahl_einordnen(w, herkunft);
            }
            Verlaufsobjekt::Beitrag(_) => {}
        }
        if !u
            .offene_ereignisse
            .iter()
            .any(|o| o.objekt_id == e.objekt_id)
        {
            u.offene_ereignisse.push(e);
        }
    }

    /// Sendet alle gesicherten Schreibabsichten erneut (nach Neustart oder Hubausfall), in Anlagereihenfolge.
    /// Absichten fuer inzwischen geloeschte Objekte werden verworfen, nie gesendet.
    pub(super) fn ausgang_nachziehen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        let offen: Vec<(Sammlung, Hex128, Hex128)> = self
            .z
            .ausgang
            .iter()
            .map(|a| {
                (
                    a.umschlag.sammlung,
                    a.umschlag.objekt_id.clone(),
                    a.umschlag.unterhaltung_ref.clone(),
                )
            })
            .collect();
        for (s, oid, r) in offen {
            if s != Sammlung::Loeschmarke && self.z.grabsteine.schlaegt(&r, &oid) {
                self.ausgang_entfernen(s, &oid);
                self.sichern()?;
                continue;
            }
            let ereignis = self
                .z
                .unterhaltungen
                .get(&r)
                .is_some_and(|u| u.offene_ereignisse.iter().any(|e| e.objekt_id == oid));
            match (s, self.ausgang_senden(hub, s, &oid)?) {
                (_, None | Some(Gesendet::Verloren)) => {}
                (_, Some(Gesendet::Grabstein)) => bericht.grabstein_abgewiesen += 1,
                (Sammlung::Loeschmarke, _) => bericht.grabsteine += 1,
                (Sammlung::Unterhaltung, _) => bericht.koepfe += 1,
                (Sammlung::Beitrag, _) if ereignis => bericht.ereignisse += 1,
                (Sammlung::Beitrag, Some(Gesendet::Neu(_))) => bericht.beitraege += 1,
                (_, Some(Gesendet::Vorhanden(_))) => bericht.bereits_beim_hub += 1,
                _ => {}
            }
        }
        Ok(())
    }

    /// Anlegen und Senden eines Erstschreibens; eine wegen Epochenwechsel verworfene Absicht wird einmal
    /// unter der aktuellen Epoche neu versiegelt.
    fn erstschreiben<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        s: Sammlung,
        oid: &Hex128,
        bauen: impl Fn(&mut Self) -> Result<Umschlag, SyncFehler>,
    ) -> Result<Gesendet, SyncFehler> {
        for _ in 0..2 {
            self.ausgang_anlegen(s, oid, |a| {
                Ok(Ausgang {
                    umschlag: bauen(a)?,
                    art: Ausgangsart::Schreiben,
                })
            })?;
            if let Some(g) = self.ausgang_senden(hub, s, oid)? {
                return Ok(g);
            }
        }
        Err(SyncFehler::EpocheUnbekannt(self.z.verzeichnis.epoche()))
    }

    /// Loeschmarken gehen an den Hub, wenn die Unterhaltung `jemals_synchron` war, unabhaengig davon, ob
    /// dieses Geraet je etwas hochgeladen hat (ein anderes Geraet koennte es getan haben). Eine Marke bleibt
    /// vorgemerkt, bis der Hub bestaetigt hat. Fuer nie synchrone `geraet_lokal`-Unterhaltungen entfaellt
    /// sie ohne Hub-Zugriff (kein Metadatenabfluss, E10).
    fn grabsteine_hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        while let Some(g) = self.z.offene_grabsteine.first().cloned() {
            if !g.jemals_synchron {
                self.z.offene_grabsteine.remove(0);
                continue;
            }
            let oid = self.grab_objekt_id(&g.ziel);
            self.erstschreiben(hub, Sammlung::Loeschmarke, &oid, |a| a.grab_umschlag(&g))?;
            self.z.offene_grabsteine.retain(|o| o.ziel != g.ziel);
            bericht.grabsteine += 1;
        }
        Ok(())
    }

    fn unterhaltung_hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        let Some(u) = self.z.unterhaltungen.get(r) else {
            return Ok(());
        };
        let einschraenkung = *u.einschraenkung();
        let kandidaten: Vec<Beitrag> = u.verlauf.beitraege().cloned().collect();
        let mut abgewiesen = false;
        for b in kandidaten {
            if let Err(v) = hochladen_erlaubt(&self.freigabe(), &einschraenkung, &b) {
                match v {
                    Verbot::VertraulichGeraetLokal => {
                        bericht.geraet_lokal += 1;
                        bericht.vertraulich_geraet_lokal += 1;
                    }
                    Verbot::GeraetLokal => bericht.geraet_lokal += 1,
                    Verbot::NichtFreigegeben => {}
                }
                continue;
            }
            if let Some(u) = self.z.unterhaltungen.get_mut(r) {
                u.jemals_synchron = true;
            }
            let oid = self.beitrag_objekt_id(r, &b.id);
            if abgewiesen
                || self.z.hochgeladen.contains(&oid)
                || self.z.grabsteine.schlaegt(r, &oid)
            {
                continue;
            }
            let objekt = Verlaufsobjekt::Beitrag(b);
            let g = self.erstschreiben(hub, Sammlung::Beitrag, &oid, |a| {
                a.versiegeln_inhalt(Sammlung::Beitrag, oid.clone(), r, None, &objekt)
            })?;
            match g {
                Gesendet::Neu(_) => bericht.beitraege += 1,
                Gesendet::Vorhanden(_) => bericht.bereits_beim_hub += 1,
                Gesendet::Grabstein | Gesendet::Verloren => {
                    bericht.grabstein_abgewiesen += 1;
                    abgewiesen = true;
                }
            }
        }
        if einschraenkung.geraet_lokal || abgewiesen {
            return Ok(());
        }
        self.ereignisse_hochladen(hub, r, bericht)?;
        self.kopf_hochladen(hub, r, bericht)
    }

    fn ereignisse_hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        loop {
            let Some(e) = self
                .z
                .unterhaltungen
                .get(r)
                .and_then(|u| u.offene_ereignisse.first().cloned())
            else {
                return Ok(());
            };
            let g = self.erstschreiben(hub, Sammlung::Beitrag, &e.objekt_id, |a| {
                a.versiegeln_inhalt(Sammlung::Beitrag, e.objekt_id.clone(), r, None, &e.objekt)
            })?;
            match g {
                Gesendet::Grabstein => bericht.grabstein_abgewiesen += 1,
                _ => bericht.ereignisse += 1,
            }
            if let Some(u) = self.z.unterhaltungen.get_mut(r) {
                u.offene_ereignisse.retain(|o| o.objekt_id != e.objekt_id);
            }
        }
    }

    /// Kopf per Vergleichsschreiben als naechste Kettenfassung; die Absicht ist vor dem Senden gesichert.
    fn kopf_hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        let Some(u) = self.z.unterhaltungen.get(r) else {
            return Ok(());
        };
        let oid = self.kopf_objekt_id(r);
        if !u.kopf_geaendert && self.ausgang_index(Sammlung::Unterhaltung, &oid).is_none() {
            return Ok(());
        }
        let inhalt = KopfInhalt {
            kopf: u.kopf.clone(),
        };
        self.ausgang_anlegen(Sammlung::Unterhaltung, &oid, |a| {
            let h = a.z.hoechst.get(Sammlung::Unterhaltung, &oid);
            let glied = Glied {
                version: h
                    .map_or(Some(1), |h| h.version.checked_add(1))
                    .ok_or(SyncFehler::Vertrag("version ueberlaeuft"))?,
                vorgaenger: h.map_or([0; 32], |h| h.hash),
            };
            let umschlag =
                a.versiegeln_inhalt(Sammlung::Unterhaltung, oid.clone(), r, Some(glied), &inhalt)?;
            let hash = oeffnen(&a.z.bund, &a.z.verzeichnis, &umschlag)?.hash;
            if let Some(u) = a.z.unterhaltungen.get_mut(r) {
                u.kopf_geaendert = false;
            }
            Ok(Self::cas_ausgang(
                umschlag,
                h.and_then(|h| h.revision),
                glied.version,
                hash,
                None,
            ))
        })?;
        match self.ausgang_senden(hub, Sammlung::Unterhaltung, &oid)? {
            Some(Gesendet::Neu(_) | Gesendet::Vorhanden(_)) => bericht.koepfe += 1,
            Some(Gesendet::Grabstein) => bericht.grabstein_abgewiesen += 1,
            Some(Gesendet::Verloren) | None => bericht.kopf_konflikte += 1,
        }
        Ok(())
    }

    /// Holt neue Verzeichniseintraege und prueft sie lokal. Faelschungen, Gabelungen und Luecken werden
    /// gezaehlt und uebergangen; lokal Bekanntes kann der Hub nicht zuruecknehmen.
    pub(super) fn verzeichnis_abgleichen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        bericht: &mut Herunterladebericht,
    ) -> Result<(), SyncFehler> {
        let mut neu = hub.verzeichnis_lesen(&self.auth, self.z.verzeichnis.laenge())?;
        neu.sort_by_key(|e| e.nummer);
        for e in neu {
            match self.z.verzeichnis.anhaengen(e) {
                Ok(true) => bericht.verzeichnis_neu += 1,
                Ok(false) => {}
                Err(_) => bericht.verzeichnis_abgewiesen += 1,
            }
        }
        bericht.nach_widerruf += self.widerrufe_nachwirken();
        Ok(())
    }

    /// Holt gewrappte Epochenschluessel fuer dieses Geraet und prueft sie gegen das Verzeichnis.
    pub(super) fn pakete_abholen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        bericht: &mut Herunterladebericht,
    ) -> Result<(), SyncFehler> {
        for p in hub.pakete_lesen(&self.auth)? {
            if self.z.bund.schluessel(p.epoche).is_some()
                && p.verzeichnis_nummer < self.z.verzeichnis.laenge()
            {
                // Bekannte Epoche: nur pruefen, ob es eine Kollision waere.
                match entpacken(&self.z.verzeichnis, &self.z.identitaet, &p) {
                    Ok(Some(e))
                        if self
                            .z
                            .bund
                            .schluessel(e.epoche)
                            .is_some_and(|k| gleich(k.bytes(), e.schluessel.bytes())) => {}
                    Ok(None) => {}
                    _ => bericht.pakete_abgewiesen += 1,
                }
                continue;
            }
            match entpacken(&self.z.verzeichnis, &self.z.identitaet, &p) {
                Ok(Some(e)) if gleich(&e.salz, self.z.bund.salz()) => {
                    match self.z.bund.epoche_aufnehmen(e.epoche, e.schluessel) {
                        Ok(()) => bericht.schluessel_neu += 1,
                        Err(_) => bericht.pakete_abgewiesen += 1,
                    }
                }
                Ok(None) => {}
                _ => bericht.pakete_abgewiesen += 1,
            }
        }
        Ok(())
    }

    /// Holt Verzeichnis, Schluessel und neue Objekte vom Hub und fuehrt sie ein. Ohne Freigabe: Fehler.
    ///
    /// Offene Widerrufsabsichten werden vorher nachgezogen; scheitert das, wird nichts gelesen.
    pub fn herunterladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Herunterladebericht, SyncFehler> {
        self.freigabe_pruefen()?;
        self.gesichert(|a| {
            let mut bericht = Herunterladebericht::default();
            a.verzeichnis_abgleichen(hub, &mut bericht)?;
            a.absichten_nachziehen(hub)?;
            a.pakete_abholen(hub, &mut bericht)?;
            a.objekte_holen(hub, &mut bericht)?;
            Ok(bericht)
        })
    }

    /// Liest neue Objekte ab dem Cursor. Eine Cursor-Anomalie (Revision fehlt, liegt nicht ueber dem Cursor
    /// oder ist unplausibel gross, z. B. `u64::MAX`) loest im selben Aufruf einen Vollabgleich aus; der
    /// Cursor bleibt dann auf 0, solange der Hub unplausible Revisionen liefert (Must-fix 5).
    fn objekte_holen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        bericht: &mut Herunterladebericht,
    ) -> Result<(), SyncFehler> {
        let anomal = |v: &[Umschlag], nach: u64| {
            v.iter()
                .any(|u| u.revision.is_none_or(|r| r <= nach || r >= REVISION_GRENZE))
        };
        let lies = |a: &Self, nach: u64| {
            hub.lesen(
                &a.auth,
                &LeseFilter {
                    nach_revision: nach,
                    ..LeseFilter::default()
                },
            )
        };
        let mut nach = self.z.cursor;
        let mut neu = lies(self, nach)?;
        if nach >= REVISION_GRENZE || (nach > 0 && anomal(&neu, nach)) {
            bericht.vollabgleich = true;
            nach = 0;
            self.z.cursor = 0;
            neu = lies(self, 0)?;
        }
        let wiederholen: Vec<Umschlag> = std::mem::take(&mut self.z.zurueckgestellt)
            .into_values()
            .collect();
        let mut sperre: Option<u64> = None;
        for u in wiederholen.iter().chain(neu.iter()) {
            self.verarbeiten(hub, u, &mut sperre, bericht);
        }
        if anomal(&neu, nach) {
            bericht.vollabgleich = true;
            self.z.cursor = 0;
        } else {
            // Der Cursor laeuft nie ueber ein Objekt hinweg, das nicht zurueckgestellt werden konnte. Er ist
            // keine Sicherheitsgrenze: ein Vollabgleich liest alles erneut.
            let hoechste = neu.iter().filter_map(|u| u.revision).max().unwrap_or(nach);
            let bis = sperre.map_or(hoechste, |r| hoechste.min(r.saturating_sub(1)));
            self.z.cursor = nach.max(bis);
        }
        bericht.zurueckgestellt = self.z.zurueckgestellt.len();
        Ok(())
    }

    fn verwerfen(&mut self, bericht: &mut Herunterladebericht) {
        self.z.verworfen += 1;
        bericht.verworfen += 1;
    }

    fn zurueckstellen(&mut self, u: &Umschlag, sperre: &mut Option<u64>) {
        let platz = self.z.zurueckgestellt.contains_key(&u.objekt_id)
            || self.z.zurueckgestellt.len() < MAX_ZURUECKGESTELLT;
        if platz {
            self.z
                .zurueckgestellt
                .insert(u.objekt_id.clone(), u.clone());
        } else {
            let rev = u.revision.unwrap_or(0);
            *sperre = Some(sperre.map_or(rev, |s| s.min(rev)));
        }
    }

    /// Widerruf eines Autors: aus dem Verzeichnis oder aus dem eigenen, noch offenen Widerruf (lokal bindend).
    pub(super) fn widerruf_von(&self, autor: &str) -> Option<Widerrufen> {
        let aus_verzeichnis = self
            .z
            .verzeichnis
            .stand()
            .mitglied(autor)
            .and_then(|m| m.widerrufen);
        let kandidat = self
            .z
            .offener_widerruf
            .as_ref()
            .filter(|o| o.geraet() == autor)
            .map(OffenerWiderruf::pin);
        // Dauerhafte Absicht mit festgelegtem Pin (Runde 4): wirkt ab dem ersten Vorbereiten, auch nach einer
        // Verdraengung (Kandidat geloescht) und einem Neustart, bis der Widerruf im Verzeichnis steht. Die
        // Epoche ist dann noch offen; es zaehlt nur der Pin.
        let absicht = self
            .z
            .widerrufsabsichten
            .get(autor)
            .copied()
            .flatten()
            .map(|bis_zaehler| Widerrufen {
                epoche: u64::MAX,
                bis_zaehler,
                nummer: u64::MAX,
            });
        // Strengster je gesehener Pin (Runde 5): wird nie gelockert, auch nicht, wenn ein fremder Widerruf mit
        // hoeherem Pin die eigene Absicht abschliesst.
        let dauerhaft = self.z.pins.get(autor).map(|p| Widerrufen {
            epoche: p.epoche,
            bis_zaehler: p.bis_zaehler,
            nummer: u64::MAX,
        });
        [aus_verzeichnis, kandidat, absicht, dauerhaft]
            .into_iter()
            .flatten()
            .reduce(|a, b| Widerrufen {
                epoche: a.epoche.min(b.epoche),
                bis_zaehler: a.bis_zaehler.min(b.bis_zaehler),
                nummer: a.nummer.min(b.nummer),
            })
    }

    /// Faltet alle aktuell bekannten Widerrufe (Verzeichnis, Kandidat, Absichten) in die dauerhafte Abbildung
    /// Geraet -> strengster Pin. Werte werden nur verschaerft, nie gelockert (Runde 5).
    pub(super) fn pins_festhalten(&mut self) {
        let mut quellen: Vec<(String, Pin)> = self
            .z
            .verzeichnis
            .stand()
            .mitglieder
            .iter()
            .filter_map(|(g, m)| {
                m.widerrufen.map(|w| {
                    (
                        g.clone(),
                        Pin {
                            epoche: w.epoche,
                            bis_zaehler: w.bis_zaehler,
                        },
                    )
                })
            })
            .collect();
        if let Some(o) = &self.z.offener_widerruf {
            let w = o.pin();
            quellen.push((
                o.geraet().to_owned(),
                Pin {
                    epoche: w.epoche,
                    bis_zaehler: w.bis_zaehler,
                },
            ));
        }
        for (g, p) in &self.z.widerrufsabsichten {
            if let Some(p) = p {
                quellen.push((
                    g.clone(),
                    Pin {
                        epoche: u64::MAX,
                        bis_zaehler: *p,
                    },
                ));
            }
        }
        for (g, p) in quellen {
            let e = self.z.pins.entry(g).or_insert(p);
            e.epoche = e.epoche.min(p.epoche);
            e.bis_zaehler = e.bis_zaehler.min(p.bis_zaehler);
        }
    }

    /// Einzige Stelle, an der eine gepruefte fremde oder zurueckkommende Fassung wirksam wird (Runde 6):
    /// `wirkung` setzt den wirksamen Inhalt und meldet, ob wirklich uebernommen wurde; genau dann wird die
    /// Herkunft derselben Fassung unter `(sammlung, objekt_id)` gesetzt. Rollback, Wiederholung, unterlegene
    /// Gabelungsseite und Konfliktfassungen melden `false` und aendern weder Inhalt noch Herkunft. Ein
    /// unveraenderliches Objekt behaelt seine erste Fassung: es wird nur uebernommen, solange es noch keine
    /// Herkunft hat.
    pub(super) fn uebernehmen(
        &mut self,
        g: &Geoeffnet,
        wirkung: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        let schluessel = (g.steuer.sammlung, g.steuer.objekt_id.clone());
        if !g.steuer.sammlung.veraenderlich() && self.z.angenommen.0.contains_key(&schluessel) {
            return false;
        }
        if !wirkung(self) {
            return false;
        }
        self.z.angenommen.0.insert(schluessel, Angenommen::aus(g));
        true
    }

    /// Kanonische Objekt-ID eines Verlaufsereignisses (Zweigwahl, Uebernahme): aus Unterhaltung, Autor und
    /// Inhalt abgeleitet (Runde 6). Damit passt jede Ereignis-ID zu Sammlung, Unterhaltung und Autor und kann
    /// keine andere Objektart treffen.
    fn ereignis_objekt_id(
        &self,
        r: &Hex128,
        autor: &str,
        objekt: &Verlaufsobjekt,
    ) -> Option<Hex128> {
        let roh = serde_json::to_vec(objekt).ok()?;
        let h = hash_teile(b"wai-assistent-sync/ereignis/v1", &[&roh]);
        Some(Hex128::abgeleitet(
            self.z.bund.id_salz(),
            "ereignis",
            &format!("{r}/{autor}/{}", hex(&h)),
        ))
    }

    /// Kanonische Objekt-ID eines transportierten Objekts (`auftragsereignis`, `hinweis`) dieses Geraets:
    /// aus Sammlung, Unterhaltung, Autor und Inhalt abgeleitet (Runde 6). Gleicher Inhalt desselben Autors
    /// ergibt dieselbe ID (idempotent).
    pub fn weiteres_objekt_id(&self, r: &Hex128, sammlung: Sammlung, inhalt: &Value) -> Hex128 {
        self.weiteres_objekt_id_von(r, sammlung, self.auth.id(), inhalt)
    }

    fn weiteres_objekt_id_von(
        &self,
        r: &Hex128,
        sammlung: Sammlung,
        autor: &str,
        inhalt: &Value,
    ) -> Hex128 {
        let roh = serde_json::to_vec(inhalt).unwrap_or_default();
        let h = hash_teile(b"wai-assistent-sync/weiteres/v1", &[&roh]);
        Hex128::abgeleitet(
            self.z.bund.id_salz(),
            sammlung.name(),
            &format!("{r}/{autor}/{}", hex(&h)),
        )
    }

    /// Gemeinsame Gueltigkeitspruefung jeder Fassung (Runde 7), bevor sie Wirkung oder Abstammung bekommt:
    /// im Abruf, als Zwischenbeleg (`belege_holen`, damit auch als Lease-Glied), beim Lesen der Lease und
    /// beim erneuten Verarbeiten Zurueckgestellter (auch nach einem Neustart). Die Signatur ist mit dem
    /// Oeffnen geprueft (ein [`Geoeffnet`] entsteht nur aus einem gueltig signierten Umschlag). Geprueft
    /// werden hier: Epoche im gepruefen Verzeichnis bekannt, Autor nicht ueber seinem wirksamen Pin
    /// (Widerruf, Kandidat, Absicht), Kette genau bei veraenderlichen Sammlungen, kanonische Objekt-ID fuer
    /// Sammlung und Unterhaltung und ein zur Sammlung passender Inhalt. Eine ungueltige Fassung belegt nichts.
    pub(super) fn fassung_gueltig(&self, g: &Geoeffnet) -> bool {
        g.steuer.key_epoch <= self.z.verzeichnis.epoche()
            && !self.ueber_pin(g)
            && g.kette.is_some() == g.steuer.sammlung.veraenderlich()
            && self.kanonisch(g)
            && inhalt_gueltig(g)
    }

    /// `true`, wenn der Autor widerrufen ist und die Fassung ueber seinem wirksamen Pin liegt.
    pub(super) fn ueber_pin(&self, g: &Geoeffnet) -> bool {
        self.widerruf_von(&g.steuer.geraet_id)
            .is_some_and(|w| g.steuer.key_epoch >= w.epoche || g.zaehler > w.bis_zaehler)
    }

    /// Jede Objekt-ID muss kanonisch zu Sammlung und Unterhaltung (bei Ereignissen und transportierten
    /// Objekten auch zu Autor und Inhalt) passen; sonst ist das Objekt ungueltig (Runde 6).
    fn kanonisch(&self, g: &Geoeffnet) -> bool {
        let r = &g.steuer.unterhaltung_ref;
        let oid = &g.steuer.objekt_id;
        match g.steuer.sammlung {
            Sammlung::Unterhaltung => self.kopf_objekt_id(r) == *oid,
            Sammlung::Sitzung => self.lease_objekt_id(r) == *oid,
            Sammlung::Beitrag => match serde_json::from_value::<Verlaufsobjekt>(g.inhalt.clone()) {
                Ok(Verlaufsobjekt::Beitrag(b)) => self.beitrag_objekt_id(r, &b.id) == *oid,
                Ok(o) => self.ereignis_objekt_id(r, &g.steuer.geraet_id, &o).as_ref() == Some(oid),
                Err(_) => false,
            },
            Sammlung::Auftragsereignis | Sammlung::Hinweis => {
                self.weiteres_objekt_id_von(r, g.steuer.sammlung, &g.steuer.geraet_id, &g.inhalt)
                    == *oid
            }
            Sammlung::Loeschmarke => g.steuer.ziel.as_ref().is_some_and(|z| {
                self.grab_objekt_id(z) == *oid && (z.typ != ZielTyp::Unterhaltung || z.wert == *r)
            }),
        }
    }

    /// Setzt bei einer eigenen Uebernahme die geschriebene Lease-Fassung und leitet danach die kanonische
    /// Ereignis-ID ab (Runde 6).
    pub(super) fn uebernahme_vollenden(&self, e: &mut OffenesEreignis, r: &Hex128, hash: [u8; 32]) {
        e.fassung_setzen(hash);
        if let Some(oid) = self.ereignis_objekt_id(r, self.auth.id(), &e.objekt) {
            e.objekt_id = oid;
        }
    }

    /// Defensiv (Runde 4): Ist ein Widerruf wirksam (Verzeichnis, Kandidat oder Absicht), werden schon
    /// angenommene Objekte des widerrufenen Geraets ueber seinem Pin rueckwirkend verworfen und die Sicht neu
    /// projiziert: Beitrag wird Platzhalter, Ereignisse und transportierte Objekte fallen weg, ein Kopf faellt
    /// auf den Standard zurueck, und ein darauf beruhender Hoechststand wird freigegeben. Gibt die Anzahl
    /// zurueck.
    pub(super) fn widerrufe_nachwirken(&mut self) -> usize {
        self.pins_festhalten();
        let treffer: Vec<((Sammlung, Hex128), Angenommen)> = self
            .z
            .angenommen
            .0
            .iter()
            .filter(|(_, a)| {
                self.widerruf_von(&a.autor)
                    .is_some_and(|w| a.epoche >= w.epoche || a.zaehler > w.bis_zaehler)
            })
            .map(|(k, a)| (k.clone(), a.clone()))
            .collect();
        for ((s, oid), a) in &treffer {
            // Inhalt und Herkunft gemeinsam zurueckziehen: dieselbe Fassung, derselbe Schluessel.
            self.z.angenommen.0.remove(&(*s, oid.clone()));
            if s.veraenderlich() {
                self.z.hoechst.entfernen(*s, oid);
            }
            let salz = self.z.bund.id_salz();
            let Some(u) = self.z.unterhaltungen.get_mut(&a.r) else {
                continue;
            };
            match s {
                Sammlung::Unterhaltung => {
                    if !u.kopf_geaendert {
                        u.kopf = Kopf::default();
                    }
                }
                Sammlung::Auftragsereignis | Sammlung::Hinweis => {
                    u.weitere.remove(oid);
                }
                Sammlung::Beitrag => {
                    u.verlauf.ereignis_entfernen(oid.as_str());
                    let beitrag = u
                        .verlauf
                        .beitraege()
                        .find(|b| {
                            Hex128::abgeleitet(salz, "beitrag", &format!("{}/{}", a.r, b.id))
                                == *oid
                        })
                        .map(|b| b.id.clone());
                    if let Some(id) = beitrag {
                        u.verlauf.loeschen(&id);
                    }
                }
                Sammlung::Sitzung | Sammlung::Loeschmarke => {}
            }
        }
        treffer.len()
    }

    /// Geprufte Zwischenfassungen eines veraenderlichen Objekts vom Hub (Belege fuer die Kette). Jede Fassung
    /// wird geoeffnet und wie jedes Objekt geprueft; Unpruefbares wird ignoriert.
    fn belege_holen<H: HubSpeicher + ?Sized>(
        &self,
        hub: &H,
        s: Sammlung,
        oid: &Hex128,
        r: &Hex128,
    ) -> Vec<Beleg> {
        hub.fassungen_lesen(&self.auth, s, oid)
            .unwrap_or_default()
            .iter()
            .filter_map(|u| oeffnen(&self.z.bund, &self.z.verzeichnis, u).ok())
            .filter(|g| {
                g.steuer.sammlung == s
                    && &g.steuer.objekt_id == oid
                    && &g.steuer.unterhaltung_ref == r
            })
            // Ein ungueltiger Beleg belegt nichts (Runde 7): dieselbe Pruefung wie im Abruf.
            .filter(|g| self.fassung_gueltig(g))
            .filter_map(|g| {
                g.kette.map(|glied| Beleg {
                    glied,
                    hash: g.hash,
                })
            })
            .collect()
    }

    /// Ordnet eine Kettenfassung gegen den Hoechststand ein; Belege werden bei einem Sprung und bei einer
    /// aelteren Fassung geholt (diese wird nie uebernommen, aber auf Widerspruch geprueft, Runde 3). Eine
    /// Gabelung wird dauerhaft festgehalten (beide Seiten). Gepruefte Lease-Glieder gehen in den Verlauf der
    /// Unterhaltung `r` (kausale Bindung der Uebernahmen, Must-fix 5).
    pub(super) fn fassung_einordnen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        s: Sammlung,
        oid: &Hex128,
        r: &Hex128,
        glied: Glied,
        hash: [u8; 32],
    ) -> Einordnung {
        let h = self.z.hoechst.get(s, oid);
        let ziel = Beleg { glied, hash };
        let mut e = kette::einordnen(h, &ziel, &[]);
        let mut belege = Vec::new();
        if matches!(e, Einordnung::Luecke | Einordnung::Rollback) {
            belege = self.belege_holen(hub, s, oid, r);
            e = kette::einordnen(h, &ziel, &belege);
        }
        if s == Sammlung::Sitzung && e != Einordnung::Ungueltig {
            belege.push(ziel);
            for b in &belege {
                self.lease_glied_merken(r, b.hash, b.glied.vorgaenger);
            }
        }
        if let (Einordnung::Gabelung { .. }, Some(h)) = (e, h) {
            for hash in [hash, h.hash] {
                self.z.gabelungen.insert(Gabelungsbeleg {
                    sammlung: s,
                    objekt_id: oid.clone(),
                    hash,
                });
            }
        }
        e
    }

    /// Merkt ein signiertes Lease-Glied (Hash -> Vorgaenger) im Verlauf der Unterhaltung.
    pub(super) fn lease_glied_merken(&mut self, r: &Hex128, hash: [u8; 32], vorgaenger: [u8; 32]) {
        if self.z.grabsteine.unterhaltung_geloescht(r) {
            return;
        }
        self.unterhaltung_holen(r)
            .verlauf
            .lease_glied_merken(hash, vorgaenger);
    }

    /// Erkannte Gabelungen veraenderlicher Objekte (dauerhaft).
    pub fn gabelungen(&self) -> &BTreeSet<Gabelungsbeleg> {
        &self.z.gabelungen
    }

    /// `true`, wenn fuer dieses Objekt eine Gabelung bekannt ist (eine Lease ist dann gesperrt).
    pub fn gegabelt(&self, s: Sammlung, oid: &Hex128) -> bool {
        self.z
            .gabelungen
            .iter()
            .any(|g| g.sammlung == s && &g.objekt_id == oid)
    }

    fn verarbeiten<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        u: &Umschlag,
        sperre: &mut Option<u64>,
        bericht: &mut Herunterladebericht,
    ) {
        let g = match oeffnen(&self.z.bund, &self.z.verzeichnis, u) {
            Ok(g) => g,
            Err(SyncFehler::EpocheUnbekannt(_) | SyncFehler::AutorUnbekannt) => {
                self.zurueckstellen(u, sperre);
                return;
            }
            Err(_) => {
                self.verwerfen(bericht);
                return;
            }
        };
        if g.steuer.key_epoch > self.z.verzeichnis.epoche() {
            // Epoche, die das gepruefte Verzeichnis noch nicht kennt: warten, nicht vertrauen.
            self.zurueckstellen(u, sperre);
            return;
        }
        if self.ueber_pin(&g) {
            bericht.nach_widerruf += 1;
            self.z.verworfen += 1;
            return;
        }
        if !self.fassung_gueltig(&g) {
            // Gemeinsame Pruefung (Runde 7): Kette, kanonische ID, Inhalt.
            self.verwerfen(bericht);
            return;
        }
        let gesehen = self
            .z
            .autoren
            .entry(g.steuer.geraet_id.clone())
            .or_insert(0);
        *gesehen = (*gesehen).max(g.zaehler);
        if g.steuer.sammlung == Sammlung::Loeschmarke {
            self.grabstein_anwenden(&g, bericht);
            return;
        }
        let r = &g.steuer.unterhaltung_ref;
        if self.z.grabsteine.schlaegt(r, &g.steuer.objekt_id) {
            bericht.unterdrueckt += 1;
            return;
        }
        if self
            .z
            .unterhaltungen
            .get(r)
            .is_some_and(|x| x.einschraenkung.geraet_lokal)
        {
            return;
        }
        // Jeder Pfad macht Inhalt und Herkunft nur gemeinsam ueber `uebernehmen` wirksam.
        let ergebnis = match g.steuer.sammlung {
            Sammlung::Beitrag => self.verlaufsobjekt_einordnen(&g, bericht),
            Sammlung::Unterhaltung => self.kopf_einordnen(hub, &g, bericht),
            Sammlung::Auftragsereignis | Sammlung::Hinweis => self.weiteres_einordnen(&g, bericht),
            Sammlung::Sitzung => self.lease_stand_einordnen(hub, &g),
            Sammlung::Loeschmarke => Eingeordnet::Ja,
        };
        match ergebnis {
            Eingeordnet::Ja | Eingeordnet::Gesehen => {}
            Eingeordnet::Nein => self.verwerfen(bericht),
            Eingeordnet::Spaeter => {
                bericht.luecken += 1;
                self.zurueckstellen(u, sperre);
            }
        }
    }

    fn unterhaltung_holen(&mut self, r: &Hex128) -> &mut Unterhaltung {
        self.z.unterhaltungen.entry(r.clone()).or_insert_with(|| {
            let mut u = Unterhaltung::neu(Kopf::default(), Einschraenkung::default());
            u.kopf_geaendert = false;
            u
        })
    }

    fn verlaufsobjekt_einordnen(
        &mut self,
        g: &Geoeffnet,
        bericht: &mut Herunterladebericht,
    ) -> Eingeordnet {
        if g.kette.is_some() {
            return Eingeordnet::Nein;
        }
        let Ok(objekt) = serde_json::from_value::<Verlaufsobjekt>(g.inhalt.clone()) else {
            return Eingeordnet::Nein;
        };
        let r = g.steuer.unterhaltung_ref.clone();
        let herkunft = Herkunft {
            autor: g.steuer.geraet_id.clone(),
            ereignis: g.steuer.objekt_id.to_string(),
        };
        match objekt {
            Verlaufsobjekt::Beitrag(beitrag) => {
                if self.beitrag_objekt_id(&r, &beitrag.id) != g.steuer.objekt_id {
                    return Eingeordnet::Nein;
                }
                self.z.hochgeladen.insert(g.steuer.objekt_id.clone());
                self.z.hochgeladene_refs.insert(r.clone());
                let autor = g.steuer.geraet_id.clone();
                // Nur ein neu eingefuegter Beitrag ist uebernommen; Doppel, Konflikt und Platzhalter nicht.
                let neu = self.uebernehmen(g, |a| {
                    let u = a.unterhaltung_holen(&r);
                    let id = beitrag.id.clone();
                    let neu = u.verlauf.einfuegen(beitrag) == Einfuegen::Neu;
                    if neu {
                        u.verlauf.schreiber_merken(&id, &autor);
                    }
                    neu
                });
                if neu {
                    bericht.beitraege_neu += 1;
                } else {
                    bericht.beitraege_doppelt += 1;
                }
                Eingeordnet::from_uebernahme(neu)
            }
            Verlaufsobjekt::Zweigwahl(w) => {
                self.z.hochgeladene_refs.insert(r.clone());
                let neu = self.uebernehmen(g, |a| {
                    a.unterhaltung_holen(&r)
                        .verlauf
                        .wahl_einordnen(&w, herkunft);
                    true
                });
                bericht.ereignisse += usize::from(neu);
                Eingeordnet::from_uebernahme(neu)
            }
            Verlaufsobjekt::Uebernahme(x) => {
                let Some(ausweis) = self
                    .z
                    .verzeichnis
                    .stand()
                    .mitglied(&g.steuer.geraet_id)
                    .map(|m| m.ausweis.hash())
                else {
                    return Eingeordnet::Nein;
                };
                self.z.hochgeladene_refs.insert(r.clone());
                let neu = self.uebernehmen(g, |a| {
                    a.unterhaltung_holen(&r)
                        .verlauf
                        .uebernahme_einordnen(&x, herkunft, ausweis);
                    true
                });
                bericht.ereignisse += usize::from(neu);
                Eingeordnet::from_uebernahme(neu)
            }
        }
    }

    /// Kopf mit authentifizierter, lueckenlos belegter Version (Must-fix 1): nie aelter als der lokale
    /// Hoechststand; ein Sprung ohne Zwischenbelege wird zurueckgestellt; eine Gabelung wird dauerhaft
    /// festgehalten und reihenfolgeunabhaengig entschieden (hoehere Version, dann kleinerer Hash).
    fn kopf_einordnen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        g: &Geoeffnet,
        bericht: &mut Herunterladebericht,
    ) -> Eingeordnet {
        let Ok(inhalt) = serde_json::from_value::<KopfInhalt>(g.inhalt.clone()) else {
            return Eingeordnet::Nein;
        };
        let r = g.steuer.unterhaltung_ref.clone();
        if self.kopf_objekt_id(&r) != g.steuer.objekt_id {
            return Eingeordnet::Nein;
        }
        let Some(glied) = g.kette else {
            return Eingeordnet::Nein;
        };
        let oid = g.steuer.objekt_id.clone();
        let uebernehmen =
            match self.fassung_einordnen(hub, Sammlung::Unterhaltung, &oid, &r, glied, g.hash) {
                Einordnung::Rollback => {
                    bericht.rollback += 1;
                    false
                }
                Einordnung::Gleich => {
                    if let (Some(mut h), Some(rev)) =
                        (self.z.hoechst.get(Sammlung::Unterhaltung, &oid), g.revision)
                    {
                        h.revision = Some(rev);
                        self.z.hoechst.setzen(Sammlung::Unterhaltung, &oid, h);
                    }
                    false
                }
                Einordnung::Gabelung { gewinnt } => {
                    bericht.gabelungen += 1;
                    gewinnt
                }
                Einordnung::Nachfolger => true,
                Einordnung::Luecke => return Eingeordnet::Spaeter,
                Einordnung::Ungueltig => return Eingeordnet::Nein,
            };
        self.z.hochgeladene_refs.insert(r.clone());
        if !uebernehmen {
            // Rollback, Wiederholung oder unterlegene Gabelungsseite: nichts uebernommen, nichts gemerkt.
            return Eingeordnet::Gesehen;
        }
        self.uebernehmen(g, |a| {
            a.z.hoechst.setzen(
                Sammlung::Unterhaltung,
                &oid,
                Hoechststand {
                    version: glied.version,
                    hash: g.hash,
                    revision: g.revision,
                },
            );
            let u = a.unterhaltung_holen(&r);
            if !u.kopf_geaendert {
                u.kopf = inhalt.kopf;
                bericht.koepfe += 1;
            }
            true
        });
        Eingeordnet::Ja
    }

    /// Lease-Objekte im Pull heben nur den lokalen Hoechststand, und nur bei lueckenloser Abstammung. Eine
    /// Gabelung sperrt die Lease dauerhaft (Must-fix 1); eine Luecke aendert nichts (`lease_lesen` belegt).
    fn lease_stand_einordnen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        g: &Geoeffnet,
    ) -> Eingeordnet {
        let Some(glied) = g.kette else {
            return Eingeordnet::Nein;
        };
        let oid = g.steuer.objekt_id.clone();
        let r = g.steuer.unterhaltung_ref.clone();
        // Kanonische Lease-ID (in `verarbeiten` geprueft) und gueltiger Lease-Inhalt (Inhaber = Schreiber,
        // passende Unterhaltung) vor jeder Uebernahme (Runde 6).
        if oid != self.lease_objekt_id(&r) || !super::lease::inhalt_passt(g) {
            return Eingeordnet::Nein;
        }
        match self.fassung_einordnen(hub, Sammlung::Sitzung, &oid, &r, glied, g.hash) {
            Einordnung::Nachfolger => {
                self.uebernehmen(g, |a| {
                    a.z.hoechst.setzen(
                        Sammlung::Sitzung,
                        &oid,
                        Hoechststand {
                            version: glied.version,
                            hash: g.hash,
                            revision: g.revision,
                        },
                    );
                    true
                });
                Eingeordnet::Ja
            }
            Einordnung::Ungueltig => Eingeordnet::Nein,
            _ => Eingeordnet::Gesehen,
        }
    }

    /// Transportiertes Objekt: die erste Fassung bleibt (Runde 6). Dieselbe Fassung erneut ist ein Doppel; eine
    /// andere Fassung unter derselben ID ist ein Konflikt und wird verworfen, nie ersetzt.
    fn weiteres_einordnen(
        &mut self,
        g: &Geoeffnet,
        bericht: &mut Herunterladebericht,
    ) -> Eingeordnet {
        if g.kette.is_some() {
            return Eingeordnet::Nein;
        }
        let r = g.steuer.unterhaltung_ref.clone();
        let vorhanden = self
            .z
            .unterhaltungen
            .get(&r)
            .and_then(|u| u.weitere.get(&g.steuer.objekt_id))
            .map(|w| w.sammlung == g.steuer.sammlung && w.inhalt == g.inhalt);
        match vorhanden {
            Some(true) => return Eingeordnet::Gesehen,
            Some(false) => return Eingeordnet::Nein,
            None => {}
        }
        let neu = self.uebernehmen(g, |a| {
            a.unterhaltung_holen(&r).weitere.insert(
                g.steuer.objekt_id.clone(),
                Weiteres {
                    sammlung: g.steuer.sammlung,
                    inhalt: g.inhalt.clone(),
                },
            );
            true
        });
        if neu {
            bericht.weitere += 1;
            Eingeordnet::Ja
        } else {
            // Schon eine Herkunft unter diesem Schluessel (Inhalt inzwischen geloescht): nicht wiederbeleben.
            Eingeordnet::Gesehen
        }
    }

    /// Wendet einen empfangenen Grabstein an (siehe [`Abgleich::grab_projizieren`]). Er bleibt dauerhaft und
    /// wird mit dem Zustand gesichert.
    fn grabstein_anwenden(&mut self, g: &Geoeffnet, bericht: &mut Herunterladebericht) {
        let Some(ziel) = g.steuer.ziel.clone() else {
            return;
        };
        if self.z.grabsteine.merken(&ziel) {
            bericht.grabsteine += 1;
        }
        self.z.offene_grabsteine.retain(|o| o.ziel != ziel);
        self.grab_projizieren(&ziel);
    }

    /// Wendet einen Grabstein rueckwirkend auf alles an, was schon wirksam ist (Must-fix 6): Unterhaltung
    /// (alles), Beitrag (Platzhalter), transportierte Objekte, Verlaufsereignisse (Zweigwahl, Uebernahme;
    /// die wirksame Wahl wird aus den uebrigen Ereignissen neu projiziert) und den Kopf. Gesicherte
    /// Schreibabsichten fuer getroffene Objekte werden verworfen.
    fn grab_projizieren(&mut self, ziel: &Ziel) {
        match ziel.typ {
            ZielTyp::Unterhaltung => {
                self.z.unterhaltungen.remove(&ziel.wert);
                self.z.angenommen.0.retain(|_, a| a.r != ziel.wert);
                self.z
                    .zurueckgestellt
                    .retain(|_, u| u.unterhaltung_ref != ziel.wert);
                self.z.ausgang.retain(|a| {
                    a.umschlag.sammlung == Sammlung::Loeschmarke
                        || a.umschlag.unterhaltung_ref != ziel.wert
                });
            }
            ZielTyp::Objekt => {
                self.z.zurueckgestellt.remove(&ziel.wert);
                self.z.angenommen.0.retain(|(_, oid), _| *oid != ziel.wert);
                self.z.ausgang.retain(|a| {
                    a.umschlag.sammlung == Sammlung::Loeschmarke
                        || a.umschlag.objekt_id != ziel.wert
                });
                // Kein Umkopieren des Kontosalzes (Runde 3, Must-fix 7): getrennte Felder, nur geliehen.
                let salz = self.z.bund.id_salz();
                for (r, u) in &mut self.z.unterhaltungen {
                    u.weitere.remove(&ziel.wert);
                    u.offene_ereignisse.retain(|e| e.objekt_id != ziel.wert);
                    u.verlauf.ereignis_entfernen(ziel.wert.as_str());
                    if Hex128::abgeleitet(salz, "kopf", r.as_str()) == ziel.wert {
                        u.kopf = Kopf::default();
                        u.kopf_geaendert = false;
                    }
                    let treffer = u
                        .verlauf
                        .beitraege()
                        .find(|b| {
                            Hex128::abgeleitet(salz, "beitrag", &format!("{r}/{}", b.id))
                                == ziel.wert
                        })
                        .map(|b| b.id.clone());
                    if let Some(id) = treffer {
                        u.verlauf.loeschen(&id);
                    }
                }
            }
        }
    }

    /// Geraetewiderruf (E3.1, E3.8) mit dauerhafter Absicht, Vorbereiten, Sichern und Abschliessen (siehe
    /// [`OffenerWiderruf`]).
    ///
    /// Der Widerruf ist ein signierter Verzeichniseintrag; er hebt die Epoche um 1, sperrt das Geraet
    /// endgueltig und pinnt seinen hoechsten gesehenen Schreibzaehler. Der neue Schluessel geht nur an die
    /// verbleibenden Geraete laut gepruftem Verzeichnis, nie an eine Liste des Hubs. Gibt die neue Epoche
    /// zurueck.
    ///
    /// **Dauerhaft** (Must-fix 3): Die Absicht wird gesichert, bevor irgendetwas den Hub erreicht, und bleibt,
    /// bis die lokal geprufte Kette den Widerruf enthaelt. Ungepruefte Konfliktdaten des Hubs verwerfen nie
    /// einen Kandidaten; nur eine vollstaendig gegen den lokalen Anker gepruefte, konkurrierende Kette
    /// verdraengt ihn, und dann wird der Widerruf auf ihr erneut vorbereitet und angehaengt. Solange eine
    /// Absicht offen ist, sind Schreibzugriffe gesperrt.
    pub fn widerrufen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        widerrufenes_geraet: &str,
    ) -> Result<u64, SyncFehler> {
        self.freigabe_pruefen()?;
        if widerrufenes_geraet == self.auth.id() {
            return Err(SyncFehler::Verzeichnis("eigenes geraet"));
        }
        let Some(m) = self.z.verzeichnis.stand().mitglied(widerrufenes_geraet) else {
            return Err(SyncFehler::Verzeichnis("geraet unbekannt"));
        };
        if let Some(w) = m.widerrufen {
            return Ok(w.epoche);
        }
        if !self.z.widerrufsabsichten.contains_key(widerrufenes_geraet) {
            self.z
                .widerrufsabsichten
                .insert(widerrufenes_geraet.to_owned(), None);
            self.sichern()?;
        }
        self.gesichert(|a| a.absichten_nachziehen(hub))?;
        self.z
            .verzeichnis
            .stand()
            .mitglied(widerrufenes_geraet)
            .and_then(|m| m.widerrufen)
            .map(|w| w.epoche)
            .ok_or(SyncFehler::Verzeichnis("widerruf offen"))
    }

    /// Dauerhafte Widerrufsabsichten, die die geprufte Kette noch nicht enthaelt.
    pub fn offene_widerrufsabsichten(&self) -> impl Iterator<Item = &str> {
        self.z.widerrufsabsichten.keys().map(String::as_str)
    }

    fn widerrufen_im_verzeichnis(&self, geraet: &str) -> bool {
        self.z
            .verzeichnis
            .stand()
            .mitglied(geraet)
            .is_some_and(|m| m.widerrufen.is_some())
    }

    /// Bringt offene Kandidaten und Absichten zum Abschluss. Jeder Fehler wird weitergegeben und sperrt damit
    /// den folgenden Schreib- bzw. Lesezugriff.
    pub(super) fn absichten_nachziehen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<(), SyncFehler> {
        for _ in 0..WIDERRUF_RUNDEN {
            // Auch eine uebernommene, gepruefte Konkurrenzkette kann Widerrufe bringen (Runde 4).
            if self.widerrufe_nachwirken() > 0 {
                self.sichern()?;
            }
            if self.z.offener_widerruf.is_some() {
                match self.widerruf_ausfuehren(hub) {
                    Ok(_) => {}
                    // Von einer gepruften, konkurrierenden Kette verdraengt: neu vorbereiten.
                    Err(SyncFehler::Hub(HubFehler::VerzeichnisKonflikt { .. }))
                        if self.z.offener_widerruf.is_none() => {}
                    Err(e) => return Err(e),
                }
            }
            // Bevor eine erledigte Absicht faellt, ihren Pin dauerhaft festhalten (Runde 5).
            self.pins_festhalten();
            let vorher = self.z.widerrufsabsichten.len();
            let erledigt: Vec<String> = self
                .z
                .widerrufsabsichten
                .keys()
                .filter(|g| self.widerrufen_im_verzeichnis(g))
                .cloned()
                .collect();
            for g in &erledigt {
                self.z.widerrufsabsichten.remove(g);
            }
            if self.z.widerrufsabsichten.len() != vorher {
                self.sichern()?;
            }
            let Some(ziel) = self.z.widerrufsabsichten.keys().next().cloned() else {
                return Ok(());
            };
            // Stand holen: Verzeichnis und Objekte, damit der Pin die zuletzt gesehenen Zaehler enthaelt.
            let mut leer = Herunterladebericht::default();
            self.verzeichnis_abgleichen(hub, &mut leer)?;
            self.pakete_abholen(hub, &mut leer)?;
            self.objekte_holen(hub, &mut leer)?;
            if self.widerrufen_im_verzeichnis(&ziel) {
                continue;
            }
            self.widerruf_vorbereiten(&ziel)?;
        }
        Err(HubFehler::VerzeichnisKonflikt {
            laenge: self.z.verzeichnis.laenge(),
        }
        .into())
    }

    /// Bereitet den Kandidaten vor und sichert ihn, bevor irgendetwas den Hub erreicht. Der Pin wird beim
    /// ersten Vorbereiten festgelegt und mit der Absicht gesichert; nach einer Verdraengung wird er nicht neu
    /// berechnet (sonst koennte er mit inzwischen gesehenen Zaehlern steigen).
    fn widerruf_vorbereiten(&mut self, ziel: &str) -> Result<(), SyncFehler> {
        let pin = match self.z.widerrufsabsichten.get(ziel).copied().flatten() {
            Some(pin) => pin,
            None => {
                let pin = self.z.autoren.get(ziel).copied().unwrap_or(0);
                self.z.widerrufsabsichten.insert(ziel.to_owned(), Some(pin));
                pin
            }
        };
        // Ab jetzt wirkt der Pin (`widerruf_von`); Angenommenes darueber faellt rueckwirkend weg.
        self.widerrufe_nachwirken();
        let eintrag = self
            .z
            .verzeichnis
            .widerruf_vorschlagen(ziel, pin, &self.z.identitaet)?;
        let epoche = self.z.verzeichnis.epoche() + 1;
        let mut danach = self.z.verzeichnis.clone();
        danach.anhaengen(eintrag.clone())?;
        let schluessel = Schluessel::zufall()?;
        let pakete = danach
            .zugelassene()
            .into_iter()
            .filter(|a| a.geraet_id != self.auth.id())
            .map(|a| {
                wrappen(
                    &danach,
                    &self.z.identitaet,
                    a,
                    epoche,
                    &schluessel,
                    self.z.bund.salz(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.z.offener_widerruf = Some(OffenerWiderruf {
            eintrag,
            schluessel,
            pakete,
        });
        self.sichern()
    }

    /// Schliesst den offenen Widerruf ab (idempotent): Eintrag anhaengen, Pakete ablegen, dann Verzeichnis,
    /// Schluessel und Ende des Vorgangs in einer Sicherung. Jeder Fehler wird weitergegeben.
    ///
    /// Meldet der Hub einen Konflikt, entscheidet allein die lokal geprufte Kette: Die Eintraege des Hubs ab
    /// der lokalen Laenge werden an eine Kopie des gepruften Verzeichnisses gehaengt (Signatur, Vorgaenger,
    /// Unterzeichner zugelassen). Steht dort an der Stelle des Kandidaten genau er, ist er gelandet. Steht dort
    /// ein anderer, gepruefter Eintrag, wird diese Kette uebernommen und der Kandidat verworfen (sein
    /// Schluessel hat das Geraet nie verlassen); die Absicht bleibt und wird neu vorbereitet. Ist nichts
    /// pruefbar, bleibt der Kandidat bestehen.
    fn widerruf_ausfuehren<H: HubSpeicher + ?Sized>(&mut self, hub: &H) -> Result<u64, SyncFehler> {
        let Some(offen) = self.z.offener_widerruf.clone() else {
            return Err(SyncFehler::Vertrag("kein offener widerruf"));
        };
        let nummer = offen.eintrag.nummer;
        let an_stelle = |v: &Verzeichnis| {
            usize::try_from(nummer)
                .ok()
                .and_then(|i| v.eintraege().get(i))
                .cloned()
        };
        let veraltet = match an_stelle(&self.z.verzeichnis) {
            Some(e) => e != offen.eintrag,
            None => match hub.verzeichnis_anhaengen(&self.auth, &offen.eintrag) {
                Ok(()) => false,
                Err(HubFehler::VerzeichnisKonflikt { .. }) => {
                    let mut kopie = self.z.verzeichnis.clone();
                    let mut dort = hub.verzeichnis_lesen(&self.auth, kopie.laenge())?;
                    dort.sort_by_key(|e| e.nummer);
                    for e in dort {
                        if kopie.anhaengen(e).is_err() {
                            break;
                        }
                    }
                    match an_stelle(&kopie) {
                        Some(e) if e == offen.eintrag => false,
                        Some(_) => {
                            self.z.verzeichnis = kopie;
                            true
                        }
                        None => {
                            return Err(HubFehler::VerzeichnisKonflikt { laenge: nummer }.into());
                        }
                    }
                }
                Err(f) => return Err(f.into()),
            },
        };
        if veraltet {
            self.z.offener_widerruf = None;
            self.sichern()?;
            return Err(HubFehler::VerzeichnisKonflikt { laenge: nummer + 1 }.into());
        }
        hub.pakete_ablegen(&self.auth, &offen.pakete)?;
        // Abschluss atomar: neues Verzeichnis, neuer Schluessel und Ende des Vorgangs in einer Sicherung.
        let mut verzeichnis = self.z.verzeichnis.clone();
        verzeichnis.anhaengen(offen.eintrag.clone())?;
        let mut bund = self.z.bund.clone();
        bund.epoche_aufnehmen(offen.epoche(), offen.schluessel.clone())?;
        let alt_v = std::mem::replace(&mut self.z.verzeichnis, verzeichnis);
        let alt_b = std::mem::replace(&mut self.z.bund, bund);
        let alt_o = self.z.offener_widerruf.take();
        if let Err(e) = self.sichern() {
            self.z.verzeichnis = alt_v;
            self.z.bund = alt_b;
            self.z.offener_widerruf = alt_o;
            return Err(e);
        }
        Ok(offen.epoche())
    }

    /// Schliesst offene Widerrufe ab. `Ok(None)`: nichts offen; sonst die aktuelle Epoche danach.
    pub fn widerruf_abschliessen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Option<u64>, SyncFehler> {
        self.freigabe_pruefen()?;
        if self.z.offener_widerruf.is_none() && self.z.widerrufsabsichten.is_empty() {
            return Ok(None);
        }
        self.gesichert(|a| a.absichten_nachziehen(hub))?;
        Ok(Some(self.epoche()))
    }

    /// Offener, gesicherter Widerruf-Kandidat (solange er besteht, sind Schreibzugriffe gesperrt).
    pub fn offener_widerruf(&self) -> Option<&OffenerWiderruf> {
        self.z.offener_widerruf.as_ref()
    }

    /// Geraete, an die ein neuer Epochenschluessel geht: zugelassene laut gepruftem Verzeichnis, ohne
    /// dieses Geraet.
    pub fn empfaenger(&self) -> Vec<String> {
        self.z
            .verzeichnis
            .zugelassene()
            .into_iter()
            .map(|a| a.geraet_id.clone())
            .filter(|g| g != self.auth.id())
            .collect()
    }

    /// Hoechster gesehener Schreibzaehler eines Autors.
    pub fn gesehener_zaehler(&self, geraet: &str) -> u64 {
        self.z.autoren.get(geraet).copied().unwrap_or(0)
    }

    /// Baut das signierbare Uebernahme-Ereignis (Generation, Hauptlinienspitze, Uebernahmekontext `basis`).
    /// Es wird erst wirksam, wenn das zugehoerige Vergleichsschreiben gelandet ist ([`Ausgang`]).
    pub(super) fn uebernahme_ereignis(
        &self,
        r: &Hex128,
        generation: u64,
        basis: [u8; 32],
    ) -> Result<OffenesEreignis, SyncFehler> {
        let spitze = self
            .z
            .unterhaltungen
            .get(r)
            .and_then(|u| u.verlauf.spitze(generation.saturating_sub(1)));
        Ok(OffenesEreignis {
            objekt_id: Hex128::zufall()?,
            objekt: Verlaufsobjekt::Uebernahme(Uebernahme {
                generation,
                spitze,
                basis,
                // Wird beim Versiegeln der Lease-Fassung gesetzt (`lease_schreiben`), vor dem Sichern.
                fassung: [0; 32],
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::memhub::MemHub;
    use crate::sync::testkonto::Testkonto;
    use crate::sync::verlauf::Modus;

    fn b(id: &str, eltern: Option<&str>) -> Beitrag {
        Beitrag::neu(id, eltern, 1, "n", Modus::Typed, "Inhalt")
    }

    #[test]
    fn debug_enthaelt_weder_inhalt_noch_schluessel() {
        let k = Testkonto::neu(&["pc"]);
        let mut a = k.geraet("pc");
        let r = a
            .unterhaltung_anlegen(
                Kopf {
                    titel: "Geheimer Titel".into(),
                    ..Kopf::default()
                },
                Einschraenkung::default(),
            )
            .unwrap();
        a.beitrag_anlegen(
            &r,
            Beitrag::neu("m1", None, 1, "n", Modus::Typed, "Geheimer Text"),
        )
        .unwrap();
        let s = format!("{a:?}");
        assert!(!s.contains("Geheimer") && !s.contains("Schluessel([") && s.contains("***"));
    }

    #[test]
    fn auftragsereignisse_werden_transportiert_und_per_grabstein_geloescht() {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "iphone"], &hub);
        let mut pc = k.geraet("pc");
        let r = pc
            .unterhaltung_anlegen(Kopf::default(), Einschraenkung::default())
            .unwrap();
        pc.hochladen(&hub).unwrap();
        let inhalt = serde_json::json!({"art": "gestartet"});
        let oid = pc.weiteres_objekt_id(&r, Sammlung::Auftragsereignis, &inhalt);
        let steuer =
            Steuerfelder::inhalt(Sammlung::Auftragsereignis, oid.clone(), r.clone(), "pc", 1);
        let u = pc.siegeln(&steuer, None, &inhalt).unwrap();
        hub.schreiben(&AuthGeraet::new("pc"), &u).unwrap();
        let mut iphone = k.geraet("iphone");
        assert_eq!(iphone.herunterladen(&hub).unwrap().weitere, 1);
        assert_eq!(iphone.herunterladen(&hub).unwrap().weitere, 0);
        // Must-fix 6: Objekt-Grabstein trifft auch transportierte Objekte.
        pc.herunterladen(&hub).unwrap();
        pc.weiteres_loeschen(&r, &oid).unwrap();
        pc.hochladen(&hub).unwrap();
        iphone.herunterladen(&hub).unwrap();
        assert!(iphone.unterhaltung(&r).unwrap().weitere().is_empty());
        // Wiederauftauchen (Hub spielt das alte Objekt erneut ein): unterdrueckt.
        hub.einschleusen(&u);
        iphone.cursor_zuruecksetzen();
        let ber = iphone.herunterladen(&hub).unwrap();
        assert!(ber.unterdrueckt >= 1);
        assert!(iphone.unterhaltung(&r).unwrap().weitere().is_empty());
    }

    #[test]
    fn beitrag_mit_falscher_objekt_id_wird_verworfen() {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "iphone"], &hub);
        let mut pc = k.geraet("pc");
        let r = Hex128::zufall().unwrap();
        // Gueltig signiert, aber die Objekt-ID gehoert nicht zur Beitrags-ID (Schluesselbesitzer-Fehler).
        let steuer = Steuerfelder::inhalt(Sammlung::Beitrag, Hex128::zufall().unwrap(), r, "pc", 1);
        let inhalt = serde_json::to_value(Verlaufsobjekt::Beitrag(b("m1", None))).unwrap();
        let u = pc.siegeln(&steuer, None, &inhalt).unwrap();
        hub.schreiben(&AuthGeraet::new("pc"), &u).unwrap();
        let mut iphone = k.geraet("iphone");
        let bericht = iphone.herunterladen(&hub).unwrap();
        assert_eq!((bericht.beitraege_neu, bericht.verworfen), (0, 1));
    }

    // -----------------------------------------------------------------------------------------
    // Runde 6: Herkunft je (Sammlung, Objekt-ID), kanonische IDs, erste Fassung bleibt, Invariante
    // -----------------------------------------------------------------------------------------

    use crate::sync::memhub::Boese;
    use crate::sync::speicher::MemSpeicher;

    const T0: u64 = 1_794_081_600;

    fn kopf_t(t: &str) -> Kopf {
        Kopf {
            titel: t.into(),
            ..Kopf::default()
        }
    }

    fn neu_laden(speicher: &MemSpeicher) -> Abgleich {
        Abgleich::wiederherstellen(Box::new(speicher.clone()))
            .unwrap()
            .unwrap()
    }

    /// GB1 (Runde 6): Kopf v2 von `a` ist wirksam. Der Hub liefert eine von `a` signierte Lease-Genesis mit
    /// kleinerem Zaehler und **derselben Objekt-ID wie der Kopf**. Sie wird als nicht kanonisch verworfen und
    /// kann die Kopfherkunft nicht ueberschreiben (Schluessel je Sammlung); ein Widerruf mit dem Pin von v1
    /// zieht v2 deshalb zurueck.
    fn gb_sammlung(neustart: bool) {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "a", "ip"], &hub);
        let speicher = MemSpeicher::new();
        let mut pc = k.geraet_mit_speicher("pc", speicher.clone());
        let mut a = k.geraet("a");
        let mut ip = k.geraet("ip");
        let r = a
            .unterhaltung_anlegen(kopf_t("v1"), Einschraenkung::default())
            .unwrap();
        a.hochladen(&hub).unwrap();
        ip.herunterladen(&hub).unwrap();
        let c1 = ip.gesehener_zaehler("a");
        a.kopf_aendern(&r, kopf_t("v2")).unwrap();
        a.hochladen(&hub).unwrap();
        pc.herunterladen(&hub).unwrap();
        let kopf_oid = pc.kopf_objekt_id(&r);
        assert_eq!(pc.unterhaltung(&r).unwrap().kopf().titel, "v2");
        assert!(pc.gesehener_zaehler("a") > c1);
        // Von `a` signiert, Sammlung `sitzung`, Objekt-ID des Kopfs, Zaehler von v1, Genesis.
        let steuer = Steuerfelder::inhalt(Sammlung::Sitzung, kopf_oid.clone(), r.clone(), "a", 1);
        let falsch = versiegeln(
            k.bund(),
            k.schluessel("a"),
            c1,
            Some(Glied {
                version: 1,
                vorgaenger: [0; 32],
            }),
            &steuer,
            &serde_json::json!({}),
        )
        .unwrap();
        hub.einschleusen(&falsch);
        let b = pc.herunterladen(&hub).unwrap();
        assert!(b.verworfen >= 1, "{b:?}");
        assert!(pc.hoechststand(Sammlung::Sitzung, &kopf_oid).is_none());
        if neustart {
            drop(pc);
            pc = neu_laden(&speicher);
        }
        // Das iPhone kennt nur v1 (der Hub verbirgt ihm den Kopf) und widerruft `a` mit Pin c1.
        hub.boese(Boese {
            zurueckhalten: [kopf_oid.clone()].into_iter().collect(),
            ..Boese::default()
        });
        assert_eq!(ip.widerrufen(&hub, "a"), Ok(2));
        hub.ehrlich();
        pc.herunterladen(&hub).unwrap();
        assert_ne!(pc.unterhaltung(&r).unwrap().kopf().titel, "v2");
        assert!(pc.hoechststand(Sammlung::Unterhaltung, &kopf_oid).is_none());
    }

    #[test]
    fn r6_fremde_sammlung_mit_kopf_id_ueberschreibt_die_kopfherkunft_nicht() {
        gb_sammlung(false);
    }

    #[test]
    fn r6_fremde_sammlung_mit_kopf_id_ueberschreibt_die_kopfherkunft_nicht_nach_neustart() {
        gb_sammlung(true);
    }

    /// GB2 (Runde 6): Ein transportiertes Objekt behaelt seine erste Fassung. Eine von demselben Autor
    /// signierte Konfliktfassung unter derselben Objekt-ID wird verworfen; Inhalt und Herkunft bleiben die der
    /// ersten Fassung, auch ueber einen Neustart. Ein Widerruf mit dem Pin der ersten Fassung laesst sie
    /// stehen (sie liegt nicht ueber dem Pin).
    fn gb_weiteres(neustart: bool) {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "a", "ip"], &hub);
        let speicher = MemSpeicher::new();
        let mut pc = k.geraet_mit_speicher("pc", speicher.clone());
        let mut a = k.geraet("a");
        let r = a
            .unterhaltung_anlegen(kopf_t("w"), Einschraenkung::default())
            .unwrap();
        a.hochladen(&hub).unwrap();
        let erst = serde_json::json!({"art": "gestartet"});
        let oid = a.weiteres_objekt_id(&r, Sammlung::Auftragsereignis, &erst);
        let steuer =
            Steuerfelder::inhalt(Sammlung::Auftragsereignis, oid.clone(), r.clone(), "a", 1);
        let u1 = a.siegeln(&steuer, None, &erst).unwrap();
        hub.schreiben(&AuthGeraet::new("a"), &u1).unwrap();
        pc.herunterladen(&hub).unwrap();
        let herkunft = pc.z.angenommen.0[&(Sammlung::Auftragsereignis, oid.clone())].clone();
        // Konfliktfassung: gleicher Autor, gleiche Objekt-ID, anderer Inhalt, hoeherer Zaehler.
        let zweit = serde_json::json!({"art": "abgebrochen"});
        let u2 = a.siegeln(&steuer, None, &zweit).unwrap();
        hub.einschleusen(&u2);
        let b = pc.herunterladen(&hub).unwrap();
        assert!(b.verworfen >= 1, "{b:?}");
        if neustart {
            drop(pc);
            pc = neu_laden(&speicher);
        }
        let w = &pc.unterhaltung(&r).unwrap().weitere()[&oid];
        assert_eq!(w.inhalt, erst, "erste Fassung bleibt");
        assert_eq!(
            pc.z.angenommen.0[&(Sammlung::Auftragsereignis, oid.clone())],
            herkunft,
            "Herkunft der ersten Fassung"
        );
        pc.cursor_zuruecksetzen();
        pc.herunterladen(&hub).unwrap();
        assert_eq!(pc.unterhaltung(&r).unwrap().weitere()[&oid].inhalt, erst);
    }

    #[test]
    fn r6_transportobjekt_behaelt_seine_erste_fassung() {
        gb_weiteres(false);
    }

    #[test]
    fn r6_transportobjekt_behaelt_seine_erste_fassung_nach_neustart() {
        gb_weiteres(true);
    }

    /// Prueft die Invariante: wirksamer Inhalt und gemerkte Herkunft stammen aus derselben Fassung, unter
    /// demselben Schluessel (Sammlung, Objekt-ID).
    fn invariante_pruefen(pc: &Abgleich, hub: &MemHub, kontext: &str) {
        // Alle Fassungen, die der Hub je gespeichert hat, nach Hash (zum Abgleich des wirksamen Inhalts).
        let mut nach_hash: BTreeMap<[u8; 32], Geoeffnet> = BTreeMap::new();
        for u in hub
            .alle_fassungen()
            .iter()
            .chain(hub.alle_umschlaege().iter())
        {
            if let Ok(g) = oeffnen(&pc.z.bund, &pc.z.verzeichnis, u) {
                nach_hash.insert(g.hash, g);
            }
        }
        for ((s, oid), a) in &pc.z.angenommen.0 {
            let u = pc.z.unterhaltungen.get(&a.r);
            match s {
                Sammlung::Unterhaltung | Sammlung::Sitzung => {
                    let kanonisch = if *s == Sammlung::Unterhaltung {
                        pc.kopf_objekt_id(&a.r)
                    } else {
                        pc.lease_objekt_id(&a.r)
                    };
                    assert_eq!(*oid, kanonisch, "{kontext}: kanonische ID {s:?}");
                    assert_eq!(
                        pc.z.hoechst.get(*s, oid).map(|h| h.hash),
                        Some(a.hash),
                        "{kontext}: Hoechststand und Herkunft {s:?}"
                    );
                    if *s == Sammlung::Unterhaltung
                        && let Some(u) = u
                        && !u.kopf_geaendert
                        && let Some(g) = nach_hash.get(&a.hash)
                    {
                        let k: KopfInhalt = serde_json::from_value(g.inhalt.clone()).unwrap();
                        assert_eq!(
                            u.kopf, k.kopf,
                            "{kontext}: wirksamer Kopf aus derselben Fassung"
                        );
                        assert_eq!(g.steuer.geraet_id, a.autor, "{kontext}: Autor");
                        assert_eq!(g.zaehler, a.zaehler, "{kontext}: Zaehler");
                    }
                }
                Sammlung::Auftragsereignis | Sammlung::Hinweis => {
                    let w = &u.expect("unterhaltung").weitere[oid];
                    assert_eq!(w.sammlung, *s, "{kontext}");
                    assert_eq!(
                        pc.weiteres_objekt_id_von(&a.r, *s, &a.autor, &w.inhalt),
                        *oid,
                        "{kontext}: Inhalt und Herkunft aus derselben Fassung"
                    );
                }
                Sammlung::Beitrag => {
                    let v = &u.expect("unterhaltung").verlauf;
                    let beitrag = v
                        .beitraege()
                        .find(|b| pc.beitrag_objekt_id(&a.r, &b.id) == *oid);
                    match beitrag {
                        Some(b) => {
                            assert_eq!(v.schreiber(&b.id), Some(a.autor.as_str()), "{kontext}")
                        }
                        None => assert_eq!(
                            v.ereignis_autor(oid.as_str()),
                            Some(a.autor.as_str()),
                            "{kontext}: Ereignis"
                        ),
                    }
                }
                Sammlung::Loeschmarke => {}
            }
        }
        // Umgekehrt: alles Wirksame hat eine Herkunft unter seinem Schluessel.
        for ((s, oid), h) in &pc.z.hoechst.0 {
            let a = pc.z.angenommen.0.get(&(*s, oid.clone()));
            assert_eq!(
                a.map(|a| a.hash),
                Some(h.hash),
                "{kontext}: Herkunft fehlt {s:?}"
            );
        }
        for (r, u) in &pc.z.unterhaltungen {
            for (oid, w) in &u.weitere {
                assert!(
                    pc.z.angenommen.0.contains_key(&(w.sammlung, oid.clone())),
                    "{kontext}"
                );
            }
            for b in u.verlauf.beitraege() {
                if u.verlauf.schreiber(&b.id) != Some(pc.geraet_id()) {
                    let a =
                        pc.z.angenommen
                            .0
                            .get(&(Sammlung::Beitrag, pc.beitrag_objekt_id(r, &b.id)));
                    assert_eq!(
                        a.map(|a| a.autor.as_str()),
                        u.verlauf.schreiber(&b.id),
                        "{kontext}: Beitrag {}",
                        b.id
                    );
                }
            }
        }
    }

    fn zufall(z: &mut u64) -> u64 {
        // xorshift64*, deterministisch.
        *z ^= *z >> 12;
        *z ^= *z << 25;
        *z ^= *z >> 27;
        z.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Property-artig (Runde 6): Nach jeder Folge zufaelliger, boesartiger Hub-Eingriffe (Wiedereinspielen
    /// alter Fassungen, Objekte mit fremder Sammlung unter vorhandener ID, Konfliktfassungen, Gabelungen ohne
    /// Vergleichsschreiben, alte Fassungen, vorgespulte Revisionen, Widerruf) und Neustarts gilt die
    /// Invariante. Deterministische Seeds.
    #[test]
    fn r6_invariante_haelt_unter_zufaelligen_hub_eingriffen() {
        for seed in [0x5eed_0001_u64, 0x5eed_0042, 0x5eed_1234, 0x5eed_beef] {
            let hub = MemHub::new();
            let k = Testkonto::mit_hub(&["pc", "a", "b", "ip"], &hub);
            let speicher = MemSpeicher::new();
            let mut pc = k.geraet_mit_speicher("pc", speicher.clone());
            let mut a = k.geraet("a");
            let mut b = k.geraet("b");
            let mut ip = k.geraet("ip");
            let r = a
                .unterhaltung_anlegen(kopf_t("start"), Einschraenkung::default())
                .unwrap();
            a.hochladen(&hub).unwrap();
            let _ = b.herunterladen(&hub);
            let mut z = seed;
            let mut n = 0usize;
            let mut widerrufen = false;
            let mut gift: BTreeSet<[u8; 32]> = BTreeSet::new();
            for schritt in 0..60 {
                n += 1;
                let wahl = zufall(&mut z) % 13;
                let wer = if zufall(&mut z).is_multiple_of(2) {
                    "a"
                } else {
                    "b"
                };
                let g = if wer == "a" { &mut a } else { &mut b };
                match wahl {
                    0 | 1 => {
                        let _ = g.herunterladen(&hub);
                        let _ = g.kopf_aendern(&r, kopf_t(&format!("{wer}{n}")));
                        let _ = g.hochladen(&hub);
                    }
                    2 => {
                        // Gabelung: Vergleichsschreiben nicht durchgesetzt.
                        hub.boese(Boese {
                            ohne_durchsetzung: true,
                            ..Boese::default()
                        });
                        let _ = g.kopf_aendern(&r, kopf_t(&format!("{wer}g{n}")));
                        let _ = g.hochladen(&hub);
                        hub.ehrlich();
                    }
                    3 => {
                        let _ = g.herunterladen(&hub);
                        let spitze = g.unterhaltung(&r).and_then(|u| u.verlauf().spitze(0));
                        let beitrag = Beitrag::neu(
                            &format!("{wer}-{n}"),
                            spitze.as_deref(),
                            1,
                            "n",
                            Modus::Typed,
                            "x",
                        );
                        let _ = g.beitrag_anlegen(&r, beitrag);
                        let _ = g.hochladen(&hub);
                    }
                    4 => {
                        let inhalt = serde_json::json!({"n": n % 3});
                        let oid = g.weiteres_objekt_id(&r, Sammlung::Hinweis, &inhalt);
                        let steuer =
                            Steuerfelder::inhalt(Sammlung::Hinweis, oid, r.clone(), wer, 1);
                        if let Ok(u) = g.siegeln(&steuer, None, &inhalt) {
                            hub.einschleusen(&u);
                        }
                    }
                    5 => {
                        // Wiedereinspielen einer alten Fassung oder eines alten Objekts.
                        let alle: Vec<Umschlag> = hub
                            .alle_fassungen()
                            .into_iter()
                            .chain(hub.alle_umschlaege())
                            .collect();
                        if !alle.is_empty() {
                            let i = (zufall(&mut z) as usize) % alle.len();
                            hub.einschleusen(&alle[i]);
                        }
                    }
                    6 => {
                        // Fremde Sammlung unter vorhandener Objekt-ID, kleiner Zaehler, Genesis-Glied.
                        let ziele: Vec<Hex128> = hub
                            .alle_umschlaege()
                            .into_iter()
                            .map(|u| u.objekt_id)
                            .collect();
                        if !ziele.is_empty() {
                            let oid = ziele[(zufall(&mut z) as usize) % ziele.len()].clone();
                            let sammlungen = [
                                Sammlung::Sitzung,
                                Sammlung::Unterhaltung,
                                Sammlung::Hinweis,
                                Sammlung::Auftragsereignis,
                                Sammlung::Beitrag,
                            ];
                            let s = sammlungen[(zufall(&mut z) as usize) % sammlungen.len()];
                            let kette = s.veraenderlich().then_some(Glied {
                                version: 1,
                                vorgaenger: [0; 32],
                            });
                            let steuer = Steuerfelder::inhalt(s, oid, r.clone(), wer, 1);
                            if let Ok(u) = versiegeln(
                                k.bund(),
                                k.schluessel(wer),
                                1,
                                kette,
                                &steuer,
                                &serde_json::json!({"n": n}),
                            ) {
                                hub.einschleusen(&u);
                            }
                        }
                    }
                    7 => {
                        // Konfliktfassung eines vorhandenen transportierten Objekts.
                        let vorhanden: Vec<Umschlag> = hub
                            .alle_umschlaege()
                            .into_iter()
                            .filter(|u| u.sammlung == Sammlung::Hinweis)
                            .collect();
                        if let Some(u0) = vorhanden.first() {
                            let steuer = Steuerfelder::inhalt(
                                Sammlung::Hinweis,
                                u0.objekt_id.clone(),
                                r.clone(),
                                &u0.geraet_id,
                                1,
                            );
                            if let Ok(u) = versiegeln(
                                k.bund(),
                                k.schluessel(&u0.geraet_id),
                                1_000 + n as u64,
                                None,
                                &steuer,
                                &serde_json::json!({"konflikt": n}),
                            ) {
                                hub.einschleusen(&u);
                            }
                        }
                    }
                    8 => {
                        let _ = g.herunterladen(&hub);
                        let gelesen = g.lease_lesen(&hub, &r).ok().flatten();
                        let _ = g.lease_uebernehmen(&hub, &r, T0 + n as u64, gelesen.as_ref());
                        let _ = g.hochladen(&hub);
                    }
                    9 => {
                        // Lesestoerungen fuer genau einen Abruf des PCs.
                        hub.boese(Boese {
                            alte_fassung: zufall(&mut z).is_multiple_of(2),
                            revision_aufschlag: zufall(&mut z) % 3,
                            umsortieren: true,
                            wiederholen: true,
                            ..Boese::default()
                        });
                        let _ = pc.herunterladen(&hub);
                        hub.ehrlich();
                    }
                    12 => {
                        // Ungueltige Zwischenfassung ueber `fassungen_lesen` (Runde 7): G ist von `wer` signiert,
                        // aber ungueltig (Kopf: fremde Unterhaltung; Lease: ungueltiger Inhalt), T ist gueltig und
                        // nennt hash(G) als Vorgaenger. G setzt an der Genesis oder direkt am Hoechststand des PCs an.
                        let lease = zufall(&mut z).is_multiple_of(2);
                        let (sg, oid) = if lease {
                            (Sammlung::Sitzung, pc.lease_objekt_id(&r))
                        } else {
                            (Sammlung::Unterhaltung, pc.kopf_objekt_id(&r))
                        };
                        let g_glied = match pc.hoechststand(sg, &oid) {
                            Some(h) if zufall(&mut z).is_multiple_of(2) => Glied {
                                version: h.version + 1,
                                vorgaenger: h.hash,
                            },
                            _ => Glied {
                                version: 1,
                                vorgaenger: [0; 32],
                            },
                        };
                        // Falsche innere Version (Runde 8): sonst gueltige Lease mit innerem "0.1.5" bzw. Kopf
                        // mit fremdem Versionsfeld.
                        let falsche_version = zufall(&mut z).is_multiple_of(2);
                        let (g_ref, g_inhalt) = if lease && falsche_version {
                            (
                                r.clone(),
                                crate::sync::lease::klartext_fuer_test(
                                    "0.1.5",
                                    wer,
                                    98,
                                    T0 + 600,
                                    &r,
                                ),
                            )
                        } else if lease {
                            (r.clone(), serde_json::json!({"n": n}))
                        } else if falsche_version {
                            let mut v =
                                serde_json::to_value(KopfInhalt { kopf: kopf_t("g") }).unwrap();
                            v["kopf"]["schema_version"] = "0.1.5".into();
                            (r.clone(), v)
                        } else {
                            (
                                oid.clone(),
                                serde_json::to_value(KopfInhalt { kopf: kopf_t("g") }).unwrap(),
                            )
                        };
                        let t_inhalt = if lease {
                            crate::sync::lease::klartext_fuer_test(
                                crate::sync::SCHEMA_VERSION,
                                wer,
                                99,
                                T0 + 600,
                                &r,
                            )
                        } else {
                            serde_json::to_value(KopfInhalt {
                                kopf: kopf_t(&format!("gift{n}")),
                            })
                            .unwrap()
                        };
                        let c = 10_000 + 2 * n as u64;
                        let gu = versiegeln(
                            k.bund(),
                            k.schluessel(wer),
                            c,
                            Some(g_glied),
                            &Steuerfelder::inhalt(sg, oid.clone(), g_ref, wer, 1),
                            &g_inhalt,
                        )
                        .unwrap();
                        let g_hash = oeffnen(&pc.z.bund, &pc.z.verzeichnis, &gu).unwrap().hash;
                        let tu = versiegeln(
                            k.bund(),
                            k.schluessel(wer),
                            c + 1,
                            Some(Glied {
                                version: g_glied.version + 1,
                                vorgaenger: g_hash,
                            }),
                            &Steuerfelder::inhalt(sg, oid, r.clone(), wer, 1),
                            &t_inhalt,
                        )
                        .unwrap();
                        gift.insert(g_hash);
                        gift.insert(oeffnen(&pc.z.bund, &pc.z.verzeichnis, &tu).unwrap().hash);
                        hub.einschleusen(&gu);
                        hub.einschleusen(&tu);
                    }
                    10 if !widerrufen && schritt > 20 => {
                        widerrufen = true;
                        let _ = ip.widerrufen(&hub, "b");
                    }
                    _ => {
                        drop(pc);
                        pc = neu_laden(&speicher);
                    }
                }
                if zufall(&mut z).is_multiple_of(4) {
                    pc.cursor_zuruecksetzen();
                }
                let _ = pc.herunterladen(&hub);
                invariante_pruefen(&pc, &hub, &format!("seed {seed:x} schritt {schritt}"));
                gift_pruefen(&pc, &gift, &format!("seed {seed:x} schritt {schritt}"));
            }
            drop(pc);
            let pc = neu_laden(&speicher);
            invariante_pruefen(&pc, &hub, &format!("seed {seed:x} nach neustart"));
            gift_pruefen(&pc, &gift, &format!("seed {seed:x} nach neustart"));
        }
    }

    /// Keine Fassung, die nur ueber einen ungueltigen Beleg abstammt (oder selbst ungueltig ist), ist wirksam,
    /// gemerkt oder als Gabelungsseite festgehalten (Runde 7).
    fn gift_pruefen(pc: &Abgleich, gift: &BTreeSet<[u8; 32]>, kontext: &str) {
        for ((s, _), h) in &pc.z.hoechst.0 {
            assert!(
                !gift.contains(&h.hash),
                "{kontext}: Hoechststand {s:?} aus ungueltigem Beleg"
            );
        }
        for ((s, _), a) in &pc.z.angenommen.0 {
            assert!(
                !gift.contains(&a.hash),
                "{kontext}: Herkunft {s:?} aus ungueltigem Beleg"
            );
        }
        for g in &pc.z.gabelungen {
            assert!(
                !gift.contains(&g.hash),
                "{kontext}: Gabelung mit ungueltigem Beleg"
            );
        }
    }

    /// MF1 (Runde 7): Der Hub liefert ueber `fassungen_lesen` eine von `a` signierte, aber ungueltige
    /// Zwischenfassung G (Kopf: Objekt-ID des Kopfs von R mit `unterhaltung_ref = S`; Lease: Genesis mit
    /// ungueltigem Inhalt) und als aktuelles Objekt eine gueltige Fassung T (Vorgaenger hash(G)). G belegt
    /// nichts: T wird weder als Nachfolger noch als Gabelungsgewinner uebernommen, auch nicht nach Neustart
    /// und Vollabgleich.
    fn mf1_ungueltiger_beleg(lease: bool, falsche_version: bool, neustart: bool) {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "a"], &hub);
        let speicher = MemSpeicher::new();
        let mut pc = k.geraet_mit_speicher("pc", speicher.clone());
        let mut a = k.geraet("a");
        let r = a
            .unterhaltung_anlegen(kopf_t("echt"), Einschraenkung::default())
            .unwrap();
        let s = a
            .unterhaltung_anlegen(kopf_t("andere"), Einschraenkung::default())
            .unwrap();
        a.hochladen(&hub).unwrap();
        pc.herunterladen(&hub).unwrap();
        let (sammlung, oid) = if lease {
            (Sammlung::Sitzung, pc.lease_objekt_id(&r))
        } else {
            (Sammlung::Unterhaltung, pc.kopf_objekt_id(&r))
        };
        let vorher = pc.hoechststand(sammlung, &oid).map(|h| h.hash);
        // Kopf: G setzt direkt am echten Kopf v1 an (ohne Pruefung waere T Nachfolger); Lease: Genesis.
        let (g_glied, g_ref, g_inhalt) = if lease {
            (
                Glied {
                    version: 1,
                    vorgaenger: [0; 32],
                },
                r.clone(),
                if falsche_version {
                    crate::sync::lease::klartext_fuer_test("0.1.5", "a", 1, T0 + 600, &r)
                } else {
                    serde_json::json!({})
                },
            )
        } else {
            let h = pc.hoechststand(sammlung, &oid).expect("echter kopf");
            (
                Glied {
                    version: h.version + 1,
                    vorgaenger: h.hash,
                },
                s.clone(),
                serde_json::to_value(KopfInhalt { kopf: kopf_t("g") }).unwrap(),
            )
        };
        let t_inhalt = if lease {
            crate::sync::lease::klartext_fuer_test(
                crate::sync::SCHEMA_VERSION,
                "a",
                1,
                T0 + 600,
                &r,
            )
        } else {
            serde_json::to_value(KopfInhalt {
                kopf: kopf_t("boese"),
            })
            .unwrap()
        };
        let c = pc.gesehener_zaehler("a") + 100;
        let gu = versiegeln(
            k.bund(),
            k.schluessel("a"),
            c,
            Some(g_glied),
            &Steuerfelder::inhalt(sammlung, oid.clone(), g_ref, "a", 1),
            &g_inhalt,
        )
        .unwrap();
        let g_hash = oeffnen(&pc.z.bund, &pc.z.verzeichnis, &gu).unwrap().hash;
        let tu = versiegeln(
            k.bund(),
            k.schluessel("a"),
            c + 1,
            Some(Glied {
                version: g_glied.version + 1,
                vorgaenger: g_hash,
            }),
            &Steuerfelder::inhalt(sammlung, oid.clone(), r.clone(), "a", 1),
            &t_inhalt,
        )
        .unwrap();
        let t_hash = oeffnen(&pc.z.bund, &pc.z.verzeichnis, &tu).unwrap().hash;
        hub.einschleusen(&gu);
        hub.einschleusen(&tu);
        let gift: BTreeSet<[u8; 32]> = [g_hash, t_hash].into_iter().collect();
        let pruefen = |pc: &mut Abgleich, kontext: &str| {
            pc.herunterladen(&hub).unwrap();
            assert_eq!(
                pc.hoechststand(sammlung, &oid).map(|h| h.hash),
                vorher,
                "{kontext}: T nicht uebernommen"
            );
            assert!(
                !pc.gegabelt(sammlung, &oid),
                "{kontext}: keine Gabelung aus G"
            );
            gift_pruefen(pc, &gift, kontext);
            if lease {
                assert!(
                    matches!(
                        pc.lease_lesen(&hub, &r),
                        Err(crate::sync::lease::LeaseFehler::Luecke)
                    ),
                    "{kontext}: Lease bleibt Luecke"
                );
                gift_pruefen(pc, &gift, kontext);
            } else {
                assert_eq!(
                    pc.unterhaltung(&r).unwrap().kopf().titel,
                    "echt",
                    "{kontext}"
                );
            }
        };
        pruefen(&mut pc, "abruf");
        if neustart {
            drop(pc);
            pc = neu_laden(&speicher);
        }
        pc.cursor_zuruecksetzen();
        pruefen(&mut pc, "vollabgleich");
    }

    #[test]
    fn r7_ungueltige_kopf_zwischenfassung_belegt_nichts() {
        mf1_ungueltiger_beleg(false, false, false);
    }

    #[test]
    fn r7_ungueltige_kopf_zwischenfassung_belegt_nichts_nach_neustart() {
        mf1_ungueltiger_beleg(false, false, true);
    }

    #[test]
    fn r7_ungueltige_lease_genesis_belegt_nichts() {
        mf1_ungueltiger_beleg(true, false, false);
    }

    #[test]
    fn r7_ungueltige_lease_genesis_belegt_nichts_nach_neustart() {
        mf1_ungueltiger_beleg(true, false, true);
    }

    #[test]
    fn r8_lease_genesis_mit_falscher_innerer_version_belegt_nichts() {
        mf1_ungueltiger_beleg(true, true, false);
    }

    #[test]
    fn r8_lease_genesis_mit_falscher_innerer_version_belegt_nichts_nach_neustart() {
        mf1_ungueltiger_beleg(true, true, true);
    }

    /// MF1/MF7 (Runde 8): Eine gueltig signierte Lease-Genesis mit innerem "0.1.5" im aeusseren 0.1.6-Umschlag
    /// ist selbst das aktuelle Objekt. Abruf und `lease_lesen` weisen sie ab, auch nach Neustart und
    /// Vollabgleich; ein Kopf mit fremdem inneren Versionsfeld ebenso.
    fn falsche_innere_version_direkt(neustart: bool) {
        let hub = MemHub::new();
        let k = Testkonto::mit_hub(&["pc", "a"], &hub);
        let speicher = MemSpeicher::new();
        let mut pc = k.geraet_mit_speicher("pc", speicher.clone());
        let mut a = k.geraet("a");
        let r = a
            .unterhaltung_anlegen(kopf_t("echt"), Einschraenkung::default())
            .unwrap();
        a.hochladen(&hub).unwrap();
        pc.herunterladen(&hub).unwrap();
        let lease_oid = pc.lease_objekt_id(&r);
        let kopf_oid = pc.kopf_objekt_id(&r);
        let kopf_vorher = pc.hoechststand(Sammlung::Unterhaltung, &kopf_oid).unwrap();
        let c = pc.gesehener_zaehler("a") + 100;
        let lu = versiegeln(
            k.bund(),
            k.schluessel("a"),
            c,
            Some(Glied {
                version: 1,
                vorgaenger: [0; 32],
            }),
            &Steuerfelder::inhalt(Sammlung::Sitzung, lease_oid.clone(), r.clone(), "a", 1),
            &crate::sync::lease::klartext_fuer_test("0.1.5", "a", 1, T0 + 600, &r),
        )
        .unwrap();
        let mut kopf = serde_json::to_value(KopfInhalt {
            kopf: kopf_t("boese"),
        })
        .unwrap();
        kopf["kopf"]["schema_version"] = "0.1.5".into();
        let ku = versiegeln(
            k.bund(),
            k.schluessel("a"),
            c + 1,
            Some(Glied {
                version: kopf_vorher.version + 1,
                vorgaenger: kopf_vorher.hash,
            }),
            &Steuerfelder::inhalt(Sammlung::Unterhaltung, kopf_oid.clone(), r.clone(), "a", 1),
            &kopf,
        )
        .unwrap();
        hub.einschleusen(&lu);
        hub.einschleusen(&ku);
        let pruefen = |pc: &mut Abgleich, kontext: &str| {
            let b = pc.herunterladen(&hub).unwrap();
            assert!(b.verworfen >= 2, "{kontext}: {b:?}");
            assert!(
                pc.hoechststand(Sammlung::Sitzung, &lease_oid).is_none(),
                "{kontext}"
            );
            assert_eq!(
                pc.hoechststand(Sammlung::Unterhaltung, &kopf_oid)
                    .map(|h| h.hash),
                Some(kopf_vorher.hash),
                "{kontext}"
            );
            assert_eq!(
                pc.unterhaltung(&r).unwrap().kopf().titel,
                "echt",
                "{kontext}"
            );
            assert!(
                matches!(
                    pc.lease_lesen(&hub, &r),
                    Err(crate::sync::lease::LeaseFehler::Ungueltig(_))
                ),
                "{kontext}: lease_lesen weist ab"
            );
            assert!(
                pc.hoechststand(Sammlung::Sitzung, &lease_oid).is_none(),
                "{kontext}"
            );
        };
        pruefen(&mut pc, "abruf");
        if neustart {
            drop(pc);
            pc = neu_laden(&speicher);
        }
        pc.cursor_zuruecksetzen();
        pruefen(&mut pc, "vollabgleich");
    }

    #[test]
    fn r8_falsche_innere_version_wird_in_abruf_und_lease_lesen_abgewiesen() {
        falsche_innere_version_direkt(false);
    }

    #[test]
    fn r8_falsche_innere_version_wird_in_abruf_und_lease_lesen_abgewiesen_nach_neustart() {
        falsche_innere_version_direkt(true);
    }

    /// Runde 8: Innere Strukturen ohne eigenes Versionsfeld (Beitrag, Kopf) weisen ein eingeschleustes
    /// fremdes Versions- oder Typfeld beim Deserialisieren ab; ein unbekannter Ereignistyp ebenso.
    #[test]
    fn r8_fremdes_versionsfeld_in_beitrag_und_kopf_ist_ungueltig() {
        let mut v = serde_json::to_value(Verlaufsobjekt::Beitrag(Beitrag::neu(
            "x",
            None,
            1,
            "a",
            Modus::Typed,
            "t",
        )))
        .unwrap();
        assert!(serde_json::from_value::<Verlaufsobjekt>(v.clone()).is_ok());
        v["schema_version"] = "0.1.5".into();
        assert!(serde_json::from_value::<Verlaufsobjekt>(v.clone()).is_err());
        v["typ"] = "beitrag_v2".into();
        assert!(serde_json::from_value::<Verlaufsobjekt>(v).is_err());
        let mut k = serde_json::to_value(KopfInhalt { kopf: kopf_t("k") }).unwrap();
        k["kopf"]["version"] = 2.into();
        assert!(serde_json::from_value::<KopfInhalt>(k).is_err());
    }
}
