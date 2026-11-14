//! Prüfer der Besprechungs-Cloud-Sperre (lva#604).
//!
//! **Stand Teilpaket 1: Rumpf.** Hier liegen nur die Typen, an denen der Prüfer hängt:
//! die versiegelte Lesequelle [`FreigabeQuelle`] samt [`Lesung`] und die einmalige,
//! gebundene [`Versandfreigabe`]. Die Prüfung selbst kommt mit Teilpaket 2.
//!
//! # TODO Teilpaket 2: der Prüfer (Cloud-Ziele)
//!
//! Ablauf je Transportversuch, nur bei [`ZielArt::Cloud`](super::ziel::ZielArt) **und**
//! `Quellen` ungleich `keine()`:
//!
//! 1. **R1 (Eingangslesung):** Richtlinie, Spiegel und Revision frisch lesen — über einen
//!    **strengen** Leseweg (siehe unten), wie `mcp/tools.rs:290` (`lauf_lesen`).
//! 2. **Quellenprüfung:** gesperrt bei Kundenprofil „nur lokal“ (`KiSperre::ganz_gesperrt`),
//!    bei einer Quelle, die nicht `sichtbar()` ist, bei `Unbestimmt` und bei jedem Lesefehler.
//! 3. **R2 (Linearisierungspunkt, vor dem Transport):** erneut lesen; weiter nur, wenn alles
//!    gleich R1 ist. Höchstens drei Prüfversuche, danach `Gesperrt`
//!    ([`grund::PRUEFVERSUCHE_ERSCHOEPFT`](super::fehler::grund)).
//! 4. Compliance und Budget (Teilpaket 3, im Versand), Pflicht bei jedem Versuch.
//! 5. Genau ein Transportversuch an das gebundene Ziel.
//! 6. **R3 (Abschlussvergleich):** erneut lesen, auch wenn der Transport fehlschlug. R3≠R2:
//!    das Ergebnis wird verworfen, `Gesperrt` ([`grund::WIDERRUF_NACH_START`](super::fehler::grund)),
//!    **terminal** — kein Fallback, kein zweiter Versuch, keine Rückgabe des Fehlertexts. Der
//!    Verbrauch eines tatsächlich ausgeführten, nachträglich verworfenen Versuchs wird
//!    weiterhin gebucht (`managers/usage.rs`).
//!
//! Lokale Ziele überspringen **ausschließlich** die Besprechungsprüfung (1–3 und 6);
//! Compliance und Budget laufen immer.
//!
//! **Vorrang:** Sperrt die Besprechungsprüfung, ist das Ergebnis immer
//! `Gesperrt { code: "besprechung_cloud_gesperrt" }`, auch wenn Compliance oder Budget
//! ebenfalls sperren würden (`managers/compliance/mod.rs:187`, „nur lokal“). Compliance und
//! Budget bleiben trotzdem Pflicht, sobald die Besprechungsprüfung bestanden ist.
//!
//! ## Strenger Leseweg
//!
//! Die Richtlinie wird **nicht** über `settings::get_settings(&AppHandle)` gelesen: das hat
//! Rettungs- und Schreibverhalten (`settings.rs:2345`, `:2393`) und würde einen Lesefehler
//! durch Standardwerte ersetzen. Vorbild ist `mcp::read_ki_sperre` (`mcp/mod.rs:140`): die
//! Datei `settings_store.json` direkt und schreibfrei lesen; unlesbar = Kundenprofil
//! „nur lokal“; unbekannter oder falsch getypter Wert sperrt. Ein Lesefehler ist nie
//! „Standard“, sondern `Gesperrt` ([`grund::LESEFEHLER`](super::fehler::grund)).
//!
//! ## Zeitliche Grenze (verbindlich dokumentiert)
//!
//! R2 und R3 erkennen **nur Änderungen zwischen zwei Lesungen**. Ein Widerruf, der vor R2
//! festgeschrieben ist, verhindert jeden Request. Ein Widerruf zwischen R2 und dem Transport
//! kann den Request nicht mehr verhindern; er wird in R3 erkannt, und das Ergebnis wird nicht
//! ausgegeben. **Ein Widerruf nach R3 hält die Ausgabe nicht zurück:** die Freigabe der
//! gepufferten Ausgabe an den Rückruf ist kein atomarer Vorgang mit Widerrufen
//! (`llm_client.rs:1354`, `:1358`). Bereits Versandtes ist nicht rückholbar (ADR-0031). Eine
//! Synchronisation mit den Widerrufsschreibern über die Dauer eines Netzaufrufs ist bewusst
//! ausgeschlossen, weil langsame Anbieter sonst Widerrufe blockieren würden.
//!
//! # Versandfreigabe
//!
//! Die [`Versandfreigabe`] ist ein **Einmalwert**: besitzend verbraucht ([`Versandfreigabe::einloesen`]
//! nimmt `self`), gebunden an das konkrete [`Ziel`] und den Fingerabdruck der [`Quellen`],
//! ohne `Clone`, `Copy`, `Default`, `Serialize`, `Deserialize`, mit privaten Feldern. Sie wird
//! nur hier im Prüfer ausgestellt (privater Konstruktor) und verlässt `llm_versand` nie
//! (`pub(super)`). Die Freigabe ersetzt R2/R3 nicht: Prüfen und Senden bleiben **eine**
//! gekapselte Operation, damit zwischen Erzeugung und Versand keine beliebige Wartezeit liegt.

