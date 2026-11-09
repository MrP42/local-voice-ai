//! DOCX/ODT (`absatz=<n>` + Überschriftenpfad) und PPTX/ODP (`folie=<n>`) über `zip` + `quick-xml`.
//! XML mit DOCTYPE oder unbekannten Entitäten wird abgewiesen ([`super::XmlLeser`]).

use super::zipsicher::ZipLeser;
use super::{attr, Abschnitt, ExtraktFehler, XmlLeser, X};

fn fehlt() -> ExtraktFehler {
    ExtraktFehler::neu("format")
}

/// Sammelt Absätze und setzt den Überschriftenpfad.
#[derive(Default)]
struct Absaetze {
    n: u32,
    pfad: Vec<String>,
    aus: Vec<Abschnitt>,
}

impl Absaetze {
    fn absatz(&mut self, text: &str, ebene: Option<usize>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.n += 1;
        self.aus.push(Abschnitt {
            anker: format!("absatz={}", self.n),
            pfad: self.pfad.clone(),
            text: text.to_string(),
        });
        if let Some(e) = ebene.filter(|e| (1..=9).contains(e)) {
            self.pfad.truncate(e - 1);
            self.pfad.push(text.chars().take(200).collect());
        }
    }
}

/// Überschriftsebene aus der Formatvorlage (`Heading2`, deutsch `berschrift2`, `Title`).
fn ebene_aus_stil(stil: &str) -> Option<usize> {
    let s = stil.to_ascii_lowercase();
    if s == "title" || s == "titel" {
        return Some(1);
    }
    let rest = s
        .strip_prefix("heading")
        .or_else(|| s.strip_prefix("berschrift"))
        .or_else(|| s.strip_prefix("überschrift"))?;
    rest.trim().parse::<usize>().ok()
}

pub fn docx(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let mut z = ZipLeser::oeffnen(bytes)?;
    let xml = z.lesen("word/document.xml")?.ok_or_else(fehlt)?;
    let mut r = XmlLeser::neu(&xml);
    let mut a = Absaetze::default();
    // Stapel offener Absätze (Textfelder enthalten eigene Absätze): (Text, Ebene).
    let mut stapel: Vec<(String, Option<usize>)> = Vec::new();
    let mut im_text = false;
    loop {
        match r.naechstes()? {
            X::Start(n, _) if n == "w:p" => stapel.push((String::new(), None)),
            X::Ende(n) if n == "w:p" => {
                if let Some((t, e)) = stapel.pop() {
                    a.absatz(&t, e);
                }
            }
            X::Start(n, _) if n == "w:t" => im_text = true,
            X::Ende(n) if n == "w:t" => im_text = false,
            X::Leer(n, attrs) | X::Start(n, attrs) if n == "w:pStyle" => {
                if let (Some(top), Some(v)) = (stapel.last_mut(), attr(&attrs, "w:val")) {
                    top.1 = top.1.or(ebene_aus_stil(v));
                }
            }
            X::Leer(n, attrs) | X::Start(n, attrs) if n == "w:outlineLvl" => {
                let lvl = attr(&attrs, "w:val").and_then(|v| v.parse::<usize>().ok());
                if let (Some(top), Some(l)) = (stapel.last_mut(), lvl.filter(|l| *l < 9)) {
                    top.1 = top.1.or(Some(l + 1));
                }
            }
            X::Leer(n, _) if n == "w:tab" => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push('\t');
                }
            }
            X::Leer(n, _) if n == "w:br" || n == "w:cr" => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push('\n');
                }
            }
            X::Text(t) if im_text => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push_str(&t);
                }
            }
            X::Eof => break,
            _ => {}
        }
    }
    Ok(a.aus)
}

pub fn odt(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let mut z = ZipLeser::oeffnen(bytes)?;
    let xml = z.lesen("content.xml")?.ok_or_else(fehlt)?;
    let mut r = XmlLeser::neu(&xml);
    let mut a = Absaetze::default();
    let mut stapel: Vec<(String, Option<usize>)> = Vec::new();
    // Nachverfolgte Änderungen: gelöschter Text steht in `text:tracked-changes` / `text:deletion`
    // und gehört nicht zum Dokument (kein Weg in den Index).
    let mut geloescht = 0usize;
    loop {
        match r.naechstes()? {
            X::Start(n, _) if n == "text:tracked-changes" || n == "text:deletion" => {
                geloescht += 1;
            }
            X::Ende(n) if n == "text:tracked-changes" || n == "text:deletion" => {
                geloescht = geloescht.saturating_sub(1);
            }
            X::Eof => break,
            _ if geloescht > 0 => {}
            X::Start(n, attrs) if n == "text:h" => {
                let e = attr(&attrs, "text:outline-level")
                    .and_then(|v| v.parse().ok())
                    .or(Some(1));
                stapel.push((String::new(), e));
            }
            X::Start(n, _) if n == "text:p" => stapel.push((String::new(), None)),
            X::Ende(n) if n == "text:p" || n == "text:h" => {
                if let Some((t, e)) = stapel.pop() {
                    a.absatz(&t, e);
                }
            }
            X::Leer(n, attrs) if n == "text:s" => {
                let c = attr(&attrs, "text:c")
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .min(100);
                if let Some(top) = stapel.last_mut() {
                    top.0.push_str(&" ".repeat(c));
                }
            }
            X::Leer(n, _) if n == "text:tab" => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push('\t');
                }
            }
            X::Leer(n, _) if n == "text:line-break" => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push('\n');
                }
            }
            X::Text(t) => {
                if let Some(top) = stapel.last_mut() {
                    top.0.push_str(&t);
                }
            }
            _ => {}
        }
    }
    Ok(a.aus)
}

