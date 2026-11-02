//! Typen mit den Feldnamen aus `contracts/core/research.schema.json` (Query, Source).

use serde::{Deserialize, Serialize};

/// `DataClass` aus `envelope.schema.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Datenklasse {
    Public,
    Internal,
    Personal,
    Special,
    Restricted,
}

/// `Query` des Schemas: Suchanfrage mit Klasse (Wortlaut steht nur im Recherche-Dokument).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anfrage {
    pub text: String,
    pub data_class: Datenklasse,
}

/// Debug ohne Wortlaut (Protokoll ohne Inhalte): nur Laenge und Klasse.
impl std::fmt::Debug for Anfrage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Anfrage")
            .field("text_len", &self.text.chars().count())
            .field("data_class", &self.data_class)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quellentyp {
    Official,
    Scholarly,
    Press,
    Forum,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Vertrauen {
    Internal,
    Project,
    Web,
}

/// `Source` des Schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quelle {
    pub index: u32,
    pub title: String,
    pub url: String,
    pub retrieved_at: String,
    pub excerpt: String,
    pub excerpt_sha256: String,
    pub source_type: Quellentyp,
    pub trust: Vertrauen,
}
