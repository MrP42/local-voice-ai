//! robots.txt nach RFC 9309 (vereinfacht): Gruppen mit `User-agent`, `Allow`, `Disallow`; Muster mit `*`
//! und Endanker `$`; Pfade werden prozent-normalisiert.
//! Hat die Datei eine Gruppe fuer die eigene Kennung, gilt nur diese, sonst die Gruppe `*`.
//! Laengste passende Regel gewinnt, bei Gleichstand gewinnt `Allow`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Robots {
    regeln: Vec<(bool, String)>, // (erlaubt, normalisiertes Muster)
}

/// Prozent-Normalisierung: unreservierte Zeichen decodieren, sonst `%XX` in Grossbuchstaben,
/// Bytes ausserhalb ASCII und Leerraum prozent-codieren. `*` und `$` bleiben unveraendert.
pub fn normiere_pfad(s: &str) -> String {
    let b = s.as_bytes();
    let mut aus = String::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let hex = |x: u8| (x as char).to_digit(16);
        if c == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            let v = (h * 16 + l) as u8;
            push_byte(&mut aus, v);
            i += 3;
            continue;
        }
        if c >= 0x80 || c <= b' ' || c == 0x7f {
            aus.push_str(&format!("%{c:02X}"));
        } else {
            aus.push(c as char);
        }
        i += 1;
    }
    aus
}

fn push_byte(aus: &mut String, v: u8) {
    if v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.' | b'_' | b'~') {
        aus.push(v as char);
    } else {
        aus.push_str(&format!("%{v:02X}"));
    }
}

fn passt(muster: &str, pfad: &str) -> bool {
    let (muster, anker) = match muster.strip_suffix('$') {
        Some(m) => (m, true),
        None => (muster, false),
    };
    let teile: Vec<&str> = muster.split('*').collect();
    let Some(erster) = teile.first() else {
        return false;
    };
    if !pfad.starts_with(erster) {
        return false;
    }
    if teile.len() == 1 {
        return !anker || pfad == *erster;
    }
    let mut pos = erster.len();
    let letzter_idx = teile.len() - 1;
    for (i, t) in teile.iter().enumerate().skip(1) {
        if i == letzter_idx && anker {
            return pfad.len() - t.len() >= pos && pfad.ends_with(t);
        }
        match pfad[pos..].find(t) {
            Some(f) => pos += f + t.len(),
            None => return false,
        }
    }
    true
}

impl Robots {
    pub fn alles_erlaubt() -> Self {
        Self::default()
    }

    /// `agent` ist die eigene Kennung in Kleinbuchstaben, z. B. `wai-recherche`.
    pub fn parse(text: &str, agent: &str) -> Self {
        let agent = agent.to_ascii_lowercase();
        let mut eigene: Vec<(bool, String)> = Vec::new();
        let mut sterne: Vec<(bool, String)> = Vec::new();
        let mut eigene_vorhanden = false;
        let mut agenten: Vec<String> = Vec::new();
        let mut regel_gesehen = false;

        for zeile in text.lines() {
            let zeile = zeile.split('#').next().unwrap_or("").trim();
            let Some((feld, wert)) = zeile.split_once(':') else {
                continue;
            };
            let feld = feld.trim().to_ascii_lowercase();
            let wert = wert.trim();
            match feld.as_str() {
                "user-agent" => {
                    if regel_gesehen {
                        agenten.clear();
                        regel_gesehen = false;
                    }
                    agenten.push(wert.to_ascii_lowercase());
                }
                "allow" | "disallow" => {
                    regel_gesehen = true;
                    let erlaubt = feld == "allow";
                    let ist_eigene = agenten
                        .iter()
                        .any(|a| !a.is_empty() && a != "*" && agent.contains(a.as_str()));
                    let ist_stern = agenten.iter().any(|a| a == "*");
                    if ist_eigene {
                        eigene_vorhanden = true;
                    }
                    if wert.is_empty() {
                        continue; // leeres Disallow = alles erlaubt; leeres Allow ohne Wirkung
                    }
                    let muster = normiere_pfad(wert);
                    if ist_eigene {
                        eigene.push((erlaubt, muster));
                    } else if ist_stern {
                        sterne.push((erlaubt, muster));
                    }
                }
                _ => {}
            }
        }
        Self {
            regeln: if eigene_vorhanden { eigene } else { sterne },
        }
    }

    /// `pfad` ist Pfad plus optionale Abfrage, beginnend mit `/`.
    pub fn erlaubt(&self, pfad: &str) -> bool {
        let pfad = normiere_pfad(pfad);
        let mut beste: Option<(usize, bool)> = None;
        for (erlaubt, muster) in &self.regeln {
            if !passt(muster, &pfad) {
                continue;
            }
            let laenge = muster.len();
            beste = match beste {
                None => Some((laenge, *erlaubt)),
                Some((l, _)) if laenge > l => Some((laenge, *erlaubt)),
                Some((l, e)) if laenge == l => Some((l, e || *erlaubt)),
                other => other,
            };
        }
        beste.map(|(_, e)| e).unwrap_or(true)
    }
}