use std::fmt;

use super::fehler::{grund, VersandFehler};
use super::quellen::Quellen;
use super::ziel::Ziel;
use crate::managers::meetings::ki_freigabe::KiSperre;

/// Siegel für [`FreigabeQuelle`]: der Trait ist öffentlich nennbar, aber nur innerhalb von
/// `llm_versand` implementierbar, weil dieses Modul privat ist.
mod versiegelt {
    pub trait Versiegelt {}
}

/// Ein konsistenter Lesestand der Freigabegrundlage (R1, R2 oder R3).
///
/// `PartialEq` ist der Vergleich der Linearisierung: **alles** gleich (Revision und
/// Richtlinie), sonst hat sich zwischen zwei Lesungen etwas geändert.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lesung {
    /// Revision der Freigabegrundlage. Sie steigt bei jeder Änderung an Bestätigungen,
    /// Richtlinienspiegel, Terminen, Teilnehmenden, Quellen und Verknüpfungen und sinkt nie
    /// (`managers/meetings/ki_freigabe.rs:76`, `:373`).
    pub revision: i64,
    /// Die frisch (streng) gelesene Richtlinie, bereits mit dem Spiegel abgeglichen.
    pub sperre: KiSperre,
}

/// Woher der Prüfer die Freigabegrundlage liest. **Versiegelt:** Produktiv gibt es genau eine
/// Implementierung (App-Handle plus Besprechungsspeicher, Teilpaket 2) und unter `cfg(test)` eine
/// Testquelle. Eine beliebig erlaubende Ersatzquelle ist so nicht möglich.
pub trait FreigabeQuelle: versiegelt::Versiegelt + Send + Sync {
    /// Richtlinie (streng, frisch), Spiegel und Revision lesen und abgleichen.
    ///
    /// Jeder Zweifel ist `Err(Gesperrt)`: Lesefehler ([`grund::LESEFEHLER`]), fehlender Spiegel
    /// ([`grund::SPIEGEL_FEHLT`]), abweichender Spiegel ([`grund::SPIEGEL_ABWEICHEND`]).
    fn lesen(&self) -> Result<Lesung, VersandFehler>;

    /// Darf ein KI-Client diese Besprechung sehen (`KiSperre::sichtbar`)? Ein Lesefehler ist
    /// `false`.
    fn sichtbar(&self, sperre: &KiSperre, besprechung_id: &str) -> bool;
}

/// Einmalige Freigabe für genau ein [`Ziel`] und genau diese [`Quellen`].
///
/// Kein `Clone`, `Copy`, `Default`, `Serialize`, `Deserialize`; Felder privat; nur im Prüfer
/// ausstellbar; nur innerhalb von `llm_versand` sichtbar. Der Beweis der fehlenden Traits steht
/// in den Tests dieses Moduls (Compile-Zeit), die Compile-fail-Proben von außen folgen mit
/// Teilpaket 4.
pub(super) struct Versandfreigabe {
    ziel: Ziel,
    quellen: [u8; 32],
}

impl Versandfreigabe {
    /// Stellt die Freigabe für dieses Ziel und diese Quellen aus. Privat: nur der Prüfer (dieses
    /// Modul und seine Untermodule) kann ausstellen, nachdem er bestanden hat.
    fn ausstellen(ziel: &Ziel, quellen: &Quellen) -> Self {
        Self {
            ziel: ziel.clone(),
            quellen: quellen.fingerabdruck(),
        }
    }

    /// Verbraucht die Freigabe beim Transport. Gilt nur für genau das Ziel und die Quellen, für
    /// die sie ausgestellt wurde. Ein Zielwechsel (auch bei gleicher Anbieter-ID, etwa eine neue
    /// Basis-URL) und andere Quellen sind `Gesperrt`. Die Route wird nicht neu aufgelöst.
    pub(super) fn einloesen(self, ziel: &Ziel, quellen: &Quellen) -> Result<(), VersandFehler> {
        if self.ziel != *ziel {
            return Err(VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT));
        }
        if self.quellen != quellen.fingerabdruck() {
            return Err(VersandFehler::gesperrt(grund::QUELLEN_GEWECHSELT));
        }
        Ok(())
    }
}

