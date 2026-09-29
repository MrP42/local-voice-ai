//! Notizblock, KI-Notizen und Vorlagen (Meilenstein M1, Issue #59).
//!
//! P1a legt das Fundament: Datenmodell (`model`) und den Vorlagenkatalog
//! samt Pruefung und Import/Export (`templates`). Persistenz liegt im
//! `MeetingStore` (`store.rs`). P1b liefert den Motor: `enhance` (Prompts,
//! Schema, Lauf, Anweisung, Handbearbeitung, Markdown) und `assemble` (die
//! deterministische Nachpruefung; setzt den Nutzertext ein). Commands und UI
//! folgen mit P1c/P1d.

// P1a liefert das Fundament, die Verbraucher (Motor, Commands, UI) kommen mit
// P1b/P1c; bis dahin meldet rustc ungenutzte Konstanten und Funktionen.
#![allow(dead_code)]

pub mod assemble;
pub mod enhance;
pub mod model;
pub mod templates;
