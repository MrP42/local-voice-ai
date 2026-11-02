//! Extraktion: HTML zu Haupttext und Titel, Auszuege mit SHA-256, Quellentyp-Heuristik.

use scraper::{Html, Node, Selector};
use sha2::{Digest, Sha256};
use url::Url;

use crate::typen::{Quelle, Quellentyp, Vertrauen};

/// Obergrenze eines Auszugs in Zeichen (Schema erlaubt 2000).
pub const AUSZUG_ZEICHEN: usize = 1500;
const TITEL_ZEICHEN: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seite {
    pub titel: Option<String>,
    /// Absaetze, getrennt durch eine Leerzeile.
    pub text: String,
}

impl Seite {
    /// Waehlt je Inhaltstyp: HTML wird extrahiert, `text/plain` unveraendert uebernommen.
    pub fn aus_inhalt(inhaltstyp: &str, body: &str) -> Seite {
        if inhaltstyp == "text/plain" {
            return Seite {
                titel: None,
                text: body.trim().to_string(),
            };
        }
        Seite::aus_html(body)
    }

    /// Parsen ist CPU-lastig: in async-Code `aus_html_async` nutzen (oder selbst `spawn_blocking`).
    pub async fn aus_html_async(html: String) -> Seite {
        tokio::task::spawn_blocking(move || Seite::aus_html(&html))
            .await
            .unwrap_or(Seite {
                titel: None,
                text: String::new(),
            })
    }

    /// Synchron und CPU-lastig; Aufrufer in async-Kontext fuehren das in `spawn_blocking` aus.
    pub fn aus_html(html: &str) -> Seite {
        let dok = Html::parse_document(html);
        let titel = Selector::parse("title")
            .ok()
            .and_then(|s| dok.select(&s).next())
            .map(|t| normalisiere(&t.text().collect::<String>()))
            .filter(|t| !t.is_empty());
        // Hauptbereich bevorzugen, sonst body.
        let wurzel = ["main", "article", "[role=main]", "body"]
            .iter()
            .filter_map(|s| Selector::parse(s).ok())
            .find_map(|s| dok.select(&s).next());
        let mut absaetze = Vec::new();
        if let Some(w) = wurzel {
            let mut aktuell = String::new();
            sammle(*w, &mut aktuell, &mut absaetze);
            abschliessen(&mut aktuell, &mut absaetze);
        }
        Seite {
            titel,
            text: absaetze.join("\n\n"),
        }
    }
}

const UEBERSPRINGEN: [&str; 12] = [
    "script", "style", "noscript", "nav", "footer", "header", "aside", "form", "svg", "template",
    "iframe", "head",
];
const BLOCK: [&str; 22] = [
    "p",
    "div",
    "section",
    "article",
    "main",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "li",
    "ul",
    "ol",
    "br",
    "tr",
    "table",
    "blockquote",
    "pre",
    "dd",
    "dt",
    "figure",
];

const MAX_TIEFE: usize = 256;

enum Schritt<'a> {
    Besuche(ego_tree::NodeRef<'a, Node>, usize),
    BlockEnde,
}

/// Iterativer Durchlauf mit explizitem Stack (keine Rekursion); tiefer als `MAX_TIEFE` verschachtelte
/// Teilbaeume werden ignoriert.
fn sammle(start: ego_tree::NodeRef<'_, Node>, aktuell: &mut String, absaetze: &mut Vec<String>) {
    let mut stapel = vec![Schritt::Besuche(start, 0)];
    while let Some(schritt) = stapel.pop() {
        let (knoten, tiefe) = match schritt {
            Schritt::BlockEnde => {
                abschliessen(aktuell, absaetze);
                continue;
            }
            Schritt::Besuche(k, t) => (k, t),
        };
        match knoten.value() {
            Node::Text(t) => aktuell.push_str(t),
            Node::Element(e) => {
                let name = e.name();
                if UEBERSPRINGEN.contains(&name) || tiefe >= MAX_TIEFE {
                    continue;
                }
                if BLOCK.contains(&name) {
                    abschliessen(aktuell, absaetze);
                    stapel.push(Schritt::BlockEnde);
                }
                let kinder: Vec<_> = knoten.children().collect();
                for k in kinder.into_iter().rev() {
                    stapel.push(Schritt::Besuche(k, tiefe + 1));
                }
            }
            _ => {
                let kinder: Vec<_> = knoten.children().collect();
                for k in kinder.into_iter().rev() {
                    stapel.push(Schritt::Besuche(k, tiefe + 1));
                }
            }
        }
    }
}

