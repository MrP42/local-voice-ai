//! Die Log-Grenze nimmt ausschließlich geschlossene Codes und eine numerische Dauer an.
//! Keine Strings vom Aufrufer: Inhalte können diese Schnittstelle nicht passieren.
use super::{Code, Weg};

pub(super) fn zeile(weg: Weg, code: Code, dauer_ms: u128) -> String {
    let weg = match weg {
        Weg::VorVersand => "vor_versand", Weg::Cli => "cli",
        Weg::Http => "http", Weg::Lokal => "lokal",
    };
    format!("voice_call: weg={weg} code={} dauer_ms={dauer_ms}", code.text())
}

pub(super) fn schreiben(weg: Weg, code: Code, dauer_ms: u128) {
    let zeile = zeile(weg, code, dauer_ms);
    if code == Code::Abgebrochen { log::info!("{zeile}"); }
    else { log::warn!("{zeile}"); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alle_wege_und_codes_liefern_nur_feste_werte_und_zahlen() {
        for (weg, name) in [
            (Weg::VorVersand, "vor_versand"), (Weg::Cli, "cli"),
            (Weg::Http, "http"), (Weg::Lokal, "lokal"),
        ] {
            for (code, text) in [
                (Code::CliMissing, "cli_missing"), (Code::CliLogin, "cli_login"),
                (Code::CliLimit, "cli_limit"),
                (Code::Zeitueberschreitung, "zeitueberschreitung"),
                (Code::Abgebrochen, "abgebrochen"), (Code::TorVerworfen, "tor_verworfen"),
                (Code::SchutzGesperrt, "schutz_gesperrt"), (Code::NurLokal, "nur_lokal"),
                (Code::Einstufung, "einstufung"), (Code::AnbieterFehlt, "anbieter_fehlt"),
                (Code::Netz, "netz"), (Code::Http(407), "http_status=407"),
                (Code::Unbekannt, "unbekannt"),
            ] {
                for dauer in [0, 12, u128::MAX] {
                    assert_eq!(zeile(weg, code, dauer),
                        format!("voice_call: weg={name} code={text} dauer_ms={dauer}"));
                }
            }
        }
    }
}
