//! Einladungstext mit Marke (Konzept 3.3/3.4).
//!
//! Die App schreibt in den Termin einen eigenen Block, als HTML (fuer Outlook) und
//! als Text (Zwischenablage, Adapter C). Der Block beginnt mit der Kopfzeile
//! [`BLOCK_KOPF`] und endet mit der Zeile `lva-plan:<id>` (der Marke). Alles
//! ausserhalb des Blocks gehoert Outlook bzw. dem Menschen und wird nie angefasst.
//!
//! Warum eine sichtbare Textmarke statt eines HTML-Kommentars: Outlook entfernt beim
//! Bearbeiten Kommentare und Attribute, und der ICS-Feed liefert nur den Text
//! (`DESCRIPTION`). Die Zeile `lva-plan:<id>` uebersteht beides.
//!
//! Konflikterkennung: die Pruefsumme [`block_hash`] laeuft ueber den Text des Blocks,
//! Leerraum zusammengefasst. So erzeugt die HTML->Text-Wandlung von Outlook keinen
//! falschen Konflikt, eine echte Textaenderung im Block aber schon.

use serde::{Deserialize, Serialize};

use crate::model::{Plan, PlanInput};

/// Erste Zeile des App-Blocks.
pub const BLOCK_KOPF: &str = "— Besprechungsplanung —";
/// Praefix der Marke.
pub const MARKE_PRAEFIX: &str = "lva-plan:";

/// Erscheinungsbild der Einladung. Die App fuellt es aus dem gewaehlten Briefkopf
/// (Vorgabe `neutral`); das Crate kennt keine Marken oder Organisationen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Gestaltung {
    /// Name der einladenden Organisationseinheit, optional.
    pub absender: Option<String>,
    /// Akzentfarbe `#RRGGBB`; ungueltige Werte fallen auf [`AKZENT_NEUTRAL`] zurueck.
    pub akzentfarbe: String,
}

/// Neutrale Akzentfarbe, solange der Briefkopf keine eigene vorgibt.
pub const AKZENT_NEUTRAL: &str = "#0F6CBD";

impl Default for Gestaltung {
    fn default() -> Self {
        Gestaltung {
            absender: None,
            akzentfarbe: AKZENT_NEUTRAL.to_string(),
        }
    }
}

/// Der fertige Einladungstext.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "specta", derive(specta::Type))]
pub struct Einladung {
    pub html: String,
    pub text: String,
    /// Pruefsumme des Blocks (aus `text`), fuer `meeting_plan_links.body_hash`.
    pub block_hash: String,
}

/// Die Marke einer Planung.
pub fn marke(plan_id: &str) -> String {
    format!("{MARKE_PRAEFIX}{plan_id}")
}

/// Baut die Einladung einer gespeicherten Planung.
pub fn einladung(plan: &Plan, gestaltung: &Gestaltung) -> Einladung {
    let input = PlanInput {
        title: plan.title.clone(),
        goal_type: plan.goal_type,
        goal_text: plan.goal_text.clone(),
        expected_result: plan.expected_result.clone(),
        duration_min: plan.duration_min,
        classification: plan.classification.clone(),
        agenda: plan.agenda.clone(),
        roles: plan.roles.clone(),
        materials: plan.materials.clone(),
        hints: plan.hints.clone(),
        transcription_planned: plan.transcription_planned,
        minutes_template_id: plan.minutes_template_id.clone(),
    };
    einladung_aus(&plan.id, &input, gestaltung)
}

/// Baut die Einladung aus Eingabedaten (Vorschau vor dem Speichern).
pub fn einladung_aus(plan_id: &str, plan: &PlanInput, gestaltung: &Gestaltung) -> Einladung {
    let zeilen = zeilen(plan_id, plan, gestaltung);
    let text = zeilen
        .iter()
        .map(|z| z.als_text())
        .collect::<Vec<_>>()
        .join("\n");
    let html = als_html(&zeilen, gestaltung);
    let block_hash = block_hash(&text);
    Einladung {
        html,
        text,
        block_hash,
    }
}

