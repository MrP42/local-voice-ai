//! Strenger Leser: Zahlentoken bleiben unveraendert, keine Rundung ueber f64.
//! Die Tiefengrenze wird beim Abstieg vor jeder Container-Allokation geprueft.
use std::collections::BTreeMap;
use std::ops::Index;

pub const MAX_GANZ: i64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wert {
    Null,
    Bool(bool),
    Zahl(String),
    Text(String),
    Liste(Vec<Wert>),
    Objekt(BTreeMap<String, Wert>),
}

impl From<&str> for Wert {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}
impl From<String> for Wert {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}
impl From<bool> for Wert {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}
impl From<i64> for Wert {
    fn from(v: i64) -> Self {
        Self::Zahl(v.to_string())
    }
}
impl Index<&str> for Wert {
    type Output = Wert;
    fn index(&self, k: &str) -> &Wert {
        static NULL: Wert = Wert::Null;
        self.get(k).unwrap_or(&NULL)
    }
}
impl Wert {
    pub fn get(&self, k: &str) -> Option<&Wert> {
        match self {
            Self::Objekt(m) => m.get(k),
            _ => None,
        }
    }
    pub fn hat(&self, k: &str) -> bool {
        self.get(k).is_some()
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }
    /// Nur fuer bereits gepruefte Vertragsfelder.
    pub fn s(&self) -> &str {
        self.text().expect("geprueftes Textfeld")
    }
    pub fn liste(&self) -> &[Wert] {
        match self {
            Self::Liste(v) => v,
            _ => &[],
        }
    }
    pub fn map(&self) -> Option<&BTreeMap<String, Wert>> {
        match self {
            Self::Objekt(v) => Some(v),
            _ => None,
        }
    }
    pub fn setzen(&mut self, k: impl Into<String>, v: impl Into<Wert>) {
        if let Self::Objekt(m) = self {
            m.insert(k.into(), v.into());
        }
    }
    pub fn ganz(&self) -> Option<i64> {
        match self {
            Self::Zahl(s) if !s.contains(['.', 'e', 'E']) => s.parse().ok(),
            _ => None,
        }
    }
    pub fn bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// Exakter Dezimalwert im sicheren Ganzzahlbereich, auch fuer riesige Exponenten.
pub fn als_ganzzahl(w: &Wert) -> Option<i64> {
    let Wert::Zahl(token) = w else { return None };
    let negativ = token.starts_with('-');
    let token = token.trim_start_matches('-');
    let (mantisse, exponent) = token.split_once(['e', 'E']).unwrap_or((token, "0"));
    let nachkomma = mantisse.split_once('.').map_or(0, |(_, n)| n.len());
    let mut ziffern = mantisse.replace('.', "");
    if ziffern.bytes().all(|b| b == b'0') {
        return Some(0);
    }
    // Nur eine Stellenzahl bis zur Eingabelaenge + 16 kann spaeter relevant sein.
    let grenze = i64::try_from(token.len()).ok()?.saturating_add(32);
    let mut exp = 0_i64;
    for b in exponent.trim_start_matches(['+', '-']).bytes() {
        exp = exp.saturating_mul(10).saturating_add(i64::from(b - b'0'));
        if exp > grenze {
            return None;
        }
    }
    if exponent.starts_with('-') {
        exp = -exp;
    }
    exp -= i64::try_from(nachkomma).ok()?;
    while ziffern.ends_with('0') {
        ziffern.pop();
        exp += 1;
    }
    let ziffern = ziffern.trim_start_matches('0');
    if exp < 0 || ziffern.len() as i64 + exp > 16 {
        return None;
    }
    let n = ziffern
        .parse::<i64>()
        .ok()?
        .checked_mul(10_i64.pow(exp as u32))?;
    (n <= MAX_GANZ).then_some(if negativ { -n } else { n })
}

pub fn lesbar(w: &Wert, ebene: usize) -> bool {
    let mut stapel = vec![(w, ebene)];
    while let Some((w, e)) = stapel.pop() {
        match w {
            Wert::Zahl(_)
                if w.ganz()
                    .is_none_or(|n| !(-MAX_GANZ..=MAX_GANZ).contains(&n)) =>
            {
                return false
            }
            Wert::Liste(v) => {
                if e >= 64 {
                    return false;
                }
                stapel.extend(v.iter().map(|v| (v, e + 1)));
            }
            Wert::Objekt(m) => {
                if e >= 64 {
                    return false;
                }
                stapel.extend(m.values().map(|v| (v, e + 1)));
            }
            _ => {}
        }
    }
    true
}

pub fn kanonisch(w: &Wert) -> Result<String, &'static str> {
    enum Teil<'a> {
        Wert(&'a Wert),
        Text(String),
    }
    let mut stapel = vec![Teil::Wert(w)];
    let mut out = String::new();
    while let Some(t) = stapel.pop() {
        match t {
            Teil::Text(t) => out.push_str(&t),
            Teil::Wert(w) => match w {
                Wert::Null => out.push_str("null"),
                Wert::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
                Wert::Zahl(n) => {
                    if n.contains(['.', 'e', 'E']) {
                        return Err("json");
                    }
                    out.push_str(if n == "-0" { "0" } else { n });
                }
                Wert::Text(t) => out.push_str(&serde_json::to_string(t).map_err(|_| "json")?),
                Wert::Liste(v) => {
                    out.push('[');
                    stapel.push(Teil::Text("]".into()));
                    for (i, v) in v.iter().enumerate().rev() {
                        stapel.push(Teil::Wert(v));
                        if i > 0 {
                            stapel.push(Teil::Text(",".into()));
                        }
                    }
                }
                Wert::Objekt(m) => {
                    out.push('{');
                    stapel.push(Teil::Text("}".into()));
                    for (i, (k, v)) in m.iter().enumerate().rev() {
                        stapel.push(Teil::Wert(v));
                        stapel.push(Teil::Text(":".into()));
                        stapel.push(Teil::Text(serde_json::to_string(k).map_err(|_| "json")?));
                        if i > 0 {
                            stapel.push(Teil::Text(",".into()));
                        }
                    }
                }
            },
        }
    }
    Ok(out)
}

