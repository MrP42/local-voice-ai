//! Geraeteseitiger Abgleich: lokaler Stand, Hochladen, Herunterladen, Loeschen, Widerruf und Lease-Zugriff.
//!
//! Der Abgleich haelt je Unterhaltung Kopf, Verlauf und Einschraenkung und sprichgt ueber
//! [`HubSpeicher`] mit dem Hub. Es gelten durchgaengig:
//! - Vor der Sync-Freigabe geht nichts an den Hub und es wird nichts vom Hub gelesen (E3.6).
//! - `geraet_lokal`-Unterhaltungen werden nie hochgeladen; vertrauliche Beitraege darin erst recht nicht (E3.5, E10).
//! - Inhalte werden erst beim Hochladen mit der **aktuellen** Epoche versiegelt; nach einem Widerruf
//!   ueberleben so auch offline entstandene Beitraege.
//! - Fehler und Berichte enthalten keine Inhalte.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::fehler::SyncFehler;
use super::freigabe::{Einschraenkung, SyncFreigabe, Verbot, hochladen_erlaubt};
use super::grabstein::Grabsteine;
use super::hub::{AuthGeraet, HubFehler, HubSpeicher, LeseFilter, Schreiben};
use super::kennung::Hex128;
use super::lease::{self, Gelesen, Halt, LeaseFehler};
use super::migration::Migrationsmarke;
use super::schluessel::{Schluessel, Schluesselbund};
use super::umschlag::{
    Geoeffnet, Sammlung, Steuerfelder, Umschlag, Ziel, ZielTyp, oeffnen, versiegeln,
};
use super::verlauf::{Beitrag, Einfuegen, Verlauf};

/// Kopf einer Unterhaltung (Inhalt des Objekts `unterhaltung`).
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct KopfInhalt {
    kopf: Kopf,
    gewaehlt: BTreeMap<String, String>,
    #[serde(default)]
    uebernahmen: BTreeMap<u64, Option<String>>,
}

/// Obergrenze der zurueckgestellten (noch nicht entschluesselbaren) Objekte.
pub const MAX_ZURUECKGESTELLT: usize = 1000;

/// Objekt, das der Kern nur transportiert (`auftragsereignis`, `hinweis`; Paket A3).
#[derive(Clone, PartialEq)]
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

/// Lokaler Stand einer Unterhaltung.
#[derive(Clone, Debug)]
pub struct Unterhaltung {
    pub(super) kopf: Kopf,
    pub(super) einschraenkung: Einschraenkung,
    pub(super) verlauf: Verlauf,
    pub(super) weitere: BTreeMap<Hex128, Weiteres>,
    pub(super) kopf_revision: Option<u64>,
    pub(super) kopf_geaendert: bool,
    /// Die Unterhaltung war je nicht `geraet_lokal` oder es gab einen Upload-Versuch. Nur dann
    /// geht ein Grabstein an den Hub (E10: sonst Metadatenabfluss).
    pub(super) jemals_synchron: bool,
}

