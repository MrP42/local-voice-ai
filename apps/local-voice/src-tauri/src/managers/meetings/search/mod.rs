//! Such-Index fuer Besprechungen (M4, Paket P4a): Fundament fuer die Suche in
//! der Liste und den Chat. Siehe `koordination/granola-besprechungen/entwurf/m4-chat-suche.md`.
//!
//! - `chunking`: reines Zerlegen von Transkript, Notizen und KI-Notizen in
//!   Chunks plus die FTS5-Abfragebauer (keine I/O).
//! - `index`: Store-Erweiterung (`impl MeetingStore`) fuer Chunks, Index-Zustand,
//!   Volltextsuche, Ordner, Recipes und Chat-Verlaeufe.
//! - `vectors`: int8-Vektorindex im RAM mit f32-Nachbewertung aus der DB.
//! - `hybrid`: Reciprocal Rank Fusion aus Wort- und Vektorsuche.
//! - `bench`: Performance-Werkzeug (`--bench-search`) auf einer synthetischen
//!   Sandbox-Datenbank im Temp-Verzeichnis.
//!
//! Der Index ist eine ABGELEITETE Kopie (aus Transkript und Notizen jederzeit
//! neu erzeugbar) und liegt in derselben `meetings.db`. Nichts hier beruehrt den
//! Audiopfad: kein Modul ruft etwas aus dem Aufnahme-Callback auf.
//!
//! Die Konsumenten (Indexer, Commands, Chat) folgen mit P4b bis P4e; bis dahin
//! sind viele Funktionen nur von den Tests erreichbar.
#![allow(dead_code)]

pub mod bench;
pub mod chunking;
pub mod hybrid;
pub mod index;
pub mod vectors;
