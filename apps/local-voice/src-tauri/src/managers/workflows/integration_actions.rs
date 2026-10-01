//! Integrations-Bausteine der Engine (B5): `mail.send`, `calendar.note`, `webhook.post`.
//!
//! Alle drei wirken nach aussen (`External`): eine zweite Ausfuehrung waere eine zweite Mail,
//! eine zweite Notiz, ein zweiter Aufruf. Deshalb gilt fuer jeden:
//!
//! - **Das Tor entscheidet, nicht der Baustein.** Die Engine fragt `integrations::gate` VOR `run`
//!   (Recht, Audit, Freigabe). Der Baustein ruft das Tor nie selbst und nie das Register-
//!   Schreibwerk (`targets::send_mail`, `m365::actions::*`) auf: ein zweiter Torgang haette bei
//!   „fragen“ eine zweite Freigabe verlangt. Er nutzt die unterste Ebene (`smtp::send_with`,
//!   `M365Service::send_mail`, `add_event_note`, `webhook::post`).
//! - **Die Freigabe zeigt, was geschieht** (`Action::gate_view`): Empfaenger vollstaendig,
//!   Betreff, Text, Anhaenge (Pfad, Groesse, Pruefsumme). Die Freigabe ist an GENAU diese Fassung
//!   gebunden; aendert sich bis zur Entscheidung etwas, wird der Schritt abgelehnt.
//! - **Empfaenger kommen nur aus dem Code.** `to` und `via` sind feste Werte der Definition
//!   (`catalog`: `literal`), die Menge bildet [`recipients::resolve`] aus der Regel und den Daten
//!   des Laufs (Teilnehmende, „Meine E-Mail-Adressen“, feste Liste). Text aus einem Modell oder
//!   Termin kann keine Adresse setzen; auch zur Laufzeit wird die Regel gegen die feste Liste
//!   geprueft.
//! - **E3 (Owner-Entscheidung): an mich automatisch, an andere nur nach Freigabe.** Geht eine Mail
//!   (auch) an jemand anderen als mich, begrenzt `GateView::max_mode` das Recht auf „fragen“, auch
//!   wenn der Nutzer „erlaubt“ eingestellt hat; der Schalter `auto: true` im Schritt hebt das fuer
//!   diesen Ablauf auf (je Ablauf aenderbar). Liegt der Anhang in einem Ordner, dessen Lesen auf
//!   „fragen“ steht, gilt dasselbe.
//! - **Nie von selbst wiederholen, wenn unklar.** `Transient` heisst „es ist nichts passiert“
//!   (Verbindung kam nicht zustande, 429, Datei gesperrt); alles, was NACH dem Senden der Daten
//!   scheitert (Zeitlimit, gerissene Verbindung, 5xx), ist `Unknown` und endet als
//!   `effect_uncertain`. Nach einem Absturz belegt `confirm` die Wirkung aus dem Provenienz-
//!   Eintrag, den der Baustein unmittelbar nach dem Senden schreibt.
//! - **Kein Defer.** Gatepflichtige Bausteine warten nie von sich aus: jede Wiederholung verlangt
//!   eine neue Freigabe (Befund B3). Mail, Notiz und Webhook haben alles, was sie brauchen, wenn
//!   sie drankommen; fehlt etwas, scheitern sie mit Klartext.
//!
//! # Bausteine
//!
//! | Baustein        | Recht (Tor)                 | Ziel                                              |
//! |-----------------|-----------------------------|---------------------------------------------------|
//! | `mail.send`     | `mail.send` an `via`        | Konto `m365` (Graph `sendMail`) oder `smtp`       |
//! | `calendar.note` | `calendar.write` an `via`   | Konto `m365`: Absatz im Text des Termins          |
//! | `webhook.post`  | `webhook.post` an `via`     | Integration `webhook`: Adresse im Geheimnisspeicher |
//!
//! # Fehlerfaelle (B5) und ihre Absicherung
//!
//! | # | Fehlerfall | Verhalten | Beleg |
//! |---|------------|-----------|-------|
//! | 1 | **Nebenlaeufigkeit**: zwei Laeufe, dieselbe Mail; Doppelklick auf „Freigeben“ | Freigabe je Lauf+Schritt (`lauf` in den Argumenten) und einmalig einloesbar; ein wiederholter Schritt sendet nicht noch einmal (Provenienz je `<lauf>:<schritt>`) | `two_runs_with_the_same_mail_ask_twice`, `a_repeated_mail_step_does_not_send_twice` |
//! | 2 | **Abbruch mitten im Vorgang**: App stirbt nach dem Senden, vor dem Journal | `confirm` belegt aus dem Provenienz-Eintrag, kein zweiter Versand; ohne Eintrag `effect_uncertain`, nie von selbst wiederholt | `a_crash_after_the_mail_went_out_is_confirmed_and_not_sent_again`, `a_crash_before_the_record_is_uncertain_and_waits_for_the_user` |
//! | 3 | **Voller Datentraeger / gesperrte Datenbank**: Audit nicht schreibbar; Provenienz nicht schreibbar; Anhang gesperrt | Tor lehnt ab (fail closed, A1); Provenienz nur geloggt (die Mail ging raus, das Journal haelt es fest); gesperrter Anhang ist `Transient` und es wurde nichts gesendet | `a_locked_attachment_is_transient_and_sends_nothing`, A1 `gate::tests::a_failing_audit_write_*` |
//! | 4 | **Fehlendes Geraet / Netz**: Mailserver, Graph, Webhook nicht erreichbar | `Transient` (nichts angekommen); Verbindung reisst NACH dem Senden, Zeitlimit, 5xx: `Unknown` | `smtp_server_down_is_transient`, `a_connection_lost_after_the_data_is_unknown_*`, `webhook_outcomes_*` |
//! | 5 | **Absturz eines Kindprozesses** | keiner: SMTP, Graph und Webhook laufen im Prozess; eine Panik im Baustein faengt die Engine als `Unknown` (B1) | B1 `a_panicking_action_*` |
//! | 6 | **Falsche Empfaenger** | nur feste Regel; `to`/`via`/`list` ohne `{{...}}`; ungueltige Adresse bricht ab (nie still ausgelassen); mehr als 20 brechen ab; Dritte immer mit Freigabe (E3) | `recipients::tests`, `third_parties_always_need_approval_even_when_allowed`, `a_forged_rule_*` |
//! | 7 | **Anhang** ausserhalb der Ordner, `..`, Symlink, Cloud-Platzhalter, zu gross, zu viele, geaendert | abgelehnt mit Klartext und Audit; geaenderte Datei bricht die Bindung der Freigabe | `attachments::tests`, `a_changed_attachment_breaks_the_approval` |
//! | 8 | **Webhook**: Umleitung, http ausserhalb Loopback, riesige Antwort, Zeitlimit, Adresse im Klartext | nie umgeleitet, `https`/Loopback, Antwort hoechstens 1 MiB, Zeitlimit 15 s, Adresse nur im Geheimnisspeicher, Payload ohne Geheimnisse | `webhook::tests`, `the_webhook_payload_carries_no_secrets_and_the_address_stays_secret` |
//! | 9 | **Integration aus / fragen** (AK8) | „aus“: Schritt `denied`, Lauf endet sauber, Audit `denied`; „fragen“: Freigabe mit Vorschau | `ak8_*` |
//! | 10 | **Echtzeit-Audiopfad, Arbeitsspeicher** | unberuehrt (kein Audio); Anhaenge hoechstens 2,5 MiB, Antwort hoechstens 1 MiB, Anfrage 256 KiB; nichts Schweres | Grenzen in `attachments`, `webhook` |
//!
//! Anhang und Freigabe (B21, QG5): Das Tor liest den Anhang fuer die Tor-Ansicht (Pfad, Groesse,
//! vollstaendiges SHA-256), die Freigabe haengt daran. `run` liest ihn GENAU EINMAL (handle-basiert,
//! `attachments::check`), vergleicht Bindung und gelesene Bytes mit den Argumenten, die das Tor fuer
//! diesen Durchgang geprueft hat (`RunCtx::gate_args`), und versendet exakt diese Bytes. Weicht
//! etwas ab (Datei zwischen Tor und Lesen ausgetauscht), ist der Schritt `Permanent` abgelehnt, im
//! Audit steht `attachment_changed_after_approval`, gesendet wird nichts.

