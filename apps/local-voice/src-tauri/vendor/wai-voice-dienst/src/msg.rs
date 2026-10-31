//! Steuernachrichten (Nutzlast der Rahmenart `Steuer`), JSON mit Feld `t` als Typ.
//!
//! Ablauf: Der Dienst beginnt mit `gruss` (Protokoll, Zufallswert). Die App antwortet mit
//! `hallo` und einem Beweis über den Zufallswert ([`crate::beweis`]); der Zugangsschlüssel selbst
//! verlässt die App nie. Danach `willkommen` (oder `fehler` und Verbindungsende). Je Operation ein
//! Strom (> 0), den die App wählt: `start` öffnet ihn, Audio fließt in Rahmen derselben
//! Stromnummer, `ende` schließt die Eingabe, `abbrechen` beendet ihn sofort. Unbekannte Felder und
//! Typen werden abgewiesen (`deny_unknown_fields`), damit ein neuer Client gegen einen alten Dienst
//! nicht stillschweigend etwas anderes bekommt, als er meint.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{PROTOKOLL, grenzen};

/// Rechte je Zugang; Standard in Local Voice AI ist „aus“, bis der Nutzer freigibt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recht {
    /// Erkennen (`erkennen`, `erkennen_strom`).
    Hoeren,
    /// Sprechen (`sprechen`).
    Sprechen,
    /// Korrektursignal annehmen (`rueckmeldung`).
    Lernen,
}

/// Souveränitätsstufe des aufrufenden Chats. Der Dienst nutzt die Schnittmenge mit seinen
/// eigenen Freigaben; gibt es nichts Passendes, antwortet er mit `fehler` (`nicht_lokal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Souveraenitaet {
    NurLokal,
    CloudErlaubt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "art", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    /// Ganze Äußerung erkennen; Ergebnis nach `ende`.
    Erkennen,
    /// Erkennen mit Teilergebnissen (Diktat).
    ErkennenStrom,
    /// Text sprechen; der Dienst liefert WAV-Häppchen als `AudioHeraus`.
    Sprechen {
        text: String,
        stimme: Option<String>,
    },
}

