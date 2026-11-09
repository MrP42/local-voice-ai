//! Anbindung des Gesprächsverlaufs an den Assistent-Sync (lva#388, ADR-0028 A1: E3, E10, E11).
//!
//! Der Sync-Kern (`wai_assistent::sync`) kennt keine Datenbank. Sein Zustand (Geräteschlüssel,
//! geprüftes Verzeichnis, Höchststände als Prüfpunkte, Grabsteine, Ausgang, Unterhaltungen) liegt
//! dauerhaft in einem Zustandsspeicher des Hosts (`assistent_zustand::DateiSpeicher`, E3.8); der Aufrufer
//! lädt ihn vor dem Lauf und übergibt den [`Abgleich`]. Dieses Modul gleicht ihn mit `conversations.db`
//! ab, lässt ihn gegen den Hub laufen und schreibt das Ergebnis zurück. Reine Logik über
//! [`ConversationStore`] und [`HubSpeicher`]: kein Tauri, kein Netz, keine Systemuhr.
//!
//! Ablauf eines Laufs ([`lauf`]):
//! 1. **Freigabe (E3.6):** Ohne Freigabe kein Hub-Zugriff und kein Lesen der Gespräche.
//! 2. **Herunterladen zuerst:** Der Stand des Hubs (Köpfe, Beiträge, Grabsteine) kommt vor den lokalen
//!    Daten in den Abgleich. So gelten bekannte Grabsteine für die Migration, und ein fremder Kopf
//!    wird nicht von einem lokalen Standardkopf überschrieben.
//! 3. **Migration (E11):** Lokale Gespräche gehen über `Abgleich::migrieren` ein (stabile Kennungen aus
//!    Datenbestand-ID und lokaler ID), Beiträge über `beitrag_anlegen`; Wiederholung ist idempotent.
//! 4. **Freigabeliste (E10), erzeugt vom Host:** `geraet_lokal` und vertrauliche Gespräche
//!    (`confidential`/`restricted`, auch unbekannte Datenklasse, sowie Sonderkategorie) gehen als
//!    `geraet_lokal` in den Abgleich und werden nie hochgeladen. Der Kern setzt das durch.
//! 5. **Grabsteine (E3.2):** Lokal gelöschte Gespräche (vom Speicher vorgemerkt) gehen je
//!    `unterhaltung_ref` hoch, wenn das Gespräch je für den Hub bestimmt war. Lokal gelöschte Gespräche,
//!    die nie für den Hub bestimmt waren, löscht der Kern nur lokal (kein Metadatenabfluss), damit sie
//!    aus dem dauerhaften Zustand nicht als „fremde“ Gespräche wiederkommen.
//! 6. **Hochladen**, dann **Zurückschreiben:** neue Gespräche und Beiträge anderer Geräte, geänderte
//!    Köpfe und eingetroffene Grabsteine (löschen lokal, belebt nichts wieder).
//!
//! Kein Inhalt in Fehlern oder Bericht: Zähler und feste Texte.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::DateTime;
use serde_json::Value;
use sha2::{Digest, Sha256};
use wai_assistent::sync::{
    Abgleich, AltUnterhaltung, Beitrag, Datenklasse, Einschraenkung, Hex128, HubSpeicher, Kopf,
    LeaseFehler, Modus, Schluesselbund, SyncFehler,
};

use super::store::{
    ConversationEntry, ConversationScope, ConversationStore, NewEntry, OffenerGrabstein, SyncStand,
    DEFAULT_DATA_CLASS, DEFAULT_TITLE, MAX_TEXT_CHARS, MAX_TITLE_CHARS,
};

const UNBEKANNT: &str = "unbekannt";
const ROLLEN: &[&str] = &["user", "assistant", "system", "tool"];
const MAX_QUELLEN: usize = 50;
const MAX_QUELLE_ZEICHEN: usize = 2000;

/// Warum ein Lauf nicht zustande kam. Keine Variante trägt Inhalte.
#[derive(Debug)]
pub enum LaufFehler {
    /// Der Assistent-Sync ist nicht freigegeben (E3.6); es gab keinen Hub-Zugriff.
    NichtFreigegeben,
    /// Fehler im Sync-Kern oder am Hub.
    Sync(SyncFehler),
    /// Fehler im lokalen Speicher (Meldungen des Speichers enthalten keine Inhalte).
    Speicher(String),
    /// Fehler an der Lease einer Sprachsitzung (E3.3).
    Lease(LeaseFehler),
    /// Das Gespräch ist dem Abgleich (noch) nicht bekannt.
    Unbekannt,
}

impl fmt::Display for LaufFehler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NichtFreigegeben => write!(f, "Der Assistent-Sync ist nicht freigegeben."),
            Self::Sync(e) => write!(f, "{e}"),
            Self::Speicher(m) => write!(f, "{m}"),
            Self::Lease(e) => write!(f, "{e}"),
            Self::Unbekannt => write!(f, "Das Gespräch ist dem Abgleich nicht bekannt."),
        }
    }
}

