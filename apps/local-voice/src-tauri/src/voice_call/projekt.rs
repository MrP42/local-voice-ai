//! Projektzweig des Sprachgesprächs (lva#365 P4c-2, Plan `docs/superpowers/plans/2026-11-10-projekte-p4c.md`).
//!
//! Gehört ein Gespräch zu einem Projekt (`conversations.scope.project_id`, aus dem Speicher gelesen, nie
//! aus der Oberfläche), laufen `voice_call_chat` (W1: Sprache und Text-Chat) und `voice_call_turn` (W2:
//! Orchestrator, Sprecher und Denker) ausschließlich über das Versandtor [`ProjektTor`]:
//! `vor_versand` → Versand mit `select!` auf den Abbruch → `abschliessen` bzw. `verwerfen`.
//!
//! Der Zweig zweigt **vor** `schutz_pruefen` (Notizbuchschutz) ab und lässt diesen unberührt: Im Projekt
//! gibt es keinen App-Kontext, kein Notizbuch und keine Werkzeuge. Der Kern ([`durchlauf`]) kennt weder
//! `AppHandle` noch Netz: Sendeweg ([`Modellweg`]) und Meldungen ([`Bote`]) sind austauschbar, damit er
//! mit Attrappen testbar ist (`projekt_tests`).
//!
//! # Regeln
//! - Fail-closed: Jeder Fehler, jede unlesbare Bindung und jede unklare Lage endet ohne Versand.
//! - Nichts verlässt Rust vor `abschliessen`: Sätze, Deltas und Verlauf werden erst nach dem Urteil des
//!   Tors an die Oberfläche gegeben. Bei `Verspaetet` und `Widerrufen` meldet der Kern `revoked`, die
//!   Rückgabe ist leer, und nichts wird registriert, gespeichert oder gesprochen.
//! - Werkzeuge sind im Projekt aus: Das Ausgabetor des Orchestrators erhält eine leere Werkzeugliste,
//!   Werkzeugmarker in der Modellausgabe fallen weg, und `voice_tool_call` verweigert in einem
//!   Projektgespräch ([`WERKZEUGE_AUS`]).
//! - `model_id` und `stufe` aus IPC wählen nur unter den aktivierten Modellen; Einstufung, Klasse und
//!   Ziel kommen aus dem Tor. Die Stufe kann nichts erweitern ([`stufe_zulaessig`]).
//! - Lokale Ziele werden nur über `Ziel::pinned_local` angesprochen, nie über `provider.base_url`
//!   ([`sendeadresse`]).
//! - Der Widerruf trifft genau die Anfragen, die `ProjektTor::widerrufen` nennt (`revoke_requests`).
//! - Keine Inhalte in Logs oder Fehlertexten: Es gehen nur Codes, Zähler und Kennungen heraus.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use once_cell::sync::Lazy;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use wai_assistent::Sprache;

use super::{mark_context, unmark_context, usage_payload, VoiceCallMessage};
use super::{CANCELLED, CANCELS, EVENT};
use crate::assistent::tor::{self, Ausgabetor, StromTor};
use crate::conversations::store::{ConversationStore, ProjectBinding};
use crate::llm_client::StreamMessage;
use crate::managers::usage::{self, Purpose};
use crate::projekte::tor::{
    Abbruch, Bindung, Nachricht, ProjektTor, PromptTeile, Rolle, Versandauftrag, Weg, Ziel,
};
use crate::projekte::zustand::ProjekteZustand;

/// Code und Text, wenn ein Werkzeug in einem Projektgespräch gerufen wird.
pub const WERKZEUGE_AUS: &str =
    "projekt.werkzeuge_aus: Im Projekt sind Werkzeuge aus. Es wurde nichts ausgeführt.";
/// Code, wenn die Bindung eines Gesprächs nicht sicher gelesen werden kann.
pub const CODE_BINDUNG: &str = "projekt.bindung";
/// Code und fester Text, wenn das Modell oder sein Sendeweg scheitert. Der Fehlertext des Anbieters
/// gelangt nie in die Oberfläche: Er kann Teile der Anfrage wiedergeben (Projektinhalte).
pub const CODE_MODELL: &str = "projekt.modell";
const MODELL_FEHLER: &str =
    "projekt.modell: Das Modell hat nicht geantwortet. Es wurde keine Antwort übernommen.";

