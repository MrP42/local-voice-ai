//! Explizite Funktionswahl oder ausschließlich ein nutzbares lokales Modell.

use std::fmt;

use crate::llm_versand::{Ziel, ZielArt};
use crate::settings::{AppSettings, FunctionModel, PostProcessProvider, FN_TEXT_AUFBEREITEN};

use super::fehler::SprachFehler;

#[derive(Clone)]
pub struct Wahl {
    provider: PostProcessProvider,
    modell: String,
    api_key: String,
    art: ZielArt,
    explizit: bool,
}

impl fmt::Debug for Wahl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Wahl")
            .field("anbieter_id", &self.provider.id)
            .field("modell", &self.modell)
            .field("art", &self.art)
            .field("explizit", &self.explizit)
            .finish()
    }
}

impl PartialEq for Wahl {
    fn eq(&self, other: &Self) -> bool {
        self.provider.id == other.provider.id
            && self.provider.base_url == other.provider.base_url
            && self.modell == other.modell
            && self.art == other.art
            && self.explizit == other.explizit
    }
}

impl Wahl {
    pub fn funktion(&self) -> &'static str {
        FN_TEXT_AUFBEREITEN
    }

    pub fn art(&self) -> ZielArt {
        self.art
    }

    pub fn anbieter_id(&self) -> &str {
        &self.provider.id
    }

    pub fn modell(&self) -> &str {
        &self.modell
    }

    pub fn explizit(&self) -> bool {
        self.explizit
    }

    pub fn provider(&self) -> &PostProcessProvider {
        &self.provider
    }

    pub(super) fn api_key(&self) -> &str {
        &self.api_key
    }
}

