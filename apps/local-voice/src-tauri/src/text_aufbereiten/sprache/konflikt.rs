//! Regelvorschläge haben Vorrang vor vollständigen KI-Zeilengruppen.

use super::fehler::{GrundCode, Zaehler};
use super::vertrag::SprachGruppe;
use crate::aufbereitung::{pruefe_aenderungen, Aenderung, ErgebnisFehler};

pub fn beruehrt(a: [usize; 2], b: [usize; 2]) -> bool {
    match (a[0] == a[1], b[0] == b[1]) {
        (true, true) => a[0] == b[0],
        (true, false) => b[0] <= a[0] && a[0] <= b[1],
        (false, true) => a[0] <= b[0] && b[0] <= a[1],
        (false, false) => a[0] < b[1] && b[0] < a[1],
    }
}

pub fn verwerfe_regelkonflikte(
    ki: Vec<SprachGruppe>,
    regeln: &[Aenderung],
    zaehler: &mut Zaehler,
) -> Vec<SprachGruppe> {
    let mut bereiche = Vec::new();
    let mut einfuegungen = Vec::new();
    for regel in regeln {
        if regel.bereich[0] == regel.bereich[1] {
            einfuegungen.push(regel.bereich[0]);
        } else {
            bereiche.push(regel.bereich);
        }
    }
    bereiche.sort_unstable();
    einfuegungen.sort_unstable();
    // Das größte bisherige Ende erfasst auch verschachtelte Regelbereiche.
    let mut max_ende = 0;
    let enden = bereiche
        .iter()
        .map(|b| {
            max_ende = max_ende.max(b[1]);
            max_ende
        })
        .collect::<Vec<_>>();
    ki.into_iter()
        .filter(|gruppe| {
            let konflikt = gruppe.aenderungen.iter().any(|a| {
                let [start, ende] = a.bereich;
                let punkt = start == ende;
                let bis =
                    bereiche.partition_point(|b| if punkt { b[0] <= start } else { b[0] < ende });
                let bereich_trifft = bis > 0
                    && if punkt {
                        enden[bis - 1] >= start
                    } else {
                        enden[bis - 1] > start
                    };
                let ab = einfuegungen.partition_point(|p| *p < start);
                bereich_trifft || einfuegungen.get(ab).is_some_and(|p| *p <= ende)
            });
            if konflikt {
                zaehler.erhoehen(GrundCode::RegelKonflikt, 1);
            }
            !konflikt
        })
        .collect()
}

