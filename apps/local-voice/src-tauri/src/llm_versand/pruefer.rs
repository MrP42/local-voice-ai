//! Prüfer der Besprechungs-Cloud-Sperre (lva#604, Teilpaket 2).
//!
//! Der Prüfer stellt für ein [`Ziel`] und die [`Quellen`] eines Aufrufs eine [`Versandfreigabe`]
//! aus ([`freigabe_ausstellen`]) und vergleicht nach dem Transport den Stand erneut
//! ([`abschluss_pruefen`], [`abschliessen`]). Er sendet nichts und kennt keinen Transport: das
//! Verdrahten in `versenden` ist Teilpaket 3. Noch ruft ihn keine bestehende Stelle.
//!
//! # Ablauf (Schritte 1–3 und 6 des Vertrags)
//!
//! Nur bei [`ZielArt::Cloud`](super::ziel::ZielArt) **und** `Quellen` ungleich `keine()`:
//!
//! 1. **R1 (Eingangslesung):** Richtlinie, Spiegel und Revision frisch lesen
//!    ([`FreigabeQuelle::lesen`]; strenger Leseweg, siehe [`app`]), wie `mcp/tools.rs:290`.
//! 2. **Quellenprüfung:** gesperrt bei `Unbestimmt` ([`grund::UNBESTIMMT`]), bei Kundenprofil
//!    „nur lokal“ (`KiSperre::ganz_gesperrt`, [`grund::NUR_LOKAL`]) und bei jeder Besprechung,
//!    die nicht `sichtbar()` ist ([`grund::NICHT_SICHTBAR`]). Jeder Lesefehler sperrt
//!    ([`grund::LESEFEHLER`]).
//! 3. **R2 (Linearisierungspunkt, vor dem Transport):** Richtlinie, Spiegel und Revision erneut
//!    lesen. Weiter geht es nur, wenn R2 **in allem** gleich R1 ist (Revision und Richtlinie).
//!    Sonst beginnt der Prüfversuch von vorn, höchstens [`PRUEFVERSUCHE`]-mal; danach
//!    `Gesperrt` ([`grund::PRUEFVERSUCHE_ERSCHOEPFT`]). Damit sahen alle Quellenprüfungen den
//!    Stand von R1, auch wenn Richtlinie/Revision und `sichtbar()` getrennt lesen: die Revision
//!    steigt bei jeder Änderung an der Freigabegrundlage und sinkt nie
//!    (`managers/meetings/ki_freigabe.rs:76`, `:373`).
//! 6. **R3 (Abschlussvergleich):** nach dem Transport erneut lesen ([`abschluss_pruefen`]).
//!    R3≠R2: das Ergebnis wird verworfen, `Gesperrt` ([`grund::WIDERRUF_NACH_START`]).
//!
//! **Lokale Ziele** und **`keine()`** überspringen die Besprechungsprüfung ganz, ohne einen
//! einzigen Lesezugriff: `ganz_gesperrt` und `sichtbar()` sind vom Ziel unabhängig und dürfen
//! lokale Verarbeitung nicht sperren (auch nicht bei Kundenprofil „nur lokal“ oder einer nicht
//! sichtbaren Quelle). `keine()` bei einem Cloud-Ziel enthält nachweislich keinen
//! Besprechungsinhalt und wird hier ebenfalls nicht geprüft. Compliance und Budget (Schritt 4)
//! laufen davon unberührt immer, siehe Vorrang.
//!
//! # Vorrang
//!
//! Sperrt die Besprechungsprüfung, ist das Ergebnis immer
//! `Gesperrt { code: "besprechung_cloud_gesperrt" }` ([`VersandFehler::gesperrt`] setzt keinen
//! anderen Code), auch wenn Compliance oder Budget ebenfalls sperren würden (etwa „nur lokal“,
//! `managers/compliance/mod.rs:108`, `:187`). Das hält die **Reihenfolge** ein: Dieser Prüfer
//! läuft **vor** `compliance::check_call` und `usage::check_budget`, und er ruft beide nie
//! selbst; ihre Fehler können den Sperrcode deshalb nie verdecken. Compliance und Budget bleiben
//! Pflicht bei jedem Versuch, sobald die Besprechungsprüfung bestanden ist (Teilpaket 3).
//!
//! # Vertrag für Teilpaket 3 (`versenden`)
//!
//! 1. [`freigabe_ausstellen`] vor jedem Transportversuch; jeder Fallback (Stream ohne Stream,
//!    CLI-Neuversuch, JSON-Reparatur) ist ein **neuer** Versuch mit eigener Prüfung.
//! 2. Compliance und Budget; danach [`Versandfreigabe::einloesen`] unmittelbar vor dem
//!    Transport (Zielwechsel bei gleicher Anbieter-ID wird abgewiesen).
//! 3. Genau **ein** Transportversuch.
//! 4. [`abschliessen`] mit dem Ergebnis des Transports, **in jedem Fall**: auch nach einem
//!    Transportfehler, bevor das Ergebnis an den Aufrufer geht und bevor ein Fallback
//!    entschieden wird. Weicht R3 von R2 ab, ist das Ergebnis `Gesperrt` und **terminal**: kein
//!    Fallback, kein zweiter Versuch, der Fehlertext des Transports geht nirgends hin (er wird
//!    intern klassifiziert und verworfen, `llm_client.rs:426`, `managers/llm/cli.rs:1678`).
//! 5. Den Verbrauch eines tatsächlich ausgeführten, nachträglich verworfenen Versuchs bucht
//!    Teilpaket 3 trotzdem (`managers/usage.rs:683`): das Ergebnis ist verworfen, der Aufruf
//!    war es nicht.
//!
//! # Strenger Leseweg
//!
//! Die Richtlinie wird **nicht** über `settings::get_settings(&AppHandle)` gelesen, sondern über
//! den strengen Leseweg des MCP-Servers (`mcp/mod.rs:140`): die Datei `settings_store.json`
//! direkt und schreibfrei; unlesbar, fehlender oder abweichender Spiegel und jeder Fehler der
//! Datenbank ergeben `Gesperrt`, nie Standardwerte. Die Quelle selbst steht in [`app`].
//!
//! # Zeitliche Grenze (verbindlich dokumentiert)
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
//! # Fehlerfälle
//!
//! - **Nebenläufigkeit:** Der Prüfer hält keinen Zustand und keine Sperre; die Quelle liest
//!   jedes Mal frisch. Änderungen zwischen den Lesungen sind genau der Fall, den R1/R2/R3
//!   abfangen (Tests mit Haken in der Testquelle, ohne Schlaf).
//! - **Abbruch mitten im Vorgang:** Die Freigabe entsteht erst nach der letzten Prüfung; bei
//!   Abbruch (Fehler, Panic, Ende des Threads) bleibt nichts zurück, weil nichts geschrieben
//!   wird.
//! - **Voller Datenträger:** Der Prüfer schreibt nie (nur lesende Transaktionen, schreibfreier
//!   Dateizugriff). Gelesen wird auch bei vollem Datenträger; scheitert es, ist das ein
//!   Lesefehler und gesperrt.
//! - **Fehlendes Gerät/fehlende Datei:** fehlende Einstellungsdatei, fehlende oder defekte
//!   Datenbank, fehlender Spiegel: `Gesperrt`.
//! - **Kindprozess:** Der Prüfer startet keinen.
//! - **Aufwand:** je Prüfversuch zwei Lesungen und eine Sichtbarkeitsabfrage je Quelle
//!   (jeweils eine kurze Lesetransaktion); die Zahl der Versuche ist auf [`PRUEFVERSUCHE`]
//!   begrenzt. Sehr viele Quellen in einem Aufruf kosten entsprechend viele Abfragen.
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