pub fn waehle(settings: &AppSettings) -> Result<Wahl, SprachFehler> {
    let eintrag = settings
        .function_models
        .get(FN_TEXT_AUFBEREITEN)
        .filter(|wahl| !wahl.model_id.is_empty());
    let explizit = eintrag.is_some();
    let fehler = if explizit {
        SprachFehler::FunktionsmodellUnbrauchbar
    } else {
        SprachFehler::KeinLokalesModell
    };
    let id = if let Some(wahl) = eintrag {
        if !settings.llm_model_usable(&wahl.model_id) {
            return Err(fehler);
        }
        wahl.model_id.as_str()
    } else {
        let lokal = |modell: &&crate::settings::LlmModelConfig| {
            modell.enabled
                && settings.llm_connections.iter().any(|verbindung| {
                    verbindung.id == modell.connection_id
                        && verbindung.enabled
                        && verbindung.kind == crate::managers::llm::LOCAL_PROVIDER_ID
                })
                && settings.llm_model_usable(&modell.id)
        };
        settings
            .llm_models
            .iter()
            .find(lokal)
            .map(|modell| modell.id.as_str())
            .ok_or(fehler)?
    };

    // Die erzwungene Funktionswahl verhindert den stillen Standard-Rückfall.
    let mut eff = settings.clone();
    eff.function_models.insert(
        FN_TEXT_AUFBEREITEN.to_string(),
        FunctionModel {
            model_id: id.to_string(),
            effort: eintrag.and_then(|wahl| wahl.effort.clone()),
        },
    );
    let s = eff.for_function(FN_TEXT_AUFBEREITEN);
    if s.llm_active_model_id.as_deref() != Some(id) {
        return Err(fehler);
    }
    let provider = s.active_post_process_provider().ok_or(fehler)?.clone();
    let modell = s
        .post_process_models
        .get(&provider.id)
        .filter(|modell| !modell.is_empty())
        .ok_or(fehler)?
        .clone();
    let art = if crate::managers::llm::is_local(&provider) {
        ZielArt::Lokal
    } else {
        Ziel::aus_provider(&provider, &modell).art()
    };
    if !explizit && art != ZielArt::Lokal {
        return Err(fehler);
    }
    let api_key = s
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();
    Ok(Wahl {
        provider,
        modell,
        api_key,
        art,
        explizit,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::settings::{LlmConnection, LlmModelConfig};

    pub(crate) fn einstellungen(lokal: bool, cloud: bool) -> AppSettings {
        let mut s = crate::settings::get_default_settings();
        s.llm_connections.clear();
        s.llm_models.clear();
        s.function_models.clear();
        s.llm_active_model_id = None;
        if lokal {
            modell_hinzufuegen(&mut s, "local", "local", "http://127.0.0.1:0/v1", "eins");
        }
        if cloud {
            modell_hinzufuegen(
                &mut s,
                "openai",
                "openai",
                "https://api.openai.com/v1",
                "fern",
            );
        }
        s
    }

    pub(crate) fn modell_hinzufuegen(
        s: &mut AppSettings,
        verbindung: &str,
        kind: &str,
        url: &str,
        remote: &str,
    ) {
        if !s.llm_connections.iter().any(|v| v.id == verbindung) {
            s.llm_connections.push(LlmConnection {
                id: verbindung.into(),
                kind: kind.into(),
                label: verbindung.into(),
                base_url: url.into(),
                enabled: true,
                monthly_budget_usd: None,
                budget_enforced: false,
                training_opt_out: true,
            });
        }
        s.llm_models.push(LlmModelConfig {
            id: format!("{verbindung}:{remote}"),
            connection_id: verbindung.into(),
            remote_id: remote.into(),
            label: remote.into(),
            enabled: true,
            context_limit: None,
            max_input_tokens: None,
            max_output_tokens: None,
            price_input_per_mtok: None,
            price_output_per_mtok: None,
            tags: Vec::new(),
            effort: None,
            fast: false,
        });
    }

    pub(crate) fn explizit(s: &mut AppSettings, id: &str) {
        s.function_models.insert(
            FN_TEXT_AUFBEREITEN.into(),
            FunctionModel {
                model_id: id.into(),
                effort: None,
            },
        );
    }

    #[test]
    fn cloud_standard_wird_nie_stillschweigend_genutzt() {
        let mut s = einstellungen(true, true);
        s.llm_active_model_id = Some("openai:fern".into());
        let wahl = waehle(&s).unwrap();
        assert_eq!(wahl.art(), ZielArt::Lokal);
        assert_eq!(wahl.modell(), "eins");
        assert!(!wahl.explizit());
        s.llm_models.retain(|m| m.connection_id != "local");
        assert_eq!(waehle(&s), Err(SprachFehler::KeinLokalesModell));
    }

    #[test]
    fn standardmodellwechsel_aendert_den_ersten_lokalen_kandidaten_nicht() {
        let mut s = einstellungen(true, false);
        modell_hinzufuegen(&mut s, "local", "local", "http://127.0.0.1:0/v1", "zwei");
        s.llm_active_model_id = Some("local:zwei".into());
        assert_eq!(waehle(&s).unwrap().modell(), "eins");
        s.llm_active_model_id = Some("local:eins".into());
        assert_eq!(waehle(&s).unwrap().modell(), "eins");
        s.llm_active_model_id = None;
        assert_eq!(waehle(&s).unwrap().modell(), "eins");
    }

    #[test]
    fn lokale_kandidaten_muessen_aktiv_und_verbunden_sein() {
        let mut s = einstellungen(true, false);
        s.llm_models[0].enabled = false;
        assert_eq!(waehle(&s), Err(SprachFehler::KeinLokalesModell));
        s.llm_models[0].enabled = true;
        s.llm_connections[0].enabled = false;
        assert_eq!(waehle(&s), Err(SprachFehler::KeinLokalesModell));
        s.llm_connections.clear();
        assert_eq!(waehle(&s), Err(SprachFehler::KeinLokalesModell));
    }

    #[test]
    fn explizite_cloud_und_lokale_wahl() {
        let mut s = einstellungen(true, true);
        explizit(&mut s, "openai:fern");
        let cloud = waehle(&s).unwrap();
        assert_eq!(cloud.art(), ZielArt::Cloud);
        assert!(cloud.explizit());
        explizit(&mut s, "local:eins");
        let lokal = waehle(&s).unwrap();
        assert_eq!(lokal.art(), ZielArt::Lokal);
        assert!(lokal.explizit());
    }

    #[test]
    fn unbrauchbare_explizite_wahl_hat_keinen_rueckfall() {
        let mut s = einstellungen(true, true);
        explizit(&mut s, "openai:fern");
        s.llm_models
            .iter_mut()
            .find(|m| m.id == "openai:fern")
            .unwrap()
            .enabled = false;
        assert_eq!(waehle(&s), Err(SprachFehler::FunktionsmodellUnbrauchbar));
        s.llm_models
            .iter_mut()
            .find(|m| m.id == "openai:fern")
            .unwrap()
            .enabled = true;
        s.llm_connections
            .iter_mut()
            .find(|v| v.id == "openai")
            .unwrap()
            .enabled = false;
        assert_eq!(waehle(&s), Err(SprachFehler::FunktionsmodellUnbrauchbar));
        explizit(&mut s, "fehlt:modell");
        assert_eq!(waehle(&s), Err(SprachFehler::FunktionsmodellUnbrauchbar));
    }

    #[test]
    fn fehlende_cli_hat_keinen_rueckfall() {
        struct Zuruecksetzen(bool);
        impl Drop for Zuruecksetzen {
            fn drop(&mut self) {
                crate::settings::TEST_CLI_MISSING.with(|c| c.set(self.0));
            }
        }
        let _reset = Zuruecksetzen(crate::settings::TEST_CLI_MISSING.with(|c| c.replace(true)));
        let mut s = einstellungen(true, false);
        modell_hinzufuegen(&mut s, "claude_cli", "claude_cli", "cli://claude", "test");
        explizit(&mut s, "claude_cli:test");
        assert_eq!(waehle(&s), Err(SprachFehler::FunktionsmodellUnbrauchbar));
    }

    #[test]
    fn leere_wahl_ist_nicht_explizit_und_funktion_ist_registriert() {
        let mut s = einstellungen(true, true);
        s.llm_active_model_id = Some("openai:fern".into());
        explizit(&mut s, "");
        let wahl = waehle(&s).unwrap();
        assert!(!wahl.explizit());
        assert_eq!(wahl.art(), ZielArt::Lokal);
        assert_eq!(wahl.funktion(), FN_TEXT_AUFBEREITEN);
        assert!(crate::settings::LLM_FUNCTIONS.contains(&FN_TEXT_AUFBEREITEN));
    }

    #[test]
    fn vergleich_beruecksichtigt_ziel_aber_keinen_schluessel() {
        let wahl = waehle(&einstellungen(true, false)).unwrap();
        let mut andere = wahl.clone();
        andere.api_key = "geheim".into();
        assert_eq!(wahl, andere);
        andere.modell.push('2');
        assert_ne!(wahl, andere);
        andere = wahl.clone();
        andere.art = ZielArt::Cloud;
        assert_ne!(wahl, andere);
        andere = wahl.clone();
        andere.provider.base_url.push('/');
        assert_ne!(wahl, andere);
        andere = wahl.clone();
        andere.provider.id.push('2');
        assert_ne!(wahl, andere);
        andere = wahl.clone();
        andere.explizit = true;
        assert_ne!(wahl, andere);
        let debug = format!("{wahl:?}");
        assert!(!debug.contains(&wahl.provider.base_url));
        assert!(!debug.contains("http"));
    }
}