impl From<SyncFehler> for LaufFehler {
    fn from(e: SyncFehler) -> Self {
        Self::Sync(e)
    }
}

impl From<anyhow::Error> for LaufFehler {
    fn from(e: anyhow::Error) -> Self {
        Self::Speicher(e.to_string())
    }
}

/// Ergebnis eines Laufs: nur Zähler.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LaufBericht {
    /// Neu an den Hub geschickte Beiträge, Köpfe und Grabsteine.
    pub beitraege_hoch: usize,
    pub koepfe_hoch: usize,
    pub grabsteine_hoch: usize,
    /// Neue Gespräche und Beiträge anderer Geräte im lokalen Verlauf.
    pub unterhaltungen_neu: usize,
    pub beitraege_neu: usize,
    /// Lokal gelöscht, weil ein Grabstein eingetroffen ist.
    pub unterhaltungen_geloescht: usize,
    pub beitraege_geloescht: usize,
    /// Beiträge, die wegen `geraet_lokal` oder Vertraulichkeit nicht hochgeladen wurden.
    pub nur_lokal: usize,
    /// Vom Kern verworfene Objekte (Bindung der Steuerfelder, Schlüssel).
    pub verworfen: usize,
    /// Objekte, die auf einen Epochenschlüssel warten.
    pub zurueckgestellt: usize,
    /// Beiträge, deren Inhalt am Hub und lokal abweicht (nichts wird überschrieben).
    pub abweichungen: usize,
    /// Beiträge anderer Geräte, die lokal nicht ablegbar sind (Kennung oder Form).
    pub nicht_uebernommen: usize,
}

/// Hub-Kennung eines lokalen Gesprächs: wie `Abgleich::migrieren` sie ableitet (E11). Der Test
/// `ref_entspricht_der_migration` hält beide zusammen.
pub fn ref_aus_lokaler_id(bund: &Schluesselbund, datenbestand_id: &str, lokale_id: &str) -> Hex128 {
    let eingabe = format!("{}/{}", datenbestand_id.len(), datenbestand_id) + "/" + lokale_id;
    Hex128::abgeleitet(bund.id_salz(), "ref", &eingabe)
}

/// Datenklasse eines Gesprächs für den Kern; unlesbar gilt fail-closed als `restricted`.
fn klasse(scope: &ConversationScope) -> Datenklasse {
    serde_json::from_value(Value::String(scope.data_class.clone()))
        .unwrap_or(Datenklasse::Restricted)
}

fn klasse_name(k: Datenklasse) -> &'static str {
    match k {
        Datenklasse::Public => "PUBLIC",
        Datenklasse::Internal => "INTERNAL",
        Datenklasse::Personal => "PERSONAL",
        Datenklasse::Special => "SPECIAL",
        Datenklasse::Restricted | Datenklasse::Confidential => "RESTRICTED",
    }
}

fn strenge(k: Datenklasse) -> u8 {
    match k {
        Datenklasse::Public => 0,
        Datenklasse::Internal => 1,
        Datenklasse::Personal => 2,
        Datenklasse::Special => 3,
        Datenklasse::Restricted | Datenklasse::Confidential => 4,
    }
}

/// Freigabeliste des Hosts (E10): Darf dieses Gespräch an den Hub? `confidential` (entspricht
/// `RESTRICTED`), unbekannte Datenklasse und Sonderkategorie bleiben auf dem Gerät.
pub fn nur_auf_dem_geraet(scope: &ConversationScope, geraet_lokal: bool) -> bool {
    geraet_lokal || scope.special_category || klasse(scope).ist_vertraulich()
}

fn modus_aus(mode: &str) -> Modus {
    match mode {
        "typed" => Modus::Typed,
        "spoken" => Modus::Spoken,
        "dictated" => Modus::Dictated,
        "transcribed" => Modus::Transcribed,
        "system" => Modus::System,
        _ => Modus::Unbekannt,
    }
}

fn modus_name(m: Modus) -> &'static str {
    match m {
        Modus::Typed | Modus::Unbekannt => "typed",
        Modus::Spoken => "spoken",
        Modus::Dictated => "dictated",
        Modus::Transcribed => "transcribed",
        Modus::System => "system",
    }
}

/// Fehlende Angabe wird `unbekannt` (E11), nie erfunden; wie in der Migration des Kerns.
fn angabe(wert: &str) -> String {
    let t = wert.trim();
    if t.is_empty() { UNBEKANNT } else { t }.to_owned()
}

