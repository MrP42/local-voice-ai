//! Notizblock, KI-Notizen und Vorlagen (Meilenstein M1, Issue #59).
//!
//! P1a legt nur das Fundament: Datenmodell (`model`) und den Vorlagenkatalog
//! samt Pruefung und Import/Export (`templates`). Persistenz liegt im
//! `MeetingStore` (`store.rs`); Motor und Commands folgen mit P1b/P1c.

// P1a liefert das Fundament, die Verbraucher (Motor, Commands, UI) kommen mit
// P1b/P1c; bis dahin meldet rustc ungenutzte Konstanten und Funktionen.
#![allow(dead_code)]

pub mod model;
pub mod templates;
