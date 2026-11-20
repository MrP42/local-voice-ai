//! Geprüfte Anwendungseinstiege mit produktiver Freigabequelle und belegter Herkunft.
//!
//! Jeder Aufruf prüft frisch über `versenden`. Nur beim eingebauten lokalen Anbieter laufen
//! Compliance und Budget schon vor dem möglichen Serverstart; bei Cloud hat die
//! Besprechungssperre Vorrang. Die Zielart folgt allein der aufgelösten Route.

use super::pruefer;
use super::uebergang::{vorpruefung, ziel_aufloesen};
use super::{versenden, Anfrage, ChatInhalt, Inhalt, Quellen};
use crate::llm_client::ChatReply;
use crate::managers::llm::cli::Cli;
use crate::managers::usage::Purpose;
use crate::settings::PostProcessProvider;

/// Sendet mit frischer Freigabeprüfung. `quellen` umfasst sämtliche Promptbestandteile;
/// fehlende Herkunft muss als `Quellen::unbestimmt()` übergeben werden.
pub async fn chat_geprueft(
    purpose: Purpose,
    provider: &PostProcessProvider,
    api_key: String,
    model: &str,
    inhalt: ChatInhalt,
    quellen: &Quellen,
) -> Result<ChatReply, String> {
    if crate::managers::llm::is_local(provider) && Cli::from_base_url(&provider.base_url).is_none()
    {
        vorpruefung(provider, model)?;
    }
    let ziel = ziel_aufloesen(provider, model).await?;
    let anfrage = Anfrage {
        zweck: purpose,
        anbieter: provider,
        api_key,
        inhalt: Inhalt::Chat(inhalt),
    };
    #[cfg(test)]
    let testquelle = pruefer::testquelle_fuer_aufruf();
    #[cfg(test)]
    let quelle: &dyn super::FreigabeQuelle = match testquelle.as_ref() {
        Some(quelle) => quelle.as_ref(),
        None => pruefer::produktiv(),
    };
    #[cfg(not(test))]
    let quelle = pruefer::produktiv();

    // Auf den Haufen: die Versandzustandsmaschine soll nicht im Future jedes Aufrufers liegen.
    Box::pin(versenden(quelle, &ziel, quellen, anfrage))
        .await
        .map(|antwort| ChatReply {
            content: antwort.text,
            truncated: antwort.abgeschnitten,
        })
        .map_err(|fehler| {
            if fehler.is_terminal() {
                fehler.to_string()
            } else {
                fehler.alte_textform()
            }
        })
}

/// Gemeinsame Cloud-Testumgebung auch für Tests außerhalb von `llm_versand`.
#[cfg(test)]
pub mod testen {
    use std::sync::Arc;

    use super::pruefer::{self, TestQuelle};
    use crate::llm_versand::{testhaken, testhilfe};
    use crate::managers::meetings::ki_freigabe::KiSperre;
    use crate::settings::PostProcessProvider;

    pub const CLOUD_BASIS: &str = "http://cloud.example/v1";

    /// Hält Quelle und Haken bis zum Testende; gilt nur auf dem aktuellen Test-Thread.
    pub struct Cloud {
        quelle: Arc<TestQuelle>,
        _wache: testhaken::Wache,
    }

    impl Drop for Cloud {
        fn drop(&mut self) {
            pruefer::setze_testquelle(None);
            // `_wache` setzt anschließend auch Proxy, Regelwerk und Zähler zurück.
        }
    }

    impl Cloud {
        pub fn freigeben(&self, id: &str) {
            self.quelle.freigeben(id);
        }

        pub fn widerrufen(&self, id: &str) {
            self.quelle.widerrufen(id);
        }

        pub fn lesungen(&self) -> usize {
            self.quelle.lesungen()
        }
    }

    /// Leitet ein echtes Cloud-Ziel an den lokalen Mock um, ohne dessen Zielart zu ändern.
    pub fn cloud_einrichten(mock_adresse: &str, nur_lokal: bool, freigegeben: &[&str]) -> Cloud {
        let wache = testhaken::wache();
        testhaken::setze_cloud_proxy(mock_adresse);
        testhaken::setze_regelwerk(|_, _, _| Ok(()));
        let quelle = Arc::new(TestQuelle::neu(
            1,
            KiSperre::Pruefen {
                kundenprofil_nur_lokal: nur_lokal,
                cloud_bei_externen: false,
                eigene_domaenen: vec!["example.org".into()],
            },
        ));
        for id in freigegeben {
            quelle.freigeben(id);
        }
        pruefer::setze_testquelle(Some(Arc::clone(&quelle)));
        Cloud {
            quelle,
            _wache: wache,
        }
    }

    pub fn cloud_anbieter() -> PostProcessProvider {
        testhilfe::anbieter("custom", CLOUD_BASIS)
    }
}