fn beitrag_aus(
    e: &ConversationEntry,
    hub_id: &str,
    eltern: Option<&str>,
    klasse: Datenklasse,
) -> Beitrag {
    Beitrag {
        id: hub_id.to_owned(),
        eltern_id: eltern.map(str::to_owned),
        lease_generation: 0,
        autor: angabe(&e.role),
        modus: modus_aus(&e.mode),
        text: e.text.clone(),
        modell: angabe(e.model.as_deref().unwrap_or("")),
        quellen: e.sources.clone(),
        anzeigezeit: angabe(&e.created_at),
        datenklasse: klasse,
    }
}

/// Fingerabdruck der lokal veränderbaren Kopffelder (Titel, Anheftung).
fn fingerabdruck(titel: &str, angeheftet: bool) -> String {
    let mut h = Sha256::new();
    h.update(titel.as_bytes());
    h.update([0, angeheftet as u8]);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sauberer_titel(titel: &str) -> String {
    let t: String = titel.split_whitespace().collect::<Vec<_>>().join(" ");
    let t: String = t.chars().take(MAX_TITLE_CHARS).collect();
    if t.trim().is_empty() || t == UNBEKANNT {
        DEFAULT_TITLE.to_owned()
    } else {
        t
    }
}

/// Ein lokales Gespräch im Lauf.
struct Lokal {
    stand: SyncStand,
    hub_ref: Hex128,
    einschraenkung: Einschraenkung,
    klasse: Datenklasse,
}

/// Hub-Kennungen der Beiträge, die nach einer Korrektur unter neuer Kennung hochgingen.
#[derive(Default)]
struct BeitragIds {
    /// Lokale Eintrags-ID → aktuelle Hub-Kennung.
    aktuell: BTreeMap<String, String>,
    /// Frühere Hub-Kennungen (Eintrags-ID, Hub-Kennung): am Hub zu begraben, sobald sie dort stehen.
    alt: Vec<(String, String)>,
}

impl BeitragIds {
    fn laden(store: &ConversationStore) -> Result<Self, LaufFehler> {
        let mut ids = Self::default();
        for (eintrag, hub, aktuell) in store.sync_beitrag_ids()? {
            if aktuell {
                ids.aktuell.insert(eintrag, hub);
            } else {
                ids.alt.push((eintrag, hub));
            }
        }
        Ok(ids)
    }

    fn hub_id<'a>(&'a self, eintrag: &'a str) -> &'a str {
        self.aktuell.get(eintrag).map_or(eintrag, String::as_str)
    }

    /// Alle Hub-Kennungen, die lokal schon einem Eintrag gehören (nie als neu importieren).
    fn bekannt(&self) -> impl Iterator<Item = &str> {
        self.aktuell
            .values()
            .map(String::as_str)
            .chain(self.alt.iter().map(|(_, h)| h.as_str()))
    }
}

/// Ein Sync-Lauf (siehe Moduldoku). `freigegeben` ist der Schalter `assistent_sync_aktiv`; ohne ihn
/// passiert nichts, auch kein Lesen und kein Hub-Zugriff. `konto` ist die Kennung des angemeldeten
/// Kontos: Gespräche, die unter einem anderen Konto für den Hub bestimmt wurden, gehen nie in dieses.
/// `abgleich` ist der aus dem Zustandsspeicher geladene Kern dieses Kontos und Geräts; er sichert sich
/// selbst (vor jedem Hub-Schreiben, nach jeder Hub-Runde) und ist nach einem Sicherungsfehler gesperrt.
pub fn lauf<H: HubSpeicher + ?Sized>(
    store: &ConversationStore,
    hub: &H,
    abgleich: &mut Abgleich,
    konto: &str,
    freigegeben: bool,
) -> Result<LaufBericht, LaufFehler> {
    lauf_mit(store, hub, abgleich, konto, freigegeben, &mut || {})
}

/// Wie [`lauf`]; `nach_momentaufnahme` läuft direkt nach der Momentaufnahme des Verlaufs und vor allem,
/// was an den Hub geht (Prüfpunkt für Tests des Löschfensters).
pub fn lauf_mit<H: HubSpeicher + ?Sized>(
    store: &ConversationStore,
    hub: &H,
    abgleich: &mut Abgleich,
    konto: &str,
    freigegeben: bool,
    nach_momentaufnahme: &mut dyn FnMut(),
) -> Result<LaufBericht, LaufFehler> {
    if !freigegeben {
        return Err(LaufFehler::NichtFreigegeben);
    }
    if abgleich.gesperrt() {
        // Eine Sicherung ist gescheitert: erst neu laden (E3.8, Must-fix 4).
        return Err(LaufFehler::Sync(SyncFehler::Speicher));
    }
    if !abgleich.freigabe().ist_aktiv() {
        abgleich.freigabe_einschalten()?;
    }
    let datenbestand = store.sync_datenbestand_id()?;
    let mut bericht = LaufBericht::default();
    let verworfen_vorher = abgleich.verworfen();

    // 2. Stand des Hubs zuerst.
    abgleich.herunterladen(hub)?;

    // 3./4. Momentaufnahme und Hub-Vormerkung in einer Transaktion: wer ab jetzt gelöscht wird,
    // hinterlässt einen Grabstein (kein Löschen ohne Grabstein im Fenster bis zum Hochladen).
    let zuordnung: BTreeMap<String, String> = store
        .sync_zuordnung()?
        .into_iter()
        .map(|(r, c)| (c, r))
        .collect();
    let staende = store.sync_alle_vormerken(konto, |st| {
        !nur_auf_dem_geraet(&st.detail.conversation.scope, st.geraet_lokal)
    })?;
    nach_momentaufnahme();
    let mut lokale: Vec<Lokal> = Vec::new();
    for stand in staende {
        // Gehört zu einem anderen Konto: nie in dieses hochladen, nie ändern.
        if stand.sync_konto.as_deref().is_some_and(|k| k != konto) {
            continue;
        }
        let conv_id = stand.detail.conversation.id.clone();
        let abgeleitet = ref_aus_lokaler_id(abgleich.bund(), &datenbestand, &conv_id);
        let Some(hub_ref) = hub_ref_fuer(
            abgleich,
            zuordnung.get(&conv_id).map(String::as_str),
            abgeleitet,
        ) else {
            // Gespeicherte fremde Kennung, die der Hub (noch) nicht kennt: nichts unter einer
            // abgeleiteten Kennung neu anlegen.
            continue;
        };
        let scope = &stand.detail.conversation.scope;
        let einschraenkung = Einschraenkung {
            geraet_lokal: nur_auf_dem_geraet(scope, stand.geraet_lokal),
            modell_lokal: false,
        };
        let klasse = klasse(scope);
        lokale.push(Lokal {
            stand,
            hub_ref,
            einschraenkung,
            klasse,
        });
    }

    let mut ids = BeitragIds::laden(store)?;
    for l in &lokale {
        lokal_einfuegen(store, abgleich, &datenbestand, l, &mut ids)?;
        // Eine bestehende Zuordnung (Gespräch von einem anderen Gerät) nie überschreiben.
        let c = &l.stand.detail.conversation.id;
        if !zuordnung.contains_key(c) {
            store.sync_zuordnung_setzen(l.hub_ref.as_str(), c)?;
        }
    }

    // 5. Lokal gelöschte Gespräche: Grabsteine vormerken. Erst jetzt gelesen, damit Löschungen seit
    // der Momentaufnahme dabei sind.
    let offen = store.sync_grabsteine_offen(konto)?;
    let mut ziele: Vec<(&OffenerGrabstein, Hex128)> = Vec::new();
    for g in &offen {
        let abgeleitet = ref_aus_lokaler_id(abgleich.bund(), &datenbestand, &g.conversation_id);
        let gespeichert = g
            .unterhaltung_ref
            .as_deref()
            .and_then(|r| Hex128::parse(r).ok());
        let hub_ref = match gespeichert {
            Some(r) if r != abgeleitet => r,
            _ => abgeleitet.clone(),
        };
        ziele.push((g, hub_ref.clone()));
        if abgleich.grabsteine().unterhaltung_geloescht(&hub_ref) {
            continue;
        }
        if abgleich.unterhaltung(&hub_ref).is_none() {
            // Nur ein lokal entstandenes Gespräch (abgeleitete Kennung) lässt sich so anlegen: für den
            // Kern eine für den Hub bestimmte Unterhaltung ohne Inhalt, damit der Grabstein hochgeht
            // (`jemals_synchron`). Eine fremde Kennung, die der Hub nicht kennt, bleibt offen.
            if hub_ref != abgeleitet {
                continue;
            }
            abgleich.migrieren(&datenbestand, &[leere_unterhaltung(&g.conversation_id)]);
        }
        abgleich.unterhaltung_loeschen(&hub_ref)?;
    }

    // Lokal gelöschte Gespräche, die nie für den Hub bestimmt waren (kein vorgemerkter Grabstein): der
    // Kern hat sie aus einem früheren Lauf im dauerhaften Zustand. Er löscht sie nur lokal (nie
    // synchron: kein Grabstein am Hub), damit sie nicht als Gespräch eines anderen Geräts wiederkommen.
    let noch_da = store.sync_ids()?;
    let vergessen: Vec<Hex128> = abgleich
        .migrationsmarke()
        .unterhaltungen
        .iter()
        .filter(|id| !noch_da.contains(*id))
        .map(|id| ref_aus_lokaler_id(abgleich.bund(), &datenbestand, id))
        .filter(|r| {
            abgleich
                .unterhaltung(r)
                .is_some_and(|u| !u.jemals_synchron())
        })
        .collect();
    for r in &vergessen {
        abgleich.unterhaltung_loeschen(r)?;
    }

    // 6. Hochladen.
    let hoch = abgleich.hochladen(hub)?;
    bericht.beitraege_hoch = hoch.beitraege;
    bericht.koepfe_hoch = hoch.koepfe;
    bericht.grabsteine_hoch = hoch.grabsteine;
    bericht.nur_lokal = hoch.geraet_lokal;
    // Als erledigt gilt ein Grabstein erst, wenn er für genau seine Kennung beim Kern steht.
    for (g, hub_ref) in &ziele {
        if abgleich.grabsteine().unterhaltung_geloescht(hub_ref) {
            store.sync_grabstein_erledigt(&g.conversation_id)?;
        }
    }

    zurueckschreiben(
        store,
        abgleich,
        &lokale,
        &datenbestand,
        konto,
        &ids,
        hoch.kopf_konflikte == 0,
        &mut bericht,
    )?;
    bericht.zurueckgestellt = abgleich.zurueckgestellt();
    bericht.verworfen = usize::try_from(abgleich.verworfen().saturating_sub(verworfen_vorher))
        .unwrap_or(usize::MAX);
    Ok(bericht)
}

/// Lease-Stand einer Sprachsitzung, ohne Inhalte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sitzungsstand {
    pub inhaber: String,
    pub generation: u64,
    /// Ablauf in Unix-Sekunden.
    pub gueltig_bis: u64,
}

