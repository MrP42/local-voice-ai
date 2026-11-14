use std::collections::BTreeSet;

use super::adressen::{parametername, pfadwert_ok, pruefe_adresse};
use super::kodierung::{kanonisch, lesbar, Wert, MAX_GANZ};
use super::objekt;
use super::werkzeug::werkzeugname;

pub const ARTEN: &[&str] = &["app", "http", "web", "mcp"];
const NETZE: &[&str] = &["oeffentlich", "privat", "geraet"];
const SPERRLISTE: &[&str] = &[
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "accept",
    "authorization",
    "proxy-authorization",
    "cookie",
];

pub fn id_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && !s.as_bytes()[0].is_ascii_uppercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
fn kennung_ok(s: &str) -> bool {
    s.len() <= 32 && !s.contains('.') && id_ok(s)
}
fn kopf_ok(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.as_bytes()[0].is_ascii_alphabetic()
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn faehigkeit_ok(s: &str) -> bool {
    let teile: Vec<_> = s.split('.').collect();
    (2..=4).contains(&teile.len())
        && teile.iter().all(|s| {
            !s.is_empty()
                && s.as_bytes()[0].is_ascii_lowercase()
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}
pub fn ist_steuer(c: char) -> bool {
    matches!(c as u32, 0..=31 | 127..=159 | 0x2028 | 0x2029)
}
fn join(p: &str, k: &str) -> String {
    if p.is_empty() {
        k.to_owned()
    } else {
        format!("{p}.{k}")
    }
}

#[derive(Default)]
struct Pruefer {
    fehler: BTreeSet<(String, String)>,
}
#[derive(Default)]
struct Text<'a> {
    pflicht: bool,
    standard: Option<&'a str>,
    min: usize,
    max: Option<usize>,
    muster: Option<fn(&str) -> bool>,
    steuer: bool,
    trim: bool,
    auswahl: Option<&'a [&'a str]>,
}
impl<'a> Text<'a> {
    fn pflicht() -> Self {
        Self {
            pflicht: true,
            ..Self::default()
        }
    }
    fn standard(s: &'a str) -> Self {
        Self {
            standard: Some(s),
            ..Self::default()
        }
    }
    fn muster(mut self, f: fn(&str) -> bool) -> Self {
        self.muster = Some(f);
        self
    }
    fn laenge(mut self, min: usize, max: usize) -> Self {
        self.min = min;
        self.max = Some(max);
        self
    }
    fn auswahl(mut self, a: &'a [&'a str]) -> Self {
        self.auswahl = Some(a);
        self
    }
}
impl Pruefer {
    fn err(&mut self, p: impl Into<String>, c: &str) {
        self.fehler.insert((p.into(), c.into()));
    }
    fn extras(&mut self, o: &Wert, bekannt: &[&str], p: &str, e: usize) -> Wert {
        let mut out = objekt(Vec::<(String, Wert)>::new());
        for (k, v) in o.map().expect("Objekt geprueft") {
            if bekannt.contains(&k.as_str()) {
                continue;
            }
            if lesbar(v, e) {
                out.setzen(k, v.clone());
            } else {
                self.err(join(p, k), "json");
            }
        }
        out
    }
    fn text(&mut self, o: &Wert, k: &str, p: &str, r: Text<'_>) -> (Wert, bool) {
        let p = join(p, k);
        let Some(w) = o.get(k) else {
            if r.pflicht {
                self.err(p, "fehlt");
                return (Wert::Null, false);
            }
            return (r.standard.map(Wert::from).unwrap_or(Wert::Null), true);
        };
        let Some(t) = w.text() else {
            self.err(p, "typ");
            return (Wert::Null, false);
        };
        let t = if r.trim { t.trim_matches(' ') } else { t };
        let mut ok = true;
        let len = t.chars().count();
        if len < r.min || r.max.is_some_and(|max| len > max) {
            self.err(&p, "laenge");
            ok = false;
        }
        if r.steuer && t.chars().any(ist_steuer) || r.muster.is_some_and(|f| !f(t)) {
            self.err(&p, "muster");
            ok = false;
        }
        if r.auswahl.is_some_and(|a| !a.contains(&t)) {
            self.err(&p, "wert");
            ok = false;
        }
        (t.into(), ok)
    }
    fn ganz(
        &mut self,
        o: &Wert,
        k: &str,
        p: &str,
        standard: Option<i64>,
        grenzen: (i64, i64),
    ) -> (Wert, bool) {
        let Some(w) = o.get(k) else {
            return (standard.map(Wert::from).unwrap_or(Wert::Null), true);
        };
        let Some(n) = w.ganz() else {
            // Ein syntaktisch ganzzahliges Token ausserhalb i64 ist ein Bereichsfehler.
            let ganz_token = matches!(w, Wert::Zahl(s) if !s.contains(['.', 'e', 'E']));
            self.err(join(p, k), if ganz_token { "wert" } else { "typ" });
            return (w.clone(), false);
        };
        if !(grenzen.0..=grenzen.1).contains(&n) {
            self.err(join(p, k), "wert");
            return (w.clone(), false);
        }
        (Wert::from(n), true)
    }
    fn bool(&mut self, o: &Wert, k: &str, p: &str) -> (Wert, bool) {
        match o.get(k) {
            None => (Wert::Bool(false), true),
            Some(Wert::Bool(b)) => (Wert::Bool(*b), true),
            Some(_) => {
                self.err(join(p, k), "typ");
                (Wert::Null, false)
            }
        }
    }
    fn verboten(&mut self, o: &Wert, p: &str, namen: &[&str]) {
        for k in namen {
            if o.hat(k) {
                self.err(join(p, k), "widerspruch");
            }
        }
    }
    fn parameter(&mut self, o: &Wert, p: &str, e: usize) -> Option<Wert> {
        o.map().or_else(|| {
            self.err(p, "typ");
            None
        })?;
        let mut out = self.extras(
            o,
            &[
                "name",
                "beschreibung",
                "typ",
                "ort",
                "pflicht",
                "max_zeichen",
                "min",
                "max",
                "optionen",
            ],
            p,
            e,
        );
        let (name, _) = self.text(o, "name", p, Text::pflicht().muster(parametername));
        let (beschr, _) = self.text(o, "beschreibung", p, Text::standard("").laenge(0, 200));
        let (typ, typ_ok) = self.text(
            o,
            "typ",
            p,
            Text::pflicht().auswahl(&["text", "ganzzahl", "wahrheitswert", "auswahl"]),
        );
        let (ort, ort_ok) = self.text(
            o,
            "ort",
            p,
            Text::pflicht().auswahl(&["pfad", "abfrage", "koerper"]),
        );
        let (pflicht, pflicht_ok) = self.bool(o, "pflicht", p);
        if ort_ok && ort.text() == Some("pfad") && pflicht_ok && pflicht.bool() != Some(true) {
            self.err(join(p, "pflicht"), "wert");
        }
        for (k, v) in [
            ("name", name),
            ("beschreibung", beschr),
            ("typ", typ.clone()),
            ("ort", ort.clone()),
            ("pflicht", pflicht),
        ] {
            out.setzen(k, v);
        }
        if typ_ok {
            match typ.s() {
                "text" => {
                    let (v, _) = self.ganz(o, "max_zeichen", p, Some(500), (1, 4000));
                    out.setzen("max_zeichen", v);
                    self.verboten(o, p, &["min", "max", "optionen"]);
                }
                "ganzzahl" => {
                    self.verboten(o, p, &["max_zeichen", "optionen"]);
                    let (mn, mn_ok) = self.ganz(o, "min", p, None, (-MAX_GANZ, MAX_GANZ));
                    let (mx, mx_ok) = self.ganz(o, "max", p, None, (-MAX_GANZ, MAX_GANZ));
                    if o.hat("min") {
                        out.setzen("min", mn.clone());
                    }
                    if o.hat("max") {
                        out.setzen("max", mx.clone());
                    }
                    if mn_ok && mx_ok && mn.ganz().zip(mx.ganz()).is_some_and(|(a, b)| a > b) {
                        self.err(join(p, "max"), "widerspruch");
                    }
                }
                "wahrheitswert" => self.verboten(o, p, &["max_zeichen", "min", "max", "optionen"]),
                "auswahl" => {
                    self.verboten(o, p, &["max_zeichen", "min", "max"]);
                    let po = join(p, "optionen");
                    match o.get("optionen") {
                        None => self.err(po, "fehlt"),
                        Some(Wert::Liste(opts)) => {
                            if opts.is_empty() {
                                self.err(&po, "laenge");
                            }
                            if opts.len() > 20 {
                                self.err(&po, "zu_viele");
                            }
                            let mut gesehen = BTreeSet::new();
                            for (i, w) in opts.iter().enumerate() {
                                let pi = format!("{po}[{i}]");
                                let Some(s) = w.text() else {
                                    self.err(pi, "typ");
                                    continue;
                                };
                                if s.is_empty() || s.chars().count() > 60 {
                                    self.err(&pi, "laenge");
                                }
                                if s.chars().any(ist_steuer) {
                                    self.err(&pi, "muster");
                                }
                                if ort_ok
                                    && ort.text() == Some("pfad")
                                    && !s.is_empty()
                                    && !pfadwert_ok(s)
                                {
                                    self.err(&pi, "wert");
                                }
                                if !gesehen.insert(s) {
                                    self.err(&pi, "doppelt");
                                }
                            }
                            out.setzen("optionen", Wert::Liste(opts.clone()));
                        }
                        Some(_) => self.err(po, "typ"),
                    }
                }
                _ => unreachable!(),
            }
        }
        Some(out)
    }
    fn netz(&mut self, o: &Wert, p: &str) -> (Wert, bool) {
        self.text(o, "netz", p, Text::standard("oeffentlich").auswahl(NETZE))
    }
    fn antwort_max(&mut self, o: &Wert, p: &str) -> Wert {
        self.ganz(o, "antwort_max_bytes", p, Some(20480), (1, 262144))
            .0
    }
    fn http(&mut self, o: &Wert, p: &str, e: usize) -> (Wert, Option<String>, Option<String>) {
        if o.map().is_none() {
            self.err(p, "typ");
            return (Wert::Null, None, None);
        }
        let mut out = self.extras(
            o,
            &[
                "methode",
                "adresse",
                "netz",
                "parameter",
                "kopfzeilen",
                "geheimnis",
                "antwort_max_bytes",
            ],
            p,
            e,
        );
        let (m, m_ok) = self.text(
            o,
            "methode",
            p,
            Text::pflicht().auswahl(&["GET", "POST", "PUT", "PATCH", "DELETE"]),
        );
        let (n, n_ok) = self.netz(o, p);
        out.setzen("methode", m.clone());
        out.setzen("netz", n.clone());
        out.setzen("antwort_max_bytes", self.antwort_max(o, p));
        let (a, a_ok) = self.text(o, "adresse", p, Text::pflicht().laenge(1, 500));
        out.setzen("adresse", a.clone());
        let adresse_ok = a_ok && n_ok && pruefe_adresse(a.s(), "http", n.s());
        if a_ok && n_ok && !adresse_ok {
            self.err(join(p, "adresse"), "adresse");
        }
        let pp = join(p, "parameter");
        let mut params = Vec::new();
        match o.get("parameter") {
            None => {}
            Some(Wert::Liste(l)) => {
                if l.len() > 16 {
                    self.err(&pp, "zu_viele");
                }
                let mut gesehen = BTreeSet::new();
                for (i, w) in l.iter().enumerate() {
                    let pi = format!("{pp}[{i}]");
                    if let Some(q) = self.parameter(w, &pi, e + 2) {
                        if let Some(name) = q["name"].text().filter(|s| parametername(s)) {
                            if !gesehen.insert(name.to_owned()) {
                                self.err(join(&pi, "name"), "doppelt");
                            }
                        }
                        if m_ok
                            && ["GET", "DELETE"].contains(&m.s())
                            && q["ort"].text() == Some("koerper")
                        {
                            self.err(join(&pi, "ort"), "widerspruch");
                        }
                        params.push(q);
                    }
                }
            }
            Some(_) => self.err(&pp, "typ"),
        }
        if adresse_ok {
            let rest = a.s().split_once("://").expect("Adresse geprueft").1;
            let pfad = &rest[rest.find('/').expect("Pfad geprueft")..];
            let platzhalter: BTreeSet<_> = pfad
                .split('/')
                .filter(|s| s.starts_with('{'))
                .map(|s| &s[1..s.len() - 1])
                .collect();
            let namen: BTreeSet<_> = params
                .iter()
                .filter(|q| q["ort"].text() == Some("pfad"))
                .filter_map(|q| q["name"].text().filter(|s| parametername(s)))
                .collect();
            if platzhalter != namen {
                self.err(join(p, "adresse"), "widerspruch");
            }
        }
        out.setzen("parameter", Wert::Liste(params));
        let pk = join(p, "kopfzeilen");
        let mut feste = BTreeSet::new();
        let mut norm = Vec::new();
        match o.get("kopfzeilen") {
            None => {}
            Some(Wert::Liste(l)) => {
                if l.len() > 8 {
                    self.err(&pk, "zu_viele");
                }
                for (i, k) in l.iter().enumerate() {
                    let pi = format!("{pk}[{i}]");
                    if k.map().is_none() {
                        self.err(pi, "typ");
                        continue;
                    }
                    let mut ko = self.extras(k, &["name", "wert"], &pi, e + 2);
                    let (name, ok) = self.text(k, "name", &pi, Text::pflicht().muster(kopf_ok));
                    if ok {
                        let lower = name.s().to_ascii_lowercase();
                        if SPERRLISTE.contains(&lower.as_str()) {
                            self.err(join(&pi, "name"), "wert");
                        }
                        if !feste.insert(lower) {
                            self.err(join(&pi, "name"), "doppelt");
                        }
                    }
                    let (wert, _) = self.text(
                        k,
                        "wert",
                        &pi,
                        Text::pflicht().laenge(1, 200).muster(ascii_text),
                    );
                    ko.setzen("name", name);
                    ko.setzen("wert", wert);
                    norm.push(ko);
                }
            }
            Some(_) => self.err(&pk, "typ"),
        }
        out.setzen("kopfzeilen", Wert::Liste(norm));
        let pg = join(p, "geheimnis");
        let g = match o.get("geheimnis") {
            None | Some(Wert::Null) => Wert::Null,
            Some(g) if g.map().is_some() => {
                let mut go = self.extras(g, &["kopf", "praefix", "schluessel"], &pg, e + 1);
                let (kopf, ok) = self.text(g, "kopf", &pg, Text::pflicht().muster(kopf_ok));
                if ok {
                    let lower = kopf.s().to_ascii_lowercase();
                    if lower != "authorization" && SPERRLISTE.contains(&lower.as_str()) {
                        self.err(join(&pg, "kopf"), "wert");
                    }
                    if feste.contains(&lower) {
                        self.err(join(&pg, "kopf"), "doppelt");
                    }
                }
                let (pfx, _) = self.text(
                    g,
                    "praefix",
                    &pg,
                    Text::standard("").laenge(0, 32).muster(ascii_text),
                );
                let (sl, _) = self.text(g, "schluessel", &pg, Text::pflicht().muster(id_ok));
                go.setzen("kopf", kopf);
                go.setzen("praefix", pfx);
                go.setzen("schluessel", sl);
                go
            }
            Some(_) => {
                self.err(pg, "typ");
                Wert::Null
            }
        };
        out.setzen("geheimnis", g);
        (
            out,
            if m_ok { Some(m.s().into()) } else { None },
            if n_ok { Some(n.s().into()) } else { None },
        )
    }
    fn web(&mut self, o: &Wert, p: &str, e: usize) -> (Wert, Option<String>) {
        if o.map().is_none() {
            self.err(p, "typ");
            return (Wert::Null, None);
        }
        let mut out = self.extras(o, &["seiten", "netz", "antwort_max_bytes"], p, e);
        let (n, n_ok) = self.netz(o, p);
        out.setzen("netz", n.clone());
        out.setzen("antwort_max_bytes", self.antwort_max(o, p));
        let ps = join(p, "seiten");
        let mut norm = Vec::new();
        match o.get("seiten") {
            None => self.err(&ps, "fehlt"),
            Some(Wert::Liste(l)) => {
                if l.is_empty() {
                    self.err(&ps, "laenge");
                }
                if l.len() > 16 {
                    self.err(&ps, "zu_viele");
                }
                let mut kennungen = BTreeSet::new();
                let mut adressen = BTreeSet::new();
                for (i, s) in l.iter().enumerate() {
                    let pi = format!("{ps}[{i}]");
                    if s.map().is_none() {
                        self.err(pi, "typ");
                        continue;
                    }
                    let mut so = self.extras(s, &["kennung", "adresse"], &pi, e + 2);
                    let (k, ok) = self.text(s, "kennung", &pi, Text::pflicht().muster(kennung_ok));
                    if ok && !kennungen.insert(k.s().to_owned()) {
                        self.err(join(&pi, "kennung"), "doppelt");
                    }
                    let (a, a_ok) = self.text(s, "adresse", &pi, Text::pflicht().laenge(1, 500));
                    if a_ok && n_ok && !pruefe_adresse(a.s(), "web", n.s()) {
                        self.err(join(&pi, "adresse"), "adresse");
                    }
                    if let Some(a) = a.text() {
                        if !adressen.insert(a.to_owned()) {
                            self.err(join(&pi, "adresse"), "doppelt");
                        }
                    }
                    so.setzen("kennung", k);
                    so.setzen("adresse", a);
                    norm.push(so);
                }
            }
            Some(_) => self.err(&ps, "typ"),
        }
        out.setzen("seiten", Wert::Liste(norm));
        (out, if n_ok { Some(n.s().into()) } else { None })
    }
    fn eintrag(&mut self, o: &Wert, p: &str) -> Option<Wert> {
        if o.map().is_none() {
            self.err(p, "typ");
            return None;
        }
        let mut out = self.extras(
            o,
            &[
                "id",
                "name",
                "beschreibung",
                "art",
                "zugriff",
                "abfluss",
                "zeitlimit_ms",
                "http",
                "web",
                "app",
                "mcp",
            ],
            p,
            3,
        );
        let (id, id_gueltig) = self.text(o, "id", p, Text::pflicht().muster(id_ok));
        if id_gueltig && werkzeugname(id.s()).len() > 64 {
            self.err(join(p, "id"), "laenge");
        }
        let (name, _) = self.text(
            o,
            "name",
            p,
            Text {
                steuer: true,
                trim: true,
                ..Text::pflicht().laenge(1, 60)
            },
        );
        let (beschr, _) = self.text(o, "beschreibung", p, Text::standard("").laenge(0, 300));
        let (art, art_ok) = self.text(o, "art", p, Text::pflicht().auswahl(ARTEN));
        let (z, z_ok) = self.text(
            o,
            "zugriff",
            p,
            Text::pflicht().auswahl(&["lesend", "schreibend"]),
        );
        let (a, a_ok) = self.text(o, "abfluss", p, Text::pflicht().auswahl(&["lokal", "netz"]));
        let (zl, _) = self.ganz(o, "zeitlimit_ms", p, Some(15000), (1000, 60000));
        for (k, v) in [
            ("id", id),
            ("name", name),
            ("beschreibung", beschr),
            ("art", art.clone()),
            ("zugriff", z.clone()),
            ("abfluss", a.clone()),
            ("zeitlimit_ms", zl),
        ] {
            out.setzen(k, v);
        }
        if !art_ok {
            return Some(out);
        }
        let art = art.s();
        for b in ARTEN {
            if *b != art && o.hat(b) {
                self.err(join(p, b), "widerspruch");
            }
        }
        let pb = join(p, art);
        let mut netz = None;
        if !o.hat(art) {
            self.err(&pb, "fehlt");
        } else {
            match art {
                "http" => {
                    let (h, m, n) = self.http(&o[art], &pb, 4);
                    netz = n;
                    out.setzen(art, h);
                    if m.as_deref()
                        .is_some_and(|m| ["PUT", "PATCH", "DELETE"].contains(&m))
                        && z_ok
                        && z.text() != Some("schreibend")
                    {
                        self.err(join(p, "zugriff"), "widerspruch");
                    }
                }
                "web" => {
                    let (w, n) = self.web(&o[art], &pb, 4);
                    netz = n;
                    out.setzen(art, w);
                    if z_ok && z.text() != Some("lesend") {
                        self.err(join(p, "zugriff"), "widerspruch");
                    }
                }
                "app" | "mcp" => {
                    let block = &o[art];
                    if block.map().is_none() {
                        self.err(&pb, "typ");
                    } else if art == "app" {
                        let mut ao = self.extras(block, &["faehigkeit"], &pb, 4);
                        let (f, _) = self.text(
                            block,
                            "faehigkeit",
                            &pb,
                            Text::pflicht().muster(faehigkeit_ok),
                        );
                        ao.setzen("faehigkeit", f);
                        out.setzen(art, ao);
                    } else {
                        let mut mo = self.extras(block, &["server", "werkzeug"], &pb, 4);
                        let (s, _) = self.text(block, "server", &pb, Text::pflicht().muster(id_ok));
                        let (w, _) = self.text(
                            block,
                            "werkzeug",
                            &pb,
                            Text::pflicht().laenge(1, 128).muster(|s| {
                                s.bytes()
                                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
                            }),
                        );
                        mo.setzen("server", s);
                        mo.setzen("werkzeug", w);
                        out.setzen(art, mo);
                    }
                }
                _ => unreachable!(),
            }
        }
        if a_ok {
            if let Some(n) = netz {
                if a.s() != if n == "geraet" { "lokal" } else { "netz" } {
                    self.err(join(p, "abfluss"), "widerspruch");
                }
            }
        }
        Some(out)
    }
    fn liste(self) -> Wert {
        Wert::Liste(
            self.fehler
                .into_iter()
                .map(|(p, c)| objekt([("pfad", p.into()), ("code", c.into())]))
                .collect(),
        )
    }
}
pub fn ascii_text(s: &str) -> bool {
    s.bytes().all(|b| (32..=126).contains(&b))
}

