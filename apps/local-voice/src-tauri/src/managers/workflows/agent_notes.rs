//! Wissens- und Fristbausteine des lokalen Agenten (Goal Lokaler Agent, Issue #68, Paket C4):
//! was aus dem Ergebnis von `agent.extract` ENTSTEHT. Der Extraktionsschritt liest und liefert
//! Daten; diese Bausteine sind deterministisch (kein Modell, kein Werkzeugzugriff des Modells) und
//! haben je ihr eigenes Recht.
//!
//! | Baustein            | Wirkung    | Schwer | Recht (Tor)                               |
//! |---------------------|------------|--------|-------------------------------------------|
//! | `agent.note`        | idempotent | nein   | `vault.write` am Vault (`via`)            |
//! | `deadline.remind`   | idempotent | nein   | keines (Windows-Mitteilung, App-eigen)    |
//! | `deadline.calendar` | extern     | nein   | `calendar.write`, **immer „fragen“**      |
//!
//! Alle drei lesen `steps.<from>` (Vorgabe `extract`), das Ergebnis von `agent.extract`
//! (`items`: Texte, Daten, Segmente, Zitate, Herkunft). Das Ergebnis ist selbst Eingabe aus einem
//! Transkript und wird vor jeder Verwendung bereinigt (`items::read`, `render::md`); es setzt nie
//! Ziel, Konto, Empfaenger oder Pfad (`via` und `from` sind feste Werte der Definition).
//!
//! - `agent.note` schreibt EINE Notiz je Besprechung in den Vault (`render`: AI-OS-Frontmatter mit
//!   `context_area`, `data_class: confidential` nach E7, Rueckverweis auf die Besprechung,
//!   To-dos/Fristen/Entscheidungen mit Zitat und Segmenten, geschaetzte Daten gekennzeichnet).
//!   Die Notiz traegt `lva_id: ergebnis-<besprechung>`; vor dem Anlegen sucht `obsidian::find_by_id`
//!   im ganzen Vault: gefunden -> nur der verwaltete Block wird ersetzt (Handarbeit ausserhalb bleibt),
//!   sonst neu. Der AI-OS-Index uebernimmt die Datei beim naechsten Rescan; das ist der Weg in die
//!   Wissenssuche (Entscheidung E4: Vault statt eigenem Schreib-Endpunkt).
//! - `deadline.remind` und `deadline.calendar`: siehe `deadlines`.
//!
//! Die Herkunft („Herkunft“ im Kontextmenue, AK8): jede Notiz bekommt einen Eintrag
//! (`SubjectKind::KnowledgeNote`, Gegenstand = `lva_id`, Operation `agent_note`) mit Modell, Token,
//! Dauer, Quellen (Besprechung und belegte Segmente) und Konfidenz aus dem Extraktionsschritt.
//!
//! # Reihenfolge im Ablauf
//!
//! `agent.extract` -> `agent.note` -> (`deadline.calendar`) -> `deadline.remind`. Die Erinnerung
//! steht zuletzt, weil sie auf den Zeitpunkt wartet (Tage); die gatepflichtigen Schritte duerfen
//! nicht dahinter haengen. Gatepflichtige Bausteine warten nie selbst (Befund B3: jedes Aufwachen
//! verlangte eine neue Freigabe); die Vorlage haengt sie deshalb an `when:`-Bedingungen
//! (`steps.extract.counts.items > 0`), damit ein leeres Ergebnis keine leere Freigabe erzeugt.
//!
//! # Fehlerfaelle (C4) und ihre Absicherung
//!
//! | # | Fehlerfall | Verhalten | Beleg |
//! |---|------------|-----------|-------|
//! | 1 | **Nebenlaeufigkeit**: zwei Laeufe, dieselbe Besprechung, gleichzeitig | ein Prozess-Schloss um Suchen und Anlegen (`VAULT_LOCK`): genau EINE Datei, der zweite Lauf aktualisiert; Erinnerungen und Termine je Frist hoechstens einmal (Herkunftsregister, auch ueber Laeufe hinweg) | `two_runs_at_once_*`, `a_second_run_*` |
//! | 2 | **Abbruch mitten im Vorgang**: App stirbt nach dem Schreiben, vor dem Journal | die Notiz ist per `lva_id` wiederzufinden: Wiederholung = „unveraendert“; Erinnerung: Mitteilung, dann Vermerk, dann Journal, ein Neustart zeigt nichts doppelt; Termin: `confirm` belegt aus dem Vermerk, sonst `effect_uncertain`, nie blind ein zweiter Termin | `a_crash_*`, `a_repeated_step_*` |
//! | 3 | **Voller Datentraeger / gesperrte Datei / gesperrte Datenbank** | Vault nicht beschreibbar: `Transient`, es wurde nichts geschrieben (`Sandbox` entfernt halbe Dateien); Vermerk nicht schreibbar nach einer Wirkung: `Transient` mit Hinweis, der Wiederholversuch ist sicher (Notiz per `lva_id`, Termin per `transactionId`); Herkunft der Notiz darf scheitern, ohne dass der Schritt scheitert | `a_locked_vault_*`, `a_broken_ledger_*` |
//! | 4 | **Fehlendes Geraet / Konto**: Vault-Ordner nicht eingehaengt, Konto nicht angemeldet | `Transient` (Ordner kann zurueckkommen) bzw. Klartext; Mitteilung ohne Windows-Dienst: `Transient` | `a_missing_vault_*` |
//! | 5 | **Absturz eines Kindprozesses** | keiner: kein Prozess in diesen Bausteinen (HTTP und Dateien im Prozess) | - |
//! | 6 | **Voller Arbeitsspeicher** | nichts Schweres: Notiz < 64 KiB, keine Modellanfrage; bei knappem RAM aendert sich nichts (das Sprachmodell lief im Schritt davor ueber das RAM-Start-Tor) | - |
//! | 7 | **Feindliche Eingaben** (Prompt-Injection im Transkript) | Texte nur als Daten: maskiert, einzeilig, in Anfuehrungszeichen; kein Link, kein Tag, kein HTML, kein Verlassen des Blocks; Ziel, Konto und Pfad kommen nie aus dem Ergebnis; Frontmatter ohne Transkripttext ausser dem Titel (maskiert) | `hostile_text_*` |
//! | 8 | **Falsche Daten** | Frist ohne gueltigen Tag faellt weg; Datum vom Modell oder ohne Herkunft ist gekennzeichnet (Notiz, Mitteilung, Termin, Freigabe) und per `skip_model_dates` ausschliessbar; Tag vorbei: keine Mitteilung, kein Termin | `model_dates_*`, `a_deadline_in_the_past_*` |
//! | 9 | **Zeit** | feste Uhr der Engine; Zeitumstellung: hoechstens 6 h Schlaf je Wachen, Luecke -> Stunde danach; App war aus: verpasste Erinnerung kommt sofort, solange die Frist aussteht | `ak7_*`, `a_missed_reminder_*` |
//!
//! Offen, benannt: zwischen `find_by_id` und dem Schreiben im Vault liegen Millisekunden, in denen
//! ein anderer Prozess (Obsidian-Sync, AI-OS) eine Notiz mit derselben Kennung anlegen koennte; das
//! Schloss gilt nur im eigenen Prozess.

