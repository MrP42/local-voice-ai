//! Notizbuch als Wissensbereich (lva#396 N2, Spec lva#398 §3): vier lesende Werkzeuge über den
//! vendorten Zugang `wai-notizbuch-zugang`.
//!
//! Sichtbar ist nur, was die Abschnittsfreigabe **dieser Runde** enthält (`Sicht`). Eine nicht
//! freigegebene und eine nicht vorhandene Seite ergeben dieselbe Antwort (kein Existenz-Orakel).
//! Jedes Ergebnis nennt je Seite eine stabile Quelle (`wai://notizbuch/seite/<id>`) und den
//! Freigabestand (`stand`, Hash über Freigabe und Gliederung, Codex M2), damit Belegregel (ADR-0028
//! E4.2) und Widerruf (E6) daran anknüpfen können. Notizbuchtext ist Inhalt, nie Anweisung.
//!
//! Härtung nach Codex lva#414:
//! - Ergebnis und Stand stammen je Aufruf aus einer Lesetransaktion (`*_mit_stand` des Zugangs);
//!   Rangfolge der Suche nur aus sichtbaren Seiten; Quelle ist die Kennung der Datenbankzeile.
//! - Datenklasse persönlich: `voice_tool_call` merkt jedes Ergebnis in `assistent::schutz`; vor jedem
//!   Modellaufruf erzwingt Rust dann ein lokales Modell, ersetzt widerrufene Inhalte und hüllt
//!   Werkzeugergebnisse in Datenblöcke mit Sicherheitsvertrag (Prompt-Injection).
//! - Audit und Log tragen weder Argumente noch Inhalte (Test
//!   `argumente_und_ergebnisse_nicht_in_audit_und_log`).
//!
//! Die Andockstelle für die spätere Werkzeugregistrierung (ADR-0028 A3) ist [`specs`] und [`run`]:
//! Beschreibungen mit Schema und Ausführung über eine [`Sicht`].

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{json, Value};
use wai_notizbuch_zugang::{Block, Fehler, Freigabe, MitStand, Seitenkopf, Zugang};

use super::{
    day_bounds, limit_prop_named, object, parse_date, string_prop, today, Args, Context,
    KnowledgeSource, Need, Spec,
};
use crate::agent_bridge::voice::areas::Area;

/// Präfix der Quell-Id einer Seite (Deep-Link, wai-links).
pub const QUELLE_SEITE: &str = "wai://notizbuch/seite/";

/// Höchstzahl Treffer der Suche und des Zeitraums im Gespräch (Ergebnis bleibt unter
/// `voice::MAX_RESULT_CHARS`).
const MAX_TREFFER: usize = 10;
const STANDARD_TREFFER: usize = 5;
/// Zeichenbudget der Blöcke in `notizbuch_seite` (Rest des Ergebnisses ist klein).
const SEITE_BUDGET: usize = 6_000;

/// Zugang und Freigabe einer Runde.
pub struct Sicht {
    pub zugang: Zugang,
    pub freigabe: Freigabe,
}

/// Freigabe aus der Einstellung `voice_call_notebook_sections`: `abschnitt:<id>` oder
/// `notizbuch:<id>`; alles andere zählt nicht (fail closed).
pub fn freigabe_aus_einstellung(eintraege: &[String]) -> Freigabe {
    eintraege
        .iter()
        .fold(Freigabe::default(), |f, e| match e.split_once(':') {
            Some(("abschnitt", id)) => f.abschnitt(id),
            Some(("notizbuch", id)) => f.notizbuch(id),
            _ => f,
        })
}