/// Eine logische Zeile des Blocks; Text und HTML entstehen aus derselben Liste,
/// damit beide denselben Inhalt (und damit dieselbe Pruefsumme) haben.
enum Zeile {
    Kopf,
    Absender(String),
    Titel(String),
    Ueberschrift(String),
    Feld(String, String),
    Punkt(String),
    Link(String, String),
    Absatz(String),
    Leer,
    Marke(String),
}

impl Zeile {
    fn als_text(&self) -> String {
        match self {
            Zeile::Kopf => BLOCK_KOPF.to_string(),
            Zeile::Absender(t) | Zeile::Titel(t) => t.clone(),
            Zeile::Ueberschrift(t) => t.clone(),
            Zeile::Feld(k, v) => format!("{k}: {v}"),
            Zeile::Punkt(t) => format!("• {t}"),
            Zeile::Link(t, u) => format!("• {t}: {u}"),
            Zeile::Absatz(t) => t.clone(),
            Zeile::Leer => String::new(),
            Zeile::Marke(m) => m.clone(),
        }
    }
}

fn zeilen(plan_id: &str, plan: &PlanInput, g: &Gestaltung) -> Vec<Zeile> {
    let mut z = vec![Zeile::Kopf];
    if let Some(a) = nicht_leer(g.absender.as_deref()) {
        z.push(Zeile::Absender(einzeilig(a)));
    }
    z.push(Zeile::Titel(einzeilig(&plan.title)));
    z.push(Zeile::Feld(
        format!("Ziel ({})", plan.goal_type.label()),
        einzeilig(&plan.goal_text),
    ));
    if let Some(e) = nicht_leer(plan.expected_result.as_deref()) {
        z.push(Zeile::Feld("Erwartetes Ergebnis".into(), einzeilig(e)));
    }
    z.push(Zeile::Feld(
        "Dauer".into(),
        format!("{} Minuten", plan.duration_min),
    ));
    z.push(Zeile::Feld(
        "Klassifizierung".into(),
        einzeilig(&plan.classification),
    ));

    if !plan.agenda.is_empty() {
        z.push(Zeile::Leer);
        z.push(Zeile::Ueberschrift("Agenda".into()));
        for (i, a) in plan.agenda.iter().enumerate() {
            let wer = nicht_leer(a.owner.as_deref())
                .map(|o| format!(" – {}", einzeilig(o)))
                .unwrap_or_default();
            z.push(Zeile::Punkt(format!(
                "{}. {} ({} Min.){wer}",
                i + 1,
                einzeilig(&a.topic),
                a.minutes
            )));
        }
    }

    let rollen = [
        ("Moderation", plan.roles.moderation.as_deref()),
        ("Ergebnissicherung", plan.roles.ergebnissicherung.as_deref()),
        ("Entscheidung", plan.roles.entscheidung.as_deref()),
    ];
    if rollen.iter().any(|(_, v)| nicht_leer(*v).is_some()) {
        z.push(Zeile::Leer);
        z.push(Zeile::Ueberschrift("Rollen".into()));
        for (k, v) in rollen {
            if let Some(v) = nicht_leer(v) {
                z.push(Zeile::Feld(k.into(), einzeilig(v)));
            }
        }
    }

    if !plan.materials.is_empty() {
        z.push(Zeile::Leer);
        z.push(Zeile::Ueberschrift("Unterlagen".into()));
        for m in &plan.materials {
            let mut label = einzeilig(&m.label);
            if let Some(d) = nicht_leer(m.due_date.as_deref()) {
                label = format!("{label} (bitte lesen bis {})", einzeilig(d));
            }
            // Eine URL mit Markenpraefix wuerde eine fremde Marke einschleusen:
            // dann nur als Text (einzeilig entschaerft sie).
            match m
                .url
                .as_deref()
                .filter(|u| sicherer_link(u) && !u.contains(MARKE_PRAEFIX))
            {
                Some(u) => z.push(Zeile::Link(label, u.trim().to_string())),
                None => match m.url.as_deref().filter(|u| sicherer_link(u)) {
                    // Sicherer Link mit Markenpraefix: als entschaerfter Text.
                    Some(u) => z.push(Zeile::Punkt(format!("{label}: {}", einzeilig(u)))),
                    // Unsicherer Link (z. B. javascript:): weglassen.
                    None => z.push(Zeile::Punkt(label)),
                },
            }
        }
    }

    let h = &plan.hints;
    let mut hinweise = Vec::new();
    if let Some(w) = h.weiterleiten_erlaubt {
        hinweise.push(if w {
            "Weiterleiten an Betroffene ist erwünscht.".to_string()
        } else {
            "Bitte nicht weiterleiten.".to_string()
        });
    }
    if let Some(b) = nicht_leer(h.barrierefreiheit.as_deref()) {
        hinweise.push(format!("Barrierefreiheit: {}", einzeilig(b)));
    }
    if let Some(f) = nicht_leer(h.ergebnisfrist.as_deref()) {
        hinweise.push(format!("Ergebnis bis: {}", einzeilig(f)));
    }
    if let Some(a) = nicht_leer(h.ablageort.as_deref()) {
        hinweise.push(format!("Ablage: {}", einzeilig(a)));
    }
    if plan.transcription_planned {
        hinweise.push(
            "Geplant ist eine lokale Transkription als Grundlage für das Protokoll. \
             Sie startet nur, wenn alle Anwesenden zustimmen; ohne Zustimmung gibt es nur Notizen. \
             Audio wird gelöscht, sobald das Protokoll freigegeben ist."
                .to_string(),
        );
    }
    if !hinweise.is_empty() {
        z.push(Zeile::Leer);
        z.push(Zeile::Ueberschrift("Hinweise".into()));
        for t in hinweise {
            z.push(Zeile::Absatz(t));
        }
    }

    z.push(Zeile::Leer);
    z.push(Zeile::Marke(marke(plan_id)));
    z
}