pub fn pruefe_gesamt(
    basis: &str,
    regeln: &[Aenderung],
    ki: &[SprachGruppe],
) -> Result<(), ErgebnisFehler> {
    let aenderungen = regeln
        .iter()
        .chain(ki.iter().flat_map(|g| &g.aenderungen))
        .cloned()
        .collect::<Vec<_>>();
    pruefe_aenderungen(basis, &aenderungen)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::{AenderungArt, AenderungStatus};

    fn gruppe(nr: usize, bereich: [usize; 2], alt: &str) -> SprachGruppe {
        SprachGruppe::neu(nr, vec![(bereich, alt.into(), "neu".into())])
    }

    fn regel(bereich: [usize; 2], alt: &str) -> Aenderung {
        let mut a = gruppe(999, bereich, alt).aenderungen.remove(0);
        a.art = AenderungArt::Regel("test".into());
        a.gruppe = None;
        a
    }

    #[test]
    fn beruehrung_unterscheidet_grenzen_und_einfuegungen() {
        for (a, b, erwartet) in [
            ([2, 5], [4, 8], true),
            ([2, 5], [2, 5], true),
            ([2, 5], [3, 4], true),
            ([2, 5], [5, 8], false),
            ([2, 5], [6, 8], false),
            ([2, 2], [2, 5], true),
            ([5, 5], [2, 5], true),
            ([3, 3], [2, 5], true),
            ([1, 1], [2, 5], false),
            ([6, 6], [2, 5], false),
            ([3, 3], [3, 3], true),
            ([3, 3], [4, 4], false),
        ] {
            assert_eq!(beruehrt(a, b), erwartet);
            assert_eq!(beruehrt(b, a), erwartet);
        }
    }

    #[test]
    fn ein_konflikt_verwirft_die_ganze_gruppe_auch_bei_abgelehnter_regel() {
        for status in [
            AenderungStatus::Offen,
            AenderungStatus::Angenommen,
            AenderungStatus::Abgelehnt,
        ] {
            let mut r = regel([3, 4], "d");
            r.status = status;
            let g = SprachGruppe::neu(
                2,
                vec![
                    ([0, 1], "a".into(), "b".into()),
                    ([3, 4], "d".into(), "e".into()),
                ],
            );
            let mut z = Zaehler::default();
            let rest = verwerfe_regelkonflikte(
                vec![gruppe(9, [8, 9], "i"), g, gruppe(1, [6, 7], "g")],
                &[r],
                &mut z,
            );
            assert_eq!(rest.iter().map(|g| g.zeile).collect::<Vec<_>>(), [9, 1]);
            assert_eq!(z.get(GrundCode::RegelKonflikt), 1);
        }
    }

    #[test]
    fn index_entspricht_beruehrung_auch_bei_verschachtelten_bereichen() {
        let regeln = vec![regel([2, 10], ""), regel([3, 4], ""), regel([12, 12], "")];
        for start in 0..15 {
            for ende in start..15 {
                let erwartet = regeln.iter().any(|r| beruehrt([start, ende], r.bereich));
                let mut z = Zaehler::default();
                let rest =
                    verwerfe_regelkonflikte(vec![gruppe(1, [start, ende], "")], &regeln, &mut z);
                assert_eq!(rest.is_empty(), erwartet);
                assert_eq!(z.summe(), u64::from(erwartet));
            }
        }
    }

    #[test]
    fn mehrere_konflikte_zaehlen_nur_eine_gruppe() {
        let g = SprachGruppe::neu(
            1,
            vec![
                ([0, 1], "a".into(), "b".into()),
                ([3, 4], "d".into(), "e".into()),
            ],
        );
        let mut z = Zaehler::default();
        let rest =
            verwerfe_regelkonflikte(vec![g], &[regel([0, 1], "a"), regel([3, 4], "d")], &mut z);
        assert!(rest.is_empty());
        assert_eq!(z.get(GrundCode::RegelKonflikt), 1);
    }

    #[test]
    fn gesamtpruefung_erkennt_ueberlappung_alt_und_schutz() {
        assert_eq!(
            pruefe_gesamt("abc", &[regel([0, 1], "a")], &[gruppe(1, [2, 3], "c")]),
            Ok(())
        );
        assert_eq!(
            pruefe_gesamt("abc", &[regel([0, 2], "ab")], &[gruppe(1, [1, 3], "bc")]),
            Err(ErgebnisFehler::Ueberlappung)
        );
        assert_eq!(
            pruefe_gesamt("abc", &[], &[gruppe(1, [0, 1], "x")]),
            Err(ErgebnisFehler::Alt)
        );
        assert_eq!(
            pruefe_gesamt("a{{S.1}}b", &[], &[gruppe(1, [1, 8], "{{S.1}}")]),
            Err(ErgebnisFehler::Geschuetzt)
        );
    }

    #[test]
    fn fuenftausend_gruppen_werden_schnell_gefiltert() {
        let ki = (0..5000)
            .map(|n| gruppe(n, [n * 4, n * 4 + 1], ""))
            .collect();
        let regeln = (0..5000)
            .map(|n| regel([n * 4 + 2, n * 4 + 3], ""))
            .collect::<Vec<_>>();
        let mut z = Zaehler::default();
        let start = std::time::Instant::now();
        let rest = verwerfe_regelkonflikte(ki, &regeln, &mut z);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
        assert_eq!(rest.len(), 5000);
        assert_eq!(z.summe(), 0);
    }
}
