//! Unveränderlicher Textstand und eindeutige Zuständigkeit der Sprachvorschläge.

use super::fehler::SprachFehler;
use crate::aufbereitung::{Aenderung, AenderungArt, AenderungStatus, Basis};
use std::fmt;

pub const PROMPT_VERSION: u32 = 1;
pub const MASKIERUNG_VERSION: u32 = 1;
pub const PRUEF_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Eq)]
pub struct Textstand {
    basis: Basis,
}

impl fmt::Debug for Textstand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Textstand")
            .field("laenge", &self.basis.laenge)
            .field(
                "revision",
                &self.revision().chars().take(8).collect::<String>(),
            )
            .finish()
    }
}

impl Textstand {
    pub fn aus_text(text: &str) -> Self {
        Self {
            basis: Basis::aus_text(text),
        }
    }

    pub fn basis(&self) -> &Basis {
        &self.basis
    }

    pub fn revision(&self) -> &str {
        &self.basis.sha256
    }

    pub fn gilt_fuer(&self, aktuell: &str) -> bool {
        self.basis == Basis::aus_text(aktuell)
    }

    pub fn pruefen(&self, aktuell: &str) -> Result<(), SprachFehler> {
        if self.gilt_fuer(aktuell) {
            Ok(())
        } else {
            Err(SprachFehler::BasisGeaendert)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Zeile {
    pub nr: usize,
    pub bereich: [usize; 2],
}

pub fn zeilen_zerlegen(basis: &str) -> Vec<Zeile> {
    let mut zeilen = Vec::new();
    let mut start = 0;
    for (ende, byte) in basis.bytes().enumerate() {
        if byte == b'\n' {
            let inhalt_ende = if ende > start && basis.as_bytes()[ende - 1] == b'\r' {
                ende - 1
            } else {
                ende
            };
            zeilen.push(Zeile {
                nr: zeilen.len() + 1,
                bereich: [start, inhalt_ende],
            });
            start = ende + 1;
        }
    }
    if start < basis.len() {
        zeilen.push(Zeile {
            nr: zeilen.len() + 1,
            bereich: [start, basis.len()],
        });
    }
    zeilen
}

pub fn zeilen_text<'a>(basis: &'a str, zeile: &Zeile) -> &'a str {
    &basis[zeile.bereich[0]..zeile.bereich[1]]
}

pub fn hat_inhalt(basis: &str, zeile: &Zeile) -> bool {
    !zeilen_text(basis, zeile).trim().is_empty()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grenzen {
    pub max_zeichen: usize,
    pub max_zeilen: usize,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Abschnitt {
    pub index: usize,
    pub zeilen: Vec<Zeile>,
    pub zeichen: usize,
    pub uebergross: bool,
}

impl Abschnitt {
    pub fn erste_zeile(&self) -> Option<usize> {
        self.zeilen.first().map(|z| z.nr)
    }

    pub fn letzte_zeile(&self) -> Option<usize> {
        self.zeilen.last().map(|z| z.nr)
    }
}

pub fn abschnitte_bilden(basis: &str, grenzen: Grenzen) -> Vec<Abschnitt> {
    let max_zeichen = grenzen.max_zeichen.max(1);
    let max_zeilen = grenzen.max_zeilen.max(1);
    let mut abschnitte: Vec<Abschnitt> = Vec::new();
    for zeile in zeilen_zerlegen(basis) {
        if !hat_inhalt(basis, &zeile) {
            continue;
        }
        let zeichen = zeilen_text(basis, &zeile).chars().count();
        let passt = abschnitte.last().is_some_and(|a| {
            !a.uebergross
                && a.zeilen.len() < max_zeilen
                && zeichen <= max_zeichen.saturating_sub(a.zeichen)
        });
        if passt {
            if let Some(abschnitt) = abschnitte.last_mut() {
                abschnitt.zeichen += zeichen;
                abschnitt.zeilen.push(zeile);
            }
        } else {
            abschnitte.push(Abschnitt {
                index: abschnitte.len(),
                zeilen: vec![zeile],
                zeichen,
                uebergross: zeichen > max_zeichen,
            });
        }
    }
    abschnitte
}

pub fn gruppen_id(zeile_nr: usize) -> String {
    format!("sprache-z{zeile_nr}")
}

#[derive(Clone, PartialEq, Eq)]
pub struct SprachGruppe {
    pub zeile: usize,
    pub aenderungen: Vec<Aenderung>,
}

impl fmt::Debug for SprachGruppe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SprachGruppe")
            .field("zeile", &self.zeile)
            .field("anzahl", &self.aenderungen.len())
            .finish()
    }
}

impl SprachGruppe {
    pub fn neu(zeile: usize, teile: Vec<([usize; 2], String, String)>) -> Self {
        Self {
            zeile,
            aenderungen: teile
                .into_iter()
                .enumerate()
                .map(|(k, (bereich, alt, neu))| Aenderung {
                    id: format!("sprache-z{zeile}-{k}"),
                    art: AenderungArt::Sprache,
                    bereich,
                    alt,
                    neu,
                    grund_code: "sprache_ki".into(),
                    zaehler: None,
                    gruppe: Some(gruppen_id(zeile)),
                    status: AenderungStatus::Offen,
                })
                .collect(),
        }
    }

