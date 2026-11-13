//! Download-Wege bei Proxy und Netzausfall (lva#573).
//!
//! - Gibt das System (PAC-Datei, Auto-Erkennung) oder die eigene Einstellung
//!   für Hugging Face einen Weg vor – Proxy oder ausdrücklich direkt –, lädt die
//!   App über die `resolve`-Adresse mit dem eigenen Downloader. hf-hub baut
//!   seinen Client selbst und kennt weder PAC noch WPAD noch die Einstellung.
//!   Größe und SHA-256 kommen aus dem Katalog-Spiegel oder aus den
//!   `X-Linked-*`-Kopfzeilen von Hugging Face; ohne SHA-256 kein Download.
//! - Scheitert ein Download an einem vorübergehenden Netzfehler, wartet die
//!   App gestaffelt und versucht es erneut. Abbrechen wirkt auch zwischen den
//!   Phasen (Abbruchzähler je Modell) und während des Wartens; je Modell läuft
//!   höchstens eine solche Schleife.

use super::download::HttpDownloadOutcome;
use super::ModelManager;
use crate::netz::fehler::{einordnen, Netzfehler};
use crate::netz::proxy::Weg;
use anyhow::{anyhow, Result};
use hf_hub::api::tokio::CancellationToken;
use log::{info, warn};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Mutex;
use std::time::Duration;
use tauri::Emitter;

/// Wartezeiten vor dem nächsten Gesamtversuch, zusammen rund zehn Minuten.
pub(super) const WARTEZEITEN: [u64; 6] = [10, 20, 40, 60, 120, 300];

/// Frist für die Abfrage der Prüfdaten bei Hugging Face.
const PRUEFDATEN_FRIST: Duration = Duration::from_secs(30);

/// Modelle, für die gerade eine Download-Schleife läuft.
static LAUFEND: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// Abbrüche je Modell (zählt nur hoch); die Schleife vergleicht vor jedem Warten.
static ABBRUECHE: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Vermerkt einen Abbruch durch den Nutzer (aus `cancel_download`), damit auch
/// ein Abbruch zwischen gescheitertem Versuch und Wartebeginn wirkt.
pub(super) fn abbruch_vermerken(model_id: &str) {
    if let Ok(mut m) = ABBRUECHE.lock() {
        *m.get_or_insert_with(HashMap::new)
            .entry(model_id.to_string())
            .or_insert(0) += 1;
    }
}

fn abbrueche(model_id: &str) -> u64 {
    ABBRUECHE
        .lock()
        .ok()
        .and_then(|m| m.as_ref().and_then(|m| m.get(model_id).copied()))
        .unwrap_or(0)
}

/// Hält den Eintrag in [`LAUFEND`] bis zum Ende der Schleife.
struct Laufend(String);

impl Laufend {
    fn beginnen(model_id: &str) -> Option<Self> {
        let mut m = LAUFEND.lock().ok()?;
        m.get_or_insert_with(HashSet::new)
            .insert(model_id.to_string())
            .then(|| Laufend(model_id.to_string()))
    }
}

impl Drop for Laufend {
    fn drop(&mut self) {
        if let Ok(mut m) = LAUFEND.lock() {
            if let Some(m) = m.as_mut() {
                m.remove(&self.0);
            }
        }
    }
}

/// Adresse, unter der Hugging Face eine Datei ausliefert (leitet aufs CDN weiter).
pub(super) fn hf_resolve_url(repo_id: &str, revision: &str, filename: &str) -> String {
    format!("https://huggingface.co/{repo_id}/resolve/{revision}/{filename}")
}

/// Lädt die App HF-Dateien selbst statt über hf-hub? Immer, wenn das System
/// oder die Einstellung einen Weg vorgibt (Proxy oder ausdrücklich direkt);
/// nur beim Standardweg bleibt hf-hub (dessen Client liest Umgebung und
/// statischen Systemproxy genauso wie reqwest).
pub(super) fn hf_selbst_laden(weg: &Weg) -> bool {
    !matches!(weg, Weg::Standard)
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

/// Liest SHA-256 und Größe aus den Kopfzeilen einer HF-`resolve`-Antwort
/// (`X-Linked-Etag`/`X-Linked-Size` bei LFS-Dateien, sonst `ETag` und
/// `Content-Length`, wenn das ETag ein SHA-256 ist). Nur ein vollständiger
/// 64-stelliger Hexwert zählt.
pub(super) fn hf_pruefdaten_aus(kopf: &reqwest::header::HeaderMap) -> Option<(u64, String)> {
    let text = |name: &str| kopf.get(name).and_then(|v| v.to_str().ok());
    let sha = |v: &str| {
        let v = v.trim().trim_start_matches("W/").trim_matches('"');
        (v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit())).then(|| v.to_ascii_lowercase())
    };
    let linked = text("x-linked-etag")
        .and_then(sha)
        .zip(text("x-linked-size").and_then(|s| s.trim().parse().ok()));
    linked
        .or_else(|| {
            text("etag")
                .and_then(sha)
                .zip(text("content-length").and_then(|s| s.trim().parse().ok()))
        })
        .map(|(sha, size)| (size, sha))
}

