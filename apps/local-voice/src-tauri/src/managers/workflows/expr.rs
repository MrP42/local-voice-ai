//! Sichere Ausdruecke und Vorlagen-Variablen fuer Workflows (B1).
//!
//! Zwei Eingaben, EIN kleiner Parser, kein Interpreter einer Programmiersprache:
//!
//! - **Bedingungen** (`when`): `{{trigger.calendar}} == 'cal-1' and len(trigger.attendees) > 2`.
//! - **Vorlagen** in Parametern: `"Protokoll: {{meeting.title}}"`. Ist ein String genau EIN
//!   `{{ausdruck}}`, behaelt das Ergebnis seinen JSON-Typ (Zahl, Liste, Objekt); sonst
//!   entsteht Text.
//!
//! Was es gibt: Literale (Text in `'`/`"`, Zahl, `true`, `false`, `null`, Listen `[..]`),
//! Pfade (`steps.doc.path`, `trigger.attendees[0].email`), Vergleiche (`== != < <= > >=`,
//! `contains`, `in`, `startswith`, `endswith`), `and`/`&&`, `or`/`||`, `not`/`!`, Klammern
//! und genau sechs Funktionen (`exists`, `len`, `lower`, `upper`, `trim`, `default`).
//!
//! Was es NICHT gibt, und damit die Sicherheitsaussage: keine Zuweisung, keine
//! Schleife, kein Aufruf von etwas ausserhalb der Liste, kein Zugriff auf Dateien,
//! Umgebung, Netz oder Geheimnisse, kein regulaerer Ausdruck. Ein Ausdruck liest
//! nur den uebergebenen JSON-Wert (den Laufkontext) und liefert einen JSON-Wert.
//!
//! Grenzen (Schutz vor Absicht und Versehen): Ausdruck hoechstens
//! [`MAX_EXPR_CHARS`] Zeichen, [`MAX_NODES`] Knoten, Tiefe [`MAX_DEPTH`]; Vorlage
//! hoechstens [`MAX_TEMPLATE_CHARS`], Ergebnis hoechstens [`MAX_RENDERED_BYTES`].
//!
//! Einsetzen geschieht in EINEM Durchgang: ein Wert, der selbst `{{...}}` enthaelt,
//! wird nie noch einmal ausgewertet (kein Vorlagen-Einschleusen ueber Trigger-Daten).
//!
//! Strenge: In Vorlagen ist ein fehlender Pfad ein Fehler (`Unresolved`), nie ein
//! stiller leerer Text (ein Empfaenger darf nicht "" werden). In Bedingungen ist ein
//! fehlender Pfad `null`; `exists(pfad)` und `default(pfad, wert)` machen das
//! ausdruecklich.

use std::fmt;

use serde_json::{Number, Value};

pub const MAX_EXPR_CHARS: usize = 1_000;
pub const MAX_TEMPLATE_CHARS: usize = 64 * 1024;
pub const MAX_RENDERED_BYTES: usize = 256 * 1024;
pub const MAX_NODES: usize = 200;
pub const MAX_DEPTH: usize = 16;
pub const MAX_PATH_SEGMENTS: usize = 12;
/// Tiefste Verschachtelung eines Parameterwerts beim Einsetzen.
pub const MAX_VALUE_DEPTH: usize = 12;

// ---------------------------------------------------------------------------
// Fehler
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprError {
    /// Der Text ist kein gueltiger Ausdruck (Stelle in Zeichen ab 0).
    Syntax { pos: usize, message: String },
    /// Zu lang, zu verschachtelt oder zu gross.
    TooComplex(String),
    /// Ein Pfad hat keinen Wert (nur in Vorlagen ein Fehler).
    Unresolved(String),
    /// Falsche Art von Wert fuer die Operation.
    Type(String),
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExprError::Syntax { pos, message } => {
                write!(f, "Ungültiger Ausdruck (Zeichen {}): {message}", pos + 1)
            }
            ExprError::TooComplex(m) => write!(f, "Ausdruck zu umfangreich: {m}"),
            ExprError::Unresolved(p) => write!(f, "Kein Wert für „{p}“."),
            ExprError::Type(m) => write!(f, "Falsche Wertart: {m}"),
        }
    }
}

impl std::error::Error for ExprError {}

fn syntax<T>(pos: usize, message: &str) -> Result<T, ExprError> {
    Err(ExprError::Syntax {
        pos,
        message: message.to_string(),
    })
}