fn als_html(zeilen: &[Zeile], g: &Gestaltung) -> String {
    let akzent = if gueltige_farbe(&g.akzentfarbe) {
        g.akzentfarbe.as_str()
    } else {
        AKZENT_NEUTRAL
    };
    let mut h = String::new();
    h.push_str(
        "<div style=\"font-family:'Segoe UI',Arial,sans-serif;font-size:11pt;color:#242424\">",
    );
    let mut in_liste = false;
    let schliesse = |h: &mut String, in_liste: &mut bool| {
        if *in_liste {
            h.push_str("</ul>");
            *in_liste = false;
        }
    };
    for z in zeilen {
        match z {
            Zeile::Kopf => {
                h.push_str(&format!(
                    "<p style=\"margin:0 0 8px 0;color:{akzent};font-weight:600\">{}</p>",
                    esc(BLOCK_KOPF)
                ));
            }
            Zeile::Absender(t) => h.push_str(&format!(
                "<p style=\"margin:0 0 8px 0;color:#616161\">{}</p>",
                esc(t)
            )),
            Zeile::Titel(t) => h.push_str(&format!(
                "<p style=\"margin:0 0 8px 0;font-size:14pt;font-weight:600\">{}</p>",
                esc(t)
            )),
            Zeile::Ueberschrift(t) => {
                schliesse(&mut h, &mut in_liste);
                h.push_str(&format!(
                    "<p style=\"margin:12px 0 4px 0;font-weight:600;color:{akzent}\">{}</p>",
                    esc(t)
                ));
            }
            Zeile::Feld(k, v) => {
                schliesse(&mut h, &mut in_liste);
                h.push_str(&format!(
                    "<p style=\"margin:0 0 4px 0\"><b>{}:</b> {}</p>",
                    esc(k),
                    esc(v)
                ));
            }
            Zeile::Punkt(t) | Zeile::Link(t, _) => {
                if !in_liste {
                    h.push_str("<ul style=\"margin:0 0 4px 0;padding-left:20px\">");
                    in_liste = true;
                }
                match z {
                    Zeile::Link(_, u) => h.push_str(&format!(
                        "<li>{}: <a href=\"{}\">{}</a></li>",
                        esc(t),
                        esc(u),
                        esc(u)
                    )),
                    _ => h.push_str(&format!("<li>{}</li>", esc(t))),
                }
            }
            Zeile::Absatz(t) => {
                schliesse(&mut h, &mut in_liste);
                h.push_str(&format!("<p style=\"margin:0 0 4px 0\">{}</p>", esc(t)));
            }
            Zeile::Leer => schliesse(&mut h, &mut in_liste),
            Zeile::Marke(m) => {
                schliesse(&mut h, &mut in_liste);
                h.push_str(&format!(
                    "<p style=\"margin:12px 0 0 0;font-size:8pt;color:#8a8a8a\">{}</p>",
                    esc(m)
                ));
            }
        }
    }
    h.push_str("</div>");
    h
}

