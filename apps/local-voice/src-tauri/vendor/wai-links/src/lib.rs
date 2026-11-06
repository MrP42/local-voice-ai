//! Deep-Links der Suite (ADR-0018): `wai://<app>/<art>/<id>[/<id>][?plan=<id>&von=<app>:<art>:<id>]`.
//!
//! Ein Link traegt nur Kennungen, nie Titel oder Inhalt, und loest nie eine Aktion aus, nur
//! Navigation. Das Schema ist bewusst eng: feste Allow-List der Apps und Arten, ein enger
//! Zeichensatz fuer Kennungen, kein Prozent-Escape, keine Zusatzbestandteile. Alles andere wird
//! verworfen, nicht repariert. Die Pruefvektoren unter `contracts/apps/links/vektoren.json` gelten
//! gleichermassen fuer das TypeScript-Gegenstueck `@wai/links`.

use std::fmt;

/// Schema ohne `://`.
pub const SCHEMA: &str = "wai";
/// Hoechstlaenge eines ganzen Links in Zeichen.
pub const MAX_LINK: usize = 512;
/// Hoechstlaenge einer Kennung.
pub const MAX_ID: usize = 128;

/// Bekannte Apps, Arten und die Zahl ihrer Kennungen im Pfad.
pub const ARTEN: &[(&str, &str, usize)] = &[
    ("planner", "aufgabe", 2),
    ("planner", "plan", 1),
    ("poststelle", "mail", 1),
    ("poststelle", "fall", 1),
    ("notizbuch", "seite", 1),
    ("teamchat", "kanal", 1),
    ("teamchat", "nachricht", 2),
    ("termine", "termin", 1),
    ("formulare", "formular", 1),
    ("projekte", "projekt", 1),
    ("corporate-llm", "recherche", 1),
    ("lva", "besprechung", 1),
    ("lva", "buch", 1),
    ("presenter", "vortrag", 1),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkFehler {
    Leer,
    ZuLang,
    Schema,
    Zeichen,
    AppUnbekannt,
    ArtUnbekannt,
    IdAnzahl,
    IdUngueltig,
    Query,
}

impl LinkFehler {
    /// Stabiler Code, in den Pruefvektoren und fuer Protokolle (ohne Linkinhalt).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Leer => "link.leer",
            Self::ZuLang => "link.zu_lang",
            Self::Schema => "link.schema",
            Self::Zeichen => "link.zeichen",
            Self::AppUnbekannt => "link.app_unbekannt",
            Self::ArtUnbekannt => "link.art_unbekannt",
            Self::IdAnzahl => "link.id_anzahl",
            Self::IdUngueltig => "link.id_ungueltig",
            Self::Query => "link.query",
        }
    }
}

impl fmt::Display for LinkFehler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for LinkFehler {}

/// Ziel, aus dem man gekommen ist; Grundlage von „Zurueck zu <App>“.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Herkunft {
    pub app: String,
    pub art: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub app: String,
    pub art: String,
    pub ids: Vec<String>,
    pub plan: Option<String>,
    pub von: Option<Herkunft>,
}

fn id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_ID
        && s != "."
        && s != ".."
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn anzahl(app: &str, art: &str) -> Result<usize, LinkFehler> {
    if !ARTEN.iter().any(|(a, _, _)| *a == app) {
        return Err(LinkFehler::AppUnbekannt);
    }
    ARTEN
        .iter()
        .find(|(a, k, _)| *a == app && *k == art)
        .map(|(_, _, n)| *n)
        .ok_or(LinkFehler::ArtUnbekannt)
}

fn ziel_pruefen(app: &str, art: &str, ids: &[String]) -> Result<(), LinkFehler> {
    if anzahl(app, art)? != ids.len() {
        return Err(LinkFehler::IdAnzahl);
    }
    if !ids.iter().all(|i| id_ok(i)) {
        return Err(LinkFehler::IdUngueltig);
    }
    Ok(())
}

fn herkunft_lesen(wert: &str) -> Result<Herkunft, LinkFehler> {
    let mut teile = wert.split(':');
    let app = teile.next().unwrap_or_default();
    let art = teile.next().ok_or(LinkFehler::Query)?;
    let ids: Vec<String> = teile.map(str::to_string).collect();
    ziel_pruefen(app, art, &ids).map_err(|_| LinkFehler::Query)?;
    Ok(Herkunft {
        app: app.to_string(),
        art: art.to_string(),
        ids,
    })
}

impl Link {
    /// Baut einen geprueften Link; dieselben Regeln wie [`parse`].
    pub fn bauen(
        app: &str,
        art: &str,
        ids: &[&str],
        plan: Option<&str>,
        von: Option<Herkunft>,
    ) -> Result<Self, LinkFehler> {
        let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
        ziel_pruefen(app, art, &ids)?;
        if let Some(p) = plan
            && !id_ok(p)
        {
            return Err(LinkFehler::Query);
        }
        if let Some(v) = &von {
            ziel_pruefen(&v.app, &v.art, &v.ids).map_err(|_| LinkFehler::Query)?;
        }
        let link = Self {
            app: app.to_string(),
            art: art.to_string(),
            ids,
            plan: plan.map(str::to_string),
            von,
        };
        if link.to_string().len() > MAX_LINK {
            return Err(LinkFehler::ZuLang);
        }
        Ok(link)
    }
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEMA}://{}/{}", self.app, self.art)?;
        for i in &self.ids {
            write!(f, "/{i}")?;
        }
        let mut trenner = '?';
        if let Some(p) = &self.plan {
            write!(f, "{trenner}plan={p}")?;
            trenner = '&';
        }
        if let Some(v) = &self.von {
            write!(f, "{trenner}von={}:{}", v.app, v.art)?;
            for i in &v.ids {
                write!(f, ":{i}")?;
            }
        }
        Ok(())
    }
}

/// Liest einen Link streng; jede Abweichung ist ein Fehler.
pub fn parse(eingabe: &str) -> Result<Link, LinkFehler> {
    if eingabe.is_empty() {
        return Err(LinkFehler::Leer);
    }
    if eingabe.chars().count() > MAX_LINK {
        return Err(LinkFehler::ZuLang);
    }
    if !eingabe
        .bytes()
        .all(|b| b.is_ascii_graphic() && !matches!(b, b'%' | b'#' | b'@' | b'\\'))
    {
        return Err(LinkFehler::Zeichen);
    }
    let rest = eingabe
        .strip_prefix(SCHEMA)
        .and_then(|r| r.strip_prefix("://"))
        .ok_or(LinkFehler::Schema)?;
    let (pfad, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (rest, None),
    };
    let mut teile = pfad.split('/');
    let app = teile.next().unwrap_or_default();
    let art = teile.next().ok_or(LinkFehler::ArtUnbekannt)?;
    let ids: Vec<String> = teile.map(str::to_string).collect();
    ziel_pruefen(app, art, &ids)?;
    let mut plan = None;
    let mut von = None;
    if let Some(q) = query {
        for paar in q.split('&') {
            let (k, v) = paar.split_once('=').ok_or(LinkFehler::Query)?;
            match k {
                "plan" if plan.is_none() && id_ok(v) => plan = Some(v.to_string()),
                "von" if von.is_none() => von = Some(herkunft_lesen(v)?),
                _ => return Err(LinkFehler::Query),
            }
        }
    }
    Ok(Link {
        app: app.to_string(),
        art: art.to_string(),
        ids,
        plan,
        von,
    })
}