// ---------------------------------------------------------------------------
// Pfade
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// Ein Lesepfad in den Laufkontext, z. B. `steps.doc.path` oder `a.list[0]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Path(pub Vec<Seg>);

impl Path {
    /// Erstes Segment (die Wurzel: `trigger`, `steps`, `vars`, ...).
    pub fn root(&self) -> &str {
        match self.0.first() {
            Some(Seg::Key(k)) => k,
            _ => "",
        }
    }

    /// Zweites Segment, wenn es ein Name ist (`steps.<id>`, `vars.<name>`, `trigger.<feld>`).
    pub fn second(&self) -> Option<&str> {
        match self.0.get(1) {
            Some(Seg::Key(k)) => Some(k),
            _ => None,
        }
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, s) in self.0.iter().enumerate() {
            match s {
                Seg::Key(k) if i == 0 => write!(f, "{k}")?,
                Seg::Key(k) => write!(f, ".{k}")?,
                Seg::Index(n) => write!(f, "[{n}]")?,
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Syntaxbaum
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    In,
    StartsWith,
    EndsWith,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Exists,
    Len,
    Lower,
    Upper,
    Trim,
    Default,
}

impl Func {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "exists" => Func::Exists,
            "len" => Func::Len,
            "lower" => Func::Lower,
            "upper" => Func::Upper,
            "trim" => Func::Trim,
            "default" => Func::Default,
            _ => return None,
        })
    }

    fn arity(self) -> usize {
        match self {
            Func::Default => 2,
            _ => 1,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Func::Exists => "exists",
            Func::Len => "len",
            Func::Lower => "lower",
            Func::Upper => "upper",
            Func::Trim => "trim",
            Func::Default => "default",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Lit(Value),
    Ref(Path),
    List(Vec<Node>),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
    Cmp(Cmp, Box<Node>, Box<Node>),
    Call(Func, Vec<Node>),
}

impl Node {
    fn collect_refs(&self, out: &mut Vec<Path>) {
        match self {
            Node::Lit(_) => {}
            Node::Ref(p) => out.push(p.clone()),
            Node::List(items) | Node::Call(_, items) => {
                items.iter().for_each(|n| n.collect_refs(out))
            }
            Node::Not(n) => n.collect_refs(out),
            Node::And(a, b) | Node::Or(a, b) | Node::Cmp(_, a, b) => {
                a.collect_refs(out);
                b.collect_refs(out);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Str(String),
    Num(Number),
    Word(String),
    Dot,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Bang,
    Minus,
    Open2,
    Close2,
}

#[derive(Clone, Debug)]
struct Spanned {
    tok: Tok,
    pos: usize,
}

fn tokenize(src: &str) -> Result<Vec<Spanned>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        let two = |a: char, b: char| c == a && next == Some(b);
        let tok = if two('{', '{') {
            i += 2;
            Tok::Open2
        } else if two('}', '}') {
            i += 2;
            Tok::Close2
        } else if two('=', '=') {
            i += 2;
            Tok::Eq
        } else if two('!', '=') {
            i += 2;
            Tok::Ne
        } else if two('<', '=') {
            i += 2;
            Tok::Le
        } else if two('>', '=') {
            i += 2;
            Tok::Ge
        } else if two('&', '&') {
            i += 2;
            Tok::AndAnd
        } else if two('|', '|') {
            i += 2;
            Tok::OrOr
        } else {
            match c {
                '<' => {
                    i += 1;
                    Tok::Lt
                }
                '>' => {
                    i += 1;
                    Tok::Gt
                }
                '!' => {
                    i += 1;
                    Tok::Bang
                }
                '.' => {
                    i += 1;
                    Tok::Dot
                }
                '[' => {
                    i += 1;
                    Tok::LBracket
                }
                ']' => {
                    i += 1;
                    Tok::RBracket
                }
                '(' => {
                    i += 1;
                    Tok::LParen
                }
                ')' => {
                    i += 1;
                    Tok::RParen
                }
                ',' => {
                    i += 1;
                    Tok::Comma
                }
                '-' => {
                    i += 1;
                    Tok::Minus
                }
                '\'' | '"' => {
                    let quote = c;
                    i += 1;
                    let mut s = String::new();
                    loop {
                        let Some(&d) = chars.get(i) else {
                            return syntax(start, "Text nicht geschlossen");
                        };
                        i += 1;
                        if d == quote {
                            break;
                        }
                        if d == '\\' {
                            let Some(&e) = chars.get(i) else {
                                return syntax(start, "Text nicht geschlossen");
                            };
                            i += 1;
                            match e {
                                '\\' | '\'' | '"' => s.push(e),
                                'n' => s.push('\n'),
                                't' => s.push('\t'),
                                _ => return syntax(i - 2, "unbekannte Escape-Folge im Text"),
                            }
                        } else {
                            s.push(d);
                        }
                    }
                    Tok::Str(s)
                }
                d if d.is_ascii_digit() => {
                    let mut j = i;
                    while chars.get(j).is_some_and(|x| x.is_ascii_digit()) {
                        j += 1;
                    }
                    let mut is_float = false;
                    if chars.get(j) == Some(&'.') && chars.get(j + 1).is_some_and(|x| x.is_ascii_digit())
                    {
                        is_float = true;
                        j += 1;
                        while chars.get(j).is_some_and(|x| x.is_ascii_digit()) {
                            j += 1;
                        }
                    }
                    let text: String = chars[i..j].iter().collect();
                    i = j;
                    let n = if is_float {
                        text.parse::<f64>().ok().and_then(Number::from_f64)
                    } else {
                        text.parse::<i64>().ok().map(Number::from)
                    };
                    match n {
                        Some(n) => Tok::Num(n),
                        None => return syntax(start, "Zahl nicht darstellbar"),
                    }
                }
                d if d.is_ascii_alphabetic() || d == '_' => {
                    let mut j = i;
                    while chars
                        .get(j)
                        .is_some_and(|x| x.is_ascii_alphanumeric() || *x == '_')
                    {
                        j += 1;
                    }
                    let word: String = chars[i..j].iter().collect();
                    i = j;
                    Tok::Word(word)
                }
                _ => {
                    return syntax(start, &format!("unerwartetes Zeichen „{c}“"));
                }
            }
        };
        out.push(Spanned { tok, pos: start });
        if out.len() > 4 * MAX_NODES {
            return Err(ExprError::TooComplex("zu viele Zeichen-Einheiten".to_string()));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "and",
    "or",
    "not",
    "in",
    "contains",
    "startswith",
    "endswith",
];

struct Parser {
    toks: Vec<Spanned>,
    i: usize,
    nodes: usize,
    depth: usize,
    end_pos: usize,
    allow_mustache: bool,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|s| &s.tok)
    }

    fn pos(&self) -> usize {
        self.toks.get(self.i).map(|s| s.pos).unwrap_or(self.end_pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|s| s.tok.clone());
        if t.is_some() {
            self.i += 1;
        }
        t
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(x)) if x == w)
    }

    fn count(&mut self) -> Result<(), ExprError> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(ExprError::TooComplex(format!(
                "mehr als {MAX_NODES} Bestandteile"
            )));
        }
        Ok(())
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ExprError::TooComplex(format!(
                "mehr als {MAX_DEPTH} Ebenen verschachtelt"
            )));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn parse_or(&mut self) -> Result<Node, ExprError> {
        self.enter()?;
        let mut left = self.parse_and()?;
        while self.is_word("or") || self.peek() == Some(&Tok::OrOr) {
            self.bump();
            let right = self.parse_and()?;
            self.count()?;
            left = Node::Or(Box::new(left), Box::new(right));
        }
        self.leave();
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Node, ExprError> {
        let mut left = self.parse_not()?;
        while self.is_word("and") || self.peek() == Some(&Tok::AndAnd) {
            self.bump();
            let right = self.parse_not()?;
            self.count()?;
            left = Node::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Node, ExprError> {
        if self.is_word("not") || self.peek() == Some(&Tok::Bang) {
            self.bump();
            self.enter()?;
            let inner = self.parse_not()?;
            self.leave();
            self.count()?;
            return Ok(Node::Not(Box::new(inner)));
        }
        self.parse_cmp()
    }

    fn cmp_op(&self) -> Option<Cmp> {
        match self.peek()? {
            Tok::Eq => Some(Cmp::Eq),
            Tok::Ne => Some(Cmp::Ne),
            Tok::Lt => Some(Cmp::Lt),
            Tok::Le => Some(Cmp::Le),
            Tok::Gt => Some(Cmp::Gt),
            Tok::Ge => Some(Cmp::Ge),
            Tok::Word(w) => match w.as_str() {
                "contains" => Some(Cmp::Contains),
                "in" => Some(Cmp::In),
                "startswith" => Some(Cmp::StartsWith),
                "endswith" => Some(Cmp::EndsWith),
                _ => None,
            },
            _ => None,
        }
    }

    fn parse_cmp(&mut self) -> Result<Node, ExprError> {
        let left = self.parse_primary()?;
        if let Some(op) = self.cmp_op() {
            self.bump();
            let right = self.parse_primary()?;
            self.count()?;
            if self.cmp_op().is_some() {
                return syntax(self.pos(), "Vergleiche lassen sich nicht verketten");
            }
            return Ok(Node::Cmp(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<Node, ExprError> {
        self.count()?;
        let pos = self.pos();
        match self.bump() {
            None => syntax(pos, "Ausdruck endet zu früh"),
            Some(Tok::Str(s)) => Ok(Node::Lit(Value::String(s))),
            Some(Tok::Num(n)) => Ok(Node::Lit(Value::Number(n))),
            Some(Tok::Minus) => match self.bump() {
                Some(Tok::Num(n)) => {
                    let neg = if let Some(i) = n.as_i64() {
                        Number::from(-i)
                    } else if let Some(f) = n.as_f64().and_then(|f| Number::from_f64(-f)) {
                        f
                    } else {
                        return syntax(pos, "Zahl nicht darstellbar");
                    };
                    Ok(Node::Lit(Value::Number(neg)))
                }
                _ => syntax(pos, "nach „-“ muss eine Zahl folgen"),
            },
            Some(Tok::LParen) => {
                self.enter()?;
                let inner = self.parse_or()?;
                self.leave();
                match self.bump() {
                    Some(Tok::RParen) => Ok(inner),
                    _ => syntax(pos, "Klammer nicht geschlossen"),
                }
            }
            Some(Tok::Open2) if self.allow_mustache => {
                self.enter()?;
                let inner = self.parse_or()?;
                self.leave();
                match self.bump() {
                    Some(Tok::Close2) => Ok(inner),
                    _ => syntax(pos, "„{{“ nicht mit „}}“ geschlossen"),
                }
            }
            Some(Tok::LBracket) => {
                let mut items = Vec::new();
                if self.peek() == Some(&Tok::RBracket) {
                    self.bump();
                    return Ok(Node::List(items));
                }
                loop {
                    self.enter()?;
                    items.push(self.parse_or()?);
                    self.leave();
                    match self.bump() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RBracket) => break,
                        _ => return syntax(pos, "Liste nicht geschlossen"),
                    }
                }
                Ok(Node::List(items))
            }
            Some(Tok::Word(w)) => self.parse_word(w, pos),
            Some(_) => syntax(pos, "unerwartetes Zeichen"),
        }
    }

    fn parse_word(&mut self, w: String, pos: usize) -> Result<Node, ExprError> {
        match w.as_str() {
            "true" => return Ok(Node::Lit(Value::Bool(true))),
            "false" => return Ok(Node::Lit(Value::Bool(false))),
            "null" => return Ok(Node::Lit(Value::Null)),
            _ => {}
        }
        if KEYWORDS.contains(&w.as_str()) {
            return syntax(pos, &format!("„{w}“ steht an der falschen Stelle"));
        }
        if self.peek() == Some(&Tok::LParen) {
            let Some(func) = Func::parse(&w) else {
                return syntax(pos, &format!("unbekannte Funktion „{w}“"));
            };
            self.bump();
            let mut args = Vec::new();
            if self.peek() == Some(&Tok::RParen) {
                self.bump();
            } else {
                loop {
                    self.enter()?;
                    args.push(self.parse_or()?);
                    self.leave();
                    match self.bump() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RParen) => break,
                        _ => return syntax(pos, "Funktionsaufruf nicht geschlossen"),
                    }
                }
            }
            if args.len() != func.arity() {
                return syntax(
                    pos,
                    &format!(
                        "„{}“ braucht {} Argument(e), nicht {}",
                        func.name(),
                        func.arity(),
                        args.len()
                    ),
                );
            }
            return Ok(Node::Call(func, args));
        }
        // Pfad: wort ( . wort | [ zahl ] )*
        let mut segs = vec![Seg::Key(w)];
        loop {
            match self.peek() {
                Some(Tok::Dot) => {
                    self.bump();
                    match self.bump() {
                        Some(Tok::Word(k)) => segs.push(Seg::Key(k)),
                        _ => return syntax(self.pos(), "nach „.“ muss ein Name folgen"),
                    }
                }
                Some(Tok::LBracket) => {
                    self.bump();
                    let idx = match self.bump() {
                        Some(Tok::Num(n)) => n.as_u64().and_then(|u| usize::try_from(u).ok()),
                        _ => None,
                    };
                    let Some(idx) = idx else {
                        return syntax(self.pos(), "in „[ ]“ gehört eine ganze Zahl");
                    };
                    if self.bump() != Some(Tok::RBracket) {
                        return syntax(self.pos(), "„]“ fehlt");
                    }
                    segs.push(Seg::Index(idx));
                }
                _ => break,
            }
            if segs.len() > MAX_PATH_SEGMENTS {
                return Err(ExprError::TooComplex(format!(
                    "Pfad mit mehr als {MAX_PATH_SEGMENTS} Teilen"
                )));
            }
        }
        Ok(Node::Ref(Path(segs)))
    }
}

fn parse_tokens(src: &str, allow_mustache: bool) -> Result<Node, ExprError> {
    if src.chars().count() > MAX_EXPR_CHARS {
        return Err(ExprError::TooComplex(format!(
            "länger als {MAX_EXPR_CHARS} Zeichen"
        )));
    }
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return syntax(0, "leerer Ausdruck");
    }
    let mut p = Parser {
        toks,
        i: 0,
        nodes: 0,
        depth: 0,
        end_pos: src.chars().count(),
        allow_mustache,
    };
    let node = p.parse_or()?;
    if p.i < p.toks.len() {
        return syntax(p.pos(), "unerwartetes Zeichen nach dem Ausdruck");
    }
    Ok(node)
}

// ---------------------------------------------------------------------------
// Auswertung
// ---------------------------------------------------------------------------

/// Wie streng ein Pfad ohne Wert behandelt wird.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Bedingung: ein fehlender Pfad ist `null`.
    Lenient,
    /// Vorlage: ein fehlender Pfad ist ein Fehler.
    Strict,
}