/// Abstand des Lebenszeichens („läuft“) an die Oberfläche, solange das Tor die Antwort puffert. Das
/// Ereignis trägt nur `kind` und `request_id`: weder Text noch Längen oder Zähler. Die Oberfläche
/// verlängert damit ihre Frist bis zur harten Obergrenze (lva#459).
pub const HERZSCHLAG: Duration = Duration::from_secs(5);

/// Höchstzahl gemerkter gelöschter Projektgespräche (Grabsteine, siehe [`gespraech_geloescht`]).
const MAX_GRABSTEINE: usize = 256;

static GRABSTEINE_LISTE: Lazy<Mutex<VecDeque<String>>> = Lazy::new(Default::default);

fn grabsteine() -> std::sync::MutexGuard<'static, VecDeque<String>> {
    GRABSTEINE_LISTE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Merkt ein gelöschtes Projektgespräch. Läuft danach noch ein Aufruf mit seiner Kennung, wäre das
/// Gespräch „nicht gefunden“ und würde als gewöhnliches Gespräch behandelt: der Verlauf aus der
/// Oberfläche könnte Projektinhalte tragen. Mit dem Grabstein ist es ein Fehler.
pub fn gespraech_geloescht(id: &str) {
    let mut q = grabsteine();
    if !q.iter().any(|x| x == id) {
        q.push_back(id.to_string());
        while q.len() > MAX_GRABSTEINE {
            q.pop_front();
        }
    }
}

/// Löscht ein Gespräch aus dem Speicher. Gehört es zu einem Projekt (oder lässt sich die Bindung nicht
/// lesen), kommt der Grabstein **vor** dem Löschen: Nach dem Löschen wäre das Gespräch „nicht gefunden“,
/// also gewöhnlich, und ein Aufruf in diesem Augenblick bekäme den Verlauf der Oberfläche ungeprüft durch.
/// Scheitert das Löschen, bleibt der Grabstein harmlos, denn er gilt nur für ein fehlendes Gespräch.
/// Gibt zurück, ob es ein Projektgespräch war.
pub fn gespraech_loeschen(store: &ConversationStore, id: &str) -> anyhow::Result<bool> {
    gespraech_loeschen_mit(store, id, |s| s.delete(id))
}

fn gespraech_loeschen_mit(
    store: &ConversationStore,
    id: &str,
    loeschen: impl FnOnce(&ConversationStore) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    let projekt = !matches!(
        store.project_binding(id),
        Ok(ProjectBinding::None | ProjectBinding::Missing)
    );
    if projekt {
        gespraech_geloescht(id);
    }
    loeschen(store)?;
    Ok(projekt)
}

fn ist_geloescht(id: &str) -> bool {
    grabsteine().iter().any(|x| x == id)
}

fn bindung_fehler() -> String {
    format!(
        "{CODE_BINDUNG}: Die Zuordnung des Gesprächs zu einem Projekt ist nicht lesbar. Es wurde \
         nichts gesendet."
    )
}

/// Das Projekt des Gesprächs aus dem Speicher. `Ok(None)`: ein gewöhnliches Gespräch (oder noch keins
/// angelegt). Jeder Lesefehler ist ein `Err` (fail-closed), ebenso ein gelöschtes Projektgespräch.
pub fn bindung_aus_speicher(
    store: &ConversationStore,
    gespraech: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(id) = gespraech else {
        return Ok(None);
    };
    match store.project_binding(id) {
        Ok(ProjectBinding::Project(p)) => Ok(Some(p)),
        Ok(ProjectBinding::None) => Ok(None),
        Ok(ProjectBinding::Missing) if ist_geloescht(id) => Err(bindung_fehler()),
        Ok(ProjectBinding::Missing) => Ok(None),
        Err(_) => Err(bindung_fehler()),
    }
}

/// Wie [`bindung_aus_speicher`] mit dem Speicher der App. Fehlt der Speicher oder ist er nicht lesbar,
/// gibt es einen Fehler: Ohne Gewissheit über die Bindung wird nichts gesendet.
pub async fn bindung(app: &AppHandle, gespraech: Option<&str>) -> Result<Option<String>, String> {
    let Some(id) = gespraech else {
        return Ok(None);
    };
    let store = app
        .try_state::<Arc<ConversationStore>>()
        .map(|s| s.inner().clone())
        .ok_or_else(bindung_fehler)?;
    let id = id.to_string();
    tauri::async_runtime::spawn_blocking(move || bindung_aus_speicher(&store, Some(&id)))
        .await
        .map_err(|_| bindung_fehler())?
}