/// Liest die Lease der Sprachsitzung eines lokalen Gesprächs (E3.3) und übernimmt sie auf Wunsch
/// („hier übernehmen“; die eigene, laufende Lease wird dabei mit gleicher Generation verlängert).
/// `None`, wenn der Hub keine Lease kennt und nicht übernommen werden soll. Ein Gespräch, das der
/// Abgleich nicht kennt (nie hochgeladen), ergibt [`LaufFehler::Unbekannt`].
pub fn sitzung<H: HubSpeicher + ?Sized>(
    store: &ConversationStore,
    hub: &H,
    abgleich: &mut Abgleich,
    conversation_id: &str,
    jetzt: u64,
    uebernehmen: bool,
) -> Result<Option<Sitzungsstand>, LaufFehler> {
    if !abgleich.freigabe().ist_aktiv() {
        return Err(LaufFehler::NichtFreigegeben);
    }
    let datenbestand = store.sync_datenbestand_id()?;
    let gespeichert = store
        .sync_zuordnung()?
        .into_iter()
        .find(|(_, c)| c == conversation_id)
        .map(|(r, _)| r);
    let abgeleitet = ref_aus_lokaler_id(abgleich.bund(), &datenbestand, conversation_id);
    let r = hub_ref_fuer(abgleich, gespeichert.as_deref(), abgeleitet)
        .filter(|r| abgleich.unterhaltung(r).is_some())
        .ok_or(LaufFehler::Unbekannt)?;
    let gelesen = abgleich.lease_lesen(hub, &r).map_err(LaufFehler::Lease)?;
    if !uebernehmen {
        return Ok(gelesen.map(|g| Sitzungsstand {
            inhaber: g.lease.inhaber,
            generation: g.lease.generation,
            gueltig_bis: g.lease.gueltig_bis,
        }));
    }
    let halt = abgleich
        .lease_uebernehmen(hub, &r, jetzt, gelesen.as_ref())
        .map_err(LaufFehler::Lease)?;
    Ok(Some(Sitzungsstand {
        inhaber: abgleich.geraet_id().to_owned(),
        generation: halt.generation,
        gueltig_bis: halt.gueltig_bis,
    }))
}