impl fmt::Debug for Versandfreigabe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Versandfreigabe").finish_non_exhaustive()
    }
}

#[cfg(test)]
pub(super) use testquelle::TestQuelle;

/// Testquelle: frei einstellbare Lesung und Sichtbarkeit, mit Zähler. Nur unter `cfg(test)`;
/// produktiv gibt es sie nicht.
#[cfg(test)]
mod testquelle {
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::{FreigabeQuelle, KiSperre, Lesung, VersandFehler};

    pub(in crate::llm_versand) struct TestQuelle {
        lesung: Mutex<Result<Lesung, VersandFehler>>,
        sichtbar: Mutex<BTreeSet<String>>,
        gelesen: AtomicUsize,
    }

    impl super::versiegelt::Versiegelt for TestQuelle {}

    impl TestQuelle {
        /// Eine Quelle mit dieser Revision und Richtlinie, ohne sichtbare Besprechung.
        pub(in crate::llm_versand) fn neu(revision: i64, sperre: KiSperre) -> Self {
            Self {
                lesung: Mutex::new(Ok(Lesung { revision, sperre })),
                sichtbar: Mutex::new(BTreeSet::new()),
                gelesen: AtomicUsize::new(0),
            }
        }

        /// Gibt eine Besprechung für KI-Clients frei.
        pub(in crate::llm_versand) fn freigeben(&self, id: &str) {
            self.sichtbar.lock().unwrap().insert(id.to_owned());
        }

        /// Widerruft die Freigabe einer Besprechung.
        pub(in crate::llm_versand) fn widerrufen(&self, id: &str) {
            self.sichtbar.lock().unwrap().remove(id);
        }

        /// Stellt die nächsten Lesungen ein (neuer Stand oder Fehler).
        pub(in crate::llm_versand) fn setze_lesung(&self, lesung: Result<Lesung, VersandFehler>) {
            *self.lesung.lock().unwrap() = lesung;
        }

        /// Wie oft wurde `lesen()` aufgerufen?
        pub(in crate::llm_versand) fn lesungen(&self) -> usize {
            self.gelesen.load(Ordering::SeqCst)
        }
    }

    impl FreigabeQuelle for TestQuelle {
        fn lesen(&self) -> Result<Lesung, VersandFehler> {
            self.gelesen.fetch_add(1, Ordering::SeqCst);
            self.lesung.lock().unwrap().clone()
        }

        fn sichtbar(&self, _sperre: &KiSperre, besprechung_id: &str) -> bool {
            self.sichtbar.lock().unwrap().contains(besprechung_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn richtlinie() -> KiSperre {
        KiSperre::Pruefen {
            kundenprofil_nur_lokal: false,
            cloud_bei_externen: false,
            eigene_domaenen: Vec::new(),
        }
    }

    fn cloud() -> Ziel {
        Ziel::http("openai", "https://api.example.com/v1", "m")
    }

    // Compile-Zeit-Beweis: `Versandfreigabe` implementiert diese Traits NICHT. Implementierte
    // der Typ eines davon, wäre `<T as Mehrdeutig<_>>::pruefen` mehrdeutig und diese Datei
    // würde nicht übersetzen (Technik der Kiste `static_assertions`, ohne Abhängigkeit).
    macro_rules! implementiert_nicht {
        ($typ:ty, $($schranke:tt)+) => {
            const _: fn() = || {
                trait Mehrdeutig<A> {
                    fn pruefen() {}
                }
                impl<T: ?Sized> Mehrdeutig<()> for T {}
                impl<T: ?Sized + $($schranke)+> Mehrdeutig<u8> for T {}
                let _ = <$typ as Mehrdeutig<_>>::pruefen;
            };
        };
    }
    implementiert_nicht!(Versandfreigabe, Clone);
    implementiert_nicht!(Versandfreigabe, Copy);
    implementiert_nicht!(Versandfreigabe, Default);
    implementiert_nicht!(Versandfreigabe, serde::Serialize);
    implementiert_nicht!(Versandfreigabe, for<'de> serde::Deserialize<'de>);

    #[test]
    fn freigabe_gilt_fuer_genau_ziel_und_quellen() {
        let quellen = Quellen::besprechungen(["m1", "m2"]);
        let freigabe = Versandfreigabe::ausstellen(&cloud(), &quellen);
        assert_eq!(freigabe.einloesen(&cloud(), &quellen), Ok(()));
    }

    #[test]
    fn freigabe_ist_unabhaengig_von_der_reihenfolge_der_quellen() {
        let freigabe = Versandfreigabe::ausstellen(&cloud(), &Quellen::besprechungen(["m1", "m2"]));
        assert_eq!(
            freigabe.einloesen(&cloud(), &Quellen::besprechungen(["m2", "m1", "m2"])),
            Ok(())
        );
    }

    #[test]
    fn zielwechsel_bei_gleicher_anbieter_id_wird_abgewiesen() {
        let quellen = Quellen::besprechung("m1");
        let freigabe = Versandfreigabe::ausstellen(&cloud(), &quellen);
        let umgebogen = Ziel::http("openai", "https://anderer-host.example/v1", "m");
        assert_eq!(umgebogen.anbieter_id(), cloud().anbieter_id());
        assert_eq!(
            freigabe.einloesen(&umgebogen, &quellen),
            Err(VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT))
        );
    }

    #[test]
    fn modellwechsel_und_artwechsel_werden_abgewiesen() {
        let quellen = Quellen::besprechung("m1");
        let andere_modelle = Ziel::http("openai", "https://api.example.com/v1", "anderes");
        let lokal = Ziel::http("openai", "http://127.0.0.1:11434/v1", "m");
        for ziel in [andere_modelle, lokal] {
            let freigabe = Versandfreigabe::ausstellen(&cloud(), &quellen);
            assert_eq!(
                freigabe.einloesen(&ziel, &quellen),
                Err(VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT))
            );
        }
    }

