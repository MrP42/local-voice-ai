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
use super::herkunft::{
    besprechung_link, buch_link, id_gueltig, recherche_link, suite_id_gueltig, LvaHerkunft,
    KLASSE_BESPRECHUNG, KLASSE_BUCH, LINK_APP,
};
use super::quellen::LvaQuellen;
use super::recherche::RechercheStore;
use super::{Fehler, Res};
use serde::Serialize;
use specta::Type;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use wai_projekte::{
    Aenderung, Art, Eintragsansicht, Klasse, LoeschBericht, NeuerEintrag, NeuesProjekt, Projekt,
    ProjektFehler, Projektverzeichnis, ZugeordnetDurch,
};
use wai_security::Principal;

/// Hoechstlaenge von Titeln in Eintraegen (Vertrag `project-entry.title`).
const MAX_TITEL: usize = 300;

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
            sperre: tokio::sync::Mutex::new(()),
        })
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

    async fn verweis_zuordnen(
        &self,
        suite_id: &str,
        art: Art,
        ref_: String,
        titel: &str,
        klasse: Klasse,
        durch: ZugeordnetDurch,
    ) -> Res<()> {
        let h = self.herkunft().await?;
        let neu = NeuerEintrag {
            kind: art,
            title: clip(titel, MAX_TITEL),
            origin_app: LINK_APP.into(),
            ref_,
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
            Ok(_) | Err(ProjektFehler::Doppelt) => Ok(()),
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