/// Kennung eines Gesprächs beim Hub. Ohne Zuordnung oder bei einer Zuordnung, die der lokalen
/// Ableitung entspricht, die abgeleitete Kennung. Eine fremde gespeicherte Kennung (Gespräch von einem
/// anderen Gerät) gilt nur, solange der Abgleich sie kennt oder ein Grabstein sie trägt; sonst `None`.
fn hub_ref_fuer(
    abgleich: &Abgleich,
    gespeichert: Option<&str>,
    abgeleitet: Hex128,
) -> Option<Hex128> {
    let Some(r) = gespeichert.and_then(|r| Hex128::parse(r).ok()) else {
        return Some(abgeleitet);
    };
    if r == abgeleitet
        || abgleich.unterhaltung(&r).is_some()
        || abgleich.grabsteine().unterhaltung_geloescht(&r)
    {
        return Some(r);
    }
    None
}

fn leere_unterhaltung(conv_id: &str) -> AltUnterhaltung {
    AltUnterhaltung {
        id: conv_id.to_owned(),
        titel: None,
        bereich: None,
        geraet_lokal: false,
        modell_lokal: false,
        eintraege: Vec::new(),
    }
}

/// Bringt ein lokales Gespräch samt Beiträgen in den Abgleich (idempotent).
///
/// Ein Beitrag, der schon am Hub steht und lokal anders lautet (Korrektur, unterbrochene Antwort),
/// geht als Grabstein auf das alte Objekt und als neuer Beitrag unter neuer Hub-Kennung hoch; lokal
/// bleibt die ID, die Zuordnung (`BeitragIds`) zeigt auf die neue Kennung.
fn lokal_einfuegen(
    store: &ConversationStore,
    abgleich: &mut Abgleich,
    datenbestand: &str,
    l: &Lokal,
    ids: &mut BeitragIds,
) -> Result<(), LaufFehler> {
    let c = &l.stand.detail.conversation;
    if abgleich.grabsteine().unterhaltung_geloescht(&l.hub_ref) {
        // Gelöscht (Grabstein): nichts hochladen, nichts wiederbeleben; der Rückschreib-Schritt
        // löscht das lokale Gespräch.
        return Ok(());
    }
    if abgleich.unterhaltung(&l.hub_ref).is_none() {
        abgleich.migrieren(
            datenbestand,
            &[AltUnterhaltung {
                id: c.id.clone(),
                titel: Some(c.title.clone()),
                bereich: c.scope.knowledge_area.clone(),
                geraet_lokal: l.einschraenkung.geraet_lokal,
                modell_lokal: false,
                eintraege: Vec::new(),
            }],
        );
        // Durch einen Grabstein gelöscht: der Kern legt sie nicht an; der Rückschreib-Schritt löscht
        // das lokale Gespräch.
        if abgleich.unterhaltung(&l.hub_ref).is_none() {
            return Ok(());
        }
    }
    abgleich.einschraenkung_setzen(&l.hub_ref, l.einschraenkung)?;
    kopf_abgleichen(abgleich, l)?;
    // Frühere Hub-Kennungen korrigierter Beiträge begraben (idempotent, auch nach einem Absturz).
    if !l.einschraenkung.geraet_lokal {
        for (_, alt) in &ids.alt {
            let am_hub = abgleich
                .unterhaltung(&l.hub_ref)
                .is_some_and(|u| u.verlauf().beitrag(alt).is_some());
            let oid = abgleich.beitrag_objekt_id(&l.hub_ref, alt);
            if am_hub && !abgleich.grabsteine().schlaegt(&l.hub_ref, &oid) {
                abgleich.beitrag_loeschen(&l.hub_ref, alt)?;
            }
        }
    }
    // Lineare Kette in lokaler Reihenfolge. Fehlerhafte Beiträge (`error`) bleiben lokal. Ist ein
    // Beitrag schon am Hub, gilt dessen Elternteil (die Struktur legt der erste Schreiber fest, auch
    // wenn sich lokal Vorgänger durch Löschen oder Einsortieren ändern).
    let mut vorher: Option<String> = None;
    for e in l
        .stand
        .detail
        .entries
        .iter()
        .filter(|e| e.status != "error")
    {
        let mut hub_id = ids.hub_id(&e.id).to_owned();
        let am_hub = abgleich
            .unterhaltung(&l.hub_ref)
            .and_then(|u| u.verlauf().beitrag(&hub_id))
            .map(|b| (b.eltern_id.clone(), b.text != e.text));
        let mut eltern = match &am_hub {
            Some((eltern, _)) => eltern.clone(),
            None => vorher.clone(),
        };
        if let Some((alte_eltern, true)) = am_hub {
            if !l.einschraenkung.geraet_lokal {
                // Korrektur eines schon hochgeladenen Beitrags: altes Objekt begraben, neu anlegen.
                let neu = neue_beitrags_id(&e.id);
                store.sync_beitrag_neu_setzen(&e.id, &hub_id, &neu)?;
                abgleich.beitrag_loeschen(&l.hub_ref, &hub_id)?;
                ids.alt.push((e.id.clone(), hub_id.clone()));
                ids.aktuell.insert(e.id.clone(), neu.clone());
                hub_id = neu;
                eltern = alte_eltern;
            }
        }
        match abgleich.beitrag_anlegen(
            &l.hub_ref,
            beitrag_aus(e, &hub_id, eltern.as_deref(), l.klasse),
        ) {
            Ok(_) | Err(SyncFehler::Geloescht) => {}
            Err(f) => return Err(f.into()),
        }
        vorher = Some(hub_id);
    }
    Ok(())
}

