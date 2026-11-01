//! Sprachdienst von Local Voice AI (ADR-0021, Paket S2): Hören (STT) und Sprechen (TTS) für
//! andere WAI-Apps über eine lokale Verbindung.
//!
//! Der Vertrag (Rahmen, Nachrichten, Grenzen, Handschlag) kommt unverändert aus dem WAI SDK
//! (`vendor/wai-voice-dienst`, `scripts/vendor-wai-voice-dienst.py`); beschrieben ist er in
//! `docs/leitfaeden/sprachdienst-protokoll.md` des SDK. Hier steht die **Serverseite**:
//!
//! ```text
//! App (Corporate LLM, ...)  --Pipe / Unix-Socket, nur aktueller Benutzer-->  unix / pipe
//!                                                                              |
//!                                                                   verbindung  (Protokoll je Verbindung)
//!                                                                    |       |       |
//!                                                        zugang (Fingerabdrücke, Rechte)  kopplung (Code, Frist, Ratenbremse)
//!                                                                    |
//!                                                               engine::Sprache  --> app (TranscriptionManager, TtsManager)
//! ```
//!
//! Bausteine:
//! - `engine`: die Engines hinter dem Dienst als Schnittstelle (`Sprache`), Umrechnungen.
//! - `wav`: WAV-Häppchen für Audio heraus.
//! - `zugang`: gekoppelte Apps, Rechte, Widerruf; Datei neben den Einstellungen.
//! - `kopplung`: offene Kopplungsanfragen mit Frist (120 s) und Ratenbremse (3 je Minute).
//! - `kontext`: gemeinsamer Zustand und Grenzen.
//! - `verbindung`: das Protokoll einer Verbindung (Handschlag mit HMAC-Beweis, Kopplung, Ströme,
//!   Guthaben, Abbruch, Fristen).
//! - `unix` (macOS, Linux) und `pipe` (Windows): der Endpunkt mit Rechten und Prüfung der Gegenstelle.
//! - `dienst`: Start/Stopp und Zustand.
//! - `app`: Anbindung an die laufende App (Engines, Ereignisse, Fenster) — nicht Teil der
//!   unabhängig testbaren Bausteine.
//!
//! Der Dienst ist standardmäßig **aus**. Keine Inhalte im Log (weder Audio noch Text); das
//! Rohaudio liegt nur im Arbeitsspeicher einer Verbindung und wird nach dem Erkennen verworfen.

pub mod app;
pub mod dienst;
pub mod engine;
pub mod kontext;
pub mod kopplung;
pub mod verbindung;
pub mod wav;
pub mod zugang;

#[cfg(unix)]
pub mod unix;

#[cfg(windows)]
pub mod pipe;

#[cfg(test)]
mod tests;