mod app;
#[cfg(test)]
mod testquelle;

pub use app::AppFreigabeQuelle;
#[cfg(test)]
pub(super) use testquelle::TestQuelle;

/// Prüfversuche je Versand (R1, Quellenprüfung, R2), bevor `Gesperrt` kommt. Wie `VERSUCHE` im
/// MCP-Server (`mcp/tools.rs`).
pub(super) const PRUEFVERSUCHE: usize = 3;

/// Siegel für [`FreigabeQuelle`]: der Trait ist öffentlich nennbar, aber nur innerhalb dieses
/// Moduls und seiner Untermodule implementierbar (`app`, `testquelle`), weil `versiegelt` privat
/// ist.
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
/// Implementierung ([`AppFreigabeQuelle`]: Einstellungsdatei plus Besprechungsspeicher) und unter
/// `cfg(test)` eine Testquelle. Eine beliebig erlaubende Ersatzquelle ist so nicht möglich.
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

/// Eine bestandene Eingangsprüfung: die Freigabe für den Transport und der Stand R2 für den
/// Abschlussvergleich.
///
/// Kein `Clone`: die Freigabe ist ein Einmalwert. Der Transport löst sie ein
/// ([`Versandfreigabe::einloesen`]), `r2` bleibt für [`abschliessen`] übrig.
pub(super) struct Geprueft {
    /// Die einmalige, an Ziel und Quellen gebundene Freigabe.
    pub(super) freigabe: Versandfreigabe,
    /// Der Stand am Linearisierungspunkt. `None`, wenn keine Besprechungsprüfung nötig war
    /// (lokales Ziel oder `keine()`): dann gibt es nichts zu vergleichen.
    pub(super) r2: Option<Lesung>,
}