pub mod attachments;
pub mod recipients;

use std::sync::Arc;

use rusqlite::Connection;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::managers::integrations::m365::event::{EventRef, NoteOutcome, MAX_NOTE_CHARS};
use crate::managers::integrations::m365::mail::{
    MailAttachment, MailBody, MailMessage as GraphMail,
};
use crate::managers::integrations::m365::{Acct, M365Error, M365Service};
use crate::managers::integrations::model::{
    AuditOutcome, Caller, Capability, GrantMode, Integration, Kind, NewAudit,
};
use crate::managers::integrations::smtp::{self, ConnectOpts, SendFailure, SmtpConfig};
use crate::managers::integrations::{audit, store as integrations_store, webhook};
use crate::managers::meetings::mail::valid_address;
use crate::managers::provenance::SubjectKind;

use super::action::{
    Action, EffectKind, GateEnv, GateView, Needs, NeedsError, RunCtx, StepError, StepOutput,
};
use super::app_actions::{has_template, previous_result, record, text_param, AppServices};
use super::catalog::{self, ActionSpec};
use super::engine::Engine;

use attachments::{AttachError, Checked};
use recipients::{RecipientError, Rule, Sources};

const MAX_SUBJECT_CHARS: usize = 150;
const MAX_BODY_CHARS: usize = 100_000;
const DEFAULT_BODY: &str = "Diese Nachricht wurde automatisch von Local Voice AI gesendet.";
/// Laengster Antworttext des Webhooks im Ergebnis des Schritts (Zeichen).
const MAX_REPLY_TEXT_CHARS: usize = 4_000;
/// Groesste JSON-Antwort des Webhooks, die als `json` im Ergebnis steht (Bytes).
const MAX_REPLY_JSON_BYTES: usize = 8 * 1024;

fn spec_of(id: &str) -> &'static ActionSpec {
    catalog::action_spec(id).unwrap_or_else(|| panic!("Katalogeintrag {id} fehlt"))
}