/// Neue Hub-Kennung für einen korrigierten Beitrag: gleicher Präfix wie die lokale ID, neue ULID.
fn neue_beitrags_id(lokal: &str) -> String {
    let praefix = lokal.split_once('_').map_or("entry", |(p, _)| p);
    format!("{praefix}_{}", ulid::Ulid::new())
}

/// Lokaler Titel oder lokale Anheftung gewinnen nur, wenn sie sich seit dem letzten Abgleich
/// geändert haben; sonst gilt der Kopf des Hubs (Fingerabdruck in `sync_kopf`).
fn kopf_abgleichen(abgleich: &mut Abgleich, l: &Lokal) -> Result<(), SyncFehler> {
    let c = &l.stand.detail.conversation;
    let lokal_fp = fingerabdruck(&c.title, c.pinned);
    let lokal_geaendert = l.stand.sync_kopf.as_deref() != Some(lokal_fp.as_str());
    if !lokal_geaendert {
        return Ok(());
    }
    let Some(u) = abgleich.unterhaltung(&l.hub_ref) else {
        return Ok(());
    };
    let mut kopf = u.kopf().clone();
    kopf.titel = c.title.clone();
    kopf.angeheftet = c.pinned;
    abgleich.kopf_aendern(&l.hub_ref, kopf)
}