    #[test]
    fn andere_quellen_werden_abgewiesen() {
        for andere in [
            Quellen::besprechung("m2"),
            Quellen::besprechungen(["m1", "m2"]),
            Quellen::keine(),
            Quellen::unbestimmt(),
        ] {
            let freigabe = Versandfreigabe::ausstellen(&cloud(), &Quellen::besprechung("m1"));
            assert_eq!(
                freigabe.einloesen(&cloud(), &andere),
                Err(VersandFehler::gesperrt(grund::QUELLEN_GEWECHSELT))
            );
        }
    }

    #[test]
    fn ziel_hat_vorrang_vor_quellen_im_fehlergrund() {
        let freigabe = Versandfreigabe::ausstellen(&cloud(), &Quellen::besprechung("m1"));
        let anderes = Ziel::http("openai", "https://anderer-host.example/v1", "m");
        assert_eq!(
            freigabe.einloesen(&anderes, &Quellen::besprechung("m2")),
            Err(VersandFehler::gesperrt(grund::ZIEL_GEWECHSELT))
        );
    }

    #[test]
    fn debug_der_freigabe_verraet_weder_ziel_noch_quellen() {
        let freigabe = Versandfreigabe::ausstellen(&cloud(), &Quellen::besprechung("m-geheim"));
        let text = format!("{freigabe:?}");
        assert!(!text.contains("api.example.com"), "{text}");
        assert!(!text.contains("m-geheim"), "{text}");
    }

    #[test]
    fn testquelle_liefert_lesung_sichtbarkeit_und_zaehlt() {
        let quelle = TestQuelle::neu(7, richtlinie());
        assert_eq!(quelle.lesungen(), 0);
        let erste = quelle.lesen().unwrap();
        assert_eq!(erste.revision, 7);
        assert_eq!(quelle.lesungen(), 1);

        assert!(!quelle.sichtbar(&erste.sperre, "m1"));
        quelle.freigeben("m1");
        assert!(quelle.sichtbar(&erste.sperre, "m1"));
        quelle.widerrufen("m1");
        assert!(!quelle.sichtbar(&erste.sperre, "m1"));

        quelle.setze_lesung(Err(VersandFehler::gesperrt(grund::LESEFEHLER)));
        assert_eq!(
            quelle.lesen(),
            Err(VersandFehler::gesperrt(grund::LESEFEHLER))
        );
        assert_eq!(quelle.lesungen(), 2);
    }

    #[test]
    fn lesungen_sind_vergleichbar_fuer_die_linearisierung() {
        let a = Lesung {
            revision: 1,
            sperre: richtlinie(),
        };
        assert_eq!(a, a.clone());
        assert_ne!(
            a,
            Lesung {
                revision: 2,
                ..a.clone()
            }
        );
        assert_ne!(
            a,
            Lesung {
                sperre: KiSperre::Pruefen {
                    kundenprofil_nur_lokal: true,
                    cloud_bei_externen: false,
                    eigene_domaenen: Vec::new(),
                },
                ..a.clone()
            }
        );
    }
}