impl ModelManager {
    /// Größe und SHA-256 einer HF-Datei: aus dem Katalog-Spiegel (derselbe
    /// Inhalt), sonst per HEAD auf die `resolve`-Adresse über denselben Weg.
    async fn hf_pruefdaten(model_id: &str, url: &str) -> Result<(u64, String)> {
        if let Some(m) = crate::catalog::mirror_fallbacks(model_id)
            .into_iter()
            .next()
        {
            return Ok((m.size_bytes, m.sha256));
        }
        let client = crate::netz::proxy::client_fuer(url)
            .await
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15))
            .timeout(PRUEFDATEN_FRIST)
            .build()?;
        let antwort = client.head(url).send().await?;
        let status = antwort.status();
        if !(status.is_success() || status.is_redirection()) {
            return Err(anyhow!("server returned HTTP {}", status));
        }
        hf_pruefdaten_aus(antwort.headers())
            .ok_or_else(|| anyhow!("Hugging Face liefert keine SHA-256 für {}", model_id))
    }

    /// Lädt eine Hugging-Face-Datei selbst (über den vom System bzw. von der
    /// Einstellung vorgegebenen Weg) in den Modellordner, den die HF-Auflösung
    /// ebenfalls annimmt, mit Größen- und SHA-256-Prüfung.
    /// `Ok(false)` = abgebrochen, Teildatei bleibt.
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
        let (groesse, sha) = tokio::select! {
            r = Self::hf_pruefdaten(model_id, url) => r?,
            _ = cancel_token.cancelled() => return Ok(false),
        };
        match self
            .download_http_resumable(
                model_id,
                url,
                &partial_path,
                Some(groesse),
                Some(&sha),
                &cancel_token,
            )
            .await?
        {
            HttpDownloadOutcome::Cancelled => Ok(false),
            HttpDownloadOutcome::Completed => {
                fs::rename(&partial_path, &model_path)?;
                info!(
                    "Hugging-Face-Download von {} über eigenen Weg abgeschlossen ({:?})",
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
    /// Läuft für das Modell schon eine Schleife, endet ein zweiter Aufruf
    /// sofort mit `Ok` (der laufende meldet Fortschritt und Ergebnis).
    pub async fn download_model_mit_warten(&self, model_id: &str) -> Result<()> {
        let Some(_laufend) = Laufend::beginnen(model_id) else {
            info!("Download von {} läuft bereits", model_id);
            return Ok(());
        };
        let abbrueche_beim_start = abbrueche(model_id);
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

            // Erst das Token eintragen, dann den Abbruchzähler lesen: ein
            // Abbruch davor ist am Zähler sichtbar, einer danach am Token.
            let token = CancellationToken::new();
            self.cancel_flags
                .lock()
                .unwrap()
                .insert(model_id.to_string(), token.clone());
            if abbrueche(model_id) != abbrueche_beim_start {
                self.cancel_flags.lock().unwrap().remove(model_id);
                info!(
                    "Download von {} wurde abgebrochen; kein neuer Versuch",
                    model_id
                );
                return Ok(());
            }
            warn!(
                "Download von {} gescheitert ({}), neuer Versuch {} in {}s",
                model_id,
                art.code(),
                runde,
                warten.as_secs()
            );
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
            // Je Modell läuft nur diese Schleife ([`Laufend`]); das Token ist unseres.
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
    use reqwest::header::{HeaderMap, HeaderValue};

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

    #[test]
    fn eigener_weg_auch_bei_direkt() {
        assert!(hf_selbst_laden(&Weg::Direkt));
        assert!(hf_selbst_laden(&Weg::Ueber(
            "http://p.example.org:1".into()
        )));
        assert!(!hf_selbst_laden(&Weg::Standard));
    }

    fn kopf(paare: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in paare {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn pruefdaten_nur_mit_vollem_sha256() {
        let sha = "a".repeat(64);
        assert_eq!(
            hf_pruefdaten_aus(&kopf(&[
                ("x-linked-etag", &format!("\"{sha}\"")),
                ("x-linked-size", "1234"),
            ])),
            Some((1234, sha.clone()))
        );
        assert_eq!(
            hf_pruefdaten_aus(&kopf(&[
                ("etag", &format!("W/\"{}\"", sha.to_uppercase())),
                ("content-length", "7")
            ])),
            Some((7, sha.clone()))
        );
        // Git-SHA-1 (40 Stellen) ist keine Prüfsumme des Inhalts.
        assert_eq!(
            hf_pruefdaten_aus(&kopf(&[
                ("etag", &format!("\"{}\"", "b".repeat(40))),
                ("content-length", "7")
            ])),
            None
        );
        assert_eq!(
            hf_pruefdaten_aus(&kopf(&[
                ("x-linked-etag", &"z".repeat(64)),
                ("x-linked-size", "1")
            ])),
            None
        );
        assert_eq!(hf_pruefdaten_aus(&kopf(&[("x-linked-etag", &sha)])), None);
    }

    #[test]
    fn nur_eine_schleife_je_modell() {
        let a = Laufend::beginnen("netz-test-modell").expect("erste");
        assert!(Laufend::beginnen("netz-test-modell").is_none());
        drop(a);
        assert!(Laufend::beginnen("netz-test-modell").is_some());
    }

    #[test]
    fn abbruch_wird_gezaehlt() {
        let vorher = abbrueche("netz-test-abbruch");
        abbruch_vermerken("netz-test-abbruch");
        assert_eq!(abbrueche("netz-test-abbruch"), vorher + 1);
    }
}