fn db_err(e: impl std::fmt::Display) -> StepError {
    StepError::Transient(format!("Das Register ist nicht erreichbar ({e})."))
}

/// Eine Zeile Text: Steuerzeichen weg, Leerraum zusammengefasst, gekuerzt.
fn one_line(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        flat
    }
}

/// Text mit Zeilenumbruechen: andere Steuerzeichen weg.
fn clean_body(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\r')
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn seed_of(key: &str) -> u128 {
    let d = Sha256::digest(key.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&d[..16]);
    u128::from_be_bytes(b)
}

/// Die Integration `via` aus dem Register; Art muss in `kinds` stehen.
fn integration_of(
    conn: &Connection,
    params: &Value,
    kinds: &[Kind],
    what: &str,
) -> Result<Integration, StepError> {
    let via = text_param(params, "via").ok_or_else(|| {
        StepError::Permanent("Es ist keine Integration angegeben (Parameter via).".to_string())
    })?;
    let i = integrations_store::get(conn, via)
        .map_err(db_err)?
        .ok_or_else(|| StepError::Permanent(format!("Die Integration „{via}“ gibt es nicht.")))?;
    if !kinds.contains(&i.kind) {
        return Err(StepError::Permanent(format!(
            "Die Integration „{via}“ ist {what}."
        )));
    }
    Ok(i)
}

fn m365_error(e: M365Error) -> StepError {
    let text = e.to_string();
    match e {
        // Ob die Anfrage ankam, ist unklar: nie von selbst wiederholen.
        M365Error::Uncertain(_) => StepError::Unknown(text),
        M365Error::Http { status, .. } if status >= 500 => StepError::Unknown(text),
        // 412: der Termin wurde zwischenzeitlich geaendert, geschrieben wurde nichts.
        M365Error::Http { status: 412, .. } => StepError::Transient(text),
        M365Error::Throttled { .. }
        | M365Error::Network(_)
        | M365Error::Timeout
        | M365Error::Store(_)
        | M365Error::MemoryLow(_)
        | M365Error::Cancelled => StepError::Transient(text),
        _ => StepError::Permanent(text),
    }
}

fn smtp_error(f: SendFailure) -> StepError {
    let text = f.error.to_string();
    if f.maybe_delivered {
        return StepError::Unknown(text);
    }
    match f.error {
        smtp::SmtpError::Connect(_) | smtp::SmtpError::Timeout | smtp::SmtpError::Protocol(_) => {
            StepError::Transient(text)
        }
        _ => StepError::Permanent(text),
    }
}

fn webhook_error(e: webhook::WebhookError) -> StepError {
    let text = e.to_string();
    match e.class() {
        webhook::Class::NotSent => StepError::Transient(text),
        webhook::Class::Rejected => StepError::Permanent(text),
        webhook::Class::Unknown => StepError::Unknown(text),
    }
}

fn m365_service(services: &dyn AppServices) -> Result<Arc<M365Service>, StepError> {
    services.m365().ok_or_else(|| {
        StepError::NotAvailable(
            "Das Microsoft-365-Konto ist in dieser Umgebung nicht bereit.".to_string(),
        )
    })
}

/// Haelt eine Ablehnung ohne Tor im Audit fest (Anhang ausserhalb, Lesen ausgeschaltet).
fn audit_refusal(
    conn: &Connection,
    integration: Option<&str>,
    capability: Option<Capability>,
    target: &str,
    reason: &str,
) {
    let entry = NewAudit {
        caller: Caller::Workflow.as_str().to_string(),
        integration_id: integration.map(str::to_string),
        capability: capability.map(|c| c.as_str().to_string()),
        target: Some(target.to_string()),
        outcome: AuditOutcome::Denied,
        detail: Some(json!({ "reason": reason })),
    };
    if let Err(e) = audit::record(conn, &entry) {
        log::warn!("workflows: Audit der Ablehnung nicht geschrieben: {e}");
    }
}

fn attach_error(conn: &Connection, planning: bool, e: AttachError) -> StepError {
    let text = e.to_string();
    match &e {
        AttachError::Outside(name) => {
            if !planning {
                audit_refusal(conn, None, None, name, "attachment_outside_allowed_folders");
            }
            StepError::Denied(text)
        }
        AttachError::ReadOff { name, reason, .. } => {
            if !planning {
                audit_refusal(conn, None, Some(Capability::FilesRead), name, reason);
            }
            StepError::Denied(text)
        }
        AttachError::Io(_) => StepError::Transient(text),
        _ => StepError::Permanent(text),
    }
}

// ---------------------------------------------------------------------------
// mail.send
// ---------------------------------------------------------------------------

pub struct MailSend {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl MailSend {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("mail.send"),
            services,
        }
    }

    /// Die Adresse der Integration selbst (Rueckfall fuer „ich“).
    fn own_address(&self, i: &Integration) -> Option<String> {
        match i.kind {
            Kind::Smtp => SmtpConfig::from_config_json(&i.config_json)
                .ok()
                .map(|c| c.from_address),
            Kind::M365 => self
                .services
                .m365()
                .and_then(|svc| svc.vault.load(i).ok().flatten())
                .and_then(|a| a.address.clone()),
            _ => None,
        }
    }

    /// Bildet die Mail aus den eingesetzten Parametern und dem Laufkontext (siehe Moduldoku).
    fn build(
        &self,
        conn: &Connection,
        context: &Value,
        params: &Value,
        planning: bool,
    ) -> Result<Built, StepError> {
        let integration = integration_of(
            conn,
            params,
            &[Kind::M365, Kind::Smtp],
            "kein Mail-Konto (Microsoft 365 oder SMTP)",
        )?;
        let rule = text_param(params, "to")
            .and_then(Rule::parse)
            .ok_or_else(|| {
                StepError::Permanent(
                    "Die Empfängerregel fehlt oder ist unbekannt (me, participants, all, internal, list)."
                        .to_string(),
                )
            })?;
        let list: Vec<String> = match params.get("list") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        };
        if rule != Rule::List && !list.is_empty() {
            return Err(StepError::Permanent(
                "Die feste Liste gilt nur mit der Regel „list“.".to_string(),
            ));
        }
        let attendees = recipients::attendees_of(context);
        let self_emails = self.services.self_emails();
        let own = self.own_address(&integration);
        let to = recipients::resolve(
            rule,
            &Sources {
                attendees: &attendees,
                self_emails: &self_emails,
                own_address: own.as_deref(),
                list: &list,
            },
        )
        .map_err(|e: RecipientError| StepError::Permanent(e.to_string()))?;

        let subject = one_line(
            text_param(params, "subject").unwrap_or_default(),
            MAX_SUBJECT_CHARS,
        );
        if subject.is_empty() {
            return Err(StepError::Permanent("Es fehlt ein Betreff.".to_string()));
        }
        let body = match text_param(params, "body") {
            Some(b) => clean_body(b),
            None => DEFAULT_BODY.to_string(),
        };
        if body.chars().count() > MAX_BODY_CHARS {
            return Err(StepError::Permanent(format!(
                "Der Mailtext ist zu lang ({} Zeichen, höchstens {MAX_BODY_CHARS}).",
                body.chars().count()
            )));
        }

        let paths = attach_paths(params)?;
        let pending = planning && paths.iter().any(|p| has_template(p));
        let attachments = if pending {
            Vec::new()
        } else {
            attachments::check(conn, &paths).map_err(|e| attach_error(conn, planning, e))?
        };
        Ok(Built {
            integration,
            rule,
            to,
            subject,
            body,
            attachments,
            attachments_pending: pending,
            auto: params.get("auto").and_then(Value::as_bool).unwrap_or(false),
        })
    }
}