#[derive(Clone, Debug)]
enum V {
    Missing(String),
    J(Value),
}

fn resolve<'a>(root: &'a Value, path: &Path) -> Option<&'a Value> {
    let mut cur = root;
    for seg in &path.0 {
        cur = match (seg, cur) {
            (Seg::Key(k), Value::Object(m)) => m.get(k)?,
            (Seg::Index(i), Value::Array(a)) => a.get(*i)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "nichts (null)",
        Value::Bool(_) => "Ja/Nein",
        Value::Number(_) => "Zahl",
        Value::String(_) => "Text",
        Value::Array(_) => "Liste",
        Value::Object(_) => "Objekt",
    }
}

fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

struct Eval<'a> {
    root: &'a Value,
    mode: Mode,
}

impl Eval<'_> {
    /// Wert mit dem Modus abgeglichen: fehlend wird `null` (streng: Fehler).
    fn value(&self, v: V) -> Result<Value, ExprError> {
        match v {
            V::J(j) => Ok(j),
            V::Missing(p) => match self.mode {
                Mode::Lenient => Ok(Value::Null),
                Mode::Strict => Err(ExprError::Unresolved(p)),
            },
        }
    }

    fn truth(&self, v: V) -> Result<bool, ExprError> {
        match self.value(v)? {
            Value::Bool(b) => Ok(b),
            Value::Null => Ok(false),
            other => Err(ExprError::Type(format!(
                "erwartet Ja/Nein, gefunden {}",
                kind_name(&other)
            ))),
        }
    }

    fn eval(&self, node: &Node) -> Result<V, ExprError> {
        Ok(match node {
            Node::Lit(v) => V::J(v.clone()),
            Node::Ref(p) => match resolve(self.root, p) {
                Some(v) => V::J(v.clone()),
                None => V::Missing(p.to_string()),
            },
            Node::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for n in items {
                    let v = self.eval(n)?;
                    out.push(self.value(v)?);
                }
                V::J(Value::Array(out))
            }
            Node::Not(n) => {
                let v = self.eval(n)?;
                V::J(Value::Bool(!self.truth(v)?))
            }
            Node::And(a, b) => {
                let left = self.eval(a)?;
                if !self.truth(left)? {
                    return Ok(V::J(Value::Bool(false)));
                }
                let right = self.eval(b)?;
                V::J(Value::Bool(self.truth(right)?))
            }
            Node::Or(a, b) => {
                let left = self.eval(a)?;
                if self.truth(left)? {
                    return Ok(V::J(Value::Bool(true)));
                }
                let right = self.eval(b)?;
                V::J(Value::Bool(self.truth(right)?))
            }
            Node::Cmp(op, a, b) => {
                let left = self.eval(a)?;
                let left = self.value(left)?;
                let right = self.eval(b)?;
                let right = self.value(right)?;
                V::J(Value::Bool(compare(*op, &left, &right)?))
            }
            Node::Call(f, args) => self.call(*f, args)?,
        })
    }

    fn call(&self, f: Func, args: &[Node]) -> Result<V, ExprError> {
        match f {
            Func::Exists => {
                let present = match self.eval(&args[0])? {
                    V::Missing(_) => false,
                    V::J(Value::Null) => false,
                    V::J(_) => true,
                };
                Ok(V::J(Value::Bool(present)))
            }
            Func::Default => {
                let first = self.eval(&args[0])?;
                match first {
                    V::J(v) if !v.is_null() => Ok(V::J(v)),
                    _ => {
                        let fallback = self.eval(&args[1])?;
                        Ok(V::J(self.value(fallback)?))
                    }
                }
            }
            Func::Len => {
                let v = self.eval(&args[0])?;
                let v = self.value(v)?;
                let n = match &v {
                    Value::String(s) => s.chars().count(),
                    Value::Array(a) => a.len(),
                    Value::Object(o) => o.len(),
                    Value::Null => 0,
                    other => {
                        return Err(ExprError::Type(format!(
                            "len() braucht Text, Liste oder Objekt, gefunden {}",
                            kind_name(other)
                        )))
                    }
                };
                Ok(V::J(Value::from(n as u64)))
            }
            Func::Lower | Func::Upper | Func::Trim => {
                let v = self.eval(&args[0])?;
                let v = self.value(v)?;
                let Value::String(s) = &v else {
                    return Err(ExprError::Type(format!(
                        "{}() braucht Text, gefunden {}",
                        f.name(),
                        kind_name(&v)
                    )));
                };
                Ok(V::J(Value::String(match f {
                    Func::Lower => s.to_lowercase(),
                    Func::Upper => s.to_uppercase(),
                    _ => s.trim().to_string(),
                })))
            }
        }
    }
}