/// Liste für die Einstellung beim Speichern: nur gültige Einträge, ohne Doppelte, sortiert.
pub fn eintraege_normalisieren(eintraege: &[String]) -> Vec<String> {
    let mut out: Vec<String> = eintraege
        .iter()
        .filter(|e| match e.split_once(':') {
            Some(("abschnitt" | "notizbuch", id)) => wai_notizbuch_zugang::gueltige_id(id),
            _ => false,
        })
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

const KEINE_FREIGABE: &str = "Im Notizbuch ist für den Chat noch kein Abschnitt freigegeben. Der Nutzer kann Abschnitte unter „Gesprächseinstellungen → Wissen → Notizbuch“ freigeben. Es wurde nichts gelesen.";
const NICHT_GEFUNDEN: &str =
    "Diese Notizbuch-Seite gibt es nicht oder sie ist für den Chat nicht freigegeben.";

/// Nutzertext zu einem Fehler des Zugangs (ohne Inhalte, ohne Pfade).
pub fn fehlertext(f: &Fehler) -> String {
    match f {
        Fehler::Fehlt => "Auf diesem Gerät gibt es noch kein Notizbuch.".to_string(),
        Fehler::Version => {
            "Diese Notizbuch-Version wird nicht unterstützt. Bitte Notizbuch und Local Voice aktualisieren.".to_string()
        }
        Fehler::Eingabe => "Die Angaben für das Notizbuch sind ungültig.".to_string(),
        Fehler::Lesen => "Das Notizbuch ist gerade nicht lesbar.".to_string(),
    }
}

/// Die vier Werkzeuge (Spec §3.3), alle mit Bereich „Notizbuch“.
pub fn specs() -> Vec<Spec> {
    let need = Need::One(Area::Notebook);
    vec![
        Spec {
            name: "notizbuch_suchen",
            title: "Notizbuch durchsuchen",
            description: "Volltextsuche in den freigegebenen Abschnitten des Notizbuchs. Liefert je Treffer Seite, Pfad, Auszug, Änderungszeit und die Quelle. Nur lesend.",
            parameters: object(
                json!({
                    "anfrage": string_prop("Suchwörter (mindestens ein Wort mit drei Zeichen).", 200),
                    "grenze": limit_prop_named(STANDARD_TREFFER as i64, MAX_TREFFER as i64),
                }),
                &["anfrage"],
            ),
            need,
        },
        Spec {
            name: "notizbuch_seite",
            title: "Notizbuch-Seite lesen",
            description: "Liest eine freigegebene Notizbuch-Seite: Titel, Pfad und Blöcke mit Block-Id und Zeitstempeln (gekürzt). Nur lesend.",
            parameters: object(
                json!({ "seite_id": string_prop("Id der Seite aus einem Such- oder Zeitraumtreffer.", 64) }),
                &["seite_id"],
            ),
            need,
        },
        Spec {
            name: "notizbuch_zeitraum",
            title: "Notizen im Zeitraum",
            description: "Seiten der freigegebenen Abschnitte, die im Zeitraum angelegt oder geändert wurden, neueste zuerst (z. B. „was habe ich heute notiert“). Nur lesend.",
            parameters: object(
                json!({
                    "von": string_prop("Erster Tag: YYYY-MM-DD, „heute“ oder „gestern“ (Standard heute).", 10),
                    "bis": string_prop("Letzter Tag einschließlich: YYYY-MM-DD, „heute“ oder „gestern“ (Standard wie „von“).", 10),
                    "grenze": limit_prop_named(STANDARD_TREFFER as i64, MAX_TREFFER as i64),
                }),
                &[],
            ),
            need,
        },
        Spec {
            name: "notizbuch_baum",
            title: "Notizbuch-Gliederung",
            description: "Freigegebene Notizbücher und Abschnitte mit Seitenzahl (z. B. für die Rückfrage, in welches Notizbuch etwas gehört). Nur lesend.",
            parameters: object(json!({}), &[]),
            need,
        },
    ]
}

/// Ist `name` eines der Notizbuch-Werkzeuge?
pub fn ist_werkzeug(name: &str) -> bool {
    name.starts_with("notizbuch_") && specs().iter().any(|s| s.name == name)
}

/// Führt ein Notizbuch-Werkzeug aus.
pub(super) fn run(
    source: &dyn KnowledgeSource,
    ctx: &Context<'_>,
    tool: &str,
    mut a: Args,
) -> Result<Value, String> {
    // Argumente zuerst prüfen: ein Fehler darin öffnet die Ablage gar nicht erst.
    let auftrag = match tool {
        "notizbuch_suchen" => {
            let anfrage = a.required("anfrage", 200)?;
            let grenze = grenze(&mut a)?;
            Auftrag::Suchen { anfrage, grenze }
        }
        "notizbuch_seite" => Auftrag::Seite {
            id: a.required("seite_id", 64)?,
        },
        "notizbuch_zeitraum" => {
            let von = match a.text("von", 10)? {
                Some(t) => parse_date("von", &t, ctx.now_ms)?,
                None => today(ctx.now_ms)?,
            };
            let bis = match a.text("bis", 10)? {
                Some(t) => parse_date("bis", &t, ctx.now_ms)?,
                None => von,
            };
            if bis < von {
                return Err("„bis“ liegt vor „von“.".to_string());
            }
            if (bis - von).num_days() > 366 {
                return Err("Der Zeitraum ist zu lang (höchstens ein Jahr).".to_string());
            }
            let (beginn, _) = day_bounds(von)?;
            let (_, ende) = day_bounds(bis)?;
            Auftrag::Zeitraum {
                von: iso(beginn)?,
                bis: iso(ende)?,
                grenze: grenze(&mut a)?,
            }
        }
        "notizbuch_baum" => Auftrag::Baum,
        _ => return Err("Dieses Notizbuch-Werkzeug gibt es nicht.".to_string()),
    };
    a.done()?;

    let sicht = source.notizbuch()?;
    if sicht.freigabe.ist_leer() {
        return Err(KEINE_FREIGABE.to_string());
    }
    let z = &sicht.zugang;
    let f = &sicht.freigabe;
    let fehler = |e: Fehler| fehlertext(&e);
    // Ergebnis und Stand stammen je Aufruf aus EINER Lesetransaktion (`*_mit_stand`, Codex lva#414 M2).
    let (mut out, stand) = match auftrag {
        Auftrag::Suchen { anfrage, grenze } => {
            let MitStand { wert: s, stand } =
                z.suchen_mit_stand(&anfrage, grenze, f).map_err(fehler)?;
            if !s.genutzt {
                return Err("Die Suche braucht mindestens ein Wort mit drei Zeichen.".to_string());
            }
            let treffer: Vec<Value> = s
                .treffer
                .iter()
                .map(|t| {
                    let mut v = kopf_json(&t.kopf);
                    v["auszug"] = json!(t.auszug);
                    v
                })
                .collect();
            (json!({ "treffer": treffer }), stand)
        }
        Auftrag::Seite { id } => {
            // Ungültige Id und unbekannte Seite: dieselbe Antwort.
            let (seite, stand) = match z.seite_mit_stand(&id, f) {
                Ok(MitStand {
                    wert: Some(s),
                    stand,
                }) => (s, stand),
                Ok(MitStand { wert: None, .. }) | Err(Fehler::Eingabe) => {
                    return Err(NICHT_GEFUNDEN.to_string())
                }
                Err(e) => return Err(fehler(e)),
            };
            let (bloecke, gekuerzt) = bloecke_json(&seite.bloecke);
            let mut v = kopf_json(&seite.kopf);
            v["bloecke"] = Value::Array(bloecke);
            v["gekuerzt"] = json!(gekuerzt || seite.unvollstaendig);
            (v, stand)
        }
        Auftrag::Zeitraum { von, bis, grenze } => {
            let MitStand {
                wert: koepfe,
                stand,
            } = z
                .zeitraum_mit_stand(&von, &bis, grenze, f)
                .map_err(fehler)?;
            (
                json!({ "seiten": koepfe.iter().map(kopf_json).collect::<Vec<_>>() }),
                stand,
            )
        }
        Auftrag::Baum => {
            let MitStand { wert: nbs, stand } = z.baum_mit_stand(f).map_err(fehler)?;
            let liste: Vec<Value> = nbs
                .iter()
                .map(|n| {
                    json!({
                        "id": n.id,
                        "name": n.name,
                        "abschnitte": n.abschnitte.iter().map(|a| json!({
                            "id": a.id,
                            "name": a.name,
                            "gruppe": a.gruppe,
                            "seiten": a.seiten,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect();
            (json!({ "notizbuecher": liste }), stand)
        }
    };
    out["stand"] = json!(stand);
    Ok(out)
}

enum Auftrag {
    Suchen {
        anfrage: String,
        grenze: usize,
    },
    Seite {
        id: String,
    },
    Zeitraum {
        von: String,
        bis: String,
        grenze: usize,
    },
    Baum,
}

/// `grenze` (1 bis [`MAX_TREFFER`], Standard [`STANDARD_TREFFER`]).
fn grenze(a: &mut Args) -> Result<usize, String> {
    let falsch = || format!("„grenze“ muss eine ganze Zahl von 1 bis {MAX_TREFFER} sein.");
    match a.map.remove("grenze") {
        None | Some(Value::Null) => Ok(STANDARD_TREFFER),
        Some(Value::Number(n)) => match n.as_i64() {
            Some(v) if v >= 1 => Ok((v as usize).min(MAX_TREFFER)),
            _ => Err(falsch()),
        },
        Some(_) => Err(falsch()),
    }
}

fn iso(ms: i64) -> Result<String, String> {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|d| d.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| "Das Datum ist ungültig.".to_string())
}

fn kopf_json(k: &Seitenkopf) -> Value {
    json!({
        "seite_id": k.id,
        "titel": k.titel,
        "pfad": k.pfad,
        "erstellt": k.erstellt,
        "geaendert": k.geaendert,
        "quelle": format!("{QUELLE_SEITE}{}", k.id),
    })
}

/// Blöcke bis zum Zeichenbudget; `true`, wenn gekürzt wurde. Leere Blöcke entfallen.
fn bloecke_json(bloecke: &[Block]) -> (Vec<Value>, bool) {
    let mut out = Vec::new();
    let mut verbraucht = 0usize;
    for b in bloecke.iter().filter(|b| !b.text.trim().is_empty()) {
        let v = serde_json::to_value(b).unwrap_or(Value::Null);
        let laenge = v.to_string().chars().count();
        if verbraucht + laenge > SEITE_BUDGET {
            return (out, true);
        }
        verbraucht += laenge;
        out.push(v);
    }
    (out, false)
}

#[cfg(test)]
mod tests;