/// `kopf_ok`: Der Hub hat alle Kopfänderungen dieses Laufs angenommen (kein Konflikt); nur dann gilt
/// ein lokal geänderter Kopf als abgeglichen.
fn zurueckschreiben(
    store: &ConversationStore,
    abgleich: &Abgleich,
    lokale: &[Lokal],
    datenbestand: &str,
    konto: &str,
    ids: &BeitragIds,
    kopf_ok: bool,
    bericht: &mut LaufBericht,
) -> Result<(), LaufFehler> {
    let mut behandelt: BTreeSet<Hex128> = BTreeSet::new();
    // Was seit der Momentaufnahme lokal gelöscht wurde, wird nicht mehr angefasst.
    let noch_da = store.sync_ids()?;

    // Lokale Gespräche: Grabsteine, Beiträge und Köpfe anderer Geräte.
    for l in lokale {
        behandelt.insert(l.hub_ref.clone());
        let conv_id = &l.stand.detail.conversation.id;
        if !noch_da.contains(conv_id) {
            continue;
        }
        if abgleich.grabsteine().unterhaltung_geloescht(&l.hub_ref) {
            store.sync_loeschen(conv_id)?;
            bericht.unterhaltungen_geloescht += 1;
            continue;
        }
        // Eingetroffene Grabsteine einzelner Beiträge.
        for e in &l.stand.detail.entries {
            let oid = abgleich.beitrag_objekt_id(&l.hub_ref, ids.hub_id(&e.id));
            if abgleich.grabsteine().schlaegt(&l.hub_ref, &oid) {
                store.sync_eintrag_loeschen(&e.id)?;
                bericht.beitraege_geloescht += 1;
            }
        }
        let Some(u) = abgleich.unterhaltung(&l.hub_ref) else {
            continue;
        };
        bericht.abweichungen += u.verlauf().konflikte().count();
        if l.einschraenkung.geraet_lokal {
            // Kein Hub-Sync dieses Gesprächs: auch nichts übernehmen.
            continue;
        }
        let vorhanden: BTreeSet<&str> = l
            .stand
            .detail
            .entries
            .iter()
            .map(|e| e.id.as_str())
            .chain(ids.bekannt())
            .collect();
        let n = beitraege_importieren(
            store,
            conv_id,
            u.verlauf().beitraege(),
            &vorhanden,
            &mut bericht.nicht_uebernommen,
        )?;
        bericht.beitraege_neu += n;
        kopf_uebernehmen(store, l, u.kopf(), kopf_ok)?;
    }

    // Lokal bekanntes Löschwissen bleibt dauerhaft: ein älterer oder fehlerhafter Hub-Stand belebt
    // nichts wieder. Frisch gelesen, damit auch Löschungen aus diesem Lauf zählen.
    let geloeschte: BTreeSet<Hex128> = store
        .sync_grabsteine_alle(konto)?
        .iter()
        .map(|g| {
            let abgeleitet = ref_aus_lokaler_id(abgleich.bund(), datenbestand, &g.conversation_id);
            g.unterhaltung_ref
                .as_deref()
                .and_then(|r| Hex128::parse(r).ok())
                .unwrap_or(abgeleitet)
        })
        .collect();

    // Gespräche anderer Geräte, die lokal noch fehlen.
    for (r, u) in abgleich.unterhaltungen() {
        if behandelt.contains(r)
            || geloeschte.contains(r)
            || abgleich.grabsteine().unterhaltung_geloescht(r)
        {
            continue;
        }
        let c = store.create(Some(sauberer_titel(&u.kopf().titel)))?;
        let strengste = u
            .verlauf()
            .beitraege()
            .map(|b| b.datenklasse)
            .max_by_key(|k| strenge(*k))
            .unwrap_or(Datenklasse::Internal);
        if klasse_name(strengste) != DEFAULT_DATA_CLASS {
            store.sync_datenklasse_setzen(&c.id, klasse_name(strengste), false)?;
        }
        let c = if u.kopf().angeheftet {
            store.set_pinned(&c.id, true)?
        } else {
            c
        };
        store.sync_zuordnung_setzen(r.as_str(), &c.id)?;
        store.sync_jemals_setzen(std::slice::from_ref(&c.id), konto)?;
        let n = beitraege_importieren(
            store,
            &c.id,
            u.verlauf().beitraege(),
            &BTreeSet::new(),
            &mut bericht.nicht_uebernommen,
        )?;
        bericht.beitraege_neu += n;
        bericht.unterhaltungen_neu += 1;
        store.sync_kopf_setzen(&c.id, &fingerabdruck(&c.title, u.kopf().angeheftet))?;
    }
    Ok(())
}