/// Eine fertig gebildete Mail.
struct Built {
    integration: Integration,
    rule: Rule,
    to: Vec<String>,
    subject: String,
    body: String,
    attachments: Vec<Checked>,
    /// Plan: die Pfade stehen noch als `{{...}}` da.
    attachments_pending: bool,
    auto: bool,
}

impl Built {
    /// B21 (QG5): Die Anhaenge wurden eben EINMAL gelesen (`attachments::check`, handle-basiert);
    /// ihre Bindung (Pfad, Groesse, vollstaendiges SHA-256) muss der entsprechen, die das Tor fuer
    /// diesen Durchgang geprueft hat und an die die Freigabe gebunden ist (`RunCtx::gate_args`).
    /// Abweichung: der Schritt wird abgelehnt (`Permanent`, Audit `denied`), gesendet wird nichts.
    /// Ohne Tor-Argumente (Direktaufruf in Tests) gibt es nichts, wogegen man pruefen koennte.
    fn check_bound(&self, conn: &Connection, ctx: &RunCtx<'_>) -> Result<(), StepError> {
        let Some(bound) = ctx.gate_args else {
            return Ok(());
        };
        let now = self.view().args;
        let (was, is) = (bound.get("attachments"), now.get("attachments"));
        if was == is {
            return Ok(());
        }
        // Wessen Bindung weicht ab? (Nur der Name kommt in Audit und Meldung.)
        let bound_list = was.and_then(Value::as_array).cloned().unwrap_or_default();
        let current_list = is.and_then(Value::as_array).cloned().unwrap_or_default();
        let first = (0..current_list.len().max(bound_list.len()))
            .find(|i| bound_list.get(*i) != current_list.get(*i))
            .unwrap_or(0);
        let name = self
            .attachments
            .get(first)
            .or_else(|| self.attachments.first())
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "(Anhang)".to_string());
        audit_refusal(
            conn,
            Some(&self.integration.id),
            Some(Capability::MailSend),
            &name,
            "attachment_changed_after_approval",
        );
        Err(StepError::Permanent(format!(
            "Der Anhang „{name}“ hat sich geändert, seit die Freigabe erteilt wurde; die Mail wurde nicht gesendet."
        )))
    }

    /// Hoechstens „fragen“, wenn die Mail an Dritte geht (E3) oder ein Anhang aus einem Ordner
    /// stammt, dessen Lesen auf „fragen“ steht.
    fn max_mode(&self) -> Option<GrantMode> {
        let others = self.rule.reaches_others() && !self.auto;
        let read_asks = self
            .attachments
            .iter()
            .any(|a| a.read_mode == GrantMode::Ask);
        (others || read_asks).then_some(GrantMode::Ask)
    }

    /// Was das Tor sieht und der Nutzer in der Freigabe (siehe Moduldoku).
    fn view(&self) -> GateView {
        let mut args = json!({
            "via": self.integration.id,
            "rule": self.rule.as_str(),
            "to": self.to,
            "subject": self.subject,
            "body": self.body,
        });
        if !self.attachments.is_empty() {
            args["attachments"] = json!(self
                .attachments
                .iter()
                .map(|a| json!({"file": a.display, "bytes": a.size, "sha256": a.sha256}))
                .collect::<Vec<_>>());
        } else if self.attachments_pending {
            args["attachments"] = json!(["(Pfad wird beim Lauf eingesetzt)"]);
        }
        GateView {
            target: Some(recipients::target_of(&self.to)),
            args,
            max_mode: self.max_mode(),
        }
    }
}

