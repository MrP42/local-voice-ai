//! Suite-Projekte in Local Voice AI: Verknuepfung der Besprechungsordner und Buecher, Abgleich der
//! Loeschung (ADR-0026 PR-7).
//!
//! Ein Besprechungsordner kann auf ein Suite-Projekt zeigen (`meeting_folders.projekt_id`). Seine
//! Besprechungen erscheinen im Projektordner als Eintraege `wai://lva/besprechung/<id>` (Art
//! `meeting`), die nur auf die Besprechung zeigen. Buecher der Bibliothek lassen sich einem Projekt
//! zuordnen (`wai://lva/buch/<id>`, Art `book`); das ist nur der Verweis. Recherchen liegen als
//! `wai://lva/recherche/<id>` (Art `document`: `research` ist laut Vertrag Corporate LLM vorbehalten).
//!
//! Sicherheitsentscheidungen:
//! - Mandant und Person kommen aus dem `Principal`; fremde Datensaetze sind "nicht gefunden".
//! - Die Datenklasse des Suite-Projekts begrenzt die Zuordnung (`KlasseUeberProjekt`): Besprechungen
//!   (`PERSONAL`) werden in ein niedriger eingestuftes Projekt nicht still aufgenommen, sondern
//!   uebersprungen und gezaehlt.
//! - Rechte werden bei jedem Auflisten neu geprueft (`LvaHerkunft` ist eine frische Momentaufnahme).
//! - Loeschkaskade: verschwindet die Herkunft, entfernt `abgleich` den Eintrag und stellt eine
//!   Meldung in die Outbox; dieser Dienst bestaetigt nur Meldungen fuer Verweise der eigenen App
//!   (`wai://lva/`) und laesst fremde stehen. Wird das Suite-Projekt geloescht, loest der Dienst die
//!   Verknuepfung der Ordner und loescht dessen Recherche-Dokumente.
//! - Protokoll ohne Inhalte.

use super::audit::{AuditEreignis, AuditSenke};
use super::dateien;
use super::herkunft::{
    ablage_id, ablage_klasse, ablage_link, besprechung_id, besprechung_link, buch_id, buch_link,
    id_gueltig, projektdatei_id, projektdatei_link, recherche_id, recherche_link, seite_link,
    suite_id_gueltig, LvaHerkunft, KLASSE_BESPRECHUNG, KLASSE_BUCH, KLASSE_DATEI, KLASSE_SEITE,
    LINK_APP,
};
use super::kontext::{self, Form, Kontext, KontextForm, Mitglied, Schalter, Sorte};
use super::quellen::LvaQuellen;
use super::recherche::RechercheStore;
use super::{Fehler, Res};
use serde::Serialize;
use specta::Type;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wai_projekte::{
    Aenderung, Art, Eintragsansicht, Klasse, LoeschBericht, NeuerEintrag, NeuesProjekt, Projekt,
    ProjektFehler, Projektverzeichnis, ZugeordnetDurch,
};
use wai_security::Principal;

/// Hoechstlaenge von Titeln in Eintraegen (Vertrag `project-entry.title`).
const MAX_TITEL: usize = 300;
/// Projekt-KI erst nach P4c (Codex-Auflage 3): bis dahin liefert `kontext_fuer_ki` nichts.
const PROJEKT_KI_FREIGEGEBEN: bool = false;

impl From<ProjektFehler> for Fehler {
    fn from(e: ProjektFehler) -> Self {
        let (code, message) = match &e {
            ProjektFehler::NichtGefunden => ("projekte.not_found", "Nicht gefunden".to_string()),
            ProjektFehler::RevisionKonflikt { .. } => (
                "projekte.conflict",
                "Das Projekt wurde zwischenzeitlich geändert. Bitte neu laden.".to_string(),
            ),
            ProjektFehler::KlasseUeberProjekt => (
                "projekte.class",
                "Die Datenklasse liegt über der des Suite-Projekts. Das Projekt müsste zuerst hochgestuft werden."
                    .to_string(),
            ),
            ProjektFehler::KeinHerkunftsrecht => (
                "projekte.denied",
                "Kein Leserecht an der Herkunft des Eintrags.".to_string(),
            ),
            ProjektFehler::Archiviert => (
                "projekte.archived",
                "Das Suite-Projekt ist archiviert.".to_string(),
            ),
            ProjektFehler::SchemaNeuer { .. } => (
                "projekte.schema",
                "Das Projektverzeichnis stammt von einer neueren App-Version. Bitte die App aktualisieren (nur lesend)."
                    .to_string(),
            ),
            ProjektFehler::Doppelt => (
                "projekte.duplicate",
                "Der Eintrag ist dem Projekt bereits zugeordnet.".to_string(),
            ),
            ProjektFehler::Ungueltig(_)
            | ProjektFehler::LinkUngueltig(_)
            | ProjektFehler::ArtPasstNichtZurApp
            | ProjektFehler::HerabstufenGesperrt
            | ProjektFehler::PlanVergeben
            | ProjektFehler::ProjektHatPlan => ("projekte.invalid", e.to_string()),
            ProjektFehler::Speicher(m) => ("projekte.storage", m.clone()),
        };
        Fehler { code, message }
    }
}

fn clip(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

/// Klasse in der Schreibweise des Vertrags (`DataClass`), wie sie die Oberflaeche erwartet.
pub fn klasse_text(k: Klasse) -> &'static str {
    match k {
        Klasse::Public => "PUBLIC",
        Klasse::Internal => "INTERNAL",
        Klasse::Personal => "PERSONAL",
        Klasse::Special => "SPECIAL",
        Klasse::Restricted => "RESTRICTED",
    }
}

pub fn klasse_aus_text(s: &str) -> Option<Klasse> {
    Some(match s {
        "PUBLIC" => Klasse::Public,
        "INTERNAL" => Klasse::Internal,
        "PERSONAL" => Klasse::Personal,
        "SPECIAL" => Klasse::Special,
        "RESTRICTED" => Klasse::Restricted,
        _ => return None,
    })
}

fn art_text(a: Art) -> String {
    serde_json::to_value(a)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "document".into())
}

// ---------------------------------------------------------------- Sichten fuer die Oberflaeche

/// Suite-Projekt in der Sicht der Oberflaeche.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SuiteProjekt {
    pub id: String,
    pub title: String,
    pub goal: Option<String>,
    pub data_class: String,
    pub special_category: bool,
    pub archived: bool,
    /// Web ist fuer dieses Projekt freigegeben (Opt-in, protokolliert).
    pub web_research_enabled: bool,
    pub entry_count: u32,
    pub revision: String,
}