fn compare(op: Cmp, a: &Value, b: &Value) -> Result<bool, ExprError> {
    match op {
        Cmp::Eq => Ok(json_eq(a, b)),
        Cmp::Ne => Ok(!json_eq(a, b)),
        Cmp::Lt | Cmp::Le | Cmp::Gt | Cmp::Ge => {
            // Ohne Wert (null) ist jeder Vergleich "falsch", nie ein Fehler.
            if a.is_null() || b.is_null() {
                return Ok(false);
            }
            let ord = match (a, b) {
                (Value::Number(x), Value::Number(y)) => x
                    .as_f64()
                    .partial_cmp(&y.as_f64())
                    .ok_or_else(|| ExprError::Type("Zahlen nicht vergleichbar".to_string()))?,
                (Value::String(x), Value::String(y)) => x.cmp(y),
                _ => {
                    return Err(ExprError::Type(format!(
                        "{} und {} lassen sich nicht der Größe nach vergleichen",
                        kind_name(a),
                        kind_name(b)
                    )))
                }
            };
            Ok(match op {
                Cmp::Lt => ord.is_lt(),
                Cmp::Le => ord.is_le(),
                Cmp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        Cmp::Contains => contains(a, b),
        Cmp::In => contains(b, a),
        Cmp::StartsWith | Cmp::EndsWith => match (a, b) {
            (Value::String(x), Value::String(y)) => Ok(if op == Cmp::StartsWith {
                x.starts_with(y.as_str())
            } else {
                x.ends_with(y.as_str())
            }),
            (Value::Null, _) | (_, Value::Null) => Ok(false),
            _ => Err(ExprError::Type(
                "startswith/endswith brauchen zwei Texte".to_string(),
            )),
        },
    }
}

/// `haystack` enthaelt `needle`: Text in Text, Element in Liste.
fn contains(haystack: &Value, needle: &Value) -> Result<bool, ExprError> {
    match (haystack, needle) {
        (Value::Null, _) | (_, Value::Null) => Ok(false),
        (Value::String(h), Value::String(n)) => Ok(h.contains(n.as_str())),
        (Value::Array(items), n) => Ok(items.iter().any(|i| json_eq(i, n))),
        (h, n) => Err(ExprError::Type(format!(
            "„enthält“ braucht Text in Text oder Element in Liste, gefunden {} und {}",
            kind_name(h),
            kind_name(n)
        ))),
    }
}

// ---------------------------------------------------------------------------
// Oeffentliche Typen
// ---------------------------------------------------------------------------

/// Ein geparster Ausdruck (Bedingung oder Inhalt einer `{{ }}`-Klammer).
#[derive(Clone, Debug, PartialEq)]
pub struct Expr(Node);

impl Expr {
    /// Alle gelesenen Pfade (fuer die statische Pruefung).
    pub fn refs(&self) -> Vec<Path> {
        let mut out = Vec::new();
        self.0.collect_refs(&mut out);
        out
    }

    /// Wahrheitswert gegen den Laufkontext. Fehlende Pfade sind `null`.
    pub fn eval_bool(&self, ctx: &Value) -> Result<bool, ExprError> {
        let ev = Eval {
            root: ctx,
            mode: Mode::Lenient,
        };
        let v = ev.eval(&self.0)?;
        ev.truth(v)
    }

    /// Wert gegen den Laufkontext im angegebenen Modus.
    pub fn eval(&self, ctx: &Value, mode: Mode) -> Result<Value, ExprError> {
        let ev = Eval { root: ctx, mode };
        let v = ev.eval(&self.0)?;
        ev.value(v)
    }
}

/// Parst eine Bedingung (`{{ }}`-Klammern um Operanden sind erlaubt).
pub fn parse_condition(src: &str) -> Result<Expr, ExprError> {
    parse_tokens(src, true).map(Expr)
}

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Lit(String),
    Expr(Node),
}

/// Ein Text mit `{{ausdruck}}`-Stellen.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

impl Template {
    pub fn is_plain(&self) -> bool {
        self.parts.iter().all(|p| matches!(p, Part::Lit(_)))
    }

    /// Alle gelesenen Pfade aller Stellen.
    pub fn refs(&self) -> Vec<Path> {
        let mut out = Vec::new();
        for p in &self.parts {
            if let Part::Expr(n) = p {
                n.collect_refs(&mut out);
            }
        }
        out
    }

    /// Fuer die ANZEIGE (Trockenlauf, Protokoll): fehlende Werte, Fehler und
    /// Objekte erscheinen als „…“, Listen als „a, b“. Wird nie zum Ausfuehren benutzt.
    pub fn render_display(&self, ctx: &Value) -> String {
        fn show(v: &Value) -> String {
            match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                Value::Array(items) => items.iter().map(show).collect::<Vec<_>>().join(", "),
                _ => "…".to_string(),
            }
        }
        let ev = Eval {
            root: ctx,
            mode: Mode::Lenient,
        };
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Lit(s) => out.push_str(s),
                Part::Expr(node) => {
                    // Fehlender Pfad: nicht `null` (Lenient), sondern sichtbar „…“.
                    let text = match ev.eval(node) {
                        Ok(V::J(v)) if !v.is_null() => show(&v),
                        _ => "…".to_string(),
                    };
                    out.push_str(&text);
                }
            }
        }
        out
    }

    /// Setzt die Stellen ein. Genau eine Stelle ohne Text drumherum behaelt ihren Typ.
    pub fn render(&self, ctx: &Value) -> Result<Value, ExprError> {
        let ev = Eval {
            root: ctx,
            mode: Mode::Strict,
        };
        if let [Part::Expr(node)] = self.parts.as_slice() {
            let v = ev.eval(node)?;
            return ev.value(v);
        }
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Lit(s) => out.push_str(s),
                Part::Expr(node) => {
                    let v = ev.eval(node)?;
                    match ev.value(v)? {
                        Value::String(s) => out.push_str(&s),
                        Value::Number(n) => out.push_str(&n.to_string()),
                        Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
                        other => {
                            return Err(ExprError::Type(format!(
                                "{} lässt sich nicht in einen Text einsetzen",
                                kind_name(&other)
                            )))
                        }
                    }
                }
            }
            if out.len() > MAX_RENDERED_BYTES {
                return Err(ExprError::TooComplex(format!(
                    "Ergebnis größer als {} KiB",
                    MAX_RENDERED_BYTES / 1024
                )));
            }
        }
        Ok(Value::String(out))
    }
}

