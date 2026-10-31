//! Erzeugt `src/bindings.ts` ohne die App zu starten (headless, CI/Windows-Tor).
//!
//! `cargo run --example bindings_export -- ../src/bindings.ts`

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "../src/bindings.ts".to_string());
    if let Err(e) = local_voice_ai_lib::export_bindings(std::path::Path::new(&out)) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    println!("bindings geschrieben: {out}");
}