pub mod deadlines;
pub mod items;
pub mod render;

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::managers::integrations::folder::{FolderError, Sandbox};
use crate::managers::integrations::model::Kind as IntegrationKind;
use crate::managers::integrations::obsidian::{
    self, merge_block, ObsidianConfig, ObsidianError, SaveKind, SaveResult,
};
use crate::managers::provenance::{ActorKind, Locality, NewProvenance, SourceRef, SubjectKind};

use super::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::app_actions::{has_template, previous_result, spec_of, svc, text_param, AppServices};
use super::catalog::{self, ActionSpec};
use super::engine::Engine;
use super::integration_actions::{db_err, integration_of};

use items::{clean, Extracted, Input, Kind};
use render::{de_date, note_id, NoteMeta, TITLE_CHARS};

pub use deadlines::{DeadlineCalendar, DeadlineRemind};

const OPERATION: &str = "agent_note";
const MAX_SEGMENT_SOURCES: usize = 100;

/// Schuetzt Suchen und Anlegen im Vault gegen zwei Laeufe im selben Prozess.
static VAULT_LOCK: Mutex<()> = Mutex::new(());

fn unavailable_vault(e: ObsidianError) -> StepError {
    match e {
        // Vault nicht eingehaengt, Platte gesperrt: kann zurueckkommen.
        ObsidianError::Folder(FolderError::RootNotFound) | ObsidianError::Io(_) => {
            StepError::Transient(e.to_string())
        }
        ObsidianError::Folder(FolderError::Io(_)) => StepError::Transient(e.to_string()),
        other => StepError::Permanent(other.to_string()),
    }
}