/// Die Pfade aus `attach`: ein Text oder eine Liste von Texten; leere Eintraege zaehlen nicht.
fn attach_paths(params: &Value) -> Result<Vec<String>, StepError> {
    let bad = || {
        StepError::Permanent(
            "Anhänge (attach) sind ein Pfad oder eine Liste von Pfaden.".to_string(),
        )
    };
    match params.get("attach") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(if s.trim().is_empty() {
            Vec::new()
        } else {
            vec![s.clone()]
        }),
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for v in items {
                match v {
                    Value::String(s) if s.trim().is_empty() => {}
                    Value::String(s) => out.push(s.clone()),
                    _ => return Err(bad()),
                }
            }
            Ok(out)
        }
        Some(_) => Err(bad()),
    }
}

fn mail_output(
    built_rule: Rule,
    via: &str,
    recipients: u64,
    attachments: &[String],
    subject: &str,
    message_id: Option<&str>,
    reused: bool,
) -> StepOutput {
    let mut data = json!({
        "sent": true,
        "via": via,
        "rule": built_rule.as_str(),
        "recipients": recipients,
        "attachments": attachments,
        "subject": subject,
    });
    if let Some(id) = message_id {
        data["message_id"] = json!(id);
    }
    StepOutput::with_data(data).summary(&if reused {
        "Die Mail war schon gesendet (Wiederaufnahme).".to_string()
    } else {
        format!(
            "Mail an {recipients} Empfänger gesendet ({}).",
            built_rule.label()
        )
    })
}

/// Das Ergebnis eines frueheren Versuchs dieses Schritts (Provenienz), falls die Mail schon
/// hinausging.
fn earlier_mail(ctx: &RunCtx<'_>) -> Option<StepOutput> {
    let (_, prev) = previous_result(ctx, SubjectKind::Export, "mail")?;
    let p = prev?;
    let rule = p
        .get("rule")
        .and_then(Value::as_str)
        .and_then(Rule::parse)?;
    let names: Vec<String> = p
        .get("attachments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(mail_output(
        rule,
        p.get("via").and_then(Value::as_str).unwrap_or(""),
        p.get("recipients").and_then(Value::as_u64).unwrap_or(0),
        &names,
        p.get("subject").and_then(Value::as_str).unwrap_or(""),
        p.get("message_id").and_then(Value::as_str),
        true,
    ))
}

