//! Besprechungsplanung (Epic lva#534, Konzept `docs/plans/2026-11-12_besprechungsplanung.md`).
//!
//! Kernlogik ohne App- und ohne `wai-*`-Abhaengigkeit, damit das Modul auch in einer
//! offenen Ausgabe der App bleibt:
//! - [`model`]: Planung, Verknuepfung, Einwilligung;
//! - [`rules`]: Regelpruefung (Hinweise, blockiert nur bei Fehlern);
//! - [`invitation`]: Einladungstext (HTML + Text) mit Marke `lva-plan:<id>`;
//! - [`sync`]: Abgleich mit dem Kalender (Outlook fuehrt Zeit, Ort, Teilnehmende);
//! - [`consent`]: Einwilligungs-Tor vor der Transkription;
//! - [`store`]: SQLite-Schema und Zugriffe.
//!
//! Die Outlook-Adapter (Datei/ICS, Outlook lokal, Microsoft Graph) leben in der App
//! und liefern dem Abgleich eine [`sync::Beobachtung`].

pub mod consent;
pub mod invitation;
pub mod model;
pub mod rules;
pub mod store;
pub mod sync;

#[cfg(test)]
mod tests;