impl fmt::Debug for Geprueft {
    /// Ohne Stand und Richtlinie (die enthält die eigenen Domänen): nur, ob ein Abschlussvergleich
    /// ansteht.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Geprueft")
            .field("abschlussvergleich", &self.r2.is_some())
            .finish_non_exhaustive()
    }
}

/// Muss die Besprechungsprüfung laufen? Nur bei einem Cloud-Ziel **und** Quellen ungleich
/// `keine()`.
fn besprechung_zu_pruefen(ziel: &Ziel, quellen: &Quellen) -> bool {
    !ziel.ist_lokal() && !quellen.ist_keine()
}

/// Eine Lesung der Quelle. Jeder Fehler ist `Gesperrt`: liefert eine Quelle (gegen ihren
/// Vertrag) einen anderen Fehler, wird er zum Lesefehler, nie zu etwas Durchlässigem.
fn lesen(quelle: &dyn FreigabeQuelle) -> Result<Lesung, VersandFehler> {
    quelle.lesen().map_err(|fehler| {
        if fehler.is_terminal() {
            fehler
        } else {
            VersandFehler::gesperrt(grund::LESEFEHLER)
        }
    })
}

/// Quellenprüfung auf dem Stand R1: Kundenprofil „nur lokal“, dann jede Besprechung `sichtbar()`.
fn quellen_pruefen(
    quelle: &dyn FreigabeQuelle,
    r1: &Lesung,
    quellen: &Quellen,
) -> Result<(), VersandFehler> {
    if r1.sperre.ganz_gesperrt() {
        return Err(VersandFehler::gesperrt(grund::NUR_LOKAL));
    }
    if !quellen
        .besprechungs_ids()
        .all(|id| quelle.sichtbar(&r1.sperre, id))
    {
        return Err(VersandFehler::gesperrt(grund::NICHT_SICHTBAR));
    }
    Ok(())
}

