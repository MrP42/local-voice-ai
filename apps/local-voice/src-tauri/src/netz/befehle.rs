//! Proxy-Wahl für den JavaScript-Updater (lva#640).

use super::proxy::{weg_fuer_async, Weg};

/// Liefert den Proxy für das Update-Manifest, ohne ihn zu protokollieren.
#[tauri::command]
#[specta::specta]
pub async fn update_proxy() -> Option<String> {
    let weg = weg_fuer_async(
        "https://github.com/MrP42/local-voice-ai/releases/latest/download/latest.json",
    )
    .await;
    // Mit Windows-Anmeldung (Negotiate/NTLM) ueber den lokalen Weiterleitungsproxy (lva#641).
    proxy_fuer_updater(&weg).map(|proxy| super::proxy_auth::vermittelt(&proxy))
}

/// Zieladressen der App fuer die IT-Freigabe (lva#739), je Funktion mit Port und Zweck.
#[tauri::command]
#[specta::specta]
pub fn netz_ziele() -> Vec<super::ziele::NetzZiel> {
    super::ziele::ZIELE.to_vec()
}

/// Dieselbe Liste als Klartext zum Kopieren.
#[tauri::command]
#[specta::specta]
pub fn netz_ziele_text() -> String {
    super::ziele::klartext()
}

fn proxy_fuer_updater(weg: &Weg) -> Option<String> {
    match weg {
        Weg::Ueber(proxy) => Some(proxy.clone()),
        Weg::Direkt | Weg::Standard => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adressbefehle_liefern_die_vollstaendige_liste() {
        assert_eq!(netz_ziele(), super::super::ziele::ZIELE);
        assert_eq!(netz_ziele_text(), super::super::ziele::klartext());
    }

    #[test]
    fn nur_ein_ausdruecklicher_proxy_wird_weitergegeben() {
        let proxy = "http://proxy.example.invalid:8080";
        assert_eq!(
            proxy_fuer_updater(&Weg::Ueber(proxy.to_string())),
            Some(proxy.to_string())
        );
        assert_eq!(proxy_fuer_updater(&Weg::Direkt), None);
        assert_eq!(proxy_fuer_updater(&Weg::Standard), None);
    }
}
