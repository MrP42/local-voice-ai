// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use local_voice_ai_lib::CliArgs;

fn main() {
    // lva#408 D2: Extraktor der Ablage als Unterprozess (vor allem anderen: kein Fenster, kein Log).
    // `haupt` nimmt den Aufruf nur mit dem Kennzeichen des Elternprozesses (Umgebung + stdin) und einem
    // Pfad unter dem Ablageordner an; sonst Exitcode 64 ohne jede Ausgabe.
    let roh: Vec<String> = std::env::args().skip(1).collect();
    if roh.first().map(String::as_str) == Some("--lva-extrakt") {
        std::process::exit(local_voice_ai_lib::lva_extrakt_main(&roh[1..]));
    }

    let cli_args = if local_voice_ai_lib::deep_link::is_url_launch(&roh) {
        CliArgs::default()
    } else {
        CliArgs::parse()
    };

    // lva#882: reiner Pruefaufruf vor jeder Tauri-Initialisierung.
    if cli_args.richtlinie_zeigen {
        #[cfg(windows)]
        unsafe {
            // GUI-Subsystem: Elternkonsole nutzen; umgeleitete Standardhandles bleiben erhalten.
            #[link(name = "kernel32")]
            extern "system" {
                fn GetStdHandle(kind: u32) -> *mut std::ffi::c_void;
                fn AttachConsole(process_id: u32) -> i32;
            }
            let stdout = GetStdHandle(-11_i32 as u32); // STD_OUTPUT_HANDLE
            if stdout.is_null() || stdout as isize == -1 {
                let _ = AttachConsole(u32::MAX); // ATTACH_PARENT_PROCESS
            }
        }
        let code = match local_voice_ai_lib::richtlinie::zeigen(&mut std::io::stdout().lock()) {
            Ok(()) => 0,
            Err(_) => 1,
        };
        std::process::exit(code);
    }

    // M6-P6e: der lokale MCP-Server startet VOR jeder Tauri-Initialisierung
    // (kein Fenster, kein Single-Instance-Plugin, kein Logger auf stdout).
    if cli_args.mcp {
        std::process::exit(local_voice_ai_lib::mcp::run_stdio());
    }

    // A7: `ctl` spricht ueber die Named Pipe mit der laufenden App und beendet sich dann;
    // kein Fenster, kein zweiter Programmstart der App.
    if let Some(local_voice_ai_lib::cli::Command::Ctl(args)) = cli_args.command.clone() {
        std::process::exit(local_voice_ai_lib::agent_bridge::ctl::run(args));
    }

    #[cfg(target_os = "linux")]
    {
        // DMABUF renderer causes crashes on various GPU/display server configurations
        // See: https://github.com/tauri-apps/tauri/issues/9394
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    local_voice_ai_lib::run(cli_args)
}
