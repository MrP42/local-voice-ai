//! Zustand der Suite-Projekte in der Tauri-App: Dienste werden beim ersten Gebrauch geoeffnet.
//!
//! `projekte.db` und `recherche.db` liegen im Datenordner der App (wie in Corporate LLM PR-6; der
//! gemeinsame Suite-Pfad nach ADR-0026 E1 folgt mit dem Suite-Konto). Das Oeffnen ist traege, damit ein
//! defekter Speicher weder den Start noch Diktat und Vorlesen verhindert: die Befehle melden dann
//! "nicht verfuegbar".

use super::audit::RegisterAudit;
use super::dienst::ProjektDienst;
use super::quellen::{Inhalt, ProdQuellen, SeiteKurz};
use super::recherche::{ModellQuelle, RechercheDienst, RechercheStore, RueckrufModell, SearxngWeb};
use super::{lokaler_principal, Res};
use crate::managers::meetings::store::MeetingStore;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tokio::sync::OnceCell;
use wai_recherche::Modell;

/// Beide Dienste, gemeinsam geoeffnet.
pub struct Dienste {
    pub projekte: Arc<ProjektDienst>,
    pub recherche: Arc<RechercheDienst>,
}

pub struct ProjekteZustand {
    app: AppHandle,
    dienste: OnceCell<Arc<Dienste>>,
}

impl ProjekteZustand {
    pub fn neu(app: AppHandle) -> Self {
        Self {
            app,
            dienste: OnceCell::new(),
        }
    }

    /// Die Dienste; beim ersten Aufruf werden die Dateien geoeffnet.
    pub async fn dienste(&self) -> Result<Arc<Dienste>, String> {
        // Ein Fehler wird nicht gemerkt: der naechste Aufruf versucht es erneut.
        self.dienste
            .get_or_try_init(|| self.oeffnen())
            .await
            .cloned()
    }

    async fn oeffnen(&self) -> Result<Arc<Dienste>, String> {
        let store = self
            .app
            .try_state::<Arc<MeetingStore>>()
            .map(|s| s.inner().clone())
            .ok_or_else(|| "Der Besprechungsspeicher ist nicht verfügbar.".to_string())?;
        let ordner = crate::portable::app_data_dir(&self.app)
            .map_err(|e| e.to_string())?
            .join("projekte");
        std::fs::create_dir_all(&ordner).map_err(|e| e.to_string())?;
        let app = self.app.clone();
        let app_seiten = self.app.clone();
        let app_texte = self.app.clone();
        let store_texte = store.clone();
        let quellen = Arc::new(
            ProdQuellen::neu(store.clone(), move || {
                crate::library::library_list(app.clone()).map(|buecher| {
                    buecher
                        .into_iter()
                        .map(|b| Inhalt {
                            id: b.id,
                            titel: b.title,
                        })
                        .collect()
                })
            })
            .mit_seiten(move || {
                crate::commands::pages::seiten_bestand(&app_seiten).map(|alle| {
                    alle.into_iter()
                        .map(|(id, titel, lesbar, _lose)| SeiteKurz { id, titel, lesbar })
                        .collect()
                })
            })
            .mit_texten(move |link, wahl| {
                super::texte::lesen(&app_texte, &store_texte, link, wahl)
            }),
        );
        let audit = Arc::new(RegisterAudit(store));
        let rstore = RechercheStore::open(ordner.join("recherche.db"))
            .await
            .map_err(|e| e.message)?;
        let projekte = ProjektDienst::open(
            ordner.join("projekte.db"),
            quellen,
            rstore.clone(),
            lokaler_principal(),
        )
        .await
        .map_err(|e| e.message)?
        .with_audit(audit.clone())
        .with_ablage(ordner.clone());
        let projekte = Arc::new(projekte);
        let recherche = RechercheDienst::open(
            rstore,
            projekte.clone(),
            Arc::new(AppModelle {
                app: self.app.clone(),
            }),
            Arc::new(SearxngWeb::default()),
            lokaler_principal(),
        )
        .await
        .map_err(|e| e.message)?
        .with_audit(audit);
        Ok(Arc::new(Dienste {
            projekte,
            recherche: Arc::new(recherche),
        }))
    }
}

/// Startet die Pflege im Hintergrund: Abgleich der Loeschungen und Aufbewahrung der Recherchen, danach
/// stuendlich. Fehler werden nur als Code protokolliert (keine Inhalte).
pub fn pflege_starten(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Der Besprechungsspeicher wird erst spaeter im Start bereitgestellt.
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if let Some(z) = app.try_state::<ProjekteZustand>() {
                if let Ok(d) = z.dienste().await {
                    pflege(&d).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

async fn pflege(d: &Dienste) {
    if let Err(e) = d.projekte.dateien_wiederanlauf().await {
        log::warn!(
            "projekte: Wiederanlauf der Projektdateien fehlgeschlagen ({})",
            e.code
        );
    }
    if let Err(e) = d.projekte.abgleichen().await {
        log::warn!("projekte: Abgleich fehlgeschlagen ({})", e.code);
    }
    if let Err(e) = d.projekte.aufbewahrung_recherchen().await {
        log::warn!("projekte: Aufbewahrung fehlgeschlagen ({})", e.code);
    }
    if let Err(e) = d.projekte.verwaiste_recherchen_raeumen().await {
        log::warn!(
            "projekte: Aufraeumen verwaister Dokumente fehlgeschlagen ({})",
            e.code
        );
    }
    if let Err(e) = d.projekte.verwaiste_projektordner_raeumen().await {
        log::warn!(
            "projekte: Aufraeumen verwaister Projektordner fehlgeschlagen ({})",
            e.code
        );
    }
}

/// Sprachmodell der Recherche: der in den Einstellungen gewaehlte Anbieter, aufgeloest beim Aufruf.
/// Die Klassenregel (Cloud-Modell nur bis INTERNAL) prueft `RueckrufModell`.
struct AppModelle {
    app: AppHandle,
}

impl ModellQuelle for AppModelle {
    fn modell(&self, _model_id: &str) -> Arc<dyn Modell> {
        let einstellungen = crate::settings::get_settings(&self.app);
        let aufgeloest = crate::managers::meetings::llm_call::resolve_provider(&einstellungen);
        let lokal = aufgeloest
            .as_ref()
            .map(|(p, _, _)| crate::managers::llm::is_local(p))
            .unwrap_or(false);
        let name = aufgeloest
            .as_ref()
            .map(|(_, m, _)| m.clone())
            .unwrap_or_else(|_| "kein-modell".into());
        Arc::new(RueckrufModell::neu(
            &name,
            lokal,
            move |anweisung, eingabe| {
                let aufgeloest = aufgeloest.clone();
                Box::pin(async move {
                    let (provider, modell, schluessel) = aufgeloest?;
                    let antwort = crate::llm_client::send_chat_completion_checked(
                        crate::managers::usage::Purpose::Chat,
                        &provider,
                        schluessel,
                        &modell,
                        eingabe,
                        Some(anweisung),
                        None,
                        None,
                        None,
                    )
                    .await?;
                    antwort
                        .content
                        .ok_or_else(|| "Antwort ohne Inhalt".to_string())
                })
            },
        ))
    }
}

/// Ist ein Anbieter mit Modell eingestellt? Sonst der verstaendliche Grund (ohne Inhalte).
pub fn modell_bereit(app: &AppHandle) -> Res<()> {
    let einstellungen = crate::settings::get_settings(app);
    crate::managers::meetings::llm_call::resolve_provider(&einstellungen)
        .map(|_| ())
        .map_err(|m| super::Fehler::neu("recherche.no_model", m))
}