/// Darf ein Werkzeug laufen? In einem Projektgespräch nie ([`WERKZEUGE_AUS`]); ist die Bindung nicht
/// lesbar, ebenfalls nicht.
pub fn werkzeuge_erlaubt(bindung: &Result<Option<String>, String>) -> Result<(), String> {
    match bindung {
        Ok(None) => Ok(()),
        Ok(Some(_)) => Err(WERKZEUGE_AUS.to_string()),
        Err(e) => Err(e.clone()),
    }
}

// ---------------------------------------------------------------- Nähte

/// Meldungen an die Oberfläche (Ereignis `voice-call`). Produktion: `AppBote`.
pub trait Bote: Send + Sync {
    fn melden(&self, ereignis: serde_json::Value);
}

/// Sendeweg zum Modell. Produktion: [`LlmWeg`]; die Tests setzen eine Attrappe ein.
#[async_trait]
pub trait Modellweg: Send + Sync {
    /// Sendet und ruft `on_delta` je Textstück. Ein lokales Ziel geht nur an `ziel.pinned_local`.
    async fn senden(
        &self,
        ziel: &Ziel,
        nachrichten: Vec<StreamMessage>,
        on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<String, String>;
}

/// Adresse, an die der Versand gebunden ist: bei einem lokalen Ziel die vom Tor gestartete Adresse
/// (`pinned_local`), sonst keine. Die `base_url` des Anbieters gilt für Projektdaten nie als lokal:
/// Ein Endpunkt auf localhost kann weiterleiten. Fehlt die Adresse bei einem lokalen Ziel, gibt es
/// einen Fehler statt eines Versands.
pub fn sendeadresse(ziel: &Ziel) -> Result<Option<&str>, String> {
    match (ziel.anbieter.lokal, ziel.pinned_local.as_deref()) {
        (true, Some(a)) => Ok(Some(a)),
        (true, None) => Err("Lokales Modell nicht bereit".to_string()),
        (false, _) => Ok(None),
    }
}

/// Der echte Sendeweg über `llm_client`.
pub struct LlmWeg;

#[async_trait]
impl Modellweg for LlmWeg {
    #[allow(deprecated)]
    async fn senden(
        &self,
        ziel: &Ziel,
        nachrichten: Vec<StreamMessage>,
        on_delta: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<String, String> {
        let adresse = sendeadresse(ziel)?;
        crate::llm_versand::uebergang::send_chat_completion_stream_routed(
            Purpose::Chat,
            &ziel.provider,
            ziel.schluessel.clone().unwrap_or_default(),
            &ziel.modell,
            nachrichten,
            on_delta,
            adresse,
        )
        .await
    }
}

/// Die Meldungen der App: Ereignis `voice-call`.
pub struct AppBote(pub AppHandle);

impl Bote for AppBote {
    fn melden(&self, ereignis: serde_json::Value) {
        let _ = self.0.emit(EVENT, ereignis);
    }
}

// ---------------------------------------------------------------- Auftrag

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Art {
    /// W1: `voice_call_chat`, Sätze für Anzeige und Stimme.
    Chat,
    /// W2: `voice_call_turn`, roher Textstrom des Orchestrators (Sprecher oder Denker).
    Durchgang,
}

/// Was ein Durchgang braucht. Das Projekt und das Gespräch stammen aus dem Speicher, `modell` und
/// `stufe` aus der Oberfläche (nur Auswahl und Verschärfung).
pub struct Anfrage {
    pub request_id: String,
    pub projekt: String,
    pub gespraech: String,
    pub art: Art,
    pub funktion: String,
    pub modell: Option<String>,
    pub stufe: Option<String>,
    pub erwarteter_stand: Option<String>,
    pub nachrichten: Vec<VoiceCallMessage>,
    pub sprache: Sprache,
}

/// Die Stufe der Souveränität kann nichts erweitern: Es gilt das Strengere aus Stufe und Tor. `nur_lokal`
/// verlangt ein streng lokales Ziel (der eingebaute Server). Die Länderprüfung der Stufe `eu` macht der
/// Befehl vorab gegen die Verbindung (`turn::resolve`); das Tor erlaubt Cloud ohnehin nur bis
/// `Internal` und mit bekanntem Fenster.
pub fn stufe_zulaessig(stufe: &str, ziel: &Ziel) -> Result<(), String> {
    match stufe {
        "frei" | "eu" => Ok(()),
        "nur_lokal" if ziel.anbieter.lokal => Ok(()),
        "nur_lokal" => Err(format!(
            "Das Modell ist bei Souveränität „{stufe}“ nicht zulässig. Es wurde nichts gesendet."
        )),
        _ => Err("Unbekannte Souveränitätsstufe. Es wurde nichts gesendet.".to_string()),
    }
}

/// Belegregel und Legende für den Projektzweig: Aussagen enden mit der Marke der Quelle, deren Nummer im
/// Kopf des Blocks steht (`inhalt k` bzw. `auszug Qk`). Der Text gehört in den Systemtext **vor** dem
/// Tor, damit das Budget ihn mitzählt.
pub fn system_text(system: &str, sprache: Sprache) -> String {
    let legende = match sprache {
        Sprache::De => {
            "Belege sind die Projektinhalte und die Auszüge aus der Ablage im Kontext. Die Nummer im \
             Kopf eines Blocks (inhalt k, auszug Qk) ist die Quelle [Q:k]."
        }
        Sprache::En => {
            "The evidence is the project content and the excerpts from the filing system in the \
             context. The number in a block header (inhalt k, auszug Qk) is the source [Q:k]."
        }
    };
    let regel = tor::markerregel(sprache);
    if system.trim().is_empty() {
        format!("{regel}\n{legende}")
    } else {
        format!("{system}\n\n{regel}\n{legende}")
    }
}

/// Zerlegt die Nachrichten der Oberfläche in die Teile des Tors. Führende Systemnachrichten sind der
/// Systemtext; die letzte Nachricht der Person ist die Frage; davor stehen Verlauf (Nutzer, Antwort),
/// dahinter und dazwischen eingestreute Texte des Orchestrators (Denker) als Zwischenstände. Der
/// Kern kennt diese Texte nur über das Antwortregister: Ein unbekannter Zwischenstand wird verworfen.
/// Werkzeugergebnisse und Systemnachrichten mitten im Verlauf gibt es im Projekt nicht
/// (`Abbruch::Fremdkontext`).
pub fn teile_aus(nachrichten: &[VoiceCallMessage]) -> Result<PromptTeile, Abbruch> {
    let kopf = nachrichten
        .iter()
        .take_while(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect::<Vec<_>>();
    let rest = &nachrichten[kopf.len()..];
    let frage_nr = rest
        .iter()
        .rposition(|m| m.role == "user" && !tor::gerahmt(&m.content))
        .ok_or(Abbruch::Anfrage)?;
    let mut historie: Vec<Nachricht> = Vec::new();
    let mut zwischenstaende: Vec<String> = Vec::new();
    for (i, m) in rest.iter().enumerate() {
        if i == frage_nr {
            continue;
        }
        match m.role.as_str() {
            "user" if tor::werkzeugdaten(&m.content) => return Err(Abbruch::Fremdkontext),
            "user" if tor::gerahmt(&m.content) => zwischenstaende.push(m.content.clone()),
            "user" => historie.push(Nachricht::neu(Rolle::Nutzer, m.content.clone())),
            "assistant" if i < frage_nr => {
                historie.push(Nachricht::neu(Rolle::Antwort, m.content.clone()))
            }
            "assistant" => zwischenstaende.push(m.content.clone()),
            _ => return Err(Abbruch::Fremdkontext),
        }
    }
    Ok(PromptTeile {
        system: kopf.join("\n\n"),
        historie,
        zwischenstaende,
        retrieval: Vec::new(),
        frage: rest[frage_nr].content.clone(),
    })
}

fn rolle_text(r: Rolle) -> &'static str {
    match r {
        Rolle::System => "system",
        Rolle::Nutzer => "user",
        Rolle::Antwort => "assistant",
        Rolle::Werkzeug => "user",
    }
}

// ---------------------------------------------------------------- Kern

/// Räumt die Anmeldung eines Laufs auf, auch bei Abbruch der Aufgabe: Abbruch-Notify und Vermerke
/// weg. Die Markierung als Anfrage mit Kontext bleibt nach einer erfolgreichen Antwort stehen (die
/// Wiedergabe läuft über das Textende hinaus; Muster `voice_call_chat`).
struct Anmeldung {
    id: String,
    markierung_behalten: bool,
}

impl Anmeldung {
    /// Trägt den Lauf ein. Eine Kennung, die schon läuft, wird abgewiesen: Ein zweiter Aufruf darf den
    /// Abbruch des ersten nicht überschreiben und beim Aufräumen nicht mit entfernen.
    fn neu(id: &str) -> Result<(Self, Arc<Notify>), Abbruch> {
        let cancel = Arc::new(Notify::new());
        {
            let mut laufend = CANCELS.lock().unwrap_or_else(|e| e.into_inner());
            if laufend.contains_key(id) {
                return Err(Abbruch::Anfrage);
            }
            laufend.insert(id.to_string(), cancel.clone());
        }
        mark_context(id);
        Ok((
            Self {
                id: id.to_string(),
                markierung_behalten: false,
            },
            cancel,
        ))
    }
}

impl Drop for Anmeldung {
    fn drop(&mut self) {
        CANCELS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        CANCELLED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.id);
        if !self.markierung_behalten {
            unmark_context(&self.id);
        }
    }
}

/// Das Ausgabetor des Zweigs: Sätze (Chat) oder Textstrom ohne Werkzeuge (Durchgang).
enum Filter {
    Saetze(Ausgabetor),
    Strom(StromTor),
}

impl Filter {
    fn neu(art: Art, sprache: Sprache, quellen: &[(u32, String)]) -> Self {
        match art {
            Art::Chat => Filter::Saetze(Ausgabetor::neu(sprache, quellen)),
            // Leere Werkzeugliste: kein Marker ist ein gültiger Aufruf, Marker fallen weg.
            Art::Durchgang => Filter::Strom(StromTor::neu(sprache, quellen, Vec::new())),
        }
    }