impl Action for MailSend {
    fn id(&self) -> &str {
        "mail.send"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn validate(&self, params: &Map<String, Value>) -> Result<(), String> {
        if params.get("draft").and_then(Value::as_bool) == Some(true) {
            return Err(
                "draft: Entwürfe lassen sich noch nicht anlegen (dafür fehlt dem Konto das Recht Mail.ReadWrite); die Freigabe mit Vorschau ersetzt den Entwurf."
                    .to_string(),
            );
        }
        let rule = params
            .get("to")
            .and_then(Value::as_str)
            .and_then(Rule::parse);
        let list: Vec<&str> = params
            .get("list")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        match rule {
            Some(Rule::List) => {
                if list.iter().all(|a| a.trim().is_empty()) {
                    return Err(
                        "list: die Regel „list“ braucht mindestens eine Adresse.".to_string()
                    );
                }
                if let Some(bad) = list
                    .iter()
                    .find(|a| !a.trim().is_empty() && !valid_address(a))
                {
                    return Err(format!("list: „{bad}“ ist keine brauchbare Adresse."));
                }
                if list.iter().filter(|a| !a.trim().is_empty()).count() > recipients::MAX_RECIPIENTS
                {
                    return Err(format!(
                        "list: höchstens {} Adressen.",
                        recipients::MAX_RECIPIENTS
                    ));
                }
            }
            Some(_) if !list.is_empty() => {
                return Err("list: die feste Liste gilt nur mit der Regel „list“.".to_string());
            }
            _ => {}
        }
        if let Some(a) = params.get("attach") {
            let ok = match a {
                Value::String(_) | Value::Null => true,
                Value::Array(items) => items.iter().all(Value::is_string),
                _ => false,
            };
            if !ok {
                return Err("attach: ein Pfad oder eine Liste von Pfaden erwartet.".to_string());
            }
        }
        Ok(())
    }

    fn describe(&self, params: &Value) -> String {
        let base = catalog::describe_from_spec(self.spec, params);
        let n = match params.get("attach") {
            Some(Value::String(s)) if !s.trim().is_empty() => 1,
            Some(Value::Array(a)) => a
                .iter()
                .filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
                .count(),
            _ => 0,
        };
        let mut extra = Vec::new();
        if n > 0 {
            extra.push(format!("{n} Anhang"));
        }
        if params.get("auto").and_then(Value::as_bool) == Some(true) {
            extra.push("ohne Freigabe, wenn erlaubt".to_string());
        }
        if extra.is_empty() {
            base
        } else {
            format!("{base} ({})", extra.join(", "))
        }
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let built = self.build(env.conn, env.context, params, env.planning)?;
        Ok(Some(built.view()))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if let Some(out) = earlier_mail(ctx) {
            return Ok(out);
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor die Mail hinausging.".to_string(),
            ));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let built = self.build(&conn, ctx.context, params, false)?;
        // Die Bytes, die jetzt gesendet werden, muessen die sein, woran die Freigabe haengt (B21).
        built.check_bound(&conn, ctx)?;
        let names: Vec<String> = built.attachments.iter().map(|a| a.name.clone()).collect();
        let via = built.integration.id.clone();

        let message_id: Option<String> = match built.integration.kind {
            Kind::Smtp => {
                let cfg = SmtpConfig::from_config_json(&built.integration.config_json)
                    .map_err(|e| StepError::Permanent(e.to_string()))?;
                let password = match self.services.secret(&built.integration, "password") {
                    Ok(Some(p)) => p,
                    Ok(None) if cfg.username.is_empty() => Zeroizing::new(String::new()),
                    Ok(None) => {
                        return Err(StepError::Permanent(
                            smtp::SmtpError::PasswordMissing.to_string(),
                        ))
                    }
                    Err(e) => return Err(StepError::Permanent(e)),
                };
                let msg = smtp::MailMessage {
                    to: built.to.clone(),
                    cc: Vec::new(),
                    subject: built.subject.clone(),
                    body_text: built.body.clone(),
                    body_html: None,
                };
                let atts: Vec<smtp::Attachment> = built
                    .attachments
                    .iter()
                    .map(|a| smtp::Attachment {
                        name: a.name.clone(),
                        content_type: a.content_type.to_string(),
                        bytes: a.bytes.clone(),
                    })
                    .collect();
                let receipt = smtp::send_with(
                    &cfg,
                    &password,
                    &msg,
                    &atts,
                    &ConnectOpts::default(),
                    Some(seed_of(&ctx.idempotency_key)),
                )
                .map_err(smtp_error)?;
                Some(receipt.message_id)
            }
            _ => {
                let svc = m365_service(&*self.services)?;
                let acct = Acct::from_integration(built.integration.clone())
                    .map_err(|e| StepError::Permanent(e.to_string()))?;
                let msg = GraphMail::new(
                    &built.to,
                    &[],
                    &built.subject,
                    MailBody::Text(built.body.clone()),
                )
                .and_then(|m| {
                    m.with_attachments(
                        built
                            .attachments
                            .iter()
                            .map(|a| MailAttachment {
                                name: a.name.clone(),
                                content_type: a.content_type.to_string(),
                                bytes: a.bytes.clone(),
                            })
                            .collect(),
                    )
                })
                .map_err(|e| StepError::Permanent(e.to_string()))?;
                tauri::async_runtime::block_on(svc.send_mail(&acct, &msg)).map_err(m365_error)?;
                None
            }
        };

        // Unmittelbar nach dem Senden: der Beleg fuer `confirm` und die Wiederholung.
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "mail",
            Vec::new(),
            json!({
                "via": via,
                "rule": built.rule.as_str(),
                "recipients": built.to.len(),
                "attachments": names,
                "subject": built.subject,
                "message_id": message_id,
                "idempotency_key": ctx.idempotency_key,
            }),
        );
        Ok(mail_output(
            built.rule,
            &via,
            built.to.len() as u64,
            &names,
            &built.subject,
            message_id.as_deref(),
            false,
        ))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        earlier_mail(ctx)
    }
}

// ---------------------------------------------------------------------------
// calendar.note
// ---------------------------------------------------------------------------

pub struct CalendarNote {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl CalendarNote {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("calendar.note"),
            services,
        }
    }
}

/// Der Termin aus seiner Kennung `<quelle>:<uid>:<beginn-ms>` (`CalEvent::key`): die UID steht
/// zwischen dem ersten und dem letzten Doppelpunkt (sie kann selbst keinen enthalten, die
/// Quelle auch nicht, aber so bleibt die Zerlegung eindeutig).
fn event_ref_of(key: &str, context: &Value) -> Result<(EventRef, Option<String>), StepError> {
    let bad = || {
        StepError::Permanent(
            "Die Kennung des Termins ist ungültig (erwartet: <Quelle>:<UID>:<Beginn>).".to_string(),
        )
    };
    let (rest, start) = key.rsplit_once(':').ok_or_else(bad)?;
    let starts_at: i64 = start.parse().map_err(|_| bad())?;
    let (_, uid) = rest.split_once(':').ok_or_else(bad)?;
    if uid.is_empty() {
        return Err(bad());
    }
    // Das Ende steht nur beim Termin des Ausloesers fest; sonst reicht die Minute nach dem Beginn.
    let is_trigger = context.pointer("/trigger/event_id").and_then(Value::as_str) == Some(key);
    let ends_at = if is_trigger {
        context
            .pointer("/trigger/end")
            .and_then(Value::as_str)
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis())
            .unwrap_or(starts_at + 60_000)
    } else {
        starts_at + 60_000
    };
    let title = is_trigger
        .then(|| context.pointer("/trigger/title").and_then(Value::as_str))
        .flatten()
        .map(|t| one_line(t, 80));
    Ok((
        EventRef {
            uid: uid.to_string(),
            starts_at,
            ends_at,
        },
        title,
    ))
}

