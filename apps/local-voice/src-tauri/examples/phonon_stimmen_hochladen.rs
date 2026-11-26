//! Einmaliger Stimmtransport für den Gerätebau (#898), standardmäßig ohne Schreibzugriffe.
//! Anleitung und Prüfaufträge: `examples/phonon_stimmen_hochladen.md`.
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    std::process::exit(local_voice_ai_lib::phonon_stimmen_hochladen(&args));
}