/// Speicher-Cache je Ursprung mit Hoechstgroesse und Lebensdauer.
#[derive(Debug)]
pub struct RobotsCache {
    max: usize,
    ttl: Duration,
    eintraege: HashMap<String, (Robots, Instant)>,
}

impl RobotsCache {
    pub fn neu(max: usize, ttl: Duration) -> Self {
        Self {
            max,
            ttl,
            eintraege: HashMap::new(),
        }
    }

    pub fn holen(&mut self, schluessel: &str) -> Option<Robots> {
        match self.eintraege.get(schluessel) {
            Some((r, t)) if t.elapsed() < self.ttl => Some(r.clone()),
            Some(_) => {
                self.eintraege.remove(schluessel);
                None
            }
            None => None,
        }
    }

    pub fn setzen(&mut self, schluessel: String, r: Robots) {
        if !self.eintraege.contains_key(&schluessel) && self.eintraege.len() >= self.max {
            let ttl = self.ttl;
            self.eintraege.retain(|_, (_, t)| t.elapsed() < ttl);
            if self.eintraege.len() >= self.max
                && let Some(aeltester) = self
                    .eintraege
                    .iter()
                    .min_by_key(|(_, (_, t))| *t)
                    .map(|(k, _)| k.clone())
            {
                self.eintraege.remove(&aeltester);
            }
        }
        self.eintraege.insert(schluessel, (r, Instant::now()));
    }

    pub fn laenge(&self) -> usize {
        self.eintraege.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AGENT: &str = "wai-recherche";

    #[test]
    fn stern_gruppe() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /privat\nAllow: /privat/offen\n",
            AGENT,
        );
        assert!(!r.erlaubt("/privat/x"));
        assert!(r.erlaubt("/privat/offen/seite"));
        assert!(r.erlaubt("/oeffentlich"));
    }

    #[test]
    fn eigene_gruppe_hat_vorrang() {
        let t = "User-agent: *\nDisallow: /\n\nUser-agent: WAI-Recherche\nDisallow: /geheim\n";
        let r = Robots::parse(t, AGENT);
        assert!(r.erlaubt("/artikel"));
        assert!(!r.erlaubt("/geheim/1"));
    }

    #[test]
    fn disallow_alles_und_leer() {
        assert!(!Robots::parse("User-agent: *\nDisallow: /\n", AGENT).erlaubt("/a"));
        assert!(Robots::parse("User-agent: *\nDisallow:\n", AGENT).erlaubt("/a"));
        assert!(Robots::parse("", AGENT).erlaubt("/a"));
    }

    #[test]
    fn kommentare_und_mehrere_agenten() {
        let t =
            "# Kommentar\nUser-agent: a\nUser-agent: wai-recherche # wir\nDisallow: /x # nein\n";
        let r = Robots::parse(t, AGENT);
        assert!(!r.erlaubt("/x"));
        assert!(r.erlaubt("/y"));
    }

    #[test]
    fn gleichstand_allow_gewinnt() {
        let r = Robots::parse("User-agent: *\nDisallow: /a\nAllow: /a\n", AGENT);
        assert!(r.erlaubt("/a/b"));
    }

    #[test]
    fn wildcard_und_anker() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /*.pdf$\nDisallow: /a/*/geheim\nAllow: /a/x/geheim/offen\n",
            AGENT,
        );
        assert!(!r.erlaubt("/doc/x.pdf"));
        assert!(r.erlaubt("/doc/x.pdf?v=1"));
        assert!(r.erlaubt("/doc/x.pdfx"));
        assert!(!r.erlaubt("/a/b/geheim/z"));
        assert!(r.erlaubt("/a/x/geheim/offen"));
        let r = Robots::parse("User-agent: *\nDisallow: /exakt$\n", AGENT);
        assert!(!r.erlaubt("/exakt"));
        assert!(r.erlaubt("/exakt/mehr"));
    }

    #[test]
    fn prozent_normalisierung() {
        let r = Robots::parse(
            "User-agent: *\nDisallow: /priv%61t\nDisallow: /%7euser\nDisallow: /a%2fb\n",
            AGENT,
        );
        assert!(!r.erlaubt("/privat/x"));
        assert!(!r.erlaubt("/~user/x"));
        assert!(!r.erlaubt("/a%2Fb"));
        assert!(r.erlaubt("/a/b"));
        assert_eq!(normiere_pfad("/\u{e4}"), "/%C3%A4");
    }

    #[test]
    fn cache_grenze_und_ttl() {
        let mut c = RobotsCache::neu(2, Duration::from_secs(3600));
        c.setzen("a".into(), Robots::alles_erlaubt());
        c.setzen("b".into(), Robots::alles_erlaubt());
        c.setzen("c".into(), Robots::alles_erlaubt());
        assert_eq!(c.laenge(), 2);
        assert!(c.holen("c").is_some());
        let mut k = RobotsCache::neu(5, Duration::from_millis(0));
        k.setzen("a".into(), Robots::alles_erlaubt());
        assert!(k.holen("a").is_none());
    }
}