fn note_event_key(context: &Value, params: &Value) -> Result<String, StepError> {
    text_param(params, "event")
        .map(str::to_string)
        .or_else(|| {
            context
                .pointer("/trigger/event_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| {
            StepError::Permanent(
                "Der Termin fehlt: der Auslöser nennt keinen, und im Schritt ist keiner angegeben (event)."
                    .to_string(),
            )
        })
}

fn note_text(params: &Value) -> Result<String, StepError> {
    let text = clean_body(text_param(params, "text").unwrap_or_default());
    if text.trim().is_empty() {
        return Err(StepError::Permanent("Die Notiz ist leer.".to_string()));
    }
    if text.chars().count() > MAX_NOTE_CHARS {
        return Err(StepError::Permanent(format!(
            "Die Notiz ist zu lang (höchstens {MAX_NOTE_CHARS} Zeichen)."
        )));
    }
    Ok(text)
}

fn note_output(outcome: NoteOutcome, reused: bool) -> StepOutput {
    let added = matches!(outcome, NoteOutcome::Added);
    StepOutput::with_data(json!({"added": added, "already_there": !added})).summary(&if reused {
        "Die Notiz stand schon im Termin (Wiederaufnahme).".to_string()
    } else if added {
        "Notiz an den Termin gehängt.".to_string()
    } else {
        "Die Notiz stand schon im Termin.".to_string()
    })
}

fn earlier_note(ctx: &RunCtx<'_>) -> Option<StepOutput> {
    previous_result(ctx, SubjectKind::Export, "calendar_note")?;
    Some(note_output(NoteOutcome::AlreadyThere, true))
}

impl Action for CalendarNote {
    fn id(&self) -> &str {
        "calendar.note"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let key = note_event_key(env.context, params)?;
        let (eref, title) = event_ref_of(&key, env.context)?;
        let text = note_text(params)?;
        integration_of(env.conn, params, &[Kind::M365], "kein Microsoft-365-Konto")?;
        let start = chrono::DateTime::from_timestamp_millis(eref.starts_at)
            .map(|d| d.format("%d.%m.%Y %H:%M UTC").to_string())
            .unwrap_or_default();
        let target = match title {
            Some(t) if !t.is_empty() => format!("Termin „{t}“, {start}"),
            _ => format!(
                "Termin {}, {start}",
                eref.uid.chars().take(40).collect::<String>()
            ),
        };
        Ok(Some(GateView {
            target: Some(target),
            args: json!({
                "via": text_param(params, "via"),
                "event": key,
                "note": text,
            }),
            max_mode: None,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if let Some(out) = earlier_note(ctx) {
            return Ok(out);
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor die Notiz geschrieben wurde.".to_string(),
            ));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let integration = integration_of(&conn, params, &[Kind::M365], "kein Microsoft-365-Konto")?;
        let key = note_event_key(ctx.context, params)?;
        let (eref, _) = event_ref_of(&key, ctx.context)?;
        let text = note_text(params)?;
        let svc = m365_service(&*self.services)?;
        let acct = Acct::from_integration(integration.clone())
            .map_err(|e| StepError::Permanent(e.to_string()))?;
        let outcome = tauri::async_runtime::block_on(svc.add_event_note(&acct, &eref, &text))
            .map_err(m365_error)?;
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "calendar_note",
            Vec::new(),
            json!({
                "via": integration.id,
                "event": key,
                "chars": text.chars().count(),
                "added": matches!(outcome, NoteOutcome::Added),
            }),
        );
        Ok(note_output(outcome, false))
    }

    /// Die Notiz traegt eine Marke im Termin: ein zweiter Versuch haengt nichts doppelt an. Hier
    /// genuegt der Beleg aus dem Provenienz-Eintrag.
    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        earlier_note(ctx)
    }
}

// ---------------------------------------------------------------------------
// webhook.post
// ---------------------------------------------------------------------------

pub struct WebhookPost {
    spec: &'static ActionSpec,
    services: Arc<dyn AppServices>,
}

impl WebhookPost {
    pub fn new(services: Arc<dyn AppServices>) -> Self {
        Self {
            spec: spec_of("webhook.post"),
            services,
        }
    }
}

/// Die Nutzdaten ohne Geheimnisse: Felder wie `token`/`password` werden `***`, Zugangsdaten in
/// Adressen, Bearer-Tokens und JWTs in Texten ebenfalls (`audit::redact_params`).
fn scrubbed_body(params: &Value) -> Value {
    match params.get("body") {
        None | Some(Value::Null) => Value::Null,
        Some(v) => audit::redact_params(v),
    }
}

fn reply_output(reply: &webhook::WebhookReply, reused: bool) -> StepOutput {
    let text = one_line(&reply.body, MAX_REPLY_TEXT_CHARS);
    let mut data = json!({
        "ok": true,
        "status": reply.status,
        "bytes": reply.bytes,
        "truncated": reply.truncated,
        "text": text,
    });
    if !reply.truncated {
        if let Ok(v) = serde_json::from_str::<Value>(&reply.body) {
            if v.to_string().len() <= MAX_REPLY_JSON_BYTES {
                data["json"] = v;
            }
        }
    }
    StepOutput::with_data(data).summary(&if reused {
        "Der Webhook hatte die Daten schon bekommen (Wiederaufnahme).".to_string()
    } else {
        format!("Daten an den Webhook gesendet (HTTP {}).", reply.status)
    })
}

fn earlier_webhook(ctx: &RunCtx<'_>) -> Option<StepOutput> {
    let (_, prev) = previous_result(ctx, SubjectKind::Export, "webhook")?;
    let p = prev?;
    Some(
        StepOutput::with_data(json!({
            "ok": true,
            "status": p.get("status").cloned().unwrap_or(Value::Null),
            "reused": true,
        }))
        .summary("Der Webhook hatte die Daten schon bekommen (Wiederaufnahme)."),
    )
}

impl Action for WebhookPost {
    fn id(&self) -> &str {
        "webhook.post"
    }

    fn effect(&self) -> EffectKind {
        EffectKind::External
    }

    fn needs(&self, params: &Value) -> Result<Option<Needs>, NeedsError> {
        catalog::needs_from_spec(self.spec, params)
    }

    fn describe(&self, params: &Value) -> String {
        catalog::describe_from_spec(self.spec, params)
    }

    fn gate_view(&self, env: &GateEnv<'_>, params: &Value) -> Result<Option<GateView>, StepError> {
        let i = integration_of(env.conn, params, &[Kind::Webhook], "kein Webhook")?;
        let host = webhook::host_from_config(&i.config_json);
        let label = one_line(&i.label, 60);
        Ok(Some(GateView {
            // Nur der Name und der Server, nie die Adresse (sie ist ein Geheimnis).
            target: Some(if host.is_empty() {
                format!("Webhook „{label}“")
            } else {
                format!("Webhook „{label}“ ({host})")
            }),
            args: json!({
                "via": i.id,
                "body": scrubbed_body(params),
            }),
            max_mode: None,
        }))
    }

    fn run(&self, ctx: &RunCtx<'_>, params: &Value) -> Result<StepOutput, StepError> {
        if let Some(out) = earlier_webhook(ctx) {
            return Ok(out);
        }
        if ctx.cancelled() {
            return Err(StepError::Transient(
                "Der Lauf wurde abgebrochen, bevor der Webhook gerufen wurde.".to_string(),
            ));
        }
        let conn = ctx.conn().map_err(db_err)?;
        let i = integration_of(&conn, params, &[Kind::Webhook], "kein Webhook")?;
        let raw = match self.services.secret(&i, "url") {
            Ok(Some(u)) => u,
            Ok(None) => {
                return Err(StepError::Permanent(
                    webhook::WebhookError::UrlMissing.to_string(),
                ))
            }
            Err(e) => return Err(StepError::Permanent(e)),
        };
        let url = webhook::parse_url(&raw).map_err(webhook_error)?;
        let payload = json!({
            "schema": "lva-webhook@1",
            "workflow": {
                "id": ctx.workflow_id,
                "name": ctx.context.pointer("/workflow/name").and_then(Value::as_str).unwrap_or(""),
            },
            "run": ctx.run_id,
            "step": ctx.step_id,
            "idempotency_key": ctx.idempotency_key,
            "body": scrubbed_body(params),
        });
        let reply = webhook::post(
            &url,
            &payload,
            &ctx.idempotency_key,
            &webhook::HttpOpts::default(),
        )
        .map_err(webhook_error)?;
        record(
            ctx,
            SubjectKind::Export,
            &ctx.idempotency_key,
            "webhook",
            Vec::new(),
            json!({
                "via": i.id,
                "host": webhook::host_of(&url),
                "status": reply.status,
                "bytes": reply.bytes,
                "idempotency_key": ctx.idempotency_key,
            }),
        );
        Ok(reply_output(&reply, false))
    }

    fn confirm(&self, ctx: &RunCtx<'_>, _params: &Value) -> Option<StepOutput> {
        earlier_webhook(ctx)
    }
}

// ---------------------------------------------------------------------------
// Einhaengen
// ---------------------------------------------------------------------------

/// Alle Integrations-Bausteine dieses Pakets.
pub fn actions(services: Arc<dyn AppServices>) -> Vec<Arc<dyn Action>> {
    vec![
        Arc::new(MailSend::new(services.clone())),
        Arc::new(CalendarNote::new(services.clone())),
        Arc::new(WebhookPost::new(services)),
    ]
}

/// Haengt die Bausteine in die Engine (ersetzt die Katalogbausteine).
pub fn install(engine: &Engine, services: Arc<dyn AppServices>) {
    for action in actions(services) {
        engine.register_action(action);
    }
}

#[cfg(test)]
mod tests;