/// Pruefsumme eines Blocktexts (FNV-1a 64 Bit, hex). Toleriert wird nur
/// Formatierung, wie sie Outlook bei der HTML->Text-Wandlung aendert: Zeilenenden,
/// Leerraum (zusammengefasst), Leerzeilen und ein Aufzaehlungszeichen am Zeilenanfang
/// (`•`, `*`, `·`, `-`, `o` gefolgt von Leerraum). Jedes andere Zeichen zaehlt, also
/// auch Ziffern, Satzzeichen, Gross-/Kleinschreibung und Zeichen in URLs.
/// Keine Kryptographie: nur Aenderungserkennung.
pub fn block_hash(block_text: &str) -> String {
    let norm = block_text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(|z| {
            let z = z.trim();
            let z = match z.split_once(char::is_whitespace) {
                Some(("•" | "*" | "·" | "-" | "o", rest)) => rest,
                _ => z,
            };
            z.split_whitespace().collect::<Vec<_>>().join(" ")
        })
        .filter(|z| !z.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in norm.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Findet die Plan-Id in einem Termintext (Text oder HTML). Gilt nur eine Marke,
/// die allein steht: davor Textanfang, Zeilenumbruch, Leerraum oder `>`, danach
/// Textende, Leerraum oder `<`. So zaehlt `https://x/lva-plan:abc` nicht. Die erste
/// solche Marke gilt.
pub fn finde_marke(beschreibung: &str) -> Option<String> {
    let mut ab = 0;
    while let Some(rel) = beschreibung[ab..].find(MARKE_PRAEFIX) {
        let pos = ab + rel;
        ab = pos + MARKE_PRAEFIX.len();
        let davor_ok = beschreibung[..pos]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == '>');
        let rest = &beschreibung[ab..];
        let id: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let danach_ok = rest[id.len()..]
            .chars()
            .next()
            .is_none_or(|c| c.is_whitespace() || c == '<');
        if davor_ok && danach_ok && !id.is_empty() && id.len() <= 64 {
            return Some(id);
        }
    }
    None
}

/// Schneidet den App-Block aus einem Termintext (Text). `None`, wenn Kopf oder
/// Marke fehlen; dann gilt der Block als entfernt.
pub fn finde_block(beschreibung: &str, plan_id: &str) -> Option<String> {
    let text = beschreibung.replace("\r\n", "\n");
    let start = text.find(BLOCK_KOPF)?;
    let m = marke(plan_id);
    let ende_rel = text[start..].find(&m)?;
    Some(text[start..start + ende_rel + m.len()].to_string())
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&#39;"),
            _ => o.push(c),
        }
    }
    o
}

/// Zeilenumbrueche im Inhalt wuerden die Blockstruktur brechen; eine Zeile daraus.
/// Ausserdem darf kein Inhalt eine fremde Marke einschleusen.
fn einzeilig(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(MARKE_PRAEFIX, "lva-plan\u{2060}:")
        .replace(BLOCK_KOPF, "Besprechungsplanung")
}

fn nicht_leer(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}

fn sicherer_link(u: &str) -> bool {
    let u = u.trim().to_ascii_lowercase();
    (u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:"))
        && !u
            .chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '<' || c == '>')
}

fn gueltige_farbe(f: &str) -> bool {
    f.len() == 7 && f.starts_with('#') && f[1..].chars().all(|c| c.is_ascii_hexdigit())
}
