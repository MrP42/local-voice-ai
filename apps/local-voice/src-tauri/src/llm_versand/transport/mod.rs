//! Privater Rohtransport des Versandmoduls (lva#604).
//!
//! **Stand Teilpaket 1: leerer Rumpf.** Mit Teilpaket 3 wandert hierher der gesamte Weg ins
//! Netz und zu den CLIs, der heute verstreut liegt:
//!
//! - aus `llm_client.rs`: `send_inner`, die Streaming-Varianten, die Client-Builder
//!   (`create_client`, `create_pinned_local_client`), der Kern hinter `send_chat_completion_checked`,
//!   `send_chat_completion_stream_*`, `post_image_prompt`, die nativen Ollama-Aufrufe,
//!   `fetch_models` und `ollama_unload`;
//! - aus `managers/llm/cli.rs`: die CLI-Einstiege `call`, `call_with_timeout`,
//!   `call_with_options` einschließlich der Vorschauprüfung (`preview_precheck`,
//!   `verify_preview`), die dabei mit dem neuen Modulpfad angepasst wird.
//!
//! Regeln für dieses Modul (der Compiler erzwingt sie):
//!
//! - Alles hier ist höchstens `pub(super)`, also nur für `llm_versand` sichtbar.
//! - Es gibt keine Reexporte, weder hier noch in `llm_versand`.
//! - Ein Transportaufruf nimmt die besitzende `Versandfreigabe` entgegen und macht **genau
//!   einen** Versuch an das gebundene [`Ziel`](super::ziel::Ziel); Fallbacks (Stream ohne
//!   Stream, CLI-Neuversuch, JSON-Reparatur) laufen außerhalb, jeder als neuer Versuch mit
//!   eigener Prüfung.
//! - Die lokale Route läuft ohne Proxy und ohne Redirect, Cloud wie bisher mit Proxy aber ohne
//!   Redirect; Anbieter-Antworttext gelangt nie in Fehler oder Log.