    fn einspeisen(&mut self, delta: &str) -> Vec<String> {
        match self {
            Filter::Saetze(t) => t.einspeisen(delta),
            Filter::Strom(t) => vec![t.einspeisen(delta)],
        }
    }

    fn beenden(&mut self) -> Vec<String> {
        match self {
            Filter::Saetze(t) => t.beenden(),
            Filter::Strom(t) => vec![t.beenden()],
        }
    }

    /// Der freigegebene Text: das, was die Oberfläche als Antwort erhält und was das Register kennt.
    fn text(&self) -> String {
        match self {
            Filter::Saetze(t) => t.freigegebener_text(),
            Filter::Strom(t) => t.ausgabe(),
        }
    }
}

enum Ausgang {
    Widerrufen,
    Abgebrochen,
    Fertig(Result<String, String>),
}

fn budget_json(b: &crate::projekte::tor::Budgetbeleg) -> serde_json::Value {
    serde_json::json!({
        "fenster": b.fenster,
        "benoetigt": b.gesamt(),
        "system": b.system,
        "kontext": b.kontext,
        "historie": b.historie,
        "auszuege": b.auszuege,
        "zwischen": b.zwischen,
        "frage": b.frage,
        "puffer": b.puffer,
        "ausgabe": b.ausgabe,
    })
}

/// Hinweis an die Oberfläche (ohne Inhalte): Code, Zähler und, beim Budgetstopp, die Anteile.
fn hinweis(
    request_id: &str,
    code: &str,
    zurueckgesetzt: bool,
    entzogen: u32,
    budget: Option<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "kind": "projekt",
        "request_id": request_id,
        "code": code,
        "zurueckgesetzt": zurueckgesetzt,
        "entzogen": entzogen,
        "budget": budget,
    })
}

