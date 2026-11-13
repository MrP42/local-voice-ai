//! Download-Wege bei Proxy und Netzausfall (lva#573).
//!
//! - Verlangt das System für Hugging Face einen Proxy aus PAC-Datei,
//!   Auto-Erkennung oder eigener Einstellung, lädt die App direkt über die
//!   `resolve`-Adresse mit dem eigenen, proxyfähigen Downloader. hf-hub baut
//!   seinen Client selbst und kennt weder PAC noch WPAD.
//! - Scheitert ein Download an einem vorübergehenden Netzfehler, wartet die
//!   App gestaffelt und versucht es erneut; Teildateien bleiben erhalten.
//!   Abbrechen wirkt auch während des Wartens.

use super::download::HttpDownloadOutcome;
use super::ModelManager;
use crate::netz::fehler::{einordnen, Netzfehler};
use anyhow::Result;
use hf_hub::api::tokio::CancellationToken;
use log::{info, warn};
use std::fs;
use std::time::Duration;
use tauri::Emitter;

/// Wartezeiten vor dem nächsten Gesamtversuch, zusammen rund zehn Minuten.
pub(super) const WARTEZEITEN: [u64; 6] = [10, 20, 40, 60, 120, 300];

/// Adresse, unter der Hugging Face eine Datei ausliefert (leitet aufs CDN weiter).
pub(super) fn hf_resolve_url(repo_id: &str, revision: &str, filename: &str) -> String {
    format!("https://huggingface.co/{repo_id}/resolve/{revision}/{filename}")
}

/// Wartezeit vor Gesamtversuch `runde` (ab 0) oder `None`, wenn nicht mehr
/// gewartet wird: nur bei vorübergehenden Netzfehlern und höchstens
/// [`WARTEZEITEN`]`.len()` Mal.
pub(super) fn naechste_wartezeit(fehler: Netzfehler, runde: usize) -> Option<Duration> {
    if !fehler.voruebergehend() {
        return None;
    }
    WARTEZEITEN.get(runde).map(|s| Duration::from_secs(*s))
}

impl ModelManager {
    /// Lädt eine Hugging-Face-Datei direkt (über den Proxy des Systems) in den
    /// Modellordner, den die HF-Auflösung ebenfalls annimmt. Größe und SHA-256
    /// kommen aus dem Spiegel-Eintrag des Katalogs, wenn es einen gibt (derselbe
    /// Dateiinhalt). `Ok(false)` = abgebrochen, Teildatei bleibt.
    pub(super) async fn download_hf_direkt(
        &self,
        model_id: &str,
        filename: &str,
        url: &str,
        cancel_token: CancellationToken,
    ) -> Result<bool> {
        fs::create_dir_all(&self.models_dir)?;
        let model_path = self.models_dir.join(filename);
        let partial_path = self.models_dir.join(format!("{}.partial", filename));
        if model_path.exists() {
            return Ok(true);
        }
        let spiegel = crate::catalog::mirror_fallbacks(model_id)
            .into_iter()
            .next();
        match self
            .download_http_resumable(
                model_id,
                url,
                &partial_path,
                spiegel.as_ref().map(|m| m.size_bytes),
                spiegel.as_ref().map(|m| m.sha256.as_str()),
                &cancel_token,
            )
            .await?
        {
            HttpDownloadOutcome::Cancelled => Ok(false),
            HttpDownloadOutcome::Completed => {
                fs::rename(&partial_path, &model_path)?;
                info!(
                    "Hugging-Face-Download von {} über Proxy abgeschlossen ({:?})",
                    model_id, model_path
                );
                Ok(true)
            }
        }
    }

    /// [`Self::download_model`] mit Warten auf das Netz: Bei einem
    /// vorübergehenden Netzfehler meldet die App `model-download-waiting` und
    /// versucht es nach [`WARTEZEITEN`] erneut. `cancel_download` beendet auch
    /// das Warten (dann `Ok`). Der letzte Fehler geht unverändert zurück.
    pub async fn download_model_mit_warten(&self, model_id: &str) -> Result<()> {
        let mut runde = 0;
        loop {
            let fehler = match self.download_model(model_id).await {
                Ok(()) => return Ok(()),
                Err(e) => e,
            };
            let art = einordnen(&format!("{:#}", fehler));
            let Some(warten) = naechste_wartezeit(art, runde) else {
                return Err(fehler);
            };
            runde += 1;
            warn!(
                "Download von {} gescheitert ({}), neuer Versuch {} in {}s",
                model_id,
                art.code(),
                runde,
                warten.as_secs()
            );

            let token = CancellationToken::new();
            self.cancel_flags
                .lock()
                .unwrap()
                .insert(model_id.to_string(), token.clone());
            self.setze_laedt(model_id, true);
            let _ = self.app_handle.emit(
                "model-download-waiting",
                serde_json::json!({
                    "model_id": model_id,
                    "kind": art.code(),
                    "message": art.meldung(),
                    "retry_in_secs": warten.as_secs(),
                    "attempt": runde,
                    "max_attempts": WARTEZEITEN.len(),
                }),
            );
            let abgebrochen = tokio::select! {
                _ = tokio::time::sleep(warten) => false,
                _ = token.cancelled() => true,
            };
            // Während des Wartens zeigt die Oberfläche den Download als laufend,
            // ein zweiter Start ist dort gesperrt; das Token gehört also uns.
            self.cancel_flags.lock().unwrap().remove(model_id);
            if abgebrochen {
                info!("Warten auf das Netz für {} abgebrochen", model_id);
                return Ok(());
            }
            self.setze_laedt(model_id, false);
        }
    }

    fn setze_laedt(&self, model_id: &str, laedt: bool) {
        if let Some(model) = self.available_models.lock().unwrap().get_mut(model_id) {
            model.is_downloading = laedt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wartet_nur_bei_netzausfall_und_begrenzt() {
        let dns = Netzfehler::NameNichtAufloesbar;
        assert_eq!(naechste_wartezeit(dns, 0), Some(Duration::from_secs(10)));
        assert_eq!(
            naechste_wartezeit(dns, WARTEZEITEN.len() - 1),
            Some(Duration::from_secs(300))
        );
        assert_eq!(naechste_wartezeit(dns, WARTEZEITEN.len()), None);
        assert_eq!(naechste_wartezeit(Netzfehler::ProxyAnmeldung, 0), None);
        assert_eq!(naechste_wartezeit(Netzfehler::Sonstiges, 0), None);
        let summe: u64 = WARTEZEITEN.iter().sum();
        assert!(summe <= 600, "höchstens zehn Minuten Warten, sind {summe}s");
    }

    #[test]
    fn resolve_adresse() {
        assert_eq!(
            hf_resolve_url("org/repo", "abc123", "m.gguf"),
            "https://huggingface.co/org/repo/resolve/abc123/m.gguf"
        );
    }
}