/// Schritte 1–3 des Vertrags: stellt die Freigabe für genau dieses `ziel` und diese `quellen`
/// aus oder sperrt (`Err(Gesperrt)`, nie ein anderer Fehler). Siehe Moduldoku für den Ablauf,
/// den Vorrang vor Compliance und Budget und den Vertrag für den Aufrufer.
///
/// Lokale Ziele und `keine()` brauchen keinen Lesezugriff und liefern ohne `r2` sofort die
/// Freigabe. Sonst: `Unbestimmt` sperrt ohne zu lesen; dann höchstens [`PRUEFVERSUCHE`]
/// Durchgänge aus R1, Quellenprüfung und R2, die ersten mit R2 = R1 bestehen.
pub(super) fn freigabe_ausstellen(
    quelle: &dyn FreigabeQuelle,
    ziel: &Ziel,
    quellen: &Quellen,
) -> Result<Geprueft, VersandFehler> {
    if !besprechung_zu_pruefen(ziel, quellen) {
        return Ok(Geprueft {
            freigabe: Versandfreigabe::ausstellen(ziel, quellen),
            r2: None,
        });
    }
    if quellen.ist_unbestimmt() {
        return Err(VersandFehler::gesperrt(grund::UNBESTIMMT));
    }
    for _ in 0..PRUEFVERSUCHE {
        let r1 = lesen(quelle)?;
        quellen_pruefen(quelle, &r1, quellen)?;
        let r2 = lesen(quelle)?;
        if r2 == r1 {
            return Ok(Geprueft {
                freigabe: Versandfreigabe::ausstellen(ziel, quellen),
                r2: Some(r2),
            });
        }
    }
    Err(VersandFehler::gesperrt(grund::PRUEFVERSUCHE_ERSCHOEPFT))
}

/// Schritt 6 des Vertrags: Abschlussvergleich. Liest R3 frisch und vergleicht mit `r2`, dem Stand
/// am Linearisierungspunkt. R3≠R2 ist `Gesperrt` ([`grund::WIDERRUF_NACH_START`]); jeder
/// Lesefehler ebenso.
///
/// Das erkennt Änderungen zwischen R2 und R3, **nicht** danach: ein Widerruf nach R3 hält die
/// Ausgabe nicht zurück (siehe „Zeitliche Grenze“ in der Moduldoku). Der Aufrufer ruft in jedem
/// Fall [`abschliessen`], diese Funktion ist sein Kern.
pub(super) fn abschluss_pruefen(
    quelle: &dyn FreigabeQuelle,
    r2: &Lesung,
) -> Result<(), VersandFehler> {
    let r3 = lesen(quelle)?;
    if r3 == *r2 {
        Ok(())
    } else {
        Err(VersandFehler::gesperrt(grund::WIDERRUF_NACH_START))
    }
}

/// Schließt einen Transportversuch ab: **vor** Rückgabe und vor jedem Fallback wird R3 gelesen
/// und mit `r2` verglichen, **auch wenn der Transport fehlschlug**.
///
/// - R3 = R2 oder kein `r2` (lokal, `keine()`): das Transportergebnis geht unverändert zurück.
///   Ein Transportfehler bleibt dann ein Transportfehler; ein Fallback ist ein neuer Versuch
///   mit eigener Prüfung ([`freigabe_ausstellen`]).
/// - R3 ≠ R2 oder Lesefehler: `Err(Gesperrt)`, **terminal**. Das Transportergebnis (Antwort wie
///   Fehler samt seinem Text) wird verworfen und nicht zurückgegeben; `Gesperrt` hat Vorrang
///   auch vor einem Abbruch oder Zeitfehler des Transports.
///
/// Den Verbrauch des tatsächlich ausgeführten Versuchs bucht der Aufrufer unabhängig davon.
pub(super) fn abschliessen<T>(
    quelle: &dyn FreigabeQuelle,
    r2: Option<&Lesung>,
    transport: Result<T, VersandFehler>,
) -> Result<T, VersandFehler> {
    if let Some(r2) = r2 {
        abschluss_pruefen(quelle, r2)?;
    }
    transport
}

#[cfg(test)]
#[path = "pruefer_tests.rs"]
mod pruefung_tests;

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