fn abbruch_melden(bote: &dyn Bote, id: &str, a: &Abbruch) {
    let budget = match a {
        Abbruch::Budget(b) => Some(budget_json(b)),
        _ => None,
    };
    bote.melden(hinweis(id, a.code(), false, 0, budget));
}

/// Ein Durchgang im Projekt. `Ok(text)`: der vom Tor freigegebene Text, den die Oberfläche genau so
/// im Verlauf führt (das Register kennt ihn); `Ok("")`: abgebrochen, widerrufen oder verspätet
/// (nichts wurde gezeigt, gesprochen oder registriert). `Err`: `<code>: <Text>` ohne Inhalte.
pub async fn durchlauf(
    tor: &ProjektTor,
    weg: &dyn Modellweg,
    bote: &dyn Bote,
    a: Anfrage,
) -> Result<String, String> {
    durchlauf_mit(tor, weg, bote, a, HERZSCHLAG).await
}

/// Wie [`durchlauf`] mit wählbarem Abstand des Lebenszeichens (Tests). Das Lebenszeichen läuft über den
/// **ganzen** Lauf (Tor vor dem Versand, Modell, Urteil des Tors mit der Ablage-Prüfung): Es wird nur
/// gesendet, solange der Kern noch wartet, und endet mit ihm.
pub async fn durchlauf_mit(
    tor: &ProjektTor,
    weg: &dyn Modellweg,
    bote: &dyn Bote,
    a: Anfrage,
    herzschlag: Duration,
) -> Result<String, String> {
    let id = a.request_id.clone();
    tokio::select! {
        biased;
        r = durchlauf_kern(tor, weg, bote, a) => r,
        nie = lebenszeichen(bote, &id, herzschlag) => match nie {},
    }
}

