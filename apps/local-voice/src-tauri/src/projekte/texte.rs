//! Kontexttexte der App-Inhalte (lva#365 P4): Seite, Buch, Besprechung, in der gewählten Form.
//!
//! Es gibt nie einen Rückgriff auf eine andere Form: fehlt das Kurzbuch oder das Protokoll, liefert
//! der Leser einen Hinweis. Fehler werden zu Hinweis-Codes ohne Inhalte oder Namen.

use super::herkunft::{besprechung_id, buch_id, seite_id};
use super::kontext::{hinweis, Form, KontextForm, Wahl};
use crate::managers::meetings::store::MeetingStore;
use std::path::Path;
use tauri::AppHandle;

/// Vorlage-Kennung des Kurzbuchs der Buch-KI.
const KURZBUCH: &str = "kurzbuch";
/// Art des Protokolls im Besprechungsspeicher.
const PROTOKOLL: &str = "minutes";

pub fn lesen(app: &AppHandle, meetings: &MeetingStore, link: &str, w: &Wahl) -> Form {
    if let Some(id) = seite_id(link) {
        return seite(app, id);
    }
    if let Some(id) = buch_id(link) {
        return buch(app, id, w);
    }
    if let Some(id) = besprechung_id(link) {
        return besprechung(meetings, id, w.form);
    }
    Form::Fehlt(hinweis::NICHT_LESBAR)
}

fn seitentext(dir: &Path) -> Form {
    let Ok(roh) = std::fs::read_to_string(dir.join("state.json")) else {
        return Form::Fehlt(hinweis::KEIN_TEXT);
    };
    match serde_json::from_str::<serde_json::Value>(&roh)
        .ok()
        .and_then(|v| v.get("text").and_then(|t| t.as_str().map(str::to_owned)))
    {
        Some(t) => Form::Text(t),
        None => Form::Fehlt(hinweis::KEIN_TEXT),
    }
}

fn seite(app: &AppHandle, id: &str) -> Form {
    match crate::commands::pages::page_lookup(app, id) {
        Ok(Some(dir)) => seitentext(&dir),
        Ok(None) => Form::Fehlt(hinweis::FEHLT),
        Err(_) => Form::Fehlt(hinweis::NICHT_LESBAR),
    }
}

fn buch(app: &AppHandle, id: &str, w: &Wahl) -> Form {
    let Ok(buecher) = crate::library::library_list(app.clone()) else {
        return Form::Fehlt(hinweis::NICHT_LESBAR);
    };
    let Some(b) = buecher.into_iter().find(|b| b.id == id) else {
        return Form::Fehlt(hinweis::FEHLT);
    };
    // Gewählte Fassung (Kennung oder Sprache), sonst die Hauptfassung.
    let seite = match w.fassung.as_deref() {
        Some(f) => b
            .editions
            .iter()
            .find(|e| e.id == f || e.lang == f)
            .and_then(|e| e.text.as_ref())
            .map(|t| t.page_id.clone()),
        None => b.text.as_ref().map(|t| t.page_id.clone()),
    };
    let Some(seite) = seite else {
        return Form::Fehlt(hinweis::KEIN_TEXT);
    };
    let dir = match crate::commands::pages::page_lookup(app, &seite) {
        Ok(Some(d)) => d,
        Ok(None) => return Form::Fehlt(hinweis::FEHLT),
        Err(_) => return Form::Fehlt(hinweis::NICHT_LESBAR),
    };
    match w.form {
        KontextForm::Volltext => seitentext(&dir),
        KontextForm::Kurzbuch => {
            let Ok(docs) = crate::book_ai::store::list(&dir) else {
                return Form::Fehlt(hinweis::KEIN_KURZBUCH);
            };
            // Neuestes Kurzbuch dieser Fassung; `list` liefert neueste zuerst.
            match docs.iter().find(|d| d.template_id == KURZBUCH) {
                Some(d) => match crate::book_ai::store::read_text(&dir, &d.id) {
                    Ok(t) => Form::Text(t),
                    Err(_) => Form::Fehlt(hinweis::KEIN_KURZBUCH),
                },
                None => Form::Fehlt(hinweis::KEIN_KURZBUCH),
            }
        }
        _ => Form::Fehlt(hinweis::FORM_UNGUELTIG),
    }
}

fn besprechung(store: &MeetingStore, id: &str, form: KontextForm) -> Form {
    match store.get_meeting(id) {
        Ok(Some(m)) if m.status == "ready" => {}
        Ok(Some(_)) => return Form::Fehlt(hinweis::KEIN_TEXT),
        Ok(None) => return Form::Fehlt(hinweis::FEHLT),
        Err(_) => return Form::Fehlt(hinweis::NICHT_LESBAR),
    }
    match form {
        KontextForm::Protokoll => {
            let Ok(docs) = store.get_documents(id) else {
                return Form::Fehlt(hinweis::NICHT_LESBAR);
            };
            match docs
                .into_iter()
                .filter(|d| d.kind == PROTOKOLL && !d.body.trim().is_empty())
                .max_by_key(|d| (d.version, d.updated_at))
            {
                Some(d) => Form::Text(d.body),
                None => Form::Fehlt(hinweis::KEIN_PROTOKOLL),
            }
        }
        KontextForm::Transkript => match crate::mcp::tools::transcript_text(store, id) {
            Ok(t) => Form::Text(t),
            Err(_) => Form::Fehlt(hinweis::NICHT_LESBAR),
        },
        _ => Form::Fehlt(hinweis::FORM_UNGUELTIG),
    }
}
