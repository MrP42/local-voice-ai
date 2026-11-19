//! Ein Versandversuch mit frischer Wahlprüfung; niemals ein automatischer Zielwechsel.

use std::fmt;
use std::sync::Arc;

use crate::llm_versand::{
    Anfrage, Antwort, AppFreigabeQuelle, ChatInhalt, FreigabeQuelle, Inhalt, Quellen,
    VersandFehler, Ziel, ZielArt,
};
use crate::settings::{AppSettings, FN_TEXT_AUFBEREITEN};

use super::fehler::SprachFehler;
use super::politik::{waehle, Wahl};

pub struct Auftrag {
    pub funktion: &'static str,
    pub system: Option<String>,
    pub nutzer: String,
}

impl Auftrag {
    pub fn neu(system: Option<String>, nutzer: String) -> Self {
        Self {
            funktion: FN_TEXT_AUFBEREITEN,
            system,
            nutzer,
        }
    }
}

impl fmt::Debug for Auftrag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auftrag")
            .field("system_laenge", &self.system.as_ref().map(String::len))
            .field("nutzer_laenge", &self.nutzer.len())
            .finish()
    }
}

pub struct ModellAntwort {
    pub text: String,
    pub abgeschnitten: bool,
}

impl fmt::Debug for ModellAntwort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModellAntwort")
            .field("text_laenge", &self.text.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersandAusfall {
    Gesperrt,
    Zeitlimit,
    Abgebrochen,
    Fehler,
}

#[async_trait::async_trait]
pub trait Versender: Send + Sync {
    async fn senden(
        &self,
        wahl: &Wahl,
        ziel: &Ziel,
        auftrag: &Auftrag,
    ) -> Result<ModellAntwort, VersandAusfall>;
}

#[async_trait::async_trait]
pub trait ZielAufloeser: Send + Sync {
    async fn aufloesen(&self, wahl: &Wahl) -> Result<Ziel, SprachFehler>;
}

pub struct LlmZielAufloeser;

#[async_trait::async_trait]
impl ZielAufloeser for LlmZielAufloeser {
    async fn aufloesen(&self, wahl: &Wahl) -> Result<Ziel, SprachFehler> {
        if crate::managers::llm::is_local(wahl.provider()) {
            let url = crate::managers::llm::resolve_base_url(wahl.provider(), wahl.modell())
                .await
                .map_err(|_| SprachFehler::ZielAufloesung)?;
            Ok(Ziel::eingebaut(wahl.modell(), &url))
        } else {
            Ok(Ziel::aus_provider(wahl.provider(), wahl.modell()))
        }
    }
}

pub struct LlmVersender {
    quelle: Arc<dyn FreigabeQuelle>,
}

impl LlmVersender {
    pub fn neu(quelle: Arc<dyn FreigabeQuelle>) -> Self {
        Self { quelle }
    }
}

fn ausfall(fehler: VersandFehler) -> VersandAusfall {
    match fehler {
        VersandFehler::Gesperrt { .. }
        | VersandFehler::Compliance { .. }
        | VersandFehler::Budget { .. } => VersandAusfall::Gesperrt,
        VersandFehler::Zeitueberschreitung => VersandAusfall::Zeitlimit,
        VersandFehler::Abgebrochen => VersandAusfall::Abgebrochen,
        _ => VersandAusfall::Fehler,
    }
}

fn modellantwort(antwort: Antwort) -> ModellAntwort {
    ModellAntwort {
        text: antwort.text.unwrap_or_default(),
        abgeschnitten: antwort.abgeschnitten,
    }
}

#[async_trait::async_trait]
impl Versender for LlmVersender {
    async fn senden(
        &self,
        wahl: &Wahl,
        ziel: &Ziel,
        auftrag: &Auftrag,
    ) -> Result<ModellAntwort, VersandAusfall> {
        crate::llm_versand::versenden(
            &*self.quelle,
            ziel,
            &Quellen::keine(),
            Anfrage {
                zweck: crate::managers::usage::Purpose::PostProcess,
                anbieter: wahl.provider(),
                api_key: wahl.api_key().to_string(),
                inhalt: Inhalt::Chat(ChatInhalt {
                    nutzer: auftrag.nutzer.clone(),
                    system: auftrag.system.clone(),
                    schema: None,
                    reasoning_effort: None,
                    reasoning: None,
                }),
            },
        )
        .await
        .map(modellantwort)
        .map_err(ausfall)
    }
}

pub struct Sprachversand {
    einstellungen: Arc<dyn Fn() -> AppSettings + Send + Sync>,
    aufloeser: Arc<dyn ZielAufloeser>,
    versender: Arc<dyn Versender>,
}

impl Sprachversand {
    pub fn neu(
        einstellungen: Arc<dyn Fn() -> AppSettings + Send + Sync>,
        aufloeser: Arc<dyn ZielAufloeser>,
        versender: Arc<dyn Versender>,
    ) -> Self {
        Self {
            einstellungen,
            aufloeser,
            versender,
        }
    }

    pub fn aus_app(app: &tauri::AppHandle) -> Result<Sprachversand, SprachFehler> {
        let quelle = AppFreigabeQuelle::aus_app(app).map_err(|_| SprachFehler::VersandGesperrt)?;
        let app = app.clone();
        Ok(Self::neu(
            Arc::new(move || crate::settings::get_settings(&app)),
            Arc::new(LlmZielAufloeser),
            Arc::new(LlmVersender::neu(Arc::new(quelle))),
        ))
    }

    pub fn vorschau(&self) -> Result<Wahl, SprachFehler> {
        waehle(&(self.einstellungen)())
    }

    pub async fn anfragen(
        &self,
        angekuendigt: &Wahl,
        auftrag: &Auftrag,
    ) -> Result<ModellAntwort, SprachFehler> {
        let wahl = self.vorschau()?;
        if wahl != *angekuendigt {
            return Err(SprachFehler::ZielGewechselt);
        }
        let ziel = self.aufloeser.aufloesen(&wahl).await?;
        if ziel.art() != wahl.art() || ziel.anbieter_id() != wahl.anbieter_id() {
            return Err(SprachFehler::ZielGewechselt);
        }
        if auftrag.funktion != FN_TEXT_AUFBEREITEN {
            return Err(SprachFehler::ModellFehler);
        }
        // Genau ein Versuch; auch lokale Fehler erlauben keinen Cloud-Rückfall.
        self.versender
            .senden(&wahl, &ziel, auftrag)
            .await
            .map_err(|fehler| match fehler {
                VersandAusfall::Gesperrt => SprachFehler::VersandGesperrt,
                VersandAusfall::Zeitlimit => SprachFehler::Zeitlimit,
                VersandAusfall::Abgebrochen => SprachFehler::Abgebrochen,
                VersandAusfall::Fehler if wahl.art() == ZielArt::Lokal => {
                    SprachFehler::LokalAusgefallen
                }
                VersandAusfall::Fehler => SprachFehler::ModellFehler,
            })
    }
}

#[cfg(test)]
pub(crate) mod mock {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    #[derive(Debug, Clone)]
    pub struct MockAufruf {
        pub funktion: &'static str,
        pub ziel_art: ZielArt,
        pub anbieter_id: String,
        pub modell: String,
        pub nutzer_laenge: usize,
    }

    type Antworter = dyn Fn(&Auftrag, usize) -> Result<ModellAntwort, VersandAusfall> + Send + Sync;

    enum Verhalten {
        Skript(VecDeque<Result<ModellAntwort, VersandAusfall>>),
        Antworter(Box<Antworter>),
    }

    struct Zustand {
        verhalten: Verhalten,
        aufrufe: Vec<MockAufruf>,
    }

    pub struct MockVersender {
        zustand: Mutex<Zustand>,
    }

    impl MockVersender {
        pub fn skript(skript: Vec<Result<ModellAntwort, VersandAusfall>>) -> Self {
            Self {
                zustand: Mutex::new(Zustand {
                    verhalten: Verhalten::Skript(skript.into()),
                    aufrufe: Vec::new(),
                }),
            }
        }

        pub fn antworter(
            antworter: impl Fn(&Auftrag, usize) -> Result<ModellAntwort, VersandAusfall>
                + Send
                + Sync
                + 'static,
        ) -> Self {
            Self {
                zustand: Mutex::new(Zustand {
                    verhalten: Verhalten::Antworter(Box::new(antworter)),
                    aufrufe: Vec::new(),
                }),
            }
        }

        pub fn aufrufe(&self) -> Vec<MockAufruf> {
            self.zustand.lock().unwrap().aufrufe.clone()
        }

        pub fn anzahl(&self) -> usize {
            self.zustand.lock().unwrap().aufrufe.len()
        }

        pub fn cloud_aufrufe(&self) -> usize {
            self.aufrufe()
                .iter()
                .filter(|a| a.ziel_art == ZielArt::Cloud)
                .count()
        }

        pub fn lokal_aufrufe(&self) -> usize {
            self.aufrufe()
                .iter()
                .filter(|a| a.ziel_art == ZielArt::Lokal)
                .count()
        }
    }

    #[async_trait::async_trait]
    impl Versender for MockVersender {
        async fn senden(
            &self,
            wahl: &Wahl,
            ziel: &Ziel,
            auftrag: &Auftrag,
        ) -> Result<ModellAntwort, VersandAusfall> {
            let mut zustand = self.zustand.lock().unwrap();
            let nummer = zustand.aufrufe.len();
            zustand.aufrufe.push(MockAufruf {
                funktion: auftrag.funktion,
                ziel_art: ziel.art(),
                anbieter_id: ziel.anbieter_id().into(),
                modell: wahl.modell().into(),
                nutzer_laenge: auftrag.nutzer.len(),
            });
            match &mut zustand.verhalten {
                Verhalten::Skript(skript) => {
                    skript.pop_front().unwrap_or(Err(VersandAusfall::Fehler))
                }
                Verhalten::Antworter(antworter) => antworter(auftrag, nummer),
            }
        }
    }

    pub struct MockAufloeser {
        erzwungen: Option<Result<Ziel, SprachFehler>>,
    }

    impl MockAufloeser {
        pub fn neu() -> Self {
            Self { erzwungen: None }
        }

        pub fn erzwinge(ziel: Ziel) -> Self {
            Self {
                erzwungen: Some(Ok(ziel)),
            }
        }

        pub fn fehlschlag() -> Self {
            Self {
                erzwungen: Some(Err(SprachFehler::ZielAufloesung)),
            }
        }
    }

    #[async_trait::async_trait]
    impl ZielAufloeser for MockAufloeser {
        async fn aufloesen(&self, wahl: &Wahl) -> Result<Ziel, SprachFehler> {
            if let Some(ergebnis) = &self.erzwungen {
                return ergebnis.clone();
            }
            if wahl.art() == ZielArt::Lokal {
                Ok(Ziel::eingebaut(wahl.modell(), "http://127.0.0.1:4321/v1"))
            } else {
                Ok(Ziel::aus_provider(wahl.provider(), wahl.modell()))
            }
        }
    }

    pub fn test_versand(s: AppSettings, versender: Arc<MockVersender>) -> Sprachversand {
        Sprachversand::neu(
            Arc::new(move || s.clone()),
            Arc::new(MockAufloeser::neu()),
            versender,
        )
    }

    pub fn test_versand_veraenderlich(
        s: Arc<Mutex<AppSettings>>,
        versender: Arc<MockVersender>,
    ) -> Sprachversand {
        Sprachversand::neu(
            Arc::new(move || s.lock().unwrap().clone()),
            Arc::new(MockAufloeser::neu()),
            versender,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::politik::tests::{einstellungen, explizit, modell_hinzufuegen};
    use super::mock::*;
    use super::*;

    fn auftrag() -> Auftrag {
        Auftrag::neu(
            Some("System-MARKE".into()),
            "Nutzer-MARKE ä🙂e\u{301}".into(),
        )
    }

    fn antwort(text: &str) -> ModellAntwort {
        ModellAntwort {
            text: text.into(),
            abgeschnitten: false,
        }
    }

    #[tokio::test]
    async fn lokaler_standard_und_explizite_cloud_senden_mit_funktionskennung() {
        for cloud in [false, true] {
            let mut s = einstellungen(true, true);
            s.llm_active_model_id = Some("local:eins".into());
            if cloud {
                explizit(&mut s, "openai:fern");
            }
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("fertig"))]));
            let versand = test_versand(s, mock.clone());
            let wahl = versand.vorschau().unwrap();
            let auftrag = auftrag();
            assert_eq!(
                versand.anfragen(&wahl, &auftrag).await.unwrap().text,
                "fertig"
            );
            assert_eq!(mock.anzahl(), 1);
            assert_eq!(mock.cloud_aufrufe(), usize::from(cloud));
            assert_eq!(mock.lokal_aufrufe(), usize::from(!cloud));
            let a = &mock.aufrufe()[0];
            assert_eq!(a.funktion, "text_aufbereiten");
            assert_eq!(a.ziel_art, wahl.art());
            assert_eq!(a.anbieter_id, wahl.anbieter_id());
            assert_eq!(a.modell, wahl.modell());
            assert_eq!(a.nutzer_laenge, auftrag.nutzer.len());
            assert!(!format!("{a:?}").contains("Nutzer-MARKE"));
        }
    }

    #[tokio::test]
    async fn ausfaelle_bleiben_bei_genau_einem_versuch_ohne_cloud_rueckfall() {
        for (ausfall, erwartet) in [
            (VersandAusfall::Fehler, SprachFehler::LokalAusgefallen),
            (VersandAusfall::Zeitlimit, SprachFehler::Zeitlimit),
            (VersandAusfall::Gesperrt, SprachFehler::VersandGesperrt),
            (VersandAusfall::Abgebrochen, SprachFehler::Abgebrochen),
        ] {
            let mut s = einstellungen(true, true);
            s.llm_active_model_id = Some("openai:fern".into());
            explizit(&mut s, "local:eins");
            let mock = Arc::new(MockVersender::skript(vec![
                Err(ausfall),
                Ok(antwort("zweiter")),
            ]));
            let versand = test_versand(s, mock.clone());
            let wahl = versand.vorschau().unwrap();
            assert_eq!(
                versand.anfragen(&wahl, &auftrag()).await.unwrap_err(),
                erwartet
            );
            assert_eq!(mock.anzahl(), 1);
            assert_eq!(mock.cloud_aufrufe(), 0);
        }
        let mut s = einstellungen(false, true);
        explizit(&mut s, "openai:fern");
        let mock = Arc::new(MockVersender::skript(vec![Err(VersandAusfall::Fehler)]));
        let versand = test_versand(s, mock.clone());
        let wahl = versand.vorschau().unwrap();
        assert_eq!(
            versand.anfragen(&wahl, &auftrag()).await.unwrap_err(),
            SprachFehler::ModellFehler
        );
        assert_eq!(mock.anzahl(), 1);
    }

    #[tokio::test]
    async fn cloud_standard_ohne_lokales_modell_sendet_nicht() {
        let angekuendigt = waehle(&einstellungen(true, false)).unwrap();
        let mut s = einstellungen(false, true);
        s.llm_active_model_id = Some("openai:fern".into());
        let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("unerlaubt"))]));
        let versand = test_versand(s, mock.clone());
        assert_eq!(
            versand
                .anfragen(&angekuendigt, &auftrag())
                .await
                .unwrap_err(),
            SprachFehler::KeinLokalesModell
        );
        assert_eq!(mock.anzahl(), 0);
    }

    #[tokio::test]
    async fn einstellungswechsel_verhindert_versand() {
        for cloud in [false, true] {
            let mut s = einstellungen(true, true);
            modell_hinzufuegen(&mut s, "local", "local", "http://127.0.0.1:0/v1", "zwei");
            let s = Arc::new(std::sync::Mutex::new(s));
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("unerlaubt"))]));
            let versand = test_versand_veraenderlich(s.clone(), mock.clone());
            let wahl = versand.vorschau().unwrap();
            explizit(
                &mut s.lock().unwrap(),
                if cloud { "openai:fern" } else { "local:zwei" },
            );
            assert_eq!(
                versand.anfragen(&wahl, &auftrag()).await.unwrap_err(),
                SprachFehler::ZielGewechselt
            );
            assert_eq!(mock.anzahl(), 0);
        }
    }

    #[tokio::test]
    async fn falsches_aufgeloestes_ziel_und_aufloesefehler_senden_nicht() {
        let mut cloud = einstellungen(false, true);
        explizit(&mut cloud, "openai:fern");
        let cloud = waehle(&cloud).unwrap();
        for (aufloeser, erwartet) in [
            (
                MockAufloeser::erzwinge(Ziel::aus_provider(cloud.provider(), cloud.modell())),
                SprachFehler::ZielGewechselt,
            ),
            (MockAufloeser::fehlschlag(), SprachFehler::ZielAufloesung),
        ] {
            let s = einstellungen(true, false);
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("unerlaubt"))]));
            let versand = Sprachversand::neu(
                Arc::new(move || s.clone()),
                Arc::new(aufloeser),
                mock.clone(),
            );
            let wahl = versand.vorschau().unwrap();
            assert_eq!(
                versand.anfragen(&wahl, &auftrag()).await.unwrap_err(),
                erwartet
            );
            assert_eq!(mock.anzahl(), 0);
        }
    }

    #[tokio::test]
    async fn fremde_funktionskennung_sendet_nicht() {
        let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("unerlaubt"))]));
        let versand = test_versand(einstellungen(true, false), mock.clone());
        let wahl = versand.vorschau().unwrap();
        let mut auftrag = auftrag();
        auftrag.funktion = "fremd";
        assert_eq!(
            versand.anfragen(&wahl, &auftrag).await.unwrap_err(),
            SprachFehler::ModellFehler
        );
        assert_eq!(mock.anzahl(), 0);
    }

    #[tokio::test]
    async fn anderer_anbieter_bei_gleicher_art_sendet_nicht() {
        let mut s = einstellungen(false, true);
        explizit(&mut s, "openai:fern");
        let wahl = waehle(&s).unwrap();
        let mut provider = wahl.provider().clone();
        provider.id = "anderer_anbieter".into();
        let ziel = Ziel::aus_provider(&provider, wahl.modell());
        assert_eq!(ziel.art(), wahl.art());
        assert_ne!(ziel.anbieter_id(), wahl.anbieter_id());
        let mock = Arc::new(MockVersender::skript(vec![Ok(antwort("unerlaubt"))]));
        let versand = Sprachversand::neu(
            Arc::new(move || s.clone()),
            Arc::new(MockAufloeser::erzwinge(ziel)),
            mock.clone(),
        );
        assert_eq!(
            versand.anfragen(&wahl, &auftrag()).await.unwrap_err(),
            SprachFehler::ZielGewechselt
        );
        assert_eq!(mock.anzahl(), 0);
    }

    #[tokio::test]
    async fn antworter_ist_deterministisch_und_erhaelt_aufrufnummern() {
        let mock = Arc::new(MockVersender::antworter(|auftrag, nummer| {
            assert!(nummer < 2);
            Ok(antwort(&auftrag.nutzer))
        }));
        let versand = test_versand(einstellungen(true, false), mock.clone());
        let wahl = versand.vorschau().unwrap();
        let a = auftrag();
        let erste = versand.anfragen(&wahl, &a).await.unwrap();
        let zweite = versand.anfragen(&wahl, &a).await.unwrap();
        assert_eq!(erste.text, a.nutzer);
        assert_eq!(erste.text, zweite.text);
        assert_eq!(mock.anzahl(), 2);

        let nummern = MockVersender::antworter(|_, nummer| Ok(antwort(&nummer.to_string())));
        let ziel = MockAufloeser::neu().aufloesen(&wahl).await.unwrap();
        assert_eq!(nummern.senden(&wahl, &ziel, &a).await.unwrap().text, "0");
        assert_eq!(nummern.senden(&wahl, &ziel, &a).await.unwrap().text, "1");
    }

    #[tokio::test]
    async fn skript_wird_verbraucht_und_bewahrt_abschnittsstatus() {
        let mock = MockVersender::skript(vec![Ok(ModellAntwort {
            text: "eins".into(),
            abgeschnitten: true,
        })]);
        let wahl = waehle(&einstellungen(true, false)).unwrap();
        let ziel = MockAufloeser::neu().aufloesen(&wahl).await.unwrap();
        let erste = mock.senden(&wahl, &ziel, &auftrag()).await.unwrap();
        assert_eq!(erste.text, "eins");
        assert!(erste.abgeschnitten);
        assert_eq!(
            mock.senden(&wahl, &ziel, &auftrag()).await.unwrap_err(),
            VersandAusfall::Fehler
        );
    }

    #[test]
    fn debug_zeigt_keinen_text_und_keinen_systemprompt() {
        let auftrag = auftrag();
        let debug = format!("{auftrag:?}");
        assert!(!debug.contains("System-MARKE"));
        assert!(!debug.contains("Nutzer-MARKE"));
        let antwort = antwort("Antwort-MARKE");
        assert!(!format!("{antwort:?}").contains("Antwort-MARKE"));
    }

    #[test]
    fn transportfehler_werden_ohne_fremdtexte_abgebildet() {
        for fehler in [
            VersandFehler::Gesperrt {
                code: "code",
                grund: "fremd",
            },
            VersandFehler::Compliance {
                meldung: "fremd".into(),
            },
            VersandFehler::Budget {
                meldung: "fremd".into(),
            },
        ] {
            assert_eq!(ausfall(fehler), VersandAusfall::Gesperrt);
        }
        assert_eq!(
            ausfall(VersandFehler::Zeitueberschreitung),
            VersandAusfall::Zeitlimit
        );
        assert_eq!(
            ausfall(VersandFehler::Abgebrochen),
            VersandAusfall::Abgebrochen
        );
        for fehler in [
            VersandFehler::Kontextueberlauf,
            VersandFehler::UngueltigesJson,
            VersandFehler::ZuWenigRam,
            VersandFehler::Absturz,
            VersandFehler::CliUpdate,
            VersandFehler::Cli {
                code: "code",
                hinweis: Some("fremd"),
            },
            VersandFehler::Http { status: 503 },
            VersandFehler::Transport,
            VersandFehler::Protokoll,
        ] {
            assert_eq!(ausfall(fehler), VersandAusfall::Fehler);
        }
    }

    #[test]
    fn fehlender_antworttext_ist_leer_und_abschnittsstatus_bleibt_erhalten() {
        let leer = modellantwort(Antwort {
            text: None,
            abgeschnitten: true,
        });
        assert!(leer.text.is_empty());
        assert!(leer.abgeschnitten);
        let voll = modellantwort(Antwort {
            text: Some("ä🙂e\u{301}".into()),
            abgeschnitten: false,
        });
        assert_eq!(voll.text, "ä🙂e\u{301}");
        assert!(!voll.abgeschnitten);
    }
}