/// Meldet in festem Abstand „läuft“, ohne Inhalt: nur `kind` und `request_id`, keine Längen, keine Zähler.
/// Die Oberfläche verlängert damit ihre Frist (bis zur eigenen Obergrenze).
async fn lebenszeichen(bote: &dyn Bote, id: &str, abstand: Duration) -> std::convert::Infallible {
    let mut takt = tokio::time::interval_at(tokio::time::Instant::now() + abstand, abstand);
    takt.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        takt.tick().await;
        bote.melden(serde_json::json!({"kind": "laeuft", "request_id": id}));
    }
}

async fn durchlauf_kern(
    tor: &ProjektTor,
    weg: &dyn Modellweg,
    bote: &dyn Bote,
    a: Anfrage,
) -> Result<String, String> {
    let id = a.request_id.clone();
    // Abbruch und Widerruf zuerst eintragen, dann prüfen: Was dazwischen kommt, trifft die Anfrage.
    // Keine Meldung bei einer doppelten Kennung: Sie gälte dem schon laufenden Aufruf.
    let (mut anmeldung, cancel) = Anmeldung::neu(&id).map_err(|ab| ab.to_string())?;

    // Die Einstufung in Rust (ADR-0028 E4) gilt auch im Projekt: S0, ein Fehler, ein leerer oder zu
    // langer Text senden nichts.
    tor::vor_nachrichten(&a.nachrichten, a.sprache, Some(&a.gespraech))?;
    let mut teile = teile_aus(&a.nachrichten).map_err(|ab| {
        abbruch_melden(bote, &id, &ab);
        ab.to_string()
    })?;
    teile.system = system_text(&teile.system, a.sprache);

    let freigabe = tor
        .vor_versand(Versandauftrag {
            request_id: &id,
            bindung: Bindung {
                projekt: a.projekt.clone(),
                gespraech: Some(a.gespraech.clone()),
            },
            weg: Weg::Sprache,
            funktion: &a.funktion,
            modell: a.modell.as_deref(),
            erwarteter_stand: a.erwarteter_stand.as_deref(),
            teile,
        })
        .await
        .map_err(|ab| {
            abbruch_melden(bote, &id, &ab);
            ab.to_string()
        })?;
    let crate::projekte::tor::Freigabe {
        schein,
        nachrichten,
        ziel,
        budget: _,
        quellen,
        entzogen,
        zurueckgesetzt,
        abbruch,
        sitzung: _,
        frage: _,
    } = freigabe;

    // Die Stufe kann nichts erweitern; ein lokales Ziel braucht seine gebundene Adresse.
    let pruefung = a
        .stufe
        .as_deref()
        .map_or(Ok(()), |s| stufe_zulaessig(s, &ziel))
        .and_then(|()| sendeadresse(&ziel).map(|_| ()));
    if let Err(m) = pruefung {
        tor.verwerfen(schein);
        return Err(m);
    }
    // Ein Abbruch, der während der Prüfung kam, verhindert den Versand.
    if super::was_cancelled(&id) {
        tor.verwerfen(schein);
        return Ok(String::new());
    }
    if zurueckgesetzt || entzogen > 0 {
        bote.melden(hinweis(
            &id,
            if zurueckgesetzt {
                "projekt.zurueckgesetzt"
            } else {
                "projekt.entzogen"
            },
            zurueckgesetzt,
            entzogen,
            None,
        ));
    }

    let strom_nachrichten: Vec<StreamMessage> = nachrichten
        .iter()
        .map(|n| StreamMessage::new(rolle_text(n.rolle), n.text.clone()))
        .collect();
    // Alles, was das Modell liefert, wird gepuffert: Das Tor urteilt erst in `abschliessen`.
    let filter = Arc::new(Mutex::new(Filter::neu(a.art, a.sprache, &quellen)));
    let stuecke = Arc::new(Mutex::new(Vec::<String>::new()));
    let on_delta = {
        let (filter, stuecke) = (filter.clone(), stuecke.clone());
        move |delta: &str| {
            let neu = filter
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .einspeisen(delta);
            stuecke
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(neu.into_iter().filter(|s| !s.is_empty()));
        }
    };

    let (ausgang, erfasst) = usage::with_capture(async {
        let call = weg.senden(&ziel, strom_nachrichten, &on_delta);
        tokio::pin!(call);
        // Abbruch zuerst: Ein schon vorliegender Widerruf verhindert den Versand, statt mit ihm zu
        // wetteifern.
        let r = tokio::select! {
            biased;
            _ = abbruch.notified() => Ausgang::Widerrufen,
            _ = cancel.notified() => Ausgang::Abgebrochen,
            r = &mut call => Ausgang::Fertig(r),
        };
        (r, usage::captured_calls().0)
    })
    .await;

    match ausgang {
        Ausgang::Widerrufen => {
            tor.verwerfen(schein);
            bote.melden(serde_json::json!({"kind": "revoked", "request_id": id}));
            Ok(String::new())
        }
        Ausgang::Abgebrochen => {
            tor.verwerfen(schein);
            Ok(String::new())
        }
        Ausgang::Fertig(Err(_anbieterfehler)) => {
            // Der Text des Anbieters wird weder gezeigt noch protokolliert: nur der feste Code.
            tor.verwerfen(schein);
            bote.melden(hinweis(&id, CODE_MODELL, false, 0, None));
            bote.melden(serde_json::json!({
                "kind": "failed",
                "request_id": id,
                "usage": (!erfasst.is_empty()).then(|| usage_payload(&erfasst)),
            }));
            Err(MODELL_FEHLER.to_string())
        }
        Ausgang::Fertig(Ok(_roh)) => {
            // Das Urteil über den freigegebenen Text, nie über die rohe Modellausgabe.
            let (rest, text) = {
                let mut f = filter.lock().unwrap_or_else(|e| e.into_inner());
                (f.beenden(), f.text())
            };
            let mut stuecke =
                std::mem::take(&mut *stuecke.lock().unwrap_or_else(|e| e.into_inner()));
            stuecke.extend(rest.into_iter().filter(|s| !s.is_empty()));
            match tor.abschliessen(schein, &text).await {
                Ok(()) if super::was_cancelled(&id) => {
                    // Der Abbruch kam, während das Tor urteilte (nur im `select!` wirkt er sonst): Der
                    // Eintrag im Register wird zurückgenommen, nichts wird gemerkt oder gemeldet.
                    tor.antwort_vergessen(&a.gespraech, &text);
                    Ok(String::new())
                }
                Ok(()) => {
                    anmeldung.markierung_behalten = true;
                    rueckfrage_merken(&a.gespraech, &text);
                    ausliefern(bote, &id, a.art, &stuecke, &erfasst);
                    Ok(text)
                }
                Err(Abbruch::Verspaetet | Abbruch::Widerrufen) => {
                    // Verspätet: nichts zeigen, nichts sprechen, nichts registrieren.
                    bote.melden(serde_json::json!({"kind": "revoked", "request_id": id}));
                    Ok(String::new())
                }
                Err(ab) => {
                    abbruch_melden(bote, &id, &ab);
                    bote.melden(serde_json::json!({"kind": "failed", "request_id": id}));
                    Err(ab.to_string())
                }
            }
        }
    }
}