/// Eintrag des Projektordners. Ohne Leserecht fehlen `title` und `subtitle` (`available` = false).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EintragSicht {
    pub id: String,
    pub kind: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub origin_app: Option<String>,
    pub data_class: Option<String>,
    pub available: bool,
    pub assigned_at: Option<String>,
    /// `person` oder `automation`.
    pub assigned_by: Option<String>,
    /// Verweis auf die Herkunft (`wai://lva/...`), nur bei lesbaren Eintraegen.
    pub link: Option<String>,
}

/// Datei im Projektordner ohne Journaleintrag (Absturz), zum bewussten Aufnehmen angeboten.
#[derive(Debug, Clone, Serialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GefundeneDatei {
    pub id: String,
    pub name: String,
    pub size: f64,
}

/// Ergebnis des Abgleichs eines Ordners mit seinem Suite-Projekt.
#[derive(Debug, Clone, Default, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncBericht {
    /// Neu zugeordnete Besprechungen.
    pub zugeordnet: u32,
    /// Bereits zugeordnete Besprechungen.
    pub bereits: u32,
    /// Nicht zugeordnet, weil ihre Datenklasse ueber der des Suite-Projekts liegt.
    pub uebersprungen: u32,
    /// Eintraege, die den Ordner verlassen haben oder deren Herkunft nicht mehr existiert.
    pub entfernt: u32,
}

/// Was beim Loeschen eines Suite-Projekts mitging.
#[derive(Debug, Clone, Default, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoeschErgebnis {
    pub eintraege_entfernt: u32,
    pub ordner_geloest: u32,
    pub recherchen_geloescht: u32,
    pub externe_kopien: u32,
}

// ---------------------------------------------------------------- Dienst

pub struct ProjektDienst {
    verz: Projektverzeichnis,
    quellen: Arc<dyn LvaQuellen>,
    recherchen: RechercheStore,
    principal: Principal,
    audit: Option<Arc<dyn AuditSenke>>,
    /// Projektordner der App (`projekte/`): Kontextauswahl und Projektdateien (lva#365 P4).
    ablage: Option<PathBuf>,
    /// Dokumentenablage (`<Datenordner>/ablage`): Leserecht und Klasse der Verweisart `ablage`
    /// (Spec Ablage §11, D3-Vorbau).
    dokumentenablage: Option<PathBuf>,
    /// Serialisiert Zuordnen und Abgleich: sonst koennte eine Momentaufnahme veralten, waehrend ein
    /// Eintrag fuer eine gerade angelegte Herkunft entsteht, und der Abgleich ihn faelschlich entfernen.
    sperre: tokio::sync::Mutex<()>,
}

fn proj_aus(p: &Projekt, anzahl: u32) -> SuiteProjekt {
    SuiteProjekt {
        id: p.id.clone(),
        title: p.title.clone(),
        goal: p.goal.clone(),
        data_class: klasse_text(p.data_class).into(),
        special_category: p.special_category,
        archived: p.archived,
        web_research_enabled: p.web_research_enabled == Some(true),
        entry_count: anzahl,
        revision: p.revision.to_string(),
    }
}

fn quelle(e: String) -> Fehler {
    Fehler::neu("projekte.quelle", e)
}

impl ProjektDienst {
    /// Oeffnet (und migriert) das Projektverzeichnis. Eine neuere Schemaversion oeffnet nur lesend.
    pub async fn open(
        pfad: impl AsRef<Path>,
        quellen: Arc<dyn LvaQuellen>,
        recherchen: RechercheStore,
        principal: Principal,
    ) -> Res<Self> {
        Ok(Self {
            verz: Projektverzeichnis::oeffnen(pfad).await?,
            quellen,
            recherchen,
            principal,
            audit: None,
            ablage: None,
            dokumentenablage: None,
            sperre: tokio::sync::Mutex::new(()),
        })
    }

    /// Projektordner für Kontextauswahl und Projektdateien (lva#365 P4).
    pub fn with_ablage(mut self, ablage: impl Into<PathBuf>) -> Self {
        self.ablage = Some(ablage.into());
        self
    }

    /// Dokumentenablage für die Verweisart `ablage` (D3-Vorbau).
    pub fn with_dokumentenablage(mut self, dir: impl Into<PathBuf>) -> Self {
        self.dokumentenablage = Some(dir.into());
        self
    }

    fn ablage(&self) -> Res<&Path> {
        self.ablage
            .as_deref()
            .ok_or_else(|| Fehler::neu("projekte.storage", "Kein Projektordner verfügbar."))
    }

