//! Gemeinsame Testhilfen: Fixtures `tooling/fixtures/buch-gespraech/`
//! (`lva-buch-gespraech-fixtures@2`, Plan §7).

use serde::Deserialize;
use serde_json::Value;

use super::lex::{Entity, EntityKind};

const BUCH: &str = include_str!("../../../../../../tooling/fixtures/buch-gespraech/buch.txt");
const FRAGEN: &str = include_str!("../../../../../../tooling/fixtures/buch-gespraech/fragen.json");
const BEFEHLE: &str =
    include_str!("../../../../../../tooling/fixtures/buch-gespraech/befehle.json");
const OFFSETS: &str =
    include_str!("../../../../../../tooling/fixtures/buch-gespraech/offsets.json");

pub fn buch() -> String {
    BUCH.replace("\r\n", "\n")
}

pub fn titel() -> String {
    serde_json::from_str::<Value>(FRAGEN).expect("fragen.json")["title"]
        .as_str()
        .expect("title")
        .to_string()
}

/// Leseposition einer Frage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pos {
    /// `null`: ganzes Buch.
    Whole,
    /// Bis Seitenende dieser Seite.
    Page(u32),
    /// `"missing"`: Bereich „bis hier“ ohne bekannte Grenze.
    Missing,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hist {
    pub question: String,
    pub scope: String,
    pub frontier_page: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Figure {
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Frage {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub position_page: Option<Value>,
    pub intent: String,
    #[serde(default)]
    pub expect_pages: Vec<u32>,
    #[serde(default)]
    pub expect_terms: Vec<String>,
    #[serde(default)]
    pub expect_out_of_scope: bool,
    #[serde(default)]
    pub expect_dropped: Option<u32>,
    #[serde(default)]
    pub expect_reason: Option<String>,
    #[serde(default)]
    pub expect_expansion: Vec<String>,
    #[serde(default)]
    pub expect_not_expanded: Vec<String>,
    #[serde(default)]
    pub expect_history_in_prompt: Option<u32>,
    #[serde(default)]
    pub expect_delimiters_neutralized: bool,
    #[serde(default)]
    pub forbidden_terms: Vec<String>,
    #[serde(default)]
    pub history: Vec<Hist>,
    #[serde(default)]
    pub memory_figures: Vec<Figure>,
    #[serde(default)]
    pub fake_answer: Option<String>,
}

impl Frage {
    pub fn pos(&self) -> Pos {
        match &self.position_page {
            None | Some(Value::Null) => Pos::Whole,
            Some(Value::Number(n)) => Pos::Page(n.as_u64().expect("Seite") as u32),
            Some(_) => Pos::Missing,
        }
    }

    pub fn entities(&self) -> Vec<Entity> {
        self.memory_figures
            .iter()
            .map(|f| Entity {
                name: f.name.clone(),
                variants: f.variants.clone(),
                kind: EntityKind::Person,
            })
            .collect()
    }
}

#[derive(Debug, Deserialize)]
pub struct Probe {
    pub id: String,
    pub excerpt_page: u32,
    pub sentence: String,
    pub expect: String,
    #[serde(default)]
    pub memory_figures: Vec<Figure>,
}

#[derive(Deserialize)]
struct Fragen {
    questions: Vec<Frage>,
    contradiction_probes: Vec<Probe>,
}

pub fn fragen() -> Vec<Frage> {
    serde_json::from_str::<Fragen>(FRAGEN)
        .expect("fragen.json")
        .questions
}

pub fn probes() -> Vec<Probe> {
    serde_json::from_str::<Fragen>(FRAGEN)
        .expect("fragen.json")
        .contradiction_probes
}

pub fn frage(id: &str) -> Frage {
    fragen()
        .into_iter()
        .find(|f| f.id == id)
        .unwrap_or_else(|| panic!("Frage {id} fehlt"))
}

#[derive(Debug, Deserialize)]
pub struct Befehl {
    pub text: String,
    pub command: Option<String>,
}

#[derive(Deserialize)]
struct Befehle {
    commands: Vec<Befehl>,
}

pub fn befehle() -> Vec<Befehl> {
    serde_json::from_str::<Befehle>(BEFEHLE)
        .expect("befehle.json")
        .commands
}

#[derive(Debug, Deserialize)]
pub struct OffsetCase {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub quote: Option<String>,
    #[serde(default)]
    pub expect_page: Option<u32>,
    #[serde(default)]
    pub frontier_after: Option<String>,
    #[serde(default)]
    pub expect_chunk_ends_with: Option<String>,
}

#[derive(Deserialize)]
struct Offsets {
    cases: Vec<OffsetCase>,
}

pub fn offsets() -> Vec<OffsetCase> {
    serde_json::from_str::<Offsets>(OFFSETS)
        .expect("offsets.json")
        .cases
}