/// Merkt nur, **ob** die letzte freigegebene Ausgabe mit einer Rückfrage endet („ja“ danach ist eine
/// Fortsetzung und kein S0, siehe `tor::mit_fortsetzung`). Von der Antwort bleibt nichts im Speicher
/// der App außer diesem einen Zeichen: Projektinhalte stehen nie in dieser Liste.
fn rueckfrage_merken(gespraech: &str, text: &str) {
    tor::ausgabe_vergessen(Some(gespraech));
    if tor::endet_mit_rueckfrage(text) {
        tor::ausgabe_merken(Some(gespraech), "?");
    }
}

/// Gibt die freigegebenen Stücke an die Oberfläche, erst nach dem Urteil des Tors.
fn ausliefern(
    bote: &dyn Bote,
    id: &str,
    art: Art,
    stuecke: &[String],
    erfasst: &[usage::CapturedCall],
) {
    match art {
        Art::Chat => {
            for (i, s) in stuecke.iter().enumerate() {
                bote.melden(serde_json::json!({
                    "kind": "sentence", "request_id": id, "index": i, "text": s,
                }));
            }
            bote.melden(serde_json::json!({
                "kind": "done", "request_id": id, "usage": usage_payload(erfasst),
            }));
        }
        Art::Durchgang => {
            let text = stuecke.concat();
            if !text.is_empty() {
                bote.melden(serde_json::json!({"kind": "delta", "request_id": id, "text": text}));
            }
            if !erfasst.is_empty() {
                bote.melden(serde_json::json!({
                    "kind": "usage", "request_id": id, "usage": usage_payload(erfasst),
                }));
            }
        }
    }
}