impl Operation {
    pub fn recht(&self) -> Recht {
        match self {
            Self::Erkennen | Self::ErkennenStrom => Recht::Hoeren,
            Self::Sprechen { .. } => Recht::Sprechen,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wortpaar {
    /// Was erkannt wurde.
    pub gehoert: String,
    /// Was gemeint war.
    pub gemeint: String,
}

/// App → Dienst.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnDienst {
    /// `zugang`: öffentliche Kennung des Zugangs; `beweis`: Hex-HMAC aus [`crate::beweis`].
    Hallo {
        protokoll: u16,
        client: String,
        zugang: String,
        beweis: String,
    },
    Start {
        strom: u32,
        op: Operation,
        bereich: String,
        souveraenitaet: Souveraenitaet,
    },
    Ende {
        strom: u32,
    },
    Abbrechen {
        strom: u32,
    },
    /// Guthaben für `AudioHeraus` (Rückstau der App beim Abspielen).
    Guthaben {
        strom: u32,
        rahmen: u32,
    },
    Rueckmeldung {
        bereich: String,
        paare: Vec<Wortpaar>,
    },
    Status {},
    /// Kopplung einer App (statt `hallo`, nur einmal je Verbindung): Die App erzeugt ihren
    /// Schlüssel selbst und schickt nur dessen Fingerabdruck. Local Voice AI zeigt `anzeige` und
    /// `code`; der Nutzer bestätigt, wenn der Code mit dem in der App übereinstimmt. Antwort
    /// `gekoppelt` (oder `fehler` `abgelehnt`, `zeit_ueberschritten` nach
    /// [`grenzen::KOPPELN_BESTAETIGUNG_S`], `beschaeftigt`, `bereits_gekoppelt`), danach endet die
    /// Verbindung. Rechte vergibt der
    /// Nutzer getrennt, Standard „aus“.
    Koppeln {
        protokoll: u16,
        client: String,
        anzeige: String,
        fingerabdruck: String,
        code: String,
    },
}

impl AnDienst {
    /// Strom, auf den sich die Nachricht bezieht (0 = Verbindung). Muss dem Strom im Rahmenkopf
    /// entsprechen ([`strom_passt`]).
    pub fn strom(&self) -> u32 {
        match self {
            Self::Start { strom, .. }
            | Self::Ende { strom }
            | Self::Abbrechen { strom }
            | Self::Guthaben { strom, .. } => *strom,
            Self::Hallo { .. }
            | Self::Koppeln { .. }
            | Self::Rueckmeldung { .. }
            | Self::Status {} => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Engines {
    pub erkennung: String,
    pub stimme: String,
    /// Beide laufen auf diesem Rechner.
    pub lokal: bool,
}

/// Dienst → App.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnApp {
    /// Erste Nachricht des Dienstes; `zufall`: 32 Bytes als Hex, je Verbindung neu.
    Gruss {
        protokoll: u16,
        zufall: String,
    },
    /// `beweis`: Gegenbeweis des Dienstes ([`crate::beweis::dienst_erzeugen`]), Hex.
    Willkommen {
        protokoll: u16,
        dienst: String,
        version: String,
        rechte: Vec<Recht>,
        engines: Engines,
        beweis: String,
    },
    /// Guthaben für `AudioHinein` (Rückstau des Dienstes).
    Guthaben {
        strom: u32,
        rahmen: u32,
    },
    Teil {
        strom: u32,
        text: String,
    },
    Ergebnis {
        strom: u32,
        text: String,
    },
    /// `sprechen` ist vollständig ausgeliefert.
    Fertig {
        strom: u32,
    },
    Fehler {
        strom: u32,
        code: String,
        nachricht: String,
    },
    Status {
        bereit: bool,
        engines: Engines,
    },
    /// Kopplung bestätigt; `zugang` ist die öffentliche Kennung für künftige `hallo`.
    Gekoppelt {
        zugang: String,
    },
}

impl AnApp {
    pub fn strom(&self) -> u32 {
        match self {
            Self::Guthaben { strom, .. }
            | Self::Teil { strom, .. }
            | Self::Ergebnis { strom, .. }
            | Self::Fertig { strom }
            | Self::Fehler { strom, .. } => *strom,
            Self::Gruss { .. }
            | Self::Willkommen { .. }
            | Self::Status { .. }
            | Self::Gekoppelt { .. } => 0,
        }
    }
}

/// Strom im Rahmenkopf und in der Nachricht müssen übereinstimmen.
pub fn strom_passt(kopf: u32, nachricht: u32) -> bool {
    kopf == nachricht
}

/// Stabile Fehlercodes (Text, damit Oberflächen sie direkt zuordnen können).
pub mod code {
    pub const VERSION_INKOMPATIBEL: &str = "version_inkompatibel";
    pub const SCHLUESSEL_UNGUELTIG: &str = "schluessel_ungueltig";
    pub const SCHLUESSEL_WIDERRUFEN: &str = "schluessel_widerrufen";
    pub const NICHT_ANGEMELDET: &str = "nicht_angemeldet";
    pub const RECHT_FEHLT: &str = "recht_fehlt";
    pub const NICHT_LOKAL: &str = "nicht_lokal";
    pub const UNGUELTIG: &str = "ungueltig";
    pub const STROM_BELEGT: &str = "strom_belegt";
    pub const STROM_UNBEKANNT: &str = "strom_unbekannt";
    pub const ZU_VIELE_STROEME: &str = "zu_viele_stroeme";
    pub const KEIN_GUTHABEN: &str = "kein_guthaben";
    pub const ZEIT_UEBERSCHRITTEN: &str = "zeit_ueberschritten";
    pub const ENGINE_FEHLT: &str = "engine_fehlt";
    pub const ABGEBROCHEN: &str = "abgebrochen";
    pub const UEBERLASTET: &str = "ueberlastet";
    pub const FEHLGESCHLAGEN: &str = "fehlgeschlagen";
    /// Nutzer hat die Kopplung abgelehnt oder nicht rechtzeitig bestätigt.
    pub const ABGELEHNT: &str = "abgelehnt";
    /// Eine Kopplung dieses Clients wartet schon oder die Drosselung greift.
    pub const BESCHAEFTIGT: &str = "beschaeftigt";
    /// Dieser Fingerabdruck ist schon gekoppelt; der Nutzer entkoppelt in LVA und koppelt neu.
    pub const BEREITS_GEKOPPELT: &str = "bereits_gekoppelt";
}

/// Prüft eine Nachricht der App (die App vor dem Senden, der Dienst nach dem Empfang).
/// Rückgabe im Fehlerfall: das ungültige Feld.
pub fn pruefen(m: &AnDienst) -> Result<(), String> {
    match m {
        AnDienst::Hallo {
            protokoll,
            client,
            zugang,
            beweis,
        } => {
            if *protokoll != PROTOKOLL {
                return Err("protokoll".into());
            }
            kennung(client, "client")?;
            kennung(zugang, "zugang")?;
            hex(beweis, 64, "beweis")
        }
        AnDienst::Start {
            strom, op, bereich, ..
        } => {
            strom_nr(*strom)?;
            kennung(bereich, "bereich")?;
            if let Operation::Sprechen { text, stimme } = op {
                text_ok(text)?;
                if let Some(s) = stimme {
                    stimm_id(s)?;
                }
            }
            Ok(())
        }
        AnDienst::Ende { strom } | AnDienst::Abbrechen { strom } => strom_nr(*strom),
        AnDienst::Guthaben { strom, rahmen } => {
            strom_nr(*strom)?;
            guthaben_ok(*rahmen)
        }
        AnDienst::Rueckmeldung { bereich, paare } => {
            kennung(bereich, "bereich")?;
            if paare.is_empty() || paare.len() > grenzen::PAARE_MAX {
                return Err("paare".into());
            }
            paare.iter().try_for_each(|p| {
                wort(&p.gehoert)?;
                wort(&p.gemeint)?;
                if p.gehoert.to_lowercase() == p.gemeint.to_lowercase() {
                    return Err("paar_gleich".into());
                }
                Ok(())
            })
        }
        AnDienst::Status {} => Ok(()),
        AnDienst::Koppeln {
            protokoll,
            client,
            anzeige,
            fingerabdruck,
            code,
        } => {
            if *protokoll != PROTOKOLL {
                return Err("protokoll".into());
            }
            kennung(client, "client")?;
            anzeige_ok(anzeige)?;
            hex(fingerabdruck, 64, "fingerabdruck")?;
            if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
                return Err("code".into());
            }
            Ok(())
        }
    }
}

/// Prüft eine Nachricht des Dienstes (die App nach dem Empfang).
pub fn pruefen_an_app(m: &AnApp) -> Result<(), String> {
    match m {
        AnApp::Gruss { protokoll, zufall } => {
            if *protokoll != PROTOKOLL {
                return Err("protokoll".into());
            }
            hex(zufall, 64, "zufall")
        }
        AnApp::Willkommen {
            protokoll, beweis, ..
        } => {
            if *protokoll != PROTOKOLL {
                return Err("protokoll".into());
            }
            hex(beweis, 64, "beweis")
        }
        AnApp::Guthaben { strom, rahmen } => {
            strom_nr(*strom)?;
            guthaben_ok(*rahmen)
        }
        AnApp::Teil { strom, text } | AnApp::Ergebnis { strom, text } => {
            strom_nr(*strom)?;
            if text.chars().count() > grenzen::TEXT_MAX * 4 {
                return Err("text".into());
            }
            Ok(())
        }
        AnApp::Fertig { strom } => strom_nr(*strom),
        AnApp::Fehler {
            code, nachricht, ..
        } => {
            if code.len() > 64 || nachricht.chars().count() > 500 {
                return Err("fehler".into());
            }
            Ok(())
        }
        AnApp::Status { .. } => Ok(()),
        AnApp::Gekoppelt { zugang } => kennung(zugang, "zugang"),
    }
}

/// Name im Bestätigungsdialog von Local Voice AI: nur Buchstaben, Ziffern, Leerzeichen und
/// `-_.()&+/`. Keine Steuer-, Format- oder Bidi-Zeichen (U+202E u. ä.), die den Dialog fälschen könnten.
fn anzeige_ok(a: &str) -> Result<(), String> {
    let n = a.chars().count();
    let ok = !a.trim().is_empty()
        && n <= 64
        && a.chars()
            .all(|c| c.is_alphanumeric() || c == ' ' || "-_.()&+/".contains(c));
    if ok { Ok(()) } else { Err("anzeige".into()) }
}

fn strom_nr(s: u32) -> Result<(), String> {
    if s == 0 { Err("strom".into()) } else { Ok(()) }
}

fn guthaben_ok(r: u32) -> Result<(), String> {
    if r == 0 || r > grenzen::GUTHABEN_MAX {
        Err("rahmen".into())
    } else {
        Ok(())
    }
}

/// Kennungen (Bereich, Client, Zugang): ASCII `[a-z0-9_-]`, 1–64 Bytes, beginnt mit Buchstabe
/// oder Ziffer. Kein `.`, `:`, Leerzeichen oder Unicode: Ein Bereich darf trotzdem nie roh als
/// Pfadteil benutzt werden; der Dienst legt Profile unter einem Hash des Bereichs ab.
fn kennung(s: &str, feld: &str) -> Result<(), String> {
    let ok = s.len() <= grenzen::BEREICH_MAX
        && s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if ok { Ok(()) } else { Err(feld.into()) }
}

/// Stimmen-Kennung des Dienstes, z. B. `provider:phonon:anton`: ASCII, `:` als Trenner erlaubt.
fn stimm_id(s: &str) -> Result<(), String> {
    let ok = s.len() <= grenzen::BEREICH_MAX
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'));
    if ok { Ok(()) } else { Err("stimme".into()) }
}

fn hex(s: &str, len: usize, feld: &str) -> Result<(), String> {
    let ok = s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if ok { Ok(()) } else { Err(feld.into()) }
}

fn text_ok(t: &str) -> Result<(), String> {
    let ok = !t.trim().is_empty()
        && t.chars().count() <= grenzen::TEXT_MAX
        && !t.chars().any(|c| c.is_control() && c != '\n');
    if ok { Ok(()) } else { Err("text".into()) }
}

/// Ein Wort des Korrektursignals: genau ein Wort mit mindestens einem Buchstaben, ohne Ziffern
/// (Codex-Auflage 3: Zahlen können Aktenzeichen, Beträge oder Gesundheitsdaten sein). Erlaubt
/// sind Buchstaben, kombinierende Zeichen (zerlegte Umlaute), `-` und `'`.
fn wort(w: &str) -> Result<(), String> {
    let n = w.chars().count();
    let zeichen_ok = w
        .chars()
        .all(|c| c.is_alphabetic() || matches!(c, '-' | '\'' | '\u{0300}'..='\u{036F}'));
    let ok = n > 0 && n <= grenzen::WORT_MAX && zeichen_ok && w.chars().any(char::is_alphabetic);
    if ok { Ok(()) } else { Err("wort".into()) }
}

/// Zustandsautomat der Dienstseite je Verbindung: vor dem Handschlag nur `hallo`, danach
/// höchstens [`grenzen::STROEME_MAX`] offene Ströme, `ende`/`guthaben` nur für offene.
#[derive(Debug, Default)]
pub struct Sitzung {
    angemeldet: bool,
    /// `hallo` oder `koppeln` ist eingegangen; ein zweites wird abgewiesen.
    begonnen: bool,
    offen: HashSet<u32>,
}

impl Sitzung {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn angemeldet(&self) -> bool {
        self.angemeldet
    }

    /// Nach geprüftem Beweis aufzurufen.
    pub fn anmelden(&mut self) {
        self.angemeldet = true;
    }

    /// Prüft Reihenfolge und Ströme (Inhalt vorher mit [`pruefen`]); Fehler = Code aus [`code`].
    pub fn annehmen(&mut self, kopf_strom: u32, m: &AnDienst) -> Result<(), &'static str> {
        if !strom_passt(kopf_strom, m.strom()) {
            return Err(code::UNGUELTIG);
        }
        match m {
            AnDienst::Hallo { .. } | AnDienst::Koppeln { .. } if self.begonnen => {
                Err(code::UNGUELTIG)
            }
            AnDienst::Hallo { .. } | AnDienst::Koppeln { .. } => {
                self.begonnen = true;
                Ok(())
            }
            _ if !self.angemeldet => Err(code::NICHT_ANGEMELDET),
            AnDienst::Start { strom, .. } => {
                if self.offen.contains(strom) {
                    Err(code::STROM_BELEGT)
                } else if self.offen.len() >= grenzen::STROEME_MAX {
                    Err(code::ZU_VIELE_STROEME)
                } else {
                    self.offen.insert(*strom);
                    Ok(())
                }
            }
            AnDienst::Abbrechen { strom } => {
                // Abbrechen eines unbekannten Stroms ist erlaubt (darf vor `start` eintreffen).
                self.offen.remove(strom);
                Ok(())
            }
            AnDienst::Ende { strom } | AnDienst::Guthaben { strom, .. } => {
                if self.offen.contains(strom) {
                    Ok(())
                } else {
                    Err(code::STROM_UNBEKANNT)
                }
            }
            AnDienst::Rueckmeldung { .. } | AnDienst::Status {} => Ok(()),
        }
    }

    /// Strom ist abgeschlossen (Ergebnis, Fertig, Fehler, Zeitüberschreitung).
    pub fn schliessen(&mut self, strom: u32) {
        self.offen.remove(&strom);
    }

    /// Audiorahmen nur für offene Ströme.
    pub fn audio_erlaubt(&self, strom: u32) -> bool {
        self.angemeldet && self.offen.contains(&strom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hallo() -> AnDienst {
        AnDienst::Hallo {
            protokoll: PROTOKOLL,
            client: "corporate-llm".into(),
            zugang: "z1".into(),
            beweis: "a".repeat(64),
        }
    }

    fn start(strom: u32) -> AnDienst {
        AnDienst::Start {
            strom,
            op: Operation::ErkennenStrom,
            bereich: "projekt-a".into(),
            souveraenitaet: Souveraenitaet::NurLokal,
        }
    }

    #[test]
    fn json_form_ist_stabil() {
        let m = start(3);
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(
            j,
            r#"{"t":"start","strom":3,"op":{"art":"erkennen_strom"},"bereich":"projekt-a","souveraenitaet":"nur_lokal"}"#
        );
        assert_eq!(serde_json::from_str::<AnDienst>(&j).unwrap(), m);
    }

    #[test]
    fn unbekannte_felder_werden_abgewiesen() {
        assert!(serde_json::from_str::<AnDienst>(r#"{"t":"status","x":1}"#).is_err());
        assert!(serde_json::from_str::<AnDienst>(r#"{"t":"loeschen"}"#).is_err());
        assert!(serde_json::from_str::<AnDienst>(r#"{"t":"ende","strom":1,"x":1}"#).is_err());
        assert!(serde_json::from_str::<AnDienst>(r#"{"t":"status"}"#).is_ok());
    }

    #[test]
    fn pruefung() {
        assert!(pruefen(&hallo()).is_ok());
        let mut alt = hallo();
        if let AnDienst::Hallo { protokoll, .. } = &mut alt {
            *protokoll = 99;
        }
        assert_eq!(pruefen(&alt), Err("protokoll".into()));
        assert!(pruefen(&AnDienst::Ende { strom: 0 }).is_err());
        assert!(
            pruefen(&AnDienst::Guthaben {
                strom: 1,
                rahmen: 1000
            })
            .is_err()
        );
        let sprechen = |text: &str| AnDienst::Start {
            strom: 1,
            op: Operation::Sprechen {
                text: text.into(),
                stimme: Some("provider:phonon:anton".into()),
            },
            bereich: "b".into(),
            souveraenitaet: Souveraenitaet::CloudErlaubt,
        };
        assert!(pruefen(&sprechen("Hallo")).is_ok());
        assert!(pruefen(&sprechen("  ")).is_err());
        assert!(pruefen(&sprechen("a\u{0007}b")).is_err());
        assert!(pruefen(&sprechen(&"a".repeat(grenzen::TEXT_MAX + 1))).is_err());
    }

    #[test]
    fn bereich_ist_keine_pfadangabe() {
        for b in ["..", ".", "a b", "a:b", "../x", "A", "-a", "ä", ""] {
            let m = AnDienst::Rueckmeldung {
                bereich: b.into(),
                paare: vec![Wortpaar {
                    gehoert: "Mayer".into(),
                    gemeint: "Meier".into(),
                }],
            };
            assert_eq!(pruefen(&m), Err("bereich".into()), "{b:?}");
        }
    }

    #[test]
    fn rueckmeldung_lernt_keine_zahlen_und_saetze() {
        let r = |g: &str, m: &str| AnDienst::Rueckmeldung {
            bereich: "b".into(),
            paare: vec![Wortpaar {
                gehoert: g.into(),
                gemeint: m.into(),
            }],
        };
        assert!(pruefen(&r("Mayer", "Meier")).is_ok());
        assert!(
            pruefen(&r("Mu\u{0308}ller", "Müller")).is_ok(),
            "zerlegter Umlaut"
        );
        assert!(pruefen(&r("Az12", "Az13")).is_err());
        assert!(pruefen(&r("zwei Worte", "drei")).is_err());
        assert!(pruefen(&r("Gleich", "gleich")).is_err());
        assert!(pruefen(&r("-", "'")).is_err());
    }

    #[test]
    fn sitzung_erzwingt_reihenfolge_und_stroeme() {
        let mut s = Sitzung::new();
        assert_eq!(s.annehmen(1, &start(1)), Err(code::NICHT_ANGEMELDET));
        assert!(s.annehmen(0, &hallo()).is_ok());
        s.anmelden();
        assert_eq!(
            s.annehmen(0, &hallo()),
            Err(code::UNGUELTIG),
            "zweites hallo"
        );
        assert_eq!(
            s.annehmen(2, &start(1)),
            Err(code::UNGUELTIG),
            "Kopf und Nachricht verschieden"
        );
        assert!(s.annehmen(1, &start(1)).is_ok());
        assert_eq!(s.annehmen(1, &start(1)), Err(code::STROM_BELEGT));
        for i in 2..=grenzen::STROEME_MAX as u32 {
            assert!(s.annehmen(i, &start(i)).is_ok());
        }
        assert_eq!(s.annehmen(99, &start(99)), Err(code::ZU_VIELE_STROEME));
        assert!(s.audio_erlaubt(1));
        assert!(s.annehmen(1, &AnDienst::Abbrechen { strom: 1 }).is_ok());
        assert!(!s.audio_erlaubt(1));
        assert_eq!(
            s.annehmen(1, &AnDienst::Ende { strom: 1 }),
            Err(code::STROM_UNBEKANNT)
        );
        assert!(
            s.annehmen(7, &AnDienst::Abbrechen { strom: 7 }).is_ok(),
            "abbrechen vor start"
        );
    }

    #[test]
    fn an_app_pruefung() {
        assert!(
            pruefen_an_app(&AnApp::Gruss {
                protokoll: PROTOKOLL,
                zufall: "0".repeat(64)
            })
            .is_ok()
        );
        assert!(
            pruefen_an_app(&AnApp::Gruss {
                protokoll: PROTOKOLL,
                zufall: "xyz".into()
            })
            .is_err()
        );
        assert!(pruefen_an_app(&AnApp::Fertig { strom: 0 }).is_err());
    }

    fn koppeln(code: &str, anzeige: &str) -> AnDienst {
        AnDienst::Koppeln {
            protokoll: PROTOKOLL,
            client: "corporate-llm".into(),
            anzeige: anzeige.into(),
            fingerabdruck: "0".repeat(64),
            code: code.into(),
        }
    }

    #[test]
    fn koppeln_felder() {
        assert!(pruefen(&koppeln("123456", "Corporate LLM")).is_ok());
        assert!(pruefen(&koppeln("12345", "Corporate LLM")).is_err());
        assert!(pruefen(&koppeln("12345a", "Corporate LLM")).is_err());
        assert!(pruefen(&koppeln("123456", "Böse\u{202E}gnalL")).is_err());
        assert!(pruefen(&koppeln("123456", "Null\u{200B}breite")).is_err());
        assert!(pruefen(&koppeln("123456", &"x".repeat(65))).is_err());
        let mut gross = koppeln("123456", "A");
        if let AnDienst::Koppeln { fingerabdruck, .. } = &mut gross {
            *fingerabdruck = "A".repeat(64);
        }
        assert!(pruefen(&gross).is_err());
        assert!(
            pruefen_an_app(&AnApp::Gekoppelt {
                zugang: "z-1".into()
            })
            .is_ok()
        );
        assert!(
            pruefen_an_app(&AnApp::Gekoppelt {
                zugang: "../z".into()
            })
            .is_err()
        );
    }

    #[test]
    fn koppeln_und_hallo_nur_einmal() {
        let mut s = Sitzung::new();
        assert!(s.annehmen(0, &koppeln("123456", "A")).is_ok());
        assert_eq!(s.annehmen(0, &hallo()), Err(code::UNGUELTIG));
        assert_eq!(s.annehmen(0, &koppeln("123456", "A")), Err(code::UNGUELTIG));
        let start = AnDienst::Start {
            strom: 1,
            op: Operation::Erkennen,
            bereich: "a".into(),
            souveraenitaet: Souveraenitaet::NurLokal,
        };
        assert_eq!(s.annehmen(1, &start), Err(code::NICHT_ANGEMELDET));
        let mut s = Sitzung::new();
        assert!(s.annehmen(0, &hallo()).is_ok());
        assert_eq!(s.annehmen(0, &koppeln("123456", "A")), Err(code::UNGUELTIG));
    }
}