/// Gueltiges Dokument mit Standards und unbekannten Feldern, sonst sortierte Fehler.
pub fn pruefe_verzeichnis(w: &Wert) -> Result<Wert, Wert> {
    let einzel =
        |p: &str, c: &str| Wert::Liste(vec![objekt([("pfad", p.into()), ("code", c.into())])]);
    if w.map().is_none() {
        return Err(einzel("", "json"));
    }
    let Some(schema) = w.get("schema") else {
        return Err(einzel("schema", "fehlt"));
    };
    let Some(s) = schema.text() else {
        return Err(einzel("schema", "typ"));
    };
    if s != "lva.schnittstellen/1" {
        let neuer = s.strip_prefix("lva.schnittstellen/").is_some_and(|n| {
            !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
        });
        return Err(einzel(
            "schema",
            if neuer {
                "schema_neuer"
            } else {
                "schema_unbekannt"
            },
        ));
    }
    let mut pr = Pruefer::default();
    let mut out = pr.extras(w, &["schema", "eintraege"], "", 1);
    out.setzen("schema", s);
    let mut norm = Vec::new();
    match w.get("eintraege") {
        None => pr.err("eintraege", "fehlt"),
        Some(Wert::Liste(l)) => {
            if l.len() > 64 {
                pr.err("eintraege", "zu_viele");
            }
            let mut gesehen = BTreeSet::new();
            for (i, e) in l.iter().enumerate() {
                let p = format!("eintraege[{i}]");
                if let Some(n) = pr.eintrag(e, &p) {
                    if let Some(id) = n["id"].text().filter(|s| id_ok(s)) {
                        if !gesehen.insert(id.to_owned()) {
                            pr.err(join(&p, "id"), "doppelt");
                        }
                    }
                    norm.push(n);
                }
            }
        }
        Some(_) => pr.err("eintraege", "typ"),
    }
    out.setzen("eintraege", Wert::Liste(norm));
    if pr.fehler.is_empty() {
        Ok(out)
    } else {
        Err(pr.liste())
    }
}

pub fn kodiertes_verzeichnis(w: &Wert) -> Result<String, Wert> {
    let doc = pruefe_verzeichnis(w)?;
    Ok(kanonisch(&doc).expect("geprueftes Verzeichnis"))
}
