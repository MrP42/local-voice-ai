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
    proxy_fuer_updater(&weg)
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