/// Schreibt die Notiz oder aktualisiert sie (siehe Moduldoku): gleiche Rechnung wie
/// `obsidian::save_note`, mit einem anderen Inhalt und einer anderen Kennung (`ergebnis-...`).
fn write_note(
    cfg: &ObsidianConfig,
    ex: &Extracted,
    meta: &NoteMeta,
) -> Result<SaveResult, ObsidianError> {
    cfg.validate_fields()?;
    let sandbox = Sandbox::open(&cfg.path)?;
    let id = note_id(&meta.meeting_id);
    let _guard = VAULT_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(found) = obsidian::find_by_id(&sandbox, &id)? {
        let existing = std::fs::read(&found).map_err(|e| ObsidianError::Io(e.to_string()))?;
        let existing = String::from_utf8_lossy(&existing).into_owned();
        let rel = sandbox.rel_of(&found.canonicalize().unwrap_or(found.clone()));
        return match merge_block(
            &existing,
            &render::managed_block(ex, meta),
            &meta.updated_iso,
        ) {
            None => Ok(SaveResult {
                kind: SaveKind::Unchanged,
                rel,
                bytes: existing.len() as u64,
            }),
            Some(updated) => {
                let placed = sandbox.replace_bytes(&found, updated.as_bytes())?;
                Ok(SaveResult {
                    kind: SaveKind::Updated,
                    rel: placed.rel,
                    bytes: placed.bytes,
                })
            }
        };
    }
    let content = render::render_note(cfg, ex, meta);
    let placed = sandbox.write_bytes_new(
        cfg.subfolder.trim(),
        &render::file_name(meta),
        content.as_bytes(),
    )?;
    Ok(SaveResult {
        kind: SaveKind::Created,
        rel: placed.rel,
        bytes: placed.bytes,
    })
}

/// Pruefsumme der Eintraege: bindet die Freigabe an genau diesen Inhalt.
fn digest_of(ex: &Extracted) -> String {
    let mut h = Sha256::new();
    for i in &ex.items {
        h.update(i.key().as_bytes());
        h.update(i.text.as_bytes());
        h.update(i.quote.as_bytes());
        h.update(if i.unverified_date { b"1" } else { b"0" });
    }
    h.finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub struct AgentNote {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl AgentNote {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("agent.note"),
            services,
        }
    }

    fn record(
        &self,
        ctx: &RunCtx<'_>,
        ex: &Extracted,
        meeting_id: &str,
        title: &str,
        saved: &SaveResult,
    ) {
        if previous_result(ctx, SubjectKind::KnowledgeNote, OPERATION).is_some() {
            return;
        }
        let o = &ex.origin;
        let mut entry = NewProvenance::new(
            SubjectKind::KnowledgeNote,
            &note_id(meeting_id),
            OPERATION,
            ActorKind::Workflow,
        );
        entry.provider = Some(if o.local { "local" } else { "remote" }.to_string());
        entry.locality = Some(if o.local {
            Locality::Local
        } else {
            Locality::Remote
        });
        entry.model_id = o.model.clone();
        entry.prompt_tokens = o.prompt_tokens;
        entry.completion_tokens = o.completion_tokens;
        entry.duration_ms = o.duration_ms;
        entry.confidence = o.confidence;
        let mut sources = vec![SourceRef::new("meeting", meeting_id, Some(title))];
        sources.extend(
            ex.segments
                .iter()
                .take(MAX_SEGMENT_SOURCES)
                .map(|n| SourceRef::new("segment", &format!("{meeting_id}:S{n}"), None)),
        );
        entry.sources = sources;
        entry.params = Some(json!({
            "note": saved.rel,
            "outcome": match saved.kind {
                SaveKind::Created => "created",
                SaveKind::Updated => "updated",
                SaveKind::Unchanged => "unchanged",
            },
            "todos": ex.count(Kind::Todo),
            "deadlines": ex.count(Kind::Deadline),
            "decisions": ex.count(Kind::Decision),
        }));
        if let Err(e) = ctx.record_provenance(entry) {
            log::warn!(
                "workflows: Herkunft der Notiz fuer {}/{} nicht geschrieben: {e}",
                ctx.run_id,
                ctx.step_id
            );
        }
    }
}

