//! lva#408 D2: Extraktor der Dokumentenablage als eigenes Hilfsprogramm (Unterprozess mit Grenzen, siehe
//! `src/ablage/textlauf.rs`). Aufruf: `lva-extrakt <pfad> <original_sha256> <typ>`; Ausgabe ein JSON-
//! Dokument (Markenformat v1 oder `{"fehler": "<code>"}`) auf stdout, nie Inhalt auf stderr.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(local_voice_ai_lib::lva_extrakt_main(&args));
}