impl Unterhaltung {
    pub(super) fn neu(kopf: Kopf, einschraenkung: Einschraenkung) -> Self {
        Self {
            kopf,
            einschraenkung,
            verlauf: Verlauf::new(),
            weitere: BTreeMap::new(),
            kopf_revision: None,
            kopf_geaendert: true,
            jemals_synchron: !einschraenkung.geraet_lokal,
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

#[derive(Clone, Debug)]
struct OffenesGrab {
    ziel: Ziel,
    unterhaltung_ref: Hex128,
    /// Die Unterhaltung war je synchron (`jemals_synchron`); nur dann geht der Grabstein hoch.
    jemals_synchron: bool,
}

/// Ergebnis von [`Abgleich::hochladen`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Hochladebericht {
    pub beitraege: usize,
    pub koepfe: usize,
    pub grabsteine: usize,
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
    /// Durch einen bekannten Grabstein unterdrueckt (keine Wiederbelebung).
    pub unterdrueckt: usize,
    /// Verworfen: Steuerfelder, Schluessel oder Klartext passen nicht.
    pub verworfen: usize,
    /// Epoche unbekannt: zurueckgestellt bis der Schluessel ankommt.
    pub zurueckgestellt: usize,
    pub weitere: usize,
}

/// Vorgemerkter, noch nicht bestaetigter Geraetewiderruf. Der neue Epochenschluessel steht schon vor
/// dem Vergleichsschreiben fest und wird zusammen mit der Vorgangskennung festgehalten; geht die Antwort
/// des Hubs verloren, kann der Widerruf mit derselben Kennung wiederholt und der Schluessel uebernommen
/// werden. Der Aufrufer sichert den Wert dauerhaft (`Abgleich::offener_widerruf`) und stellt ihn nach
/// einem Neustart wieder her (`Abgleich::offenen_widerruf_wiederherstellen`).
#[derive(Clone, Debug)]
pub struct OffenerWiderruf {
    geraet: String,
    erwartete_epoche: u64,
    vorgang: Hex128,
    schluessel: Schluessel,
}

impl OffenerWiderruf {
    pub fn neu(
        geraet: &str,
        erwartete_epoche: u64,
        vorgang: Hex128,
        schluessel: Schluessel,
    ) -> Self {
        Self {
            geraet: geraet.to_owned(),
            erwartete_epoche,
            vorgang,
            schluessel,
        }
    }

    pub fn geraet(&self) -> &str {
        &self.geraet
    }

    pub fn erwartete_epoche(&self) -> u64 {
        self.erwartete_epoche
    }

    pub fn vorgang(&self) -> &Hex128 {
        &self.vorgang
    }

    /// Kandidat fuer den Schluessel der neuen Epoche (geheim; nur zum dauerhaften Sichern).
    pub fn schluessel(&self) -> &Schluessel {
        &self.schluessel
    }
}

/// Fehler, nach denen ein Widerruf-Kandidat endgueltig verworfen wird (nicht wiederholbar).
fn widerruf_endgueltig(f: &HubFehler) -> bool {
    matches!(
        f,
        HubFehler::EpocheKonflikt { .. }
            | HubFehler::Widerrufen
            | HubFehler::NichtZugelassen
            | HubFehler::Vertrag(_)
    )
}

/// Geraeteseitiger Sync-Kern.
#[derive(Debug)]
pub struct Abgleich {
    pub(super) geraet_id: String,
    auth: AuthGeraet,
    pub(super) bund: Schluesselbund,
    pub(super) freigabe: SyncFreigabe,
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
}

impl Abgleich {
    pub fn new(geraet_id: &str, bund: Schluesselbund) -> Self {
        Self {
            geraet_id: geraet_id.to_owned(),
            auth: AuthGeraet::new(geraet_id),
            bund,
            freigabe: SyncFreigabe::default(),
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
        }
    }

    pub fn geraet_id(&self) -> &str {
        &self.geraet_id
    }

    pub fn bund(&self) -> &Schluesselbund {
        &self.bund
    }

    pub fn bund_mut(&mut self) -> &mut Schluesselbund {
        &mut self.bund
    }

    pub fn freigabe(&self) -> &SyncFreigabe {
        &self.freigabe
    }

    /// Schaltet den Assistent-Sync ein (E3.6). Erst danach wird hochgeladen.
    pub fn freigabe_einschalten(&mut self) {
        self.freigabe.einschalten();
    }

    pub fn freigabe_ausschalten(&mut self) {
        self.freigabe.ausschalten();
    }

    pub fn grabsteine(&self) -> &Grabsteine {
        &self.grabsteine
    }

    pub fn unterhaltung(&self, r: &Hex128) -> Option<&Unterhaltung> {
        self.unterhaltungen.get(r)
    }

    pub fn unterhaltungen(&self) -> impl Iterator<Item = (&Hex128, &Unterhaltung)> {
        self.unterhaltungen.iter()
    }

    /// Anzahl insgesamt verworfener Objekte (kein Inhalt, nur Zaehler).
    pub fn verworfen(&self) -> u64 {
        self.verworfen
    }

    /// Hub-Revision, bis zu der alles verarbeitet ist (steht vor dem ersten nicht verarbeitbaren Objekt).
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    pub fn zurueckgestellt(&self) -> usize {
        self.zurueckgestellt.len()
    }

    /// Stabile Hub-Objekt-ID eines Beitrags, abgeleitet aus `(unterhaltung_ref, beitrag_id)`:
    /// gleiche Beitrags-IDs in verschiedenen Unterhaltungen kollidieren nicht.
    pub fn beitrag_objekt_id(&self, r: &Hex128, beitrag_id: &str) -> Hex128 {
        Hex128::abgeleitet(self.bund.id_salz(), "beitrag", &format!("{r}/{beitrag_id}"))
    }

    pub(super) fn kopf_objekt_id(&self, r: &Hex128) -> Hex128 {
        Hex128::abgeleitet(self.bund.id_salz(), "kopf", r.as_str())
    }

    /// Stabile `unterhaltung_ref` aus `(Datenbestand-ID, lokale ID)` (Migration): lokale IDs
    /// verschiedener Datenbestaende kollidieren nicht, Wiederholung bleibt auch unter neuer Geraete-ID stabil.
    pub(super) fn ref_aus_lokaler_id(&self, datenbestand_id: &str, lokale_id: &str) -> Hex128 {
        let eingabe = format!("{}/{}", datenbestand_id.len(), datenbestand_id) + "/" + lokale_id;
        Hex128::abgeleitet(self.bund.id_salz(), "ref", &eingabe)
    }

    fn grab_objekt_id(&self, ziel: &Ziel) -> Hex128 {
        let eingabe = format!("{:?}/{}", ziel.typ, ziel.wert);
        Hex128::abgeleitet(self.bund.id_salz(), "loeschmarke", &eingabe)
    }

    fn freigabe_pruefen(&self) -> Result<(), SyncFehler> {
        if self.freigabe.ist_aktiv() {
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
        self.unterhaltungen
            .insert(r.clone(), Unterhaltung::neu(kopf, einschraenkung));
        Ok(r)
    }

    pub fn kopf_aendern(&mut self, r: &Hex128, kopf: Kopf) -> Result<(), SyncFehler> {
        let u = self
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
        if self.grabsteine.schlaegt(r, &oid) {
            return Err(SyncFehler::Geloescht);
        }
        let u = self
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        Ok(u.verlauf.einfuegen(beitrag))
    }

    /// Ausdrueckliche Zweigwahl (E3.4); wird mit dem Kopf synchronisiert.
    pub fn zweig_waehlen(
        &mut self,
        r: &Hex128,
        eltern: Option<&str>,
        kind: &str,
    ) -> Result<(), SyncFehler> {
        let u = self
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        u.verlauf.zweig_waehlen(eltern, kind)?;
        u.kopf_geaendert = true;
        Ok(())
    }

    fn war_synchron(&self, r: &Hex128) -> bool {
        self.unterhaltungen
            .get(r)
            .map_or_else(|| self.hochgeladene_refs.contains(r), |u| u.jemals_synchron)
    }

    /// Loescht eine Unterhaltung lokal und merkt einen Grabstein fuer den Hub vor.
    pub fn unterhaltung_loeschen(&mut self, r: &Hex128) {
        let ziel = Ziel {
            typ: ZielTyp::Unterhaltung,
            wert: r.clone(),
        };
        let jemals_synchron = self.war_synchron(r);
        if self.grabsteine.merken(&ziel) {
            self.offene_grabsteine.push(OffenesGrab {
                ziel,
                unterhaltung_ref: r.clone(),
                jemals_synchron,
            });
        }
        self.unterhaltungen.remove(r);
    }

    /// Loescht einen Beitrag lokal (Platzhalter im Baum) und merkt einen Grabstein vor.
    pub fn beitrag_loeschen(&mut self, r: &Hex128, beitrag_id: &str) -> Result<(), SyncFehler> {
        let oid = self.beitrag_objekt_id(r, beitrag_id);
        let u = self
            .unterhaltungen
            .get_mut(r)
            .ok_or(SyncFehler::UnbekannteUnterhaltung)?;
        u.verlauf.loeschen(beitrag_id);
        let jemals_synchron = u.jemals_synchron;
        let ziel = Ziel {
            typ: ZielTyp::Objekt,
            wert: oid,
        };
        if self.grabsteine.merken(&ziel) {
            self.offene_grabsteine.push(OffenesGrab {
                ziel,
                unterhaltung_ref: r.clone(),
                jemals_synchron,
            });
        }
        Ok(())
    }

    fn versiegeln_inhalt<T: Serialize>(
        &self,
        sammlung: Sammlung,
        objekt_id: Hex128,
        r: &Hex128,
        inhalt: &T,
    ) -> Result<Umschlag, SyncFehler> {
        let steuer = Steuerfelder::inhalt(
            sammlung,
            objekt_id,
            r.clone(),
            &self.geraet_id,
            self.bund.aktuelle_epoche(),
        );
        let wert = serde_json::to_value(inhalt).map_err(|_| SyncFehler::Inhalt)?;
        versiegeln(&self.bund, &steuer, &wert)
    }

    fn grab_umschlag(&self, g: &OffenesGrab) -> Result<Umschlag, SyncFehler> {
        let steuer = Steuerfelder::loeschmarke(
            self.grab_objekt_id(&g.ziel),
            g.unterhaltung_ref.clone(),
            &self.geraet_id,
            self.bund.aktuelle_epoche(),
            g.ziel.clone(),
        );
        versiegeln(&self.bund, &steuer, &Value::Object(serde_json::Map::new()))
    }

    /// Laedt Loeschmarken, Beitraege und Koepfe hoch. Ohne Freigabe: Fehler, kein Hub-Zugriff.
    pub fn hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Hochladebericht, SyncFehler> {
        self.freigabe_pruefen()?;
        self.offenen_widerruf_nachziehen(hub);
        let mut bericht = Hochladebericht::default();
        self.grabsteine_hochladen(hub, &mut bericht)?;
        let refs: Vec<Hex128> = self.unterhaltungen.keys().cloned().collect();
        for r in refs {
            self.unterhaltung_hochladen(hub, &r, &mut bericht)?;
        }
        Ok(bericht)
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
        while let Some(g) = self.offene_grabsteine.first().cloned() {
            if !g.jemals_synchron {
                self.offene_grabsteine.remove(0);
                continue;
            }
            let u = self.grab_umschlag(&g)?;
            hub.schreiben(&self.auth, &u)?;
            self.offene_grabsteine.remove(0);
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
        let Some(u) = self.unterhaltungen.get(r) else {
            return Ok(());
        };
        let einschraenkung = *u.einschraenkung();
        let kandidaten: Vec<Beitrag> = u.verlauf.beitraege().cloned().collect();
        let mut abgewiesen = false;
        for b in kandidaten {
            if let Err(v) = hochladen_erlaubt(&self.freigabe, &einschraenkung, &b) {
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
            if let Some(u) = self.unterhaltungen.get_mut(r) {
                u.jemals_synchron = true;
            }
            let oid = self.beitrag_objekt_id(r, &b.id);
            if abgewiesen || self.hochgeladen.contains(&oid) || self.grabsteine.schlaegt(r, &oid) {
                continue;
            }
            let umschlag = self.versiegeln_inhalt(Sammlung::Beitrag, oid.clone(), r, &b)?;
            match hub.schreiben(&self.auth, &umschlag) {
                Ok(Schreiben::Neu(_)) => bericht.beitraege += 1,
                Ok(Schreiben::Vorhanden(_)) => bericht.bereits_beim_hub += 1,
                Err(HubFehler::GrabsteinAbgewiesen) => {
                    bericht.grabstein_abgewiesen += 1;
                    abgewiesen = true;
                    continue;
                }
                Err(e) => return Err(e.into()),
            }
            self.hochgeladen.insert(oid);
            self.hochgeladene_refs.insert(r.clone());
        }
        if !einschraenkung.geraet_lokal && !abgewiesen {
            self.kopf_hochladen(hub, r, bericht)?;
        }
        Ok(())
    }

    fn kopf_hochladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        bericht: &mut Hochladebericht,
    ) -> Result<(), SyncFehler> {
        let Some(u) = self.unterhaltungen.get(r) else {
            return Ok(());
        };
        if !u.kopf_geaendert {
            return Ok(());
        }
        let inhalt = KopfInhalt {
            kopf: u.kopf.clone(),
            gewaehlt: u.verlauf.wahl().clone(),
            uebernahmen: u.verlauf.uebernahmen().clone(),
        };
        let erwartet = u.kopf_revision;
        let oid = self.kopf_objekt_id(r);
        let umschlag = self.versiegeln_inhalt(Sammlung::Unterhaltung, oid.clone(), r, &inhalt)?;
        match hub.cas(
            &self.auth,
            Sammlung::Unterhaltung,
            &oid,
            erwartet,
            &umschlag,
        ) {
            Ok(rev) => {
                if let Some(u) = self.unterhaltungen.get_mut(r) {
                    u.kopf_revision = Some(rev);
                    u.kopf_geaendert = false;
                }
                self.hochgeladene_refs.insert(r.clone());
                bericht.koepfe += 1;
            }
            Err(HubFehler::RevisionKonflikt { .. }) => bericht.kopf_konflikte += 1,
            Err(HubFehler::GrabsteinAbgewiesen) => bericht.grabstein_abgewiesen += 1,
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Holt neue Objekte vom Hub und fuehrt sie ein. Ohne Freigabe: Fehler, kein Hub-Zugriff.
    pub fn herunterladen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Herunterladebericht, SyncFehler> {
        self.freigabe_pruefen()?;
        self.offenen_widerruf_nachziehen(hub);
        let neu = hub.lesen(
            &self.auth,
            &LeseFilter {
                nach_revision: self.cursor,
                ..LeseFilter::default()
            },
        )?;
        // Die Hub-Epoche kommt NACH dem Lesen: sie ist nie kleiner als die Epoche eines gelesenen
        // Objekts. Nur was danach noch ueber ihr liegt, ist ungueltig; ein Widerruf zwischen beiden
        // Hub-Zugriffen verwirft so kein gueltiges Objekt.
        let hub_epoche = hub.epoche(&self.auth)?;
        let hoechste = neu
            .iter()
            .filter_map(|u| u.revision)
            .max()
            .unwrap_or(self.cursor);
        let wiederholen: Vec<Umschlag> = std::mem::take(&mut self.zurueckgestellt)
            .into_values()
            .collect();
        let mut bericht = Herunterladebericht::default();
        let mut sperre: Option<u64> = None;
        for u in wiederholen.iter().chain(neu.iter()) {
            self.verarbeiten(u, hub_epoche, &mut sperre, &mut bericht);
        }
        // Der Cursor laeuft nie ueber ein Objekt hinweg, das nicht zurueckgestellt werden konnte.
        let bis = sperre.map_or(hoechste, |r| hoechste.min(r.saturating_sub(1)));
        self.cursor = self.cursor.max(bis);
        bericht.zurueckgestellt = self.zurueckgestellt.len();
        Ok(bericht)
    }

    fn offenen_widerruf_nachziehen<H: HubSpeicher + ?Sized>(&mut self, hub: &H) {
        if self.offener_widerruf.is_some() {
            let _ = self.widerruf_abschliessen(hub);
        }
    }

    fn verwerfen(&mut self, bericht: &mut Herunterladebericht) {
        self.verworfen += 1;
        bericht.verworfen += 1;
    }

    fn verarbeiten(
        &mut self,
        u: &Umschlag,
        hub_epoche: u64,
        sperre: &mut Option<u64>,
        bericht: &mut Herunterladebericht,
    ) {
        if u.key_epoch > hub_epoche {
            self.verwerfen(bericht);
            return;
        }
        let g = match oeffnen(&self.bund, u) {
            Ok(g) => g,
            Err(SyncFehler::EpocheUnbekannt(_)) => {
                let platz = self.zurueckgestellt.contains_key(&u.objekt_id)
                    || self.zurueckgestellt.len() < MAX_ZURUECKGESTELLT;
                if platz {
                    self.zurueckgestellt.insert(u.objekt_id.clone(), u.clone());
                } else {
                    let rev = u.revision.unwrap_or(0);
                    *sperre = Some(sperre.map_or(rev, |s| s.min(rev)));
                }
                return;
            }
            Err(_) => {
                self.verwerfen(bericht);
                return;
            }
        };
        if g.steuer.sammlung == Sammlung::Loeschmarke {
            self.grabstein_anwenden(&g, bericht);
            return;
        }
        let r = &g.steuer.unterhaltung_ref;
        if self.grabsteine.schlaegt(r, &g.steuer.objekt_id) {
            bericht.unterdrueckt += 1;
            return;
        }
        if self
            .unterhaltungen
            .get(r)
            .is_some_and(|x| x.einschraenkung.geraet_lokal)
        {
            return;
        }
        let ok = match g.steuer.sammlung {
            Sammlung::Beitrag => self.beitrag_einordnen(&g, bericht),
            Sammlung::Unterhaltung => self.kopf_einordnen(&g, bericht),
            Sammlung::Auftragsereignis | Sammlung::Hinweis => self.weiteres_einordnen(&g, bericht),
            Sammlung::Sitzung | Sammlung::Loeschmarke => true,
        };
        if !ok {
            self.verwerfen(bericht);
        }
    }

    fn unterhaltung_holen(&mut self, r: &Hex128) -> &mut Unterhaltung {
        self.unterhaltungen.entry(r.clone()).or_insert_with(|| {
            let mut u = Unterhaltung::neu(Kopf::default(), Einschraenkung::default());
            u.kopf_geaendert = false;
            u
        })
    }

    fn beitrag_einordnen(&mut self, g: &Geoeffnet, bericht: &mut Herunterladebericht) -> bool {
        let Ok(beitrag) = serde_json::from_value::<Beitrag>(g.inhalt.clone()) else {
            return false;
        };
        if self.beitrag_objekt_id(&g.steuer.unterhaltung_ref, &beitrag.id) != g.steuer.objekt_id {
            return false;
        }
        self.hochgeladen.insert(g.steuer.objekt_id.clone());
        self.hochgeladene_refs
            .insert(g.steuer.unterhaltung_ref.clone());
        let u = self.unterhaltung_holen(&g.steuer.unterhaltung_ref);
        match u.verlauf.einfuegen(beitrag) {
            Einfuegen::Neu => bericht.beitraege_neu += 1,
            Einfuegen::Doppelt | Einfuegen::Konflikt | Einfuegen::Geloescht => {
                bericht.beitraege_doppelt += 1;
            }
        }
        true
    }

    fn kopf_einordnen(&mut self, g: &Geoeffnet, bericht: &mut Herunterladebericht) -> bool {
        let Ok(inhalt) = serde_json::from_value::<KopfInhalt>(g.inhalt.clone()) else {
            return false;
        };
        if self.kopf_objekt_id(&g.steuer.unterhaltung_ref) != g.steuer.objekt_id {
            return false;
        }
        let Some(rev) = g.revision else {
            return false;
        };
        self.hochgeladene_refs
            .insert(g.steuer.unterhaltung_ref.clone());
        let u = self.unterhaltung_holen(&g.steuer.unterhaltung_ref);
        if rev <= u.kopf_revision.unwrap_or(0) {
            return true;
        }
        if !u.kopf_geaendert {
            u.kopf = inhalt.kopf;
            u.verlauf.wahl_setzen(inhalt.gewaehlt);
            bericht.koepfe += 1;
        }
        u.verlauf.uebernahmen_vereinen(&inhalt.uebernahmen);
        u.kopf_revision = Some(rev);
        true
    }

    fn weiteres_einordnen(&mut self, g: &Geoeffnet, bericht: &mut Herunterladebericht) -> bool {
        let u = self.unterhaltung_holen(&g.steuer.unterhaltung_ref);
        let neu = u
            .weitere
            .insert(
                g.steuer.objekt_id.clone(),
                Weiteres {
                    sammlung: g.steuer.sammlung,
                    inhalt: g.inhalt.clone(),
                },
            )
            .is_none();
        if neu {
            bericht.weitere += 1;
        }
        true
    }

    fn grabstein_anwenden(&mut self, g: &Geoeffnet, bericht: &mut Herunterladebericht) {
        let Some(ziel) = g.steuer.ziel.clone() else {
            return;
        };
        if self.grabsteine.merken(&ziel) {
            bericht.grabsteine += 1;
        }
        self.offene_grabsteine.retain(|o| o.ziel != ziel);
        match ziel.typ {
            ZielTyp::Unterhaltung => {
                self.unterhaltungen.remove(&ziel.wert);
            }
            ZielTyp::Objekt => {
                let Some(id) = self
                    .unterhaltungen
                    .get(&g.steuer.unterhaltung_ref)
                    .and_then(|u| {
                        u.verlauf
                            .beitraege()
                            .find(|b| {
                                self.beitrag_objekt_id(&g.steuer.unterhaltung_ref, &b.id)
                                    == ziel.wert
                            })
                            .map(|b| b.id.clone())
                    })
                else {
                    return;
                };
                if let Some(u) = self.unterhaltungen.get_mut(&g.steuer.unterhaltung_ref) {
                    u.verlauf.loeschen(&id);
                }
            }
        }
    }

    /// Gerätewiderruf (E3.1) als Vergleichsschreiben auf die Epoche.
    ///
    /// Der Hub vergibt die neue Epochennummer (`erwartet = aktuelle Epoche dieses Geraets`); bei
    /// gleichzeitigem Widerruf verliert einer mit `HubFehler::EpocheKonflikt`, ohne dass lokal etwas
    /// geaendert wird. Der neue Schluessel und eine Vorgangskennung werden **vor** dem Schreiben als
    /// [`OffenerWiderruf`] vorgemerkt. Geht die Antwort verloren, wiederholt der naechste Aufruf (auch
    /// `hochladen`/`herunterladen`) denselben Vorgang: der Hub antwortet idempotent mit derselben Epoche
    /// und der Kandidat wird uebernommen. Der Schluessel darf nur an [`empfaenger_fuer_epoche`] gehen; das
    /// widerrufene Geraet bekommt ihn nie.
    pub fn widerrufen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        widerrufenes_geraet: &str,
    ) -> Result<u64, SyncFehler> {
        self.freigabe_pruefen()?;
        if let Some(offen) = self.offener_widerruf.clone() {
            if offen.geraet == widerrufenes_geraet {
                return self.widerruf_ausfuehren(hub, offen);
            }
            match self.widerruf_abschliessen(hub) {
                Ok(_) | Err(SyncFehler::Hub(HubFehler::EpocheKonflikt { .. })) => {}
                Err(e) => return Err(e),
            }
        }
        let offen = OffenerWiderruf {
            geraet: widerrufenes_geraet.to_owned(),
            erwartete_epoche: self.bund.aktuelle_epoche(),
            vorgang: Hex128::zufall()?,
            schluessel: Schluessel::zufall()?,
        };
        self.offener_widerruf = Some(offen.clone());
        self.widerruf_ausfuehren(hub, offen)
    }

    fn widerruf_ausfuehren<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        offen: OffenerWiderruf,
    ) -> Result<u64, SyncFehler> {
        match hub.widerrufen(
            &self.auth,
            &offen.geraet,
            offen.erwartete_epoche,
            &offen.vorgang,
        ) {
            Ok(epoche) => {
                self.offener_widerruf = None;
                self.bund.epoche_aufnehmen(epoche, offen.schluessel)?;
                Ok(epoche)
            }
            Err(f) => {
                if widerruf_endgueltig(&f) {
                    self.offener_widerruf = None;
                }
                Err(f.into())
            }
        }
    }

    /// Wiederholt einen vorgemerkten Widerruf (verlorene Antwort). `Ok(None)`: nichts offen.
    pub fn widerruf_abschliessen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
    ) -> Result<Option<u64>, SyncFehler> {
        self.freigabe_pruefen()?;
        let Some(offen) = self.offener_widerruf.clone() else {
            return Ok(None);
        };
        self.widerruf_ausfuehren(hub, offen).map(Some)
    }

    /// Vorgemerkter, noch nicht bestaetigter Widerruf (zum dauerhaften Sichern durch den Aufrufer).
    pub fn offener_widerruf(&self) -> Option<&OffenerWiderruf> {
        self.offener_widerruf.as_ref()
    }

    /// Stellt einen gesicherten Widerruf-Kandidaten nach einem Neustart wieder her.
    pub fn offenen_widerruf_wiederherstellen(&mut self, offen: OffenerWiderruf) {
        self.offener_widerruf = Some(offen);
    }

    /// Nimmt einen ueber die Kopplung verteilten Epochenschluessel auf. Eine bekannte Epochennummer mit
    /// anderem Schluessel ist ein Fehler ([`SyncFehler::EpocheKollision`]), nichts wird ueberschrieben.
    pub fn epoche_aufnehmen(
        &mut self,
        epoche: u64,
        schluessel: Schluessel,
    ) -> Result<(), SyncFehler> {
        self.bund.epoche_aufnehmen(epoche, schluessel)
    }

    /// Geraete, an die ein neuer Epochenschluessel gehen darf: die aktuelle Zulassungsliste des Hubs
    /// ohne dieses Geraet selbst.
    pub fn empfaenger_fuer_epoche<H: HubSpeicher + ?Sized>(
        &self,
        hub: &H,
    ) -> Result<Vec<String>, SyncFehler> {
        empfaenger_fuer_epoche(hub, &self.auth).map(|mut v| {
            v.retain(|g| g != &self.geraet_id);
            v
        })
    }

    fn lease_pruefen(&self, r: &Hex128) -> Result<(), LeaseFehler> {
        if !self.freigabe.ist_aktiv() {
            return Err(LeaseFehler::NichtFreigegeben);
        }
        if self
            .unterhaltungen
            .get(r)
            .is_some_and(|u| u.einschraenkung.geraet_lokal)
        {
            return Err(LeaseFehler::GeraetLokal);
        }
        if self.grabsteine.unterhaltung_geloescht(r) {
            return Err(LeaseFehler::Krypto(SyncFehler::Geloescht));
        }
        Ok(())
    }

    /// Liest die Lease der Unterhaltung samt Revision.
    pub fn lease_lesen<H: HubSpeicher + ?Sized>(
        &self,
        hub: &H,
        r: &Hex128,
    ) -> Result<Option<Gelesen>, LeaseFehler> {
        self.lease_pruefen(r)?;
        lease::lesen(hub, &self.bund, &self.auth, r)
    }

    /// "Hier uebernehmen" (E3.3): Vergleichsschreiben mit `generation + 1`.
    ///
    /// Bei Erfolg merkt der Verlauf die Uebernahme (Generation und aktuelle Hauptlinienspitze) und
    /// synchronisiert sie mit dem Kopf: Beitraege mit kleinerer Generation unterhalb dieser Spitze sind
    /// veraltet und nie Hauptlinie, auch ohne Geschwister. Vorher `herunterladen`, damit die Spitze
    /// den bekannten Stand wiedergibt; `lease_uebernehmen` ruft es selbst vorher auf.
    pub fn lease_uebernehmen<H: HubSpeicher + ?Sized>(
        &mut self,
        hub: &H,
        r: &Hex128,
        jetzt: u64,
        gelesen: Option<&Gelesen>,
    ) -> Result<Halt, LeaseFehler> {
        self.lease_pruefen(r)?;
        self.herunterladen(hub)?;
        let halt = lease::uebernehmen(hub, &self.bund, &self.auth, r, jetzt, gelesen)?;
        if let Some(u) = self.unterhaltungen.get_mut(r) {
            let spitze = u.verlauf.spitze(halt.generation.saturating_sub(1));
            if u.verlauf.uebernahme_merken(halt.generation, spitze) {
                u.kopf_geaendert = true;
            }
        }
        Ok(halt)
    }

    /// Verlaengerung durch den Inhaber, `generation` bleibt.
    pub fn lease_verlaengern<H: HubSpeicher + ?Sized>(
        &self,
        hub: &H,
        r: &Hex128,
        jetzt: u64,
        halt: &Halt,
    ) -> Result<Halt, LeaseFehler> {
        self.lease_pruefen(r)?;
        lease::verlaengern(hub, &self.bund, &self.auth, r, jetzt, halt)
    }
}

/// Geraete, an die ein neuer Epochenschluessel gehen darf: die aktuelle Zulassungsliste des Hubs
/// (zugelassen und nicht widerrufen). Ein neuer Epochenschluessel geht nur an diese Liste; der
/// Transport (Kopplung) gehoert nicht zu diesem Kern.
pub fn empfaenger_fuer_epoche<H: HubSpeicher + ?Sized>(
    hub: &H,
    auth: &AuthGeraet,
) -> Result<Vec<String>, SyncFehler> {
    Ok(hub.zugelassene(auth)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::memhub::MemHub;
    use crate::sync::verlauf::Modus;

    fn hub() -> MemHub {
        let h = MemHub::new();
        for g in ["pc", "iphone", "tablet"] {
            h.zulassen(g);
        }
        h
    }

    fn bund() -> Schluesselbund {
        Schluesselbund::zufaellig().unwrap()
    }

    fn mit_freigabe(name: &str, b: &Schluesselbund) -> Abgleich {
        let mut a = Abgleich::new(name, b.clone());
        a.freigabe_einschalten();
        a
    }

    fn b(id: &str, eltern: Option<&str>) -> Beitrag {
        Beitrag::neu(id, eltern, 1, "n", Modus::Typed, "Inhalt")
    }

    #[test]
    fn debug_enthaelt_weder_inhalt_noch_schluessel() {
        let mut a = Abgleich::new("pc", bund());
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
    fn unbekannte_epoche_wird_zurueckgestellt_und_spaeter_nachgeholt() {
        let hub = hub();
        let mut voll = bund();
        let mut pc = mit_freigabe("pc", &voll);
        let r = pc
            .unterhaltung_anlegen(Kopf::default(), Einschraenkung::default())
            .unwrap();
        pc.beitrag_anlegen(&r, b("m1", None)).unwrap();
        pc.hochladen(&hub).unwrap();
        let e = pc.widerrufen(&hub, "tablet").unwrap();
        voll.epoche_aufnehmen(e, pc.bund().schluessel(e).unwrap().clone())
            .unwrap();
        pc.beitrag_anlegen(&r, b("m2", Some("m1"))).unwrap();
        pc.hochladen(&hub).unwrap();

        // iPhone kennt Epoche 2 noch nicht (Schluessel ist unterwegs).
        let mut iphone = mit_freigabe(
            "iphone",
            &Schluesselbund::neu(
                Schluessel::aus_bytes(*pc.bund().id_salz().first_chunk::<32>().unwrap()),
                1,
                pc.bund().schluessel(1).unwrap().clone(),
            ),
        );
        let bericht = iphone.herunterladen(&hub).unwrap();
        assert_eq!(bericht.beitraege_neu, 1, "Epoche 1 ist lesbar");
        assert_eq!(bericht.zurueckgestellt, 1);
        assert_eq!(
            iphone.verworfen(),
            0,
            "unbekannte Epoche ist kein Verwerfen"
        );
        iphone
            .epoche_aufnehmen(e, pc.bund().schluessel(e).unwrap().clone())
            .unwrap();
        let bericht = iphone.herunterladen(&hub).unwrap();
        assert_eq!(bericht.beitraege_neu, 1);
        assert_eq!(iphone.zurueckgestellt(), 0);
        assert_eq!(iphone.unterhaltung(&r).unwrap().verlauf().len(), 2);
    }

    #[test]
    fn fehlgeschlagener_grabstein_upload_bleibt_vorgemerkt() {
        let hub = hub();
        let bund = bund();
        let mut pc = mit_freigabe("pc", &bund);
        let r = pc
            .unterhaltung_anlegen(Kopf::default(), Einschraenkung::default())
            .unwrap();
        pc.beitrag_anlegen(&r, b("m1", None)).unwrap();
        pc.hochladen(&hub).unwrap();
        pc.unterhaltung_loeschen(&r);
        hub.widerrufen(
            &AuthGeraet::new("iphone"),
            "pc",
            1,
            &Hex128::zufall().unwrap(),
        )
        .unwrap();
        assert_eq!(
            pc.hochladen(&hub),
            Err(SyncFehler::Hub(HubFehler::Widerrufen))
        );
        assert_eq!(
            pc.offene_grabsteine.len(),
            1,
            "Grabstein geht nicht verloren"
        );
    }

    #[test]
    fn auftragsereignisse_werden_transportiert() {
        let hub = hub();
        let bund = bund();
        let mut pc = mit_freigabe("pc", &bund);
        let r = pc
            .unterhaltung_anlegen(Kopf::default(), Einschraenkung::default())
            .unwrap();
        let steuer = Steuerfelder::inhalt(
            Sammlung::Auftragsereignis,
            Hex128::zufall().unwrap(),
            r.clone(),
            "pc",
            1,
        );
        let u = versiegeln(&bund, &steuer, &serde_json::json!({"art": "gestartet"})).unwrap();
        hub.schreiben(&AuthGeraet::new("pc"), &u).unwrap();
        let mut iphone = mit_freigabe("iphone", &bund);
        assert_eq!(iphone.herunterladen(&hub).unwrap().weitere, 1);
        let w = iphone
            .unterhaltung(&r)
            .unwrap()
            .weitere()
            .values()
            .next()
            .unwrap()
            .clone();
        assert_eq!(w.sammlung, Sammlung::Auftragsereignis);
        assert_eq!(iphone.herunterladen(&hub).unwrap().weitere, 0);
    }

    #[test]
    fn beitrag_mit_falscher_objekt_id_wird_verworfen() {
        let hub = hub();
        let bund = bund();
        let mut pc = mit_freigabe("pc", &bund);
        let r = pc
            .unterhaltung_anlegen(Kopf::default(), Einschraenkung::default())
            .unwrap();
        // Gueltig verschluesselt, aber die Objekt-ID gehoert nicht zur Beitrags-ID (Schluesselbesitzer-Fehler).
        let steuer = Steuerfelder::inhalt(Sammlung::Beitrag, Hex128::zufall().unwrap(), r, "pc", 1);
        let inhalt = serde_json::to_value(b("m1", None)).unwrap();
        let u = versiegeln(&bund, &steuer, &inhalt).unwrap();
        hub.schreiben(&AuthGeraet::new("pc"), &u).unwrap();
        let mut iphone = mit_freigabe("iphone", &bund);
        let bericht = iphone.herunterladen(&hub).unwrap();
        assert_eq!((bericht.beitraege_neu, bericht.verworfen), (0, 1));
    }

    #[test]
    fn kopfaenderung_wandert_per_vergleichsschreiben_lokale_aenderung_gewinnt_bei_konflikt() {
        let hub = hub();
        let bund = bund();
        let mut pc = mit_freigabe("pc", &bund);
        let r = pc
            .unterhaltung_anlegen(
                Kopf {
                    titel: "v1".into(),
                    ..Kopf::default()
                },
                Einschraenkung::default(),
            )
            .unwrap();
        assert_eq!(pc.hochladen(&hub).unwrap().koepfe, 1);
        assert_eq!(
            pc.hochladen(&hub).unwrap().koepfe,
            0,
            "unveraendert: kein Schreiben"
        );
        let mut iphone = mit_freigabe("iphone", &bund);
        iphone.herunterladen(&hub).unwrap();
        pc.kopf_aendern(
            &r,
            Kopf {
                titel: "v2".into(),
                ..Kopf::default()
            },
        )
        .unwrap();
        pc.hochladen(&hub).unwrap();
        iphone
            .kopf_aendern(
                &r,
                Kopf {
                    titel: "v3-iphone".into(),
                    ..Kopf::default()
                },
            )
            .unwrap();
        // iPhone kennt die neue Revision nicht: Vergleichsschreiben scheitert, nichts wird ueberschrieben.
        let bericht = iphone.hochladen(&hub).unwrap();
        assert_eq!((bericht.koepfe, bericht.kopf_konflikte), (0, 1));
        // Nach dem Pull kennt es die Revision; lokale ungesendete Aenderung bleibt und geht dann durch.
        iphone.herunterladen(&hub).unwrap();
        assert_eq!(iphone.unterhaltung(&r).unwrap().kopf().titel, "v3-iphone");
        assert_eq!(iphone.hochladen(&hub).unwrap().koepfe, 1);
        pc.herunterladen(&hub).unwrap();
        assert_eq!(pc.unterhaltung(&r).unwrap().kopf().titel, "v3-iphone");
    }
}