impl Action for AgentNote {
    fn id(&self) -> &str {
        "agent.note"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::Idempotent
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &serde_json::Map<String, Value>) -> Result<(), String> {
        if let Some(Value::String(from)) = params.get("from") {
            if !has_template(from) && !deadlines::valid_step_id(from) {
                return Err("from: Kennung eines Schritts erwartet".to_string());
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let from = text_param(params, "from").unwrap_or("extract");
        let via = text_param(params, "via").unwrap_or("…");
        format!(
            "To-dos, Fristen und Entscheidungen aus Schritt „{from}“ mit Belegen als Notiz in den Vault {via} schreiben (eine Notiz je Besprechung, ein zweiter Lauf aktualisiert sie)"
        )
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let from = deadlines::from_of(params)?;
        let via = text_param(params, "via").unwrap_or_default().to_string();
        // Trockenlauf: das Ergebnis des Extraktionsschritts gibt es noch nicht.
        if env.planning
            && env
                .context
                .pointer(&format!("/steps/{from}/outcome"))
                .is_none()
        {
            return Ok(Some(GateView {
                target: Some(format!(
                    "Vault-Notiz zum Besprechungsergebnis (Schritt „{from}“)"
                )),
                args: json!({
                    "via": via,
                    "hinweis": "Der Inhalt steht erst beim Lauf fest und wird dann zur Freigabe vorgelegt."
                }),
                max_mode: None,
            }));
        }
        let ex = match items::read(env.context, &from)? {
            Input::Data(ex) => ex,
            Input::Nothing(why) => {
                return Ok(Some(GateView {
                    target: Some("Vault-Notiz: nichts zu schreiben".to_string()),
                    args: json!({"via": via, "hinweis": clean(&why, 160)}),
                    max_mode: None,
                }))
            }
        };
        let meeting_id = deadlines::meeting_id_in(env.context, &ex)?;
        let preview: Vec<String> = ex
            .items
            .iter()
            .take(10)
            .map(|i| match i.due {
                Some(d) => format!(
                    "{}: {} ({}{})",
                    match i.kind {
                        Kind::Todo => "To-do",
                        Kind::Deadline => "Frist",
                        Kind::Decision => "Entscheidung",
                    },
                    clean(&i.text, 100),
                    de_date(d),
                    if i.unverified_date {
                        ", Datum geschätzt"
                    } else {
                        ""
                    }
                ),
                None => format!("Entscheidung: {}", clean(&i.text, 100)),
            })
            .collect();
        Ok(Some(GateView {
            target: Some(format!("Vault-Notiz {}", note_id(&meeting_id))),
            args: json!({
                "via": via,
                "notiz": note_id(&meeting_id),
                "todos": ex.count(Kind::Todo),
                "fristen": ex.count(Kind::Deadline),
                "entscheidungen": ex.count(Kind::Decision),
                "inhalt": digest_of(&ex),
                "auszug": preview,
            }),
            max_mode: None,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        let from = deadlines::from_of(params)?;
        let ex = match items::read(ctx.context, &from)? {
            Input::Data(ex) => ex,
            Input::Nothing(why) => {
                return Ok(
                    StepOutput::with_data(json!({"written": false, "reason": why}))
                        .summary(&format!("Keine Notiz geschrieben: {why}")),
                )
            }
        };
        if ex.items.is_empty() {
            return Ok(StepOutput::with_data(json!({
                "written": false,
                "reason": "Der Extraktionsschritt hat keine To-dos, Fristen oder Entscheidungen geliefert."
            }))
            .summary("Keine Notiz geschrieben: nichts extrahiert."));
        }
        let meeting_id = deadlines::meeting_id_in(ctx.context, &ex)?;
        let store = self.services.store().map_err(svc)?;
        let meeting = store
            .get_meeting(&meeting_id)
            .map_err(|e| {
                StepError::Transient(format!("Die Besprechung ließ sich nicht lesen ({e})."))
            })?
            .filter(|m| m.deleted_at.is_none())
            .ok_or_else(|| {
                StepError::Permanent("Die Besprechung gibt es nicht mehr.".to_string())
            })?;
        let title = {
            let t = clean(&meeting.title, TITLE_CHARS);
            if t.is_empty() {
                "Besprechung".to_string()
            } else {
                t
            }
        };
        let meeting_day = ex
            .meeting_date
            .unwrap_or_else(|| crate::agent::extract::meeting_date(&meeting));
        let conn = ctx.conn().map_err(db_err)?;
        let integration = integration_of(
            &conn,
            params,
            &[IntegrationKind::Obsidian],
            "kein Obsidian-Vault",
        )?;
        let cfg = ObsidianConfig::from_config_json(&integration.config_json)
            .map_err(|e| StepError::Permanent(e.to_string()))?;
        let meta = NoteMeta {
            meeting_id: meeting_id.clone(),
            title: title.clone(),
            date_label: de_date(meeting_day),
            date_iso: meeting_day.format("%Y-%m-%d").to_string(),
            updated_iso: deadlines::local_day(ctx.now_ms())
                .format("%Y-%m-%d")
                .to_string(),
        };
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor die Notiz geschrieben wurde.".to_string(),
            ));
        }
        let saved = write_note(&cfg, &ex, &meta).map_err(unavailable_vault)?;
        self.record(ctx, &ex, &meeting_id, &title, &saved);
        let outcome = match saved.kind {
            SaveKind::Created => "created",
            SaveKind::Updated => "updated",
            SaveKind::Unchanged => "unchanged",
        };
        Ok(StepOutput::with_data(json!({
            "written": true,
            "outcome": outcome,
            "note": obsidian::display_rel(&saved.rel),
            "note_id": note_id(&meeting_id),
            "meeting_id": meeting_id,
            "context_area": cfg.context_area,
            "data_class": obsidian::data_class_for("besprechung"),
            "counts": {
                "todos": ex.count(Kind::Todo),
                "deadlines": ex.count(Kind::Deadline),
                "decisions": ex.count(Kind::Decision),
            },
        }))
        .summary(&match saved.kind {
            SaveKind::Created => format!("Notiz „{}“ angelegt.", obsidian::display_rel(&saved.rel)),
            SaveKind::Updated => {
                format!(
                    "Notiz „{}“ aktualisiert.",
                    obsidian::display_rel(&saved.rel)
                )
            }
            SaveKind::Unchanged => format!(
                "Notiz „{}“ war schon auf dem Stand.",
                obsidian::display_rel(&saved.rel)
            ),
        }))
    }
}

/// Haengt die Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(engine: &Engine, services: Arc<dyn AppServices>) {
    engine.register_action(Arc::new(AgentNote::new(services.clone())));
    engine.register_action(Arc::new(DeadlineRemind::new(services.clone())));
    engine.register_action(Arc::new(DeadlineCalendar::new(services)));
}

#[cfg(test)]
mod tests;