/// Text einer Folie (DrawingML): `a:t`, Absätze `a:p` als Zeilen.
fn folientext(xml: &[u8]) -> Result<String, ExtraktFehler> {
    let mut r = XmlLeser::neu(xml);
    let mut aus = String::new();
    let mut im_text = false;
    loop {
        match r.naechstes()? {
            X::Start(n, _) if n == "a:t" => im_text = true,
            X::Ende(n) if n == "a:t" => im_text = false,
            X::Ende(n) if n == "a:p" => aus.push('\n'),
            X::Leer(n, _) if n == "a:br" => aus.push('\n'),
            X::Text(t) if im_text => aus.push_str(&t),
            X::Eof => break,
            _ => {}
        }
    }
    Ok(aus)
}

/// Folienreihenfolge aus `presentation.xml` (`p:sldId` → Beziehung → Ziel); ersatzweise nach Nummer.
fn folien_reihenfolge(z: &mut ZipLeser<'_>) -> Result<Vec<String>, ExtraktFehler> {
    let mut nach_nummer: Vec<(u32, String)> = z
        .namen()
        .into_iter()
        .filter_map(|n| {
            let nr = n
                .strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?
                .parse::<u32>()
                .ok()?;
            Some((nr, n))
        })
        .collect();
    nach_nummer.sort();
    let ersatz: Vec<String> = nach_nummer.into_iter().map(|(_, n)| n).collect();
    let (Some(pres), Some(rels)) = (
        z.lesen("ppt/presentation.xml")?,
        z.lesen("ppt/_rels/presentation.xml.rels")?,
    ) else {
        return Ok(ersatz);
    };
    let mut ziele = std::collections::BTreeMap::new();
    let mut r = XmlLeser::neu(&rels);
    loop {
        match r.naechstes()? {
            X::Leer(n, a) | X::Start(n, a) if n == "Relationship" => {
                if let (Some(id), Some(t)) = (attr(&a, "Id"), attr(&a, "Target")) {
                    let t = t.trim_start_matches('/');
                    let pfad = if t.starts_with("ppt/") {
                        t.to_string()
                    } else {
                        format!("ppt/{t}")
                    };
                    ziele.insert(id.to_string(), pfad);
                }
            }
            X::Eof => break,
            _ => {}
        }
    }
    let mut folgen = Vec::new();
    let mut r = XmlLeser::neu(&pres);
    loop {
        match r.naechstes()? {
            X::Leer(n, a) | X::Start(n, a) if n == "p:sldId" => {
                if let Some(z) = attr(&a, "r:id").and_then(|id| ziele.get(id)) {
                    folgen.push(z.clone());
                }
            }
            X::Eof => break,
            _ => {}
        }
    }
    if folgen.is_empty() {
        Ok(ersatz)
    } else {
        Ok(folgen)
    }
}

pub fn pptx(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let mut z = ZipLeser::oeffnen(bytes)?;
    let folien = folien_reihenfolge(&mut z)?;
    if folien.is_empty() {
        return Err(fehlt());
    }
    let mut aus = Vec::new();
    for (i, name) in folien.iter().enumerate() {
        let Some(xml) = z.lesen(name)? else { continue };
        aus.push(Abschnitt {
            anker: format!("folie={}", i + 1),
            pfad: vec![],
            text: folientext(&xml)?,
        });
    }
    Ok(aus)
}

pub fn odp(bytes: &[u8]) -> Result<Vec<Abschnitt>, ExtraktFehler> {
    let mut z = ZipLeser::oeffnen(bytes)?;
    let xml = z.lesen("content.xml")?.ok_or_else(fehlt)?;
    let mut r = XmlLeser::neu(&xml);
    let mut aus = Vec::new();
    let mut folie: Option<String> = None;
    let mut n = 0u32;
    loop {
        match r.naechstes()? {
            X::Start(name, _) if name == "draw:page" => {
                n += 1;
                folie = Some(String::new());
            }
            X::Ende(name) if name == "draw:page" => {
                if let Some(t) = folie.take() {
                    aus.push(Abschnitt {
                        anker: format!("folie={n}"),
                        pfad: vec![],
                        text: t,
                    });
                }
            }
            X::Ende(name) if name == "text:p" || name == "text:h" => {
                if let Some(t) = folie.as_mut() {
                    t.push('\n');
                }
            }
            X::Leer(name, _) if name == "text:s" || name == "text:tab" => {
                if let Some(t) = folie.as_mut() {
                    t.push(' ');
                }
            }
            X::Leer(name, _) if name == "text:line-break" => {
                if let Some(t) = folie.as_mut() {
                    t.push('\n');
                }
            }
            X::Text(s) => {
                if let Some(t) = folie.as_mut() {
                    t.push_str(&s);
                }
            }
            X::Leer(name, _) if name == "draw:page" => n += 1,
            X::Eof => break,
            _ => {}
        }
    }
    Ok(aus)
}

#[cfg(test)]
mod tests {
    use super::ebene_aus_stil;

    #[test]
    fn ueberschriften_stile() {
        assert_eq!(ebene_aus_stil("Heading2"), Some(2));
        assert_eq!(ebene_aus_stil("berschrift1"), Some(1));
        assert_eq!(ebene_aus_stil("Title"), Some(1));
        assert_eq!(ebene_aus_stil("Standard"), None);
    }
}