    pub fn with_audit(mut self, a: Arc<dyn AuditSenke>) -> Self {
        self.audit = Some(a);
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn verzeichnis(&self) -> &Projektverzeichnis {
        &self.verz
    }

    fn protokoll(&self, aktion: &str, objekt: &str, status: &str, code: &str) {
        if let Some(a) = &self.audit {
            a.aufzeichnen(AuditEreignis {
                aktion: aktion.into(),
                objekt: objekt.into(),
                status: status.into(),
                code: code.into(),
                dauer_ms: 0,
            });
        }
    }

    /// Frische Momentaufnahme aller Urspruenge: Besprechungen, Buecher und Recherchen.
    pub async fn herkunft(&self) -> Res<LvaHerkunft> {
        let mut h = LvaHerkunft::neu(&self.principal.tenant().0);
        for b in self.quellen.besprechungen().map_err(quelle)? {
            if id_gueltig(&b.id) {
                h.hinzufuegen(besprechung_link(&b.id), true, KLASSE_BESPRECHUNG);
            }
        }
        for b in self.quellen.buecher().map_err(quelle)? {
            if id_gueltig(&b.id) {
                h.hinzufuegen(buch_link(&b.id), true, KLASSE_BUCH);
            }
        }
        if let Some(seiten) = self.quellen.seiten().map_err(quelle)? {
            h.seiten_beurteilen();
            for s in seiten {
                if id_gueltig(&s.id) {
                    h.hinzufuegen(seite_link(&s.id), s.lesbar, KLASSE_SEITE);
                }
            }
        }
        if let Some(ablage) = &self.ablage {
            let b = dateien::bestand(ablage);
            for p in &b.gelesen {
                h.projektdateien_beurteilen(p);
            }
            for (p, d) in &b.lebend {
                if id_gueltig(d) {
                    h.hinzufuegen(projektdatei_link(p, d), true, KLASSE_DATEI);
                }
            }
        }
        // Ablage-Verweise: Leserecht und Klasse. Sie werden nie „beurteilt“ (bleiben im Abgleich
        // `Unbekannt`): eine D4-Umbenennung oder ein Lesefehler entfernt keine Mitgliedschaft.
        if let Some(dir) = &self.dokumentenablage {
            let a = crate::ablage::speicher::Ablage::neu(dir.clone());
            if let Ok(alle) = tokio::task::spawn_blocking(move || a.verweise()).await {
                for (id, lebend, klasse) in alle.unwrap_or_default() {
                    let k = ablage_klasse(klasse);
                    let lesbar = lebend && k != Klasse::Restricted;
                    h.hinzufuegen(ablage_link(&id), lesbar, k);
                }
            }
        }
        h.erweitern(self.recherchen.herkunft(&self.principal).await?);
        Ok(h)
    }

    // ---- Projekte

    async fn anzahl(&self, id: &str, h: &LvaHerkunft) -> u32 {
        self.verz
            .eintraege_auflisten(&self.principal, id, h)
            .await
            .map_or(0, |e| e.len() as u32)
    }

    pub async fn projekte(&self) -> Res<Vec<SuiteProjekt>> {
        let h = self.herkunft().await?;
        let alle = self.verz.projekte_auflisten(&self.principal, true).await?;
        let mut out = Vec::with_capacity(alle.len());
        for p in &alle {
            out.push(proj_aus(p, self.anzahl(&p.id, &h).await));
        }
        Ok(out)
    }

    pub async fn projekt(&self, id: &str) -> Res<Projekt> {
        Ok(self.verz.projekt_lesen(&self.principal, id).await?)
    }

    pub async fn anlegen(
        &self,
        titel: &str,
        klasse: Klasse,
        besondere_kategorie: bool,
    ) -> Res<SuiteProjekt> {
        let titel = titel.trim();
        if titel.is_empty() {
            return Err(Fehler::ungueltig("Der Titel darf nicht leer sein."));
        }
        let mut neu = NeuesProjekt::neu(&clip(titel, 200), klasse);
        neu.special_category = besondere_kategorie;
        let p = self.verz.projekt_anlegen(&self.principal, neu).await?;
        Ok(proj_aus(&p, 0))
    }

    /// Web-Freigabe je Projekt (Opt-in, ADR-0026 Abschnitt 4); jede Aenderung wird protokolliert.
    pub async fn web_freigabe(&self, id: &str, an: bool) -> Res<SuiteProjekt> {
        let alt = self.verz.projekt_lesen(&self.principal, id).await?;
        let p = self
            .verz
            .projekt_aendern(
                &self.principal,
                id,
                alt.revision,
                Aenderung {
                    web_research_enabled: Some(Some(an)),
                    ..Default::default()
                },
            )
            .await?;
        self.protokoll(
            "web_freigabe",
            id,
            if an { "an" } else { "aus" },
            "konfiguration_geaendert",
        );
        let h = self.herkunft().await?;
        Ok(proj_aus(&p, self.anzahl(id, &h).await))
    }

    // ---- Besprechungsordner <-> Suite-Projekt

    /// Verknuepft den Besprechungsordner mit dem Suite-Projekt und ordnet seine Besprechungen zu.
    pub async fn ordner_verknuepfen(&self, ordner_id: &str, suite_id: &str) -> Res<SyncBericht> {
        let _g = self.sperre.lock().await;
        if !suite_id_gueltig(suite_id) {
            return Err(Fehler::ungueltig("Kennung des Suite-Projekts ungültig."));
        }
        let suite = self.verz.projekt_lesen(&self.principal, suite_id).await?;
        if suite.archived {
            return Err(ProjektFehler::Archiviert.into());
        }
        let ordner = self
            .quellen
            .ordner()
            .map_err(quelle)?
            .into_iter()
            .find(|o| o.id == ordner_id)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?;
        match ordner.projekt_id.as_deref() {
            Some(s) if s == suite_id => {}
            Some(alt) => {
                // Ein haengender Verweis (das alte Suite-Projekt existiert nicht mehr) wird ersetzt.
                match self.verz.projekt_lesen(&self.principal, alt).await {
                    Err(ProjektFehler::NichtGefunden) => {
                        self.quellen
                            .ordner_projekt_setzen(ordner_id, Some(suite_id))
                            .map_err(quelle)?;
                    }
                    Ok(_) => {
                        return Err(Fehler::ungueltig(
                            "Der Ordner ist bereits mit einem anderen Suite-Projekt verknüpft. Bitte zuerst lösen.",
                        ));
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            None => {
                self.quellen
                    .ordner_projekt_setzen(ordner_id, Some(suite_id))
                    .map_err(quelle)?;
            }
        }
        self.protokoll("ordner_verknuepft", suite_id, "ok", "verknuepft");
        self.sync_ohne_sperre(suite_id).await
    }

    /// Gleicht die Besprechungen aller Ordner eines Suite-Projekts mit dessen Projektordner ab.
    pub async fn projekt_abgleichen(&self, suite_id: &str) -> Res<SyncBericht> {
        let _g = self.sperre.lock().await;
        self.verz.projekt_lesen(&self.principal, suite_id).await?;
        self.sync_ohne_sperre(suite_id).await
    }

    async fn sync_ohne_sperre(&self, suite_id: &str) -> Res<SyncBericht> {
        let h = self.herkunft().await?;
        let titel: HashMap<String, String> = self
            .quellen
            .besprechungen()
            .map_err(quelle)?
            .into_iter()
            .map(|b| (b.id, b.titel))
            .collect();
        // Soll: Besprechungen aller Ordner, die auf dieses Projekt zeigen.
        let mut soll: BTreeMap<String, String> = BTreeMap::new();
        for o in self
            .quellen
            .ordner()
            .map_err(quelle)?
            .into_iter()
            .filter(|o| o.projekt_id.as_deref() == Some(suite_id))
        {
            for id in self
                .quellen
                .besprechungen_im_ordner(&o.id)
                .map_err(quelle)?
            {
                if let Some(t) = titel.get(&id) {
                    soll.insert(besprechung_link(&id), t.clone());
                }
            }
        }
        // Ist: bereits zugeordnete Besprechungen (Ref -> (Eintrag, durch)).
        let mut ist: HashMap<String, (String, ZugeordnetDurch)> = HashMap::new();
        for e in self
            .verz
            .eintraege_auflisten(&self.principal, suite_id, &h)
            .await?
        {
            match e {
                Eintragsansicht::Sichtbar(e) if e.kind == Art::Meeting => {
                    ist.insert(e.ref_, (e.id, e.assigned_by));
                }
                _ => {}
            }
        }
        let mut b = SyncBericht::default();
        for (ref_, t) in &soll {
            if ist.contains_key(ref_) {
                b.bereits += 1;
                continue;
            }
            let neu = NeuerEintrag {
                kind: Art::Meeting,
                title: clip(t, MAX_TITEL),
                origin_app: LINK_APP.into(),
                ref_: ref_.clone(),
                data_class: KLASSE_BESPRECHUNG,
                assigned_by: ZugeordnetDurch::Automation,
                preview: None,
                external_copies: vec![],
            };
            match self
                .verz
                .eintrag_zuordnen(&self.principal, suite_id, neu, &h)
                .await
            {
                Ok(_) => b.zugeordnet += 1,
                Err(ProjektFehler::KlasseUeberProjekt) => b.uebersprungen += 1,
                Err(ProjektFehler::Doppelt) => b.bereits += 1,
                Err(ProjektFehler::KeinHerkunftsrecht) => {}
                Err(e) => return Err(e.into()),
            }
        }
        // Besprechungen, die den Ordner verlassen haben: nur automatisch zugeordnete Eintraege gehen.
        for (ref_, (eintrag, durch)) in &ist {
            if !soll.contains_key(ref_) && *durch == ZugeordnetDurch::Automation {
                self.verz
                    .eintrag_entfernen(&self.principal, eintrag)
                    .await?;
                b.entfernt += 1;
            }
        }
        b.entfernt += self.abgleich_ohne_sperre(&h).await? as u32;
        Ok(b)
    }

    /// Loest die Verknuepfung des Ordners. Die automatisch zugeordneten Eintraege seiner Besprechungen
    /// verlassen den Projektordner (die Besprechungen selbst bleiben), soweit kein anderer Ordner
    /// desselben Projekts sie noch traegt; liefert deren Zahl.
    pub async fn ordner_loesen(&self, ordner_id: &str) -> Res<u32> {
        let _g = self.sperre.lock().await;
        let alle = self.quellen.ordner().map_err(quelle)?;
        let ordner = alle
            .iter()
            .find(|o| o.id == ordner_id)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?;
        let Some(suite) = ordner.projekt_id.clone() else {
            return Ok(0);
        };
        self.quellen
            .ordner_projekt_setzen(ordner_id, None)
            .map_err(quelle)?;
        self.protokoll("ordner_geloest", &suite, "ok", "geloest");
        // Existiert das Suite-Projekt nicht mehr, ist die Verknuepfung trotzdem geloest.
        if self
            .verz
            .projekt_lesen(&self.principal, &suite)
            .await
            .is_err()
        {
            return Ok(0);
        }
        let entfernt = self.sync_ohne_sperre(&suite).await?.entfernt;
        self.ausgang_bestaetigen().await?;
        Ok(entfernt)
    }

    // ---- Eintraege

    /// Eintraege des Suite-Projekts; vorher laeuft der Abgleich, damit Geloeschtes nicht erscheint.
    pub async fn eintraege(&self, suite_id: &str) -> Res<Vec<EintragSicht>> {
        let _g = self.sperre.lock().await;
        let h = self.herkunft().await?;
        self.abgleich_ohne_sperre(&h).await?;
        let liste = self
            .verz
            .eintraege_auflisten(&self.principal, suite_id, &h)
            .await?;
        Ok(liste
            .into_iter()
            .map(|e| match e {
                Eintragsansicht::Sichtbar(e) => EintragSicht {
                    id: e.id,
                    kind: art_text(e.kind),
                    subtitle: untertitel(e.kind, &e.ref_),
                    title: Some(e.title),
                    origin_app: Some(e.origin_app),
                    data_class: Some(klasse_text(e.data_class).into()),
                    available: true,
                    assigned_at: Some(e.assigned_at),
                    assigned_by: Some(
                        match e.assigned_by {
                            ZugeordnetDurch::Person => "person",
                            ZugeordnetDurch::Automation => "automation",
                        }
                        .into(),
                    ),
                    link: Some(e.ref_),
                },
                Eintragsansicht::NichtVerfuegbar { id, kind, .. } => EintragSicht {
                    id,
                    kind: art_text(kind),
                    title: None,
                    subtitle: None,
                    origin_app: None,
                    data_class: None,
                    available: false,
                    assigned_at: None,
                    assigned_by: None,
                    link: None,
                },
            })
            .collect())
    }

    /// Ordnet zu; liefert die ID eines neu angelegten Eintrags (`None`, wenn er schon bestand).
    /// Von der Person zugeordnet: der Eintrag startet als Kontext eingeschaltet (bewusstes Zuordnen,
    /// sichtbarer Schalter); ein vorhandener Stand bleibt unverändert.
    async fn verweis_zuordnen(
        &self,
        suite_id: &str,
        art: Art,
        ref_: String,
        titel: &str,
        klasse: Klasse,
        durch: ZugeordnetDurch,
    ) -> Res<Option<String>> {
        let h = self.herkunft().await?;
        let neu = NeuerEintrag {
            kind: art,
            title: clip(titel, MAX_TITEL),
            origin_app: LINK_APP.into(),
            ref_: ref_.clone(),
            data_class: klasse,
            assigned_by: durch,
            preview: None,
            external_copies: vec![],
        };
        match self
            .verz
            .eintrag_zuordnen(&self.principal, suite_id, neu, &h)
            .await
        {
            Ok(e) => {
                if durch == ZugeordnetDurch::Person {
                    if let Some(ablage) = &self.ablage {
                        // Scheitert das Speichern, bleibt der Eintrag ohne Stand und damit aus.
                        if kontext::anlegen(ablage, suite_id, &e.id, &ref_, true).is_err() {
                            self.protokoll("kontext_anlegen", suite_id, "fehler", "speicher");
                        }
                    }
                }
                Ok(Some(e.id))
            }
            Err(ProjektFehler::Doppelt) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn verweis_entfernen(&self, suite_id: &str, ref_: &str) -> Res<bool> {
        let h = self.herkunft().await?;
        for e in self
            .verz
            .eintraege_auflisten(&self.principal, suite_id, &h)
            .await?
        {
            match e {
                Eintragsansicht::Sichtbar(e) if e.ref_ == ref_ => {
                    self.verz.eintrag_entfernen(&self.principal, &e.id).await?;
                    self.ausgang_bestaetigen().await?;
                    if let Some(ablage) = &self.ablage {
                        let _ = kontext::vergessen(ablage, std::slice::from_ref(&e.id));
                    }
                    return Ok(true);
                }
                _ => {}
            }
        }
        Ok(false)
    }

    /// Ordnet einem Projekt ein Buch der Bibliothek zu (`wai://lva/buch/<id>`). Idempotent.
    pub async fn buch_zuordnen(&self, suite_id: &str, buch_id: &str) -> Res<()> {
        let _g = self.sperre.lock().await;
        if !id_gueltig(buch_id) {
            return Err(Fehler::ungueltig("Kennung des Buchs ungültig."));
        }
        let titel = self
            .quellen
            .buecher()
            .map_err(quelle)?
            .into_iter()
            .find(|b| b.id == buch_id)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?
            .titel;
        self.verweis_zuordnen(
            suite_id,
            Art::Book,
            buch_link(buch_id),
            &titel,
            KLASSE_BUCH,
            ZugeordnetDurch::Person,
        )
        .await
        .map(|_| ())
    }

    /// Nimmt ein Buch aus dem Projekt (nie aus der Bibliothek). `true`, wenn es zugeordnet war.
    pub async fn buch_entfernen(&self, suite_id: &str, buch_id: &str) -> Res<bool> {
        let _g = self.sperre.lock().await;
        self.verweis_entfernen(suite_id, &buch_link(buch_id)).await
    }

    /// Ordnet einem Projekt eine einzelne Besprechung zu (ohne Ordner). Idempotent.
    pub async fn besprechung_zuordnen(&self, suite_id: &str, meeting_id: &str) -> Res<()> {
        let _g = self.sperre.lock().await;
        if !id_gueltig(meeting_id) {
            return Err(Fehler::ungueltig("Kennung der Besprechung ungültig."));
        }
        let titel = self
            .quellen
            .besprechungen()
            .map_err(quelle)?
            .into_iter()
            .find(|b| b.id == meeting_id)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?
            .titel;
        self.verweis_zuordnen(
            suite_id,
            Art::Meeting,
            besprechung_link(meeting_id),
            &titel,
            KLASSE_BESPRECHUNG,
            ZugeordnetDurch::Person,
        )
        .await
        .map(|_| ())
    }

    /// Nimmt eine Besprechung aus dem Projekt (nie aus der Aufnahmenliste).
    pub async fn besprechung_entfernen(&self, suite_id: &str, meeting_id: &str) -> Res<bool> {
        let _g = self.sperre.lock().await;
        self.verweis_entfernen(suite_id, &besprechung_link(meeting_id))
            .await
    }

    /// Legt eine Recherche als Eintrag `wai://lva/recherche/<id>` ab. Idempotent.
    pub async fn recherche_zuordnen(
        &self,
        suite_id: &str,
        recherche_id: &str,
        titel: &str,
        klasse: Klasse,
        durch: ZugeordnetDurch,
    ) -> Res<()> {
        let _g = self.sperre.lock().await;
        let titel = if titel.trim().is_empty() {
            "Recherche"
        } else {
            titel
        };
        self.verweis_zuordnen(
            suite_id,
            Art::Document,
            recherche_link(recherche_id),
            titel,
            klasse,
            durch,
        )
        .await
        .map(|_| ())
    }

    // ---- Texte, Projektdateien und Kontext (lva#365 P4)

    /// Ordnet einem Projekt eine Vorlesen-Seite zu (`wai://lva/seite/<id>`). Idempotent.
    pub async fn seite_zuordnen(&self, suite_id: &str, seite_id: &str) -> Res<()> {
        let _g = self.sperre.lock().await;
        if !id_gueltig(seite_id) {
            return Err(Fehler::ungueltig("Kennung der Seite ungültig."));
        }
        let titel = self
            .quellen
            .seiten()
            .map_err(quelle)?
            .ok_or_else(|| Fehler::neu("projekte.quelle", "Seiten nicht verfügbar."))?
            .into_iter()
            .find(|s| s.id == seite_id && s.lesbar)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?
            .titel;
        let titel = if titel.trim().is_empty() {
            "Text".to_string()
        } else {
            titel
        };
        self.verweis_zuordnen(
            suite_id,
            Art::Document,
            seite_link(seite_id),
            &titel,
            KLASSE_SEITE,
            ZugeordnetDurch::Person,
        )
        .await
        .map(|_| ())
    }

    /// Nimmt eine Seite aus dem Projekt (nie aus Vorlesen).
    pub async fn seite_entfernen(&self, suite_id: &str, seite_id: &str) -> Res<bool> {
        let _g = self.sperre.lock().await;
        self.verweis_entfernen(suite_id, &seite_link(seite_id))
            .await
    }

    fn projekt_pruefen(suite_id: &str) -> Res<()> {
        if suite_id_gueltig(suite_id) {
            Ok(())
        } else {
            Err(Fehler::ungueltig("Kennung des Projekts ungültig."))
        }
    }

    /// Kopiert fremde Dateien in den Projektordner und trägt sie danach ein (Spec §5.1). Liefert
    /// die Datei-IDs. Bricht beim ersten Fehler ab; schon kopierte Dateien bleiben eingetragen.
    pub async fn dateien_hinzufuegen(
        &self,
        suite_id: &str,
        quellen: &[PathBuf],
    ) -> Res<Vec<String>> {
        let _g = self.sperre.lock().await;
        Self::projekt_pruefen(suite_id)?;
        // Das Projekt muss existieren und beschreibbar sein, bevor kopiert wird.
        let p = self.verz.projekt_lesen(&self.principal, suite_id).await?;
        if p.archived {
            return Err(ProjektFehler::Archiviert.into());
        }
        let ablage = self.ablage()?.to_path_buf();
        let mut ids = Vec::new();
        for q in quellen {
            let (id, name) = dateien::hinzufuegen(&ablage, suite_id, q).map_err(quelle)?;
            self.datei_eintragen(&ablage, suite_id, &id, &name).await?;
            ids.push(id);
        }
        self.protokoll(
            "dateien_hinzugefuegt",
            suite_id,
            "ok",
            &format!("n{}", ids.len()),
        );
        Ok(ids)
    }

    async fn datei_eintragen(
        &self,
        ablage: &Path,
        suite_id: &str,
        id: &str,
        name: &str,
    ) -> Res<()> {
        let link = projektdatei_link(suite_id, id);
        let neu = self
            .verweis_zuordnen(
                suite_id,
                Art::Document,
                link.clone(),
                name,
                KLASSE_DATEI,
                ZugeordnetDurch::Person,
            )
            .await?;
        let eintrag = match neu {
            Some(e) => e,
            None => self
                .eintrag_mit_link(suite_id, &link)
                .await?
                .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?,
        };
        dateien::eingetragen(ablage, suite_id, id, &eintrag).map_err(quelle)
    }

    async fn eintrag_mit_link(&self, suite_id: &str, link: &str) -> Res<Option<String>> {
        let h = self.herkunft().await?;
        Ok(self
            .verz
            .eintraege_auflisten(&self.principal, suite_id, &h)
            .await?
            .into_iter()
            .find_map(|e| match e {
                Eintragsansicht::Sichtbar(e) if e.ref_ == link => Some(e.id),
                _ => None,
            }))
    }

    /// Entfernt eine Projektdatei: Kopie in den Papierkorb des Projekts, dann Mitgliedschaft weg.
    pub async fn datei_entfernen(&self, suite_id: &str, datei_id: &str) -> Res<bool> {
        let _g = self.sperre.lock().await;
        Self::projekt_pruefen(suite_id)?;
        let ablage = self.ablage()?.to_path_buf();
        dateien::entfernen_beginnen(&ablage, suite_id, datei_id).map_err(quelle)?;
        let weg = self
            .verweis_entfernen(suite_id, &projektdatei_link(suite_id, datei_id))
            .await?;
        // Ohne Eintrag (Abbruch davor, Abgleich): der Abgleich räumt nach, die Herkunft ist weg.
        if !weg {
            let h = self.herkunft().await?;
            self.abgleich_ohne_sperre(&h).await?;
        }
        // `false`: nicht sicher entfernt (abgelehnt, Datei liegt weiter im Projektordner und wird
        // als gefunden angeboten).
        let sicher = !dateien::liste(&ablage, suite_id)
            .map_err(quelle)?
            .get(datei_id)
            .is_some_and(|d| d.nicht_sicher);
        self.protokoll(
            "datei_entfernt",
            suite_id,
            "ok",
            if sicher { "papierkorb" } else { "abgelehnt" },
        );
        Ok(sicher)
    }

    /// Dateien im Projektordner, die der Wiederanlauf ohne Journaleintrag fand.
    pub async fn gefundene_dateien(&self, suite_id: &str) -> Res<Vec<GefundeneDatei>> {
        Self::projekt_pruefen(suite_id)?;
        self.verz.projekt_lesen(&self.principal, suite_id).await?;
        let ablage = self.ablage()?;
        Ok(dateien::liste(ablage, suite_id)
            .map_err(quelle)?
            .into_iter()
            .filter(|(_, d)| d.status == dateien::Status::Gefunden)
            .map(|(id, d)| GefundeneDatei {
                id,
                name: d.name,
                size: d.groesse as f64,
            })
            .collect())
    }

    /// Zustand des Dateijournals eines Projekts für die Oberfläche: `None` = lesbar. Bei `fehlt`
    /// oder `defekt` bietet die Projektseite „Bestand neu aufbauen“ an.
    pub async fn dateien_lage(&self, suite_id: &str) -> Res<Option<dateien::JournalLage>> {
        Self::projekt_pruefen(suite_id)?;
        self.verz.projekt_lesen(&self.principal, suite_id).await?;
        let ablage = self.ablage()?;
        dateien::lage(ablage, suite_id).map_err(quelle)
    }

    /// Ausdrücklicher Neuaufbau des Dateijournals (nur bei `fehlt`/`defekt`). Dateien werden als
    /// gefunden angeboten, nicht still aufgenommen. Liefert ihre Zahl.
    pub async fn dateien_neu_aufbauen(&self, suite_id: &str) -> Res<u32> {
        let _g = self.sperre.lock().await;
        Self::projekt_pruefen(suite_id)?;
        self.verz.projekt_lesen(&self.principal, suite_id).await?;
        let ablage = self.ablage()?.to_path_buf();
        let n = dateien::neu_aufbauen(&ablage, suite_id).map_err(quelle)?;
        self.protokoll("dateien_neu_aufgebaut", suite_id, "ok", &format!("n{n}"));
        Ok(n)
    }

    /// Bewusst aufnehmen, was der Wiederanlauf im Projektordner gefunden hat.
    pub async fn gefundene_datei_aufnehmen(&self, suite_id: &str, datei_id: &str) -> Res<()> {
        let _g = self.sperre.lock().await;
        Self::projekt_pruefen(suite_id)?;
        let ablage = self.ablage()?.to_path_buf();
        let name = dateien::gefundene_aufnehmen(&ablage, suite_id, datei_id).map_err(quelle)?;
        self.datei_eintragen(&ablage, suite_id, datei_id, &name)
            .await
    }

    /// Wiederanlauf der Projektdateien aller Projekte (Start und Pflege): unterbrochene Entfernungen
    /// fortsetzen, abgelegte Dateien eintragen, verwaiste Kopien als gefunden vermerken. Liefert die
    /// Zahl der Projekte mit Befund.
    pub async fn dateien_wiederanlauf(&self) -> Res<u32> {
        let _g = self.sperre.lock().await;
        let Some(ablage) = self.ablage.clone() else {
            return Ok(0);
        };
        let mut n = 0;
        for p in dateien::bestand(&ablage).gelesen {
            let w = match dateien::wiederanlauf(&ablage, &p) {
                Ok(w) => w,
                Err(_) => {
                    self.protokoll("dateien_wiederanlauf", &p, "fehler", "journal");
                    continue;
                }
            };
            let befund = w.gefunden + w.entfernt + w.teile + w.einzutragen.len() as u32;
            let lebt = matches!(
                self.verz.projekt_lesen(&self.principal, &p).await,
                Ok(x) if !x.archived
            );
            if lebt {
                let alle = dateien::liste(&ablage, &p).map_err(quelle)?;
                for id in &w.einzutragen {
                    if let Some(d) = alle.get(id) {
                        if self
                            .datei_eintragen(&ablage, &p, id, &d.name)
                            .await
                            .is_err()
                        {
                            self.protokoll("dateien_wiederanlauf", &p, "fehler", "eintragen");
                        }
                    }
                }
            }
            if befund > 0 {
                n += 1;
                self.protokoll("dateien_wiederanlauf", &p, "ok", &format!("n{befund}"));
            }
        }
        if n > 0 {
            let h = self.herkunft().await?;
            self.abgleich_ohne_sperre(&h).await?;
        }
        Ok(n)
    }

    /// Mitgliedschaften eines Projekts in der Sicht des Kontexts.
    fn mitglieder(liste: &[EintragSicht]) -> Vec<Mitglied> {
        liste
            .iter()
            .map(|e| {
                let link = e.link.clone().unwrap_or_default();
                let sorte = if buch_id(&link).is_some() {
                    Sorte::Buch
                } else if besprechung_id(&link).is_some() {
                    Sorte::Aufnahme
                } else if ablage_id(&link).is_some() {
                    Sorte::Ablage
                } else {
                    Sorte::Sonst
                };
                Mitglied {
                    id: e.id.clone(),
                    art: e.kind.clone(),
                    sorte,
                    titel: e.title.clone().unwrap_or_default(),
                    lesbar: e.available && !link.is_empty(),
                    link,
                }
            })
            .collect()
    }

    /// Übersicht „Als Kontext“ für die Oberfläche: Schalter, Form, Token, Stand, ohne Inhalte.
    pub async fn kontext_uebersicht(&self, suite_id: &str) -> Res<kontext::KontextUebersicht> {
        Ok(self.kontext_bauen(suite_id).await?.uebersicht)
    }

    /// Einziger Zugang der KI-Wege zum Projektkontext (`projekt_kontext`, Spec §5.2). Bis P4c
    /// (Widerruf, Versandrennen, Budget) gesperrt: liefert dann nichts, nur einen Fehler.
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn kontext_fuer_ki(&self, suite_id: &str) -> Res<Kontext> {
        Self::projekt_pruefen(suite_id)?;
        if !PROJEKT_KI_FREIGEGEBEN {
            return Err(Fehler::neu(
                "projekte.ki_gesperrt",
                "Die KI im Projekt ist noch nicht freigegeben.",
            ));
        }
        self.kontext_bauen(suite_id).await
    }

    /// Nur für Tests: der Kontext mit Inhalten, ohne die P4c-Sperre.
    #[cfg(test)]
    pub(super) async fn kontext_fuer_test(&self, suite_id: &str) -> Res<Kontext> {
        self.kontext_bauen(suite_id).await
    }

    /// Der KI-Kontext eines Projekts (Spec §5.2) mit Inhalten. Privat: aufgerufen nur aus
    /// `kontext_fuer_ki` (KI-Wege, gesperrt bis P4c), `kontext_uebersicht` (nur die Übersicht ohne
    /// Inhalte) und dem Testzugang; ein Allowlist-Test in `tests_p4` prüft das.
    async fn kontext_bauen(&self, suite_id: &str) -> Res<Kontext> {
        Self::projekt_pruefen(suite_id)?;
        let ablage = self.ablage()?.to_path_buf();
        let liste = self.eintraege(suite_id).await?;
        let mitglieder = Self::mitglieder(&liste);
        let schalter = Schalter::laden(&ablage);
        let mut recherchen = HashMap::new();
        for m in mitglieder.iter().filter(|m| m.lesbar) {
            if let Some(rid) = recherche_id(&m.link) {
                let form = match self.recherchen.lesen(&self.principal, rid).await {
                    Ok(Some(d)) => {
                        let v = super::recherche::sicht(&d);
                        Form::Text(format!("{}\n\n{}", v.question, v.read_aloud))
                    }
                    Ok(None) => Form::Fehlt(kontext::hinweis::FEHLT),
                    Err(_) => Form::Fehlt(kontext::hinweis::NICHT_LESBAR),
                };
                recherchen.insert(m.id.clone(), form);
            }
        }
        let quellen = self.quellen.clone();
        Ok(kontext::bauen(
            suite_id,
            &mitglieder,
            &schalter,
            &mut |m, w| {
                if m.sorte == Sorte::Ablage {
                    // Keine festen Inhalte: Auszüge je Frage (P4c, `ablage::ki::auszuege`).
                    return Form::Auszuege;
                }
                if let Some(f) = recherchen.get(&m.id) {
                    return f.clone();
                }
                if let Some((p, d)) = projektdatei_id(&m.link) {
                    if p != suite_id {
                        return Form::Fehlt(kontext::hinweis::NICHT_LESBAR);
                    }
                    return dateien::kontext_text(&ablage, p, d);
                }
                quellen.kontext_text(&m.link, w)
            },
        ))
    }

    /// Schaltet „Als Kontext“ eines Eintrags und setzt optional Form und Fassung. Liefert die neue
    /// Übersicht (mit neuem `stand`).
    pub async fn kontext_schalten(
        &self,
        suite_id: &str,
        eintrag_id: &str,
        an: bool,
        form: Option<KontextForm>,
        fassung: Option<Option<String>>,
    ) -> Res<kontext::KontextUebersicht> {
        Self::projekt_pruefen(suite_id)?;
        let ablage = self.ablage()?.to_path_buf();
        let liste = self.eintraege(suite_id).await?;
        let m = Self::mitglieder(&liste)
            .into_iter()
            .find(|m| m.id == eintrag_id)
            .ok_or_else(|| Fehler::neu("projekte.not_found", "Nicht gefunden"))?;
        if !m.lesbar && an {
            return Err(ProjektFehler::KeinHerkunftsrecht.into());
        }
        if let Some(Some(f)) = &fassung {
            if !id_gueltig(f) {
                return Err(Fehler::ungueltig("Kennung der Fassung ungültig."));
            }
        }
        kontext::setzen(&ablage, suite_id, &m, an, form, fassung).map_err(quelle)?;
        self.protokoll(
            "kontext_geschaltet",
            suite_id,
            "ok",
            if an { "an" } else { "aus" },
        );
        self.kontext_uebersicht(suite_id).await
    }

    // ---- Loeschkaskade und Aufbewahrung

    /// Entfernt Eintraege, deren Herkunft nicht mehr existiert; liefert deren Zahl.
    pub async fn abgleichen(&self) -> Res<usize> {
        let _g = self.sperre.lock().await;
        let h = self.herkunft().await?;
        self.abgleich_ohne_sperre(&h).await
    }

    async fn abgleich_ohne_sperre(&self, h: &LvaHerkunft) -> Res<usize> {
        let b = self.verz.abgleich(&self.principal, h).await?;
        self.ausgang_bestaetigen().await?;
        Ok(b.waisen_entfernt.len())
    }

    /// Loescht ein Suite-Projekt samt Eintraegen, loest die Verknuepfung aller Ordner und loescht die
    /// Recherche-Dokumente des Projekts (Datenschutzloeschung).
    pub async fn loeschen(&self, suite_id: &str) -> Res<LoeschErgebnis> {
        let _g = self.sperre.lock().await;
        // Erst die Dokumente, dann das Projekt: scheitert das Loeschen des Projekts, bleibt kein
        // verwaistes Dokument zurueck, das niemand mehr erreicht.
        let recherchen_geloescht = self
            .recherchen
            .loeschen_projekt(&self.principal, suite_id)
            .await? as u32;
        let bericht: LoeschBericht = self
            .verz
            .projekt_loeschen(&self.principal, suite_id)
            .await?;
        let ordner_geloest = self
            .quellen
            .ordner_projekt_loesen_alle(suite_id)
            .map_err(quelle)?;
        // Projektdateien und Kontextauswahl: der Projektordner geht in den Papierkorb. Scheitert
        // das, räumt die Pflege den verwaisten Ordner später weg (nie endgültig gelöscht).
        if let Some(ablage) = &self.ablage {
            if suite_id_gueltig(suite_id)
                && dateien::projekt_in_papierkorb(ablage, suite_id).is_err()
            {
                self.protokoll("projekt_geloescht", suite_id, "fehler", "ordner");
            }
        }
        self.ausgang_bestaetigen().await?;
        self.protokoll("projekt_geloescht", suite_id, "ok", "geloescht");
        Ok(LoeschErgebnis {
            eintraege_entfernt: bericht.eintraege_entfernt as u32,
            ordner_geloest,
            recherchen_geloescht,
            externe_kopien: bericht.externe_kopien.len() as u32,
        })
    }

    /// Aufbewahrung: gilt `retention_days` eines Suite-Projekts, gilt sie auch fuer dessen Recherche-
    /// Dokumente (nach Anlage). Liefert die Zahl der geloeschten Dokumente.
    pub async fn aufbewahrung_recherchen(&self) -> Res<u64> {
        let _g = self.sperre.lock().await;
        let jetzt = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut n = 0;
        for p in self.verz.projekte_auflisten(&self.principal, true).await? {
            let Some(tage) = p.retention_days else {
                continue;
            };
            let grenze = jetzt.saturating_sub(tage.saturating_mul(86_400_000));
            n += self
                .recherchen
                .loeschen_aelter(&self.principal, &p.id, grenze)
                .await?;
        }
        if n > 0 {
            let h = self.herkunft().await?;
            self.abgleich_ohne_sperre(&h).await?;
        }
        Ok(n)
    }

    /// Legt Projektordner, deren Suite-Projekt nicht mehr existiert (Absturz mitten im Löschen), in
    /// den Papierkorb. Liefert deren Zahl.
    pub async fn verwaiste_projektordner_raeumen(&self) -> Res<u32> {
        let _g = self.sperre.lock().await;
        let Some(ablage) = self.ablage.clone() else {
            return Ok(0);
        };
        let bekannt: HashSet<String> = self
            .verz
            .projekte_auflisten(&self.principal, true)
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let b = dateien::bestand(&ablage);
        let mut n = 0;
        for p in b.gelesen.iter().chain(b.unklar.iter()) {
            if !bekannt.contains(p) && dateien::projekt_in_papierkorb(&ablage, p).unwrap_or(false) {
                n += 1;
            }
        }
        if n > 0 {
            self.protokoll(
                "projektordner_aufgeraeumt",
                "verwaist",
                "ok",
                &format!("n{n}"),
            );
        }
        Ok(n)
    }

    /// Raeumt Recherche-Dokumente weg, deren Suite-Projekt nicht mehr existiert (z. B. nach einem
    /// Absturz mitten in der Loeschung oder einem Lauf, der ein geloeschtes Projekt traf). Liefert die
    /// Zahl der entfernten Dokumente.
    pub async fn verwaiste_recherchen_raeumen(&self) -> Res<u64> {
        let _g = self.sperre.lock().await;
        let bekannt: HashSet<String> = self
            .verz
            .projekte_auflisten(&self.principal, true)
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect();
        let mut n = 0;
        for k in self.recherchen.liste(&self.principal, None).await? {
            let Some(pid) = k.project_id else { continue };
            if !bekannt.contains(&pid) && self.recherchen.loeschen(&self.principal, &k.id).await? {
                n += 1;
            }
        }
        if n > 0 {
            self.protokoll("recherchen_aufgeraeumt", "verwaist", "ok", &format!("n{n}"));
        }
        Ok(n)
    }

    #[cfg(test)]
    pub fn verz_fuer_test(&self) -> &Projektverzeichnis {
        &self.verz
    }

    /// Bestaetigt Loeschmeldungen fuer Verweise der eigenen App; fremde Meldungen bleiben stehen,
    /// bis deren App sie anwendet.
    async fn ausgang_bestaetigen(&self) -> Res<usize> {
        let prefix = format!("wai://{LINK_APP}/");
        let mut n = 0;
        for m in self.verz.ausstehende_loeschungen(&self.principal).await? {
            if m.ref_.as_deref().is_some_and(|r| r.starts_with(&prefix)) {
                self.verz.bestaetigen(&self.principal, m.id).await?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Fuer Tests und die Diagnose: offene Loeschmeldungen (nur Kennungen und Verweis).
    #[cfg_attr(not(test), allow(dead_code))]
    pub async fn offene_meldungen(&self) -> Res<Vec<(String, Option<String>)>> {
        Ok(self
            .verz
            .ausstehende_loeschungen(&self.principal)
            .await?
            .into_iter()
            .map(|m| (m.projekt_id, m.ref_))
            .collect())
    }

    /// Kennungen der Suite-Projekte, die in Frage kommen, wenn eine Besprechung etwas recherchiert:
    /// die Projekte ihrer Ordner, die noch existieren und nicht archiviert sind.
    pub async fn projekte_der_besprechung(&self, meeting_id: &str) -> Res<Vec<String>> {
        let ids = self
            .quellen
            .projekte_der_besprechung(meeting_id)
            .map_err(quelle)?;
        let mut out = Vec::new();
        let mut gesehen = HashSet::new();
        for id in ids {
            if !gesehen.insert(id.clone()) {
                continue;
            }
            if matches!(
                self.verz.projekt_lesen(&self.principal, &id).await,
                Ok(p) if !p.archived
            ) {
                out.push(id);
            }
        }
        Ok(out)
    }
}

fn untertitel(kind: Art, ref_: &str) -> Option<String> {
    if super::herkunft::recherche_id(ref_).is_some() {
        return Some("Recherche · Schnell".into());
    }
    match kind {
        Art::Meeting => Some("Besprechung".into()),
        Art::Book => Some("Buch".into()),
        _ => None,
    }
}