    pub fn id(&self) -> String {
        gruppen_id(self.zeile)
    }

    pub fn huelle(&self) -> Option<[usize; 2]> {
        let mut teile = self.aenderungen.iter();
        let mut huelle = teile.next()?.bereich;
        for teil in teile {
            huelle[0] = huelle[0].min(teil.bereich[0]);
            huelle[1] = huelle[1].max(teil.bereich[1]);
        }
        Some(huelle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeilenenden_und_unicode_bleiben_bytegenau() {
        let text = "Ä🙂e\u{301}\r\n\n \nX\rY\nEnde\r";
        let zeilen = zeilen_zerlegen(text);
        let erwartet = ["Ä🙂e\u{301}", "", " ", "X\rY", "Ende\r"];
        for (i, (zeile, inhalt)) in zeilen.iter().zip(erwartet).enumerate() {
            assert_eq!(zeile.nr, i + 1);
            assert!(zeile.bereich.iter().all(|p| text.is_char_boundary(*p)));
            assert_eq!(zeilen_text(text, zeile), inhalt);
            assert_eq!(hat_inhalt(text, zeile), !inhalt.trim().is_empty());
        }
        assert_eq!(zeilen.len(), erwartet.len());
        assert_eq!(zeilen[0].bereich, [0, "Ä🙂e\u{301}".len()]);
        assert!(zeilen_zerlegen("").is_empty());
        assert_eq!(zeilen_zerlegen("\n\n").len(), 2);
        assert_eq!(zeilen_zerlegen("a\n").len(), 1);
        assert_eq!(zeilen_zerlegen("a").len(), 1);
    }

    #[test]
    fn abschnitte_beachten_beide_grenzen_und_einzelzeilen() {
        let text = "ä🙂\ne\u{301}\n\n \nabcde\nx\ny";
        let teile = abschnitte_bilden(
            text,
            Grenzen {
                max_zeichen: 4,
                max_zeilen: 2,
            },
        );
        assert_eq!(
            teile.iter().map(|a| a.zeichen).collect::<Vec<_>>(),
            [4, 5, 2]
        );
        assert_eq!(teile[0].erste_zeile(), Some(1));
        assert_eq!(teile[0].letzte_zeile(), Some(2));
        assert!(teile[1].uebergross);
        assert!(!teile[2].uebergross);
        let grenzen = Grenzen {
            max_zeichen: 99,
            max_zeilen: 1,
        };
        assert_eq!(abschnitte_bilden("a\nb", grenzen).len(), 2);
        assert_eq!(
            abschnitte_bilden(
                "aaa\nbb",
                Grenzen {
                    max_zeichen: 4,
                    max_zeilen: 2,
                },
            )
            .len(),
            2
        );
        assert_eq!(
            abschnitte_bilden(
                "aa\nbb",
                Grenzen {
                    max_zeichen: 4,
                    max_zeilen: 2,
                },
            )
            .len(),
            1
        );
        let null = Grenzen {
            max_zeichen: 0,
            max_zeilen: 0,
        };
        let teile = abschnitte_bilden("ä\n🙂\nxx", null);
        assert_eq!(teile.len(), 3);
        assert!(!teile[0].uebergross);
        assert!(teile[2].uebergross);
        assert!(abschnitte_bilden("\n \n\t", null).is_empty());
    }

    #[test]
    fn zweihundert_zeilen_sind_vollstaendig_und_disjunkt() {
        let text = (1..=200)
            .map(|n| {
                if n % 7 == 0 {
                    " \n"
                } else {
                    "Ä🙂e\u{301}\n"
                }
            })
            .collect::<String>();
        let teile = abschnitte_bilden(
            &text,
            Grenzen {
                max_zeichen: 17,
                max_zeilen: 3,
            },
        );
        let erwartet = zeilen_zerlegen(&text)
            .into_iter()
            .filter(|z| hat_inhalt(&text, z))
            .collect::<Vec<_>>();
        let erhalten = teile
            .iter()
            .flat_map(|a| a.zeilen.clone())
            .collect::<Vec<_>>();
        assert_eq!(erhalten, erwartet);
        for (index, teil) in teile.iter().enumerate() {
            assert_eq!(teil.index, index);
            assert!(teil.zeichen <= 17);
            assert!(teil.zeilen.len() <= 3);
        }
    }

    #[test]
    fn gruppen_bauen_feste_ids_und_huellen_ohne_textdebug() {
        let gruppe = SprachGruppe::neu(
            42,
            vec![
                ([8, 9], "GEHEIMMARKE".into(), "neu".into()),
                ([2, 4], "ab".into(), "c".into()),
            ],
        );
        assert_eq!(gruppe.id(), "sprache-z42");
        assert_eq!(gruppe.huelle(), Some([2, 9]));
        for (k, a) in gruppe.aenderungen.iter().enumerate() {
            assert_eq!(a.id, format!("sprache-z42-{k}"));
            assert_eq!(a.gruppe, Some(gruppen_id(42)));
            assert_eq!(a.art, AenderungArt::Sprache);
            assert_eq!(a.status, AenderungStatus::Offen);
            assert_eq!(a.grund_code, "sprache_ki");
            assert_eq!(a.zaehler, None);
        }
        assert_eq!(SprachGruppe::neu(1, vec![]).huelle(), None);
        assert!(!format!("{gruppe:?}").contains("GEHEIMMARKE"));
    }

    #[test]
    fn textstand_erkennt_bereits_ein_anderes_zeichen() {
        let stand = Textstand::aus_text("GEHEIMMARKE ä🙂");
        assert!(stand.gilt_fuer("GEHEIMMARKE ä🙂"));
        assert!(!stand.gilt_fuer("GEHEIMMARKE ö🙂"));
        assert_eq!(stand.pruefen("GEHEIMMARKE ä🙂"), Ok(()));
        let fehler = stand.pruefen("GEHEIMMARKE ö🙂").unwrap_err();
        assert_eq!(fehler, SprachFehler::BasisGeaendert);
        assert_eq!(fehler.code(), "basis_geaendert");
        assert_eq!(stand.basis(), &Basis::aus_text("GEHEIMMARKE ä🙂"));
        let debug = format!("{stand:?}");
        assert!(!debug.contains("GEHEIMMARKE"));
        assert!(debug.contains(&stand.revision()[..8]));
        assert!(!debug.contains(stand.revision()));
    }
}