/// Zerlegt einen Text in feste Teile und `{{ausdruck}}`-Stellen. Ein `}}` in
/// Anfuehrungszeichen innerhalb einer Stelle schliesst sie nicht.
pub fn parse_template(src: &str) -> Result<Template, ExprError> {
    if src.len() > MAX_TEMPLATE_CHARS {
        return Err(ExprError::TooComplex(format!(
            "Vorlage länger als {} KiB",
            MAX_TEMPLATE_CHARS / 1024
        )));
    }
    let chars: Vec<char> = src.chars().collect();
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1) == Some(&'{') {
            let open = i;
            i += 2;
            let inner_start = i;
            let mut quote: Option<char> = None;
            let mut closed = None;
            while i < chars.len() {
                let c = chars[i];
                match quote {
                    Some(q) => {
                        if c == '\\' {
                            i += 1;
                        } else if c == q {
                            quote = None;
                        }
                    }
                    None => {
                        if c == '\'' || c == '"' {
                            quote = Some(c);
                        } else if c == '}' && chars.get(i + 1) == Some(&'}') {
                            closed = Some(i);
                            break;
                        }
                    }
                }
                i += 1;
            }
            let Some(end) = closed else {
                return syntax(open, "„{{“ ohne schließendes „}}“");
            };
            let inner: String = chars[inner_start..end].iter().collect();
            let node = parse_tokens(&inner, false).map_err(|e| match e {
                ExprError::Syntax { pos, message } => ExprError::Syntax {
                    pos: pos + inner_start,
                    message,
                },
                other => other,
            })?;
            if !lit.is_empty() {
                parts.push(Part::Lit(std::mem::take(&mut lit)));
            }
            parts.push(Part::Expr(node));
            if parts.len() > MAX_NODES {
                return Err(ExprError::TooComplex(format!(
                    "mehr als {MAX_NODES} Stellen in einem Text"
                )));
            }
            i = end + 2;
        } else if chars[i] == '}' && chars.get(i + 1) == Some(&'}') {
            return syntax(i, "„}}“ ohne öffnendes „{{“");
        } else {
            lit.push(chars[i]);
            i += 1;
        }
    }
    if !lit.is_empty() {
        parts.push(Part::Lit(lit));
    }
    Ok(Template { parts })
}