fn abschliessen(aktuell: &mut String, absaetze: &mut Vec<String>) {
    let n = normalisiere(aktuell);
    aktuell.clear();
    if !n.is_empty() {
        absaetze.push(n);
    }
}

fn normalisiere(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Teilt Text (Absaetze durch Leerzeile) in Auszuege bis `AUSZUG_ZEICHEN`. Zu lange Absaetze werden an
/// Wortgrenzen geteilt.
pub fn auszuege(text: &str) -> Vec<String> {
    let mut aus = Vec::new();
    let mut akt = String::new();
    let mut laenge = 0usize;
    for absatz in text.split("\n\n").map(str::trim).filter(|a| !a.is_empty()) {
        for stueck in teile_lang(absatz, AUSZUG_ZEICHEN) {
            let l = stueck.chars().count();
            let sep = if akt.is_empty() { 0 } else { 2 };
            if laenge + sep + l > AUSZUG_ZEICHEN && !akt.is_empty() {
                aus.push(std::mem::take(&mut akt));
                laenge = 0;
            }
            if !akt.is_empty() {
                akt.push_str("\n\n");
                laenge += 2;
            }
            akt.push_str(&stueck);
            laenge += l;
        }
    }
    if !akt.is_empty() {
        aus.push(akt);
    }
    aus
}

fn teile_lang(absatz: &str, max: usize) -> Vec<String> {
    if absatz.chars().count() <= max {
        return vec![absatz.to_string()];
    }
    let mut teile = Vec::new();
    let mut akt = String::new();
    let mut l = 0usize;
    for wort in absatz.split(' ') {
        let wl = wort.chars().count();
        // Worte laenger als max werden hart geschnitten
        if wl > max {
            if !akt.is_empty() {
                teile.push(std::mem::take(&mut akt));
                l = 0;
            }
            let zeichen: Vec<char> = wort.chars().collect();
            for c in zeichen.chunks(max) {
                teile.push(c.iter().collect());
            }
            continue;
        }
        if l + 1 + wl > max && !akt.is_empty() {
            teile.push(std::mem::take(&mut akt));
            l = 0;
        }
        if !akt.is_empty() {
            akt.push(' ');
            l += 1;
        }
        akt.push_str(wort);
        l += wl;
    }
    if !akt.is_empty() {
        teile.push(akt);
    }
    teile
}

/// SHA-256 ueber die UTF-8-Bytes des Auszugs, Kleinbuchstaben-Hex (`excerpt_sha256`).
pub fn excerpt_sha256(auszug: &str) -> String {
    let h = Sha256::digest(auszug.as_bytes());
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn domain_passt(host: &str, dom: &str) -> bool {
    host == dom || host.ends_with(&format!(".{dom}"))
}

const OFFIZIELL: [&str; 11] = [
    "gov",
    "gov.uk",
    "bund.de",
    "europa.eu",
    "gv.at",
    "admin.ch",
    "bundestag.de",
    "bundesregierung.de",
    "destatis.de",
    "bayern.de",
    "nrw.de",
];
const WISSENSCHAFT: [&str; 6] = [
    "doi.org",
    "arxiv.org",
    "edu",
    "ac.uk",
    "pubmed.ncbi.nlm.nih.gov",
    "researchgate.net",
];
const PRESSE: [&str; 9] = [
    "tagesschau.de",
    "spiegel.de",
    "zeit.de",
    "faz.net",
    "sueddeutsche.de",
    "reuters.com",
    "apnews.com",
    "bbc.com",
    "heise.de",
];
const FORUM: [&str; 4] = [
    "reddit.com",
    "stackexchange.com",
    "stackoverflow.com",
    "quora.com",
];

/// Heuristik nach Domain; bewusst klein gehalten, keine Aussage ueber Richtigkeit.
pub fn quellentyp(url: &Url) -> Quellentyp {
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return Quellentyp::Unknown;
    };
    let in_liste = |l: &[&str]| l.iter().any(|d| domain_passt(&host, d));
    if in_liste(&OFFIZIELL) {
        return Quellentyp::Official;
    }
    if in_liste(&WISSENSCHAFT) || host.starts_with("uni-") || host.contains(".uni-") {
        return Quellentyp::Scholarly;
    }
    if in_liste(&PRESSE) {
        return Quellentyp::Press;
    }
    if in_liste(&FORUM) || host.starts_with("forum.") || host.contains(".forum.") {
        return Quellentyp::Forum;
    }
    Quellentyp::Unknown
}

/// Baut eine Quelle fuer das Recherche-Dokument; `trust` ist immer `web`.
/// `retrieved_at` ist ein Zeitstempel im Format des Schemas (vom Aufrufer, diese Crate kennt keine Uhr).
pub fn quelle_bauen(
    index: u32,
    titel: Option<&str>,
    url: &str,
    auszug: &str,
    retrieved_at: &str,
) -> Option<Quelle> {
    let u = Url::parse(url).ok()?;
    if !matches!(u.scheme(), "http" | "https") || auszug.is_empty() {
        return None;
    }
    let titel: String = titel
        .map(normalisiere)
        .filter(|t| !t.is_empty())
        .or_else(|| u.host_str().map(str::to_string))?
        .chars()
        .take(TITEL_ZEICHEN)
        .collect();
    Some(Quelle {
        index,
        title: titel,
        url: url.to_string(),
        retrieved_at: retrieved_at.to_string(),
        excerpt: auszug.to_string(),
        // Vertrag (envelope Sha256): Praefix `sha256:` vor dem Hex.
        excerpt_sha256: format!("sha256:{}", excerpt_sha256(auszug)),
        source_type: quellentyp(&u),
        trust: Vertrauen::Web,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARTIKEL: &str = include_str!("../tests/fixtures/artikel.html");

    #[test]
    fn haupttext_ohne_navigation() {
        let s = Seite::aus_html(ARTIKEL);
        assert_eq!(
            s.titel.as_deref(),
            Some("Beispielartikel zur Kommunalen Waermeplanung")
        );
        assert!(s.text.contains("Die Waermeplanung ist Pflicht."));
        assert!(s.text.contains("Zweiter Absatz mit Link und Fettdruck."));
        for raus in [
            "Startseite",
            "Impressum",
            "var x",
            "color:red",
            "Werbung",
            "Kontaktformular",
        ] {
            assert!(!s.text.contains(raus), "{raus} darf nicht im Text stehen");
        }
        let absaetze: Vec<_> = s.text.split("\n\n").collect();
        assert!(
            absaetze.len() >= 4,
            "Absaetze bleiben erhalten: {absaetze:?}"
        );
        assert_eq!(absaetze[0], "Waermeplanung im Ueberblick");
    }

    #[test]
    fn tiefe_verschachtelung_ohne_ueberlauf() {
        let tief = format!(
            "<body>{}ende{}</body>",
            "<div>".repeat(5000),
            "</div>".repeat(5000)
        );
        let s = Seite::aus_html(&tief);
        assert!(
            !s.text.contains("ende"),
            "jenseits der Tiefengrenze ignoriert"
        );
        let flach = Seite::aus_html("<body><div><div><p>ok</p></div></div></body>");
        assert_eq!(flach.text, "ok");
    }

    #[tokio::test]
    async fn async_variante() {
        let s = Seite::aus_html_async("<p>hallo</p>".into()).await;
        assert_eq!(s.text, "hallo");
    }

    #[test]
    fn text_plain_bleibt() {
        let s = Seite::aus_inhalt("text/plain", "  hallo\n\nwelt  ");
        assert_eq!(s.text, "hallo\n\nwelt");
    }

    #[test]
    fn auszuege_grenzen() {
        let satz = "wort ".repeat(100); // 500 Zeichen
        let text = format!("{0}\n\n{0}\n\n{0}\n\n{0}", satz.trim());
        let a = auszuege(&text);
        assert!(a.len() >= 2);
        assert!(a.iter().all(|x| x.chars().count() <= AUSZUG_ZEICHEN));
        let lang = "x".repeat(4000);
        let a = auszuege(&lang);
        assert!(a.iter().all(|x| x.chars().count() <= AUSZUG_ZEICHEN));
        assert_eq!(a.concat().len(), 4000);
        assert!(auszuege("  \n\n ").is_empty());
    }

    #[test]
    fn hash_stabil_und_bekannt() {
        // SHA-256("abc")
        assert_eq!(
            excerpt_sha256("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            excerpt_sha256("Gr\u{fc}\u{df}e"),
            excerpt_sha256("Gr\u{fc}\u{df}e")
        );
        assert_ne!(excerpt_sha256("a"), excerpt_sha256("b"));
        assert_eq!(excerpt_sha256("x").len(), 64);
    }

    #[test]
    fn quellentyp_heuristik() {
        let t = |u: &str| quellentyp(&Url::parse(u).unwrap());
        assert_eq!(t("https://www.bmwk.bund.de/x"), Quellentyp::Official);
        assert_eq!(
            t("https://digital-strategy.ec.europa.eu/"),
            Quellentyp::Official
        );
        assert_eq!(t("https://www.nasa.gov/"), Quellentyp::Official);
        assert_eq!(t("https://doi.org/10.1/x"), Quellentyp::Scholarly);
        assert_eq!(t("https://arxiv.org/abs/1"), Quellentyp::Scholarly);
        assert_eq!(t("https://cs.stanford.edu/"), Quellentyp::Scholarly);
        assert_eq!(t("https://www.uni-bonn.de/"), Quellentyp::Scholarly);
        assert_eq!(t("https://www.tagesschau.de/a"), Quellentyp::Press);
        assert_eq!(t("https://www.reddit.com/r/x"), Quellentyp::Forum);
        assert_eq!(t("https://forum.example.org/t"), Quellentyp::Forum);
        assert_eq!(t("https://math.stackexchange.com/q"), Quellentyp::Forum);
        assert_eq!(t("https://example.com/"), Quellentyp::Unknown);
        // Teilstring-Faelle duerfen nicht treffen
        assert_eq!(t("https://notgov.example/"), Quellentyp::Unknown);
        assert_eq!(t("https://evilbund.de/"), Quellentyp::Unknown);
    }

    #[test]
    fn quelle_felder_und_serde() {
        let q = quelle_bauen(
            1,
            Some("  Titel  "),
            "https://www.nasa.gov/a",
            "Auszug",
            "2026-11-01T10:00:00Z",
        )
        .unwrap();
        assert_eq!(q.title, "Titel");
        assert_eq!(q.trust, Vertrauen::Web);
        assert_eq!(q.source_type, Quellentyp::Official);
        assert_eq!(
            q.excerpt_sha256,
            format!("sha256:{}", excerpt_sha256("Auszug"))
        );
        let j = serde_json::to_value(&q).unwrap();
        assert_eq!(j["source_type"], "official");
        assert_eq!(j["trust"], "web");
        assert_eq!(j["index"], 1);
        assert!(quelle_bauen(1, None, "ftp://x/", "a", "t").is_none());
        let ohne_titel = quelle_bauen(2, None, "https://example.com/p", "a", "t").unwrap();
        assert_eq!(ohne_titel.title, "example.com");
    }
}