// ---------------------------------------------------------------- Einstiege der Befehle

/// Die Dienste der Projekte; ohne sie gibt es keinen Projektweg.
async fn tor_der_app(app: &AppHandle) -> Result<Arc<ProjektTor>, String> {
    let z = app
        .try_state::<ProjekteZustand>()
        .ok_or_else(|| "Die Projekte sind nicht verfügbar.".to_string())?;
    Ok(z.dienste().await?.tor.clone())
}

/// W1: `voice_call_chat` in einem Projektgespräch. Ein mitgegebener App-Kontext wird nicht gelesen.
pub async fn chat(
    app: &AppHandle,
    request_id: String,
    messages: Vec<VoiceCallMessage>,
    projekt: String,
    gespraech: String,
    erwarteter_stand: Option<String>,
) -> Result<String, String> {
    let tor = tor_der_app(app).await?;
    durchlauf(
        &tor,
        &LlmWeg,
        &AppBote(app.clone()),
        Anfrage {
            request_id,
            projekt,
            gespraech,
            art: Art::Chat,
            funktion: crate::settings::FN_VOICE_SPEAKER.to_string(),
            modell: None,
            stufe: None,
            erwarteter_stand,
            nachrichten: messages,
            sprache: crate::assistent::sprache_der_app(app),
        },
    )
    .await
}

/// W2: `voice_call_turn` in einem Projektgespräch (Sprecher oder Denker).
#[allow(clippy::too_many_arguments)]
pub async fn durchgang(
    app: &AppHandle,
    request_id: String,
    messages: Vec<VoiceCallMessage>,
    model_id: String,
    stufe: String,
    for_function: Option<String>,
    projekt: String,
    gespraech: String,
    erwarteter_stand: Option<String>,
) -> Result<String, String> {
    let tor = tor_der_app(app).await?;
    durchlauf(
        &tor,
        &LlmWeg,
        &AppBote(app.clone()),
        Anfrage {
            request_id,
            projekt,
            gespraech,
            art: Art::Durchgang,
            funktion: for_function.unwrap_or_else(|| crate::settings::FN_VOICE_SPEAKER.to_string()),
            modell: Some(model_id),
            stufe: Some(stufe),
            erwarteter_stand,
            nachrichten: messages,
            sprache: crate::assistent::sprache_der_app(app),
        },
    )
    .await
}

/// Vergisst ein gelöschtes Projektgespräch im Antwortregister und merkt es sich als gelöscht.
pub async fn gespraech_vergessen(app: &AppHandle, gespraech: &str) {
    gespraech_geloescht(gespraech);
    tor::ausgabe_vergessen(Some(gespraech));
    let Ok(tor) = tor_der_app(app).await else {
        log::warn!("projekte: Gespräch nicht aus dem Antwortregister entfernt (Dienste fehlen)");
        return;
    };
    let id = gespraech.to_string();
    let _ = tauri::async_runtime::spawn_blocking(move || tor.gespraech_vergessen(&id)).await;
}

#[cfg(test)]
mod tests;