/// Setzt in einem JSON-Wert alle Texte ein (rekursiv, Schluessel bleiben).
pub fn render_value(v: &Value, ctx: &Value) -> Result<Value, ExprError> {
    render_depth(v, ctx, 0)
}

fn render_depth(v: &Value, ctx: &Value, depth: usize) -> Result<Value, ExprError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(ExprError::TooComplex(format!(
            "Parameter tiefer als {MAX_VALUE_DEPTH} Ebenen verschachtelt"
        )));
    }
    match v {
        Value::String(s) if s.contains("{{") || s.contains("}}") => {
            parse_template(s)?.render(ctx)
        }
        Value::Array(items) => items
            .iter()
            .map(|i| render_depth(i, ctx, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                out.insert(k.clone(), render_depth(val, ctx, depth + 1)?);
            }
            Ok(Value::Object(out))
        }
        other => Ok(other.clone()),
    }
}

/// Alle Pfade, die irgendein Text in `v` liest; die Texte werden dabei geparst.
/// `Err` beim ersten ungueltigen Text (mit dessen JSON-Zeiger).
pub fn collect_value_refs(v: &Value) -> Result<Vec<Path>, (String, ExprError)> {
    fn walk(
        v: &Value,
        pointer: &str,
        depth: usize,
        out: &mut Vec<Path>,
    ) -> Result<(), (String, ExprError)> {
        if depth > MAX_VALUE_DEPTH {
            return Err((
                pointer.to_string(),
                ExprError::TooComplex(format!(
                    "tiefer als {MAX_VALUE_DEPTH} Ebenen verschachtelt"
                )),
            ));
        }
        match v {
            Value::String(s) if s.contains("{{") || s.contains("}}") => {
                let t = parse_template(s).map_err(|e| (pointer.to_string(), e))?;
                out.extend(t.refs());
            }
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    walk(item, &format!("{pointer}/{i}"), depth + 1, out)?;
                }
            }
            Value::Object(map) => {
                for (k, val) in map {
                    walk(val, &format!("{pointer}/{k}"), depth + 1, out)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(v, "", 0, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests;