/// Legt Beiträge anderer Geräte lokal ab, die es noch nicht gibt. Gibt die Zahl neu angelegter zurück.
fn beitraege_importieren<'a>(
    store: &ConversationStore,
    conv_id: &str,
    beitraege: impl Iterator<Item = &'a Beitrag>,
    vorhanden: &BTreeSet<&str>,
    nicht_uebernommen: &mut usize,
) -> Result<usize, LaufFehler> {
    let mut neu: Vec<&Beitrag> = beitraege
        .filter(|b| !vorhanden.contains(b.id.as_str()))
        .collect();
    // Anzeigezeit nur zur Anzeige; die Reihenfolge im lokalen Verlauf folgt ihr, bei Gleichstand der ID.
    neu.sort_by_key(|b| (zeit_ms(&b.anzeigezeit).unwrap_or(i64::MAX), b.id.clone()));
    let mut angelegt = 0;
    for b in neu {
        let Some(eintrag) = eintrag_aus(b) else {
            *nicht_uebernommen += 1;
            continue;
        };
        store.append_entry(conv_id, eintrag)?;
        angelegt += 1;
    }
    Ok(angelegt)
}

fn zeit_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// Ein Beitrag eines anderen Geräts als lokaler Eintrag; `None`, wenn die Kennung keine gültige
/// Eintragskennung ist (dann würde die Wiederholung nie idempotent).
fn eintrag_aus(b: &Beitrag) -> Option<NewEntry> {
    let (praefix, rest) = b.id.split_once('_')?;
    let id_ok = (2..=12).contains(&praefix.len())
        && praefix.chars().all(|c| c.is_ascii_lowercase())
        && rest.chars().count() == 26
        && rest
            .chars()
            .all(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(c));
    if !id_ok {
        return None;
    }
    // E11: Unbekanntes wird nicht erfunden. Eine Rolle, die der lokale Verlauf nicht kennt, macht den
    // Beitrag nicht übernehmbar (er bleibt am Hub und wird gezählt).
    if !ROLLEN.contains(&b.autor.as_str()) {
        return None;
    }
    let rolle = b.autor.as_str();
    let text: String = b.text.chars().take(MAX_TEXT_CHARS).collect();
    Some(NewEntry {
        id: Some(b.id.clone()),
        role: rolle.to_owned(),
        mode: modus_name(b.modus).to_owned(),
        text,
        status: "complete".to_owned(),
        created_at: zeit_ms(&b.anzeigezeit).map(|_| b.anzeigezeit.clone()),
        author_id: None,
        triggered_by: None,
        model: Some(b.modell.clone())
            .filter(|m| m != UNBEKANNT && !m.is_empty() && m.chars().count() <= 256),
        attachments: Vec::new(),
        variants: Vec::new(),
        active_variant: None,
        sources: b
            .quellen
            .iter()
            .take(MAX_QUELLEN)
            .map(|q| q.chars().take(MAX_QUELLE_ZEICHEN).collect())
            .collect(),
        untrusted: None,
        reply_to: None,
        ext: None,
    })
}

/// Übernimmt Titel und Anheftung des Hubs, wenn lokal nichts Neueres liegt, und merkt den
/// Fingerabdruck des abgeglichenen Kopfes.
fn kopf_uebernehmen(
    store: &ConversationStore,
    l: &Lokal,
    hub_kopf: &Kopf,
    kopf_ok: bool,
) -> Result<(), LaufFehler> {
    let c = &l.stand.detail.conversation;
    let lokal_fp = fingerabdruck(&c.title, c.pinned);
    let lokal_geaendert = l.stand.sync_kopf.as_deref() != Some(lokal_fp.as_str());
    if lokal_geaendert {
        // Der lokale Kopf ging in `kopf_abgleichen` an den Hub; war er dort nicht annehmbar
        // (Konflikt), bleibt er „geändert“ und wird im nächsten Lauf erneut versucht.
        if kopf_ok {
            store.sync_kopf_setzen(&c.id, &lokal_fp)?;
        }
        return Ok(());
    }
    let titel = sauberer_titel(&hub_kopf.titel);
    if titel != c.title || hub_kopf.angeheftet != c.pinned {
        let neu = store.update(&c.id, Some(titel), Some(hub_kopf.angeheftet), None)?;
        store.sync_kopf_setzen(&c.id, &fingerabdruck(&neu.title, neu.pinned))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
