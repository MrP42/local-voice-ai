// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use local_voice_ai_lib::CliArgs;

fn main() {
    let cli_args = CliArgs::parse();

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