pub fn lies_json(text: &str) -> Result<Wert, &'static str> {
    lesen(text, 64)
}

/// Die lokale Pruefstandsdatei umschliesst auch Dokumente mit 64 Ebenen und
/// Fehlerfaelle mit noch einer Ebene mehr. Ihre aeusseren Testcontainer zaehlen
/// nicht zur Vertragstiefe. Dieser Einstieg existiert nur im Test-Build.
#[cfg(test)]
pub(super) fn lies_pruefstaende(text: &str) -> Result<Wert, &'static str> {
    lesen(text, 128)
}

fn lesen(text: &str, max_ebenen: usize) -> Result<Wert, &'static str> {
    struct Leser<'a> {
        s: &'a str,
        i: usize,
        max_ebenen: usize,
    }
    impl Leser<'_> {
        fn leer(&mut self) {
            while self
                .s
                .as_bytes()
                .get(self.i)
                .is_some_and(|b| b" \t\r\n".contains(b))
            {
                self.i += 1;
            }
        }
        fn text(&mut self) -> Result<String, &'static str> {
            let start = self.i;
            if self.s.as_bytes().get(self.i) != Some(&b'"') {
                return Err("json");
            }
            self.i += 1;
            while let Some(&b) = self.s.as_bytes().get(self.i) {
                self.i += 1;
                if b == b'\\' {
                    if self.i >= self.s.len() {
                        return Err("json");
                    }
                    self.i += 1;
                } else if b == b'"' {
                    return serde_json::from_str(&self.s[start..self.i]).map_err(|_| "json");
                }
            }
            Err("json")
        }
        fn wert(&mut self, e: usize) -> Result<Wert, &'static str> {
            self.leer();
            let b = *self.s.as_bytes().get(self.i).ok_or("json")?;
            match b {
                b'"' => Ok(Wert::Text(self.text()?)),
                b'[' | b'{' => {
                    if e >= self.max_ebenen {
                        return Err("json");
                    }
                    self.i += 1;
                    self.leer();
                    let ende = if b == b'[' { b']' } else { b'}' };
                    let mut liste = Vec::new();
                    let mut map = BTreeMap::new();
                    if self.s.as_bytes().get(self.i) != Some(&ende) {
                        loop {
                            self.leer();
                            let key = if b == b'{' {
                                let k = self.text()?;
                                self.leer();
                                if self.s.as_bytes().get(self.i) != Some(&b':') {
                                    return Err("json");
                                }
                                self.i += 1;
                                Some(k)
                            } else {
                                None
                            };
                            let v = self.wert(e + 1)?;
                            if let Some(k) = key {
                                if map.insert(k, v).is_some() {
                                    return Err("json");
                                }
                            } else {
                                liste.push(v);
                            }
                            self.leer();
                            if self.s.as_bytes().get(self.i) != Some(&b',') {
                                break;
                            }
                            self.i += 1;
                        }
                    }
                    if self.s.as_bytes().get(self.i) != Some(&ende) {
                        return Err("json");
                    }
                    self.i += 1;
                    Ok(if b == b'[' {
                        Wert::Liste(liste)
                    } else {
                        Wert::Objekt(map)
                    })
                }
                b'n' | b't' | b'f' => {
                    let (wort, w) = match b {
                        b'n' => ("null", Wert::Null),
                        b't' => ("true", Wert::Bool(true)),
                        _ => ("false", Wert::Bool(false)),
                    };
                    if !self.s[self.i..].starts_with(wort) {
                        return Err("json");
                    }
                    self.i += wort.len();
                    Ok(w)
                }
                b'-' | b'0'..=b'9' => {
                    let start = self.i;
                    if b == b'-' {
                        self.i += 1;
                    }
                    match self.s.as_bytes().get(self.i) {
                        Some(b'0') => self.i += 1,
                        Some(b'1'..=b'9') => {
                            while self
                                .s
                                .as_bytes()
                                .get(self.i)
                                .is_some_and(u8::is_ascii_digit)
                            {
                                self.i += 1;
                            }
                        }
                        _ => return Err("json"),
                    }
                    if self.s.as_bytes().get(self.i) == Some(&b'.') {
                        self.i += 1;
                        let a = self.i;
                        while self
                            .s
                            .as_bytes()
                            .get(self.i)
                            .is_some_and(u8::is_ascii_digit)
                        {
                            self.i += 1;
                        }
                        if self.i == a {
                            return Err("json");
                        }
                    }
                    if self
                        .s
                        .as_bytes()
                        .get(self.i)
                        .is_some_and(|b| b"eE".contains(b))
                    {
                        self.i += 1;
                        if self
                            .s
                            .as_bytes()
                            .get(self.i)
                            .is_some_and(|b| b"+-".contains(b))
                        {
                            self.i += 1;
                        }
                        let a = self.i;
                        while self
                            .s
                            .as_bytes()
                            .get(self.i)
                            .is_some_and(u8::is_ascii_digit)
                        {
                            self.i += 1;
                        }
                        if self.i == a {
                            return Err("json");
                        }
                    }
                    Ok(Wert::Zahl(self.s[start..self.i].into()))
                }
                _ => Err("json"),
            }
        }
    }
    let mut l = Leser {
        s: text,
        i: 0,
        max_ebenen,
    };
    let w = l.wert(0)?;
    l.leer();
    if l.i == text.len() {
        Ok(w)
    } else {
        Err("json")
    }
}
