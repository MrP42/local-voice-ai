//! Sequenzieller Abschnittsjob mit Wiederaufnahme und Prüfung gegen den festen Textstand.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::cache::{CacheEintrag, CacheSchluessel, SchluesselTeile, SprachCache, ZeilenKorrektur};
use super::diff::gruppe_fuer_zeile;
use super::fehler::{GrundCode, SprachFehler, Zaehler};
use super::konflikt::{pruefe_gesamt, verwerfe_regelkonflikte};
use super::maske::{maskiere_zeile, schutzbereiche_der_zeile, waehle_begrenzer};
use super::modell::Sprachversand;
use super::parser::{parse_antwort, Erlaubt};
use super::politik::Wahl;
use super::prompt::baue_auftrag;
use super::pruefung::pruefe_zeile;
use super::vertrag::{
    abschnitte_bilden, zeilen_text, zeilen_zerlegen, Grenzen, SprachGruppe, Textstand,
};
use crate::aufbereitung::Aenderung;
use crate::llm_versand::ZielArt;

#[derive(Clone)]
pub struct Abbruch(Arc<AtomicBool>, Option<Arc<dyn Fn() -> bool + Send + Sync>>);

impl Default for Abbruch {
    fn default() -> Self {
        Self::neu()
    }
}

impl Abbruch {
    pub fn neu() -> Self {
        Self(Arc::new(AtomicBool::new(false)), None)
    }

    /// Haelt nur vor einer neuen Anfrage an; eine laufende Antwort wird noch geprueft und gecacht.
    pub fn mit_anfragegrenze(signal: Arc<dyn Fn() -> bool + Send + Sync>) -> Self {
        Self(Arc::new(AtomicBool::new(false)), Some(signal))
    }

    fn vor_anfrage(&self) -> bool {
        self.ist_abgebrochen() || self.1.as_ref().is_some_and(|signal| signal())
    }

    pub fn abbrechen(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn ist_abgebrochen(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub struct JobEingabe<'a> {
    pub dokument_id: &'a str,
    pub fassung_id: Option<&'a str>,
    pub textstand: &'a Textstand,
    pub basis_text: &'a str,
    pub regeln: &'a [Aenderung],
    pub cache: &'a SprachCache,
    pub grenzen: Grenzen,
}

impl fmt::Debug for JobEingabe<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JobEingabe")
            .field("dokument_id_laenge", &self.dokument_id.len())
            .field("fassung_id_laenge", &self.fassung_id.map(str::len))
            .field("basis_laenge", &self.basis_text.len())
            .field("regeln_anzahl", &self.regeln.len())
            .field("grenzen", &self.grenzen)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobBericht {
    pub abschnitte_gesamt: usize,
    pub abschnitte_fertig: usize,
    pub aus_cache: usize,
    pub uebergross: usize,
    pub maskenkollision: usize,
    pub teilfehler: usize,
    pub gruppen: usize,
    pub verworfen: Zaehler,
    pub fehler: Option<SprachFehler>,
    pub cache_schreibfehler: bool,
    pub abgebrochen: bool,
}

#[derive(Debug)]
pub struct JobErgebnis {
    pub gruppen: Vec<SprachGruppe>,
    pub bericht: JobBericht,
}

fn pruefe_textstand(
    eingabe: &JobEingabe<'_>,
    aktueller_text: &(dyn Fn() -> Result<String, SprachFehler> + Send + Sync),
) -> Result<(), SprachFehler> {
    eingabe.textstand.pruefen(&aktueller_text()?)
}

fn ohne_gruppen(mut bericht: JobBericht, fehler: SprachFehler) -> JobErgebnis {
    bericht.fehler = Some(fehler);
    bericht.gruppen = 0;
    JobErgebnis {
        gruppen: Vec::new(),
        bericht,
    }
}

fn fertig(bericht: &mut JobBericht, fortschritt: &(dyn Fn(usize, usize) + Send + Sync)) {
    bericht.abschnitte_fertig += 1;
    fortschritt(bericht.abschnitte_fertig, bericht.abschnitte_gesamt);
}

fn mehrzeilige_zitatzeilen(text: &str, zeilen_anzahl: usize) -> Vec<bool> {
    let mut markiert = vec![false; zeilen_anzahl];
    let mut zeile = 0;
    let mut offen: Option<(usize, char)> = None;
    for c in text.chars() {
        if c == '\n' {
            zeile += 1;
            continue;
        }
        if let Some((start, zu)) = offen {
            if c == zu {
                if start < zeile {
                    markiert[start..=zeile].fill(true);
                }
                offen = None;
            }
        } else {
            let zu = match c {
                '„' => Some('“'),
                '"' => Some('"'),
                '“' => Some('”'),
                '»' => Some('«'),
                '«' => Some('»'),
                '‘' => Some('’'),
                _ => None,
            };
            if let Some(zu) = zu {
                offen = Some((zeile, zu));
            }
        }
    }
    if let Some((start, _)) = offen {
        // Ohne Schließer nur die Öffnerzeile und höchstens 20 Folgezeilen sperren.
        let ende = start.saturating_add(21).min(zeilen_anzahl);
        markiert[start..ende].fill(true);
    }
    markiert
}

pub async fn sprache_job(
    eingabe: &JobEingabe<'_>,
    versand: &Sprachversand,
    wahl: &Wahl,
    abbruch: &Abbruch,
    aktueller_text: &(dyn Fn() -> Result<String, SprachFehler> + Send + Sync),
    fortschritt: &(dyn Fn(usize, usize) + Send + Sync),
) -> JobErgebnis {
    let mut bericht = JobBericht::default();
    if !eingabe.textstand.gilt_fuer(eingabe.basis_text) {
        return ohne_gruppen(bericht, SprachFehler::BasisGeaendert);
    }
    let basis = eingabe.basis_text;
    let mut global = crate::aufbereitung::geschuetzte_bereiche(basis);
    global.sort_unstable();
    let zeilen = zeilen_zerlegen(basis);
    let zitatzeilen = mehrzeilige_zitatzeilen(basis, zeilen.len());
    let mut schutz_alle = global.clone();
    for zeile in &zeilen {
        if zeilen_text(basis, zeile).trim().is_empty() {
            continue;
        }
        schutz_alle.extend(
            schutzbereiche_der_zeile(basis, zeile, &global)
                .into_iter()
                .map(|[von, bis]| (zeile.bereich[0] + von, zeile.bereich[0] + bis)),
        );
    }
    schutz_alle.sort_unstable();
    schutz_alle.dedup();
    // Präfixmaxima berücksichtigen auch ineinander liegende Schutzbereiche ohne Vollsuche.
    let mut max_ende = 0;
    let enden = schutz_alle
        .iter()
        .map(|bereich| {
            max_ende = max_ende.max(bereich.1);
            max_ende
        })
        .collect::<Vec<_>>();
    let beruehrt_schutz = |gruppe: &SprachGruppe| {
        // Auch unmittelbar angrenzende Änderungen gelten als Berührung. Der Wort-Diff
        // verschmilzt sonst eine Einfügung an der Grenze mit dem Nachbarhunk.
        gruppe.aenderungen.iter().any(|a| {
            let [start, ende] = a.bereich;
            let bis = schutz_alle.partition_point(|b| b.0 <= ende);
            bis > 0 && enden[bis - 1] >= start
        })
    };
    let abschnitte = abschnitte_bilden(basis, eingabe.grenzen);
    bericht.abschnitte_gesamt = abschnitte.len();
    fortschritt(0, bericht.abschnitte_gesamt);
    let mut korrekturen = BTreeMap::new();

    for abschnitt in abschnitte {
        if abbruch.vor_anfrage() {
            bericht.abgebrochen = true;
            break;
        }
        if let Err(fehler) = pruefe_textstand(eingabe, aktueller_text) {
            return ohne_gruppen(bericht, fehler);
        }
        if abschnitt.uebergross {
            bericht.uebergross += 1;
            fertig(&mut bericht, fortschritt);
            continue;
        }
        let (Some(von), Some(bis)) = (abschnitt.erste_zeile(), abschnitt.letzte_zeile()) else {
            continue;
        };
        let schluessel = CacheSchluessel::bilden(&SchluesselTeile {
            dokument_id: eingabe.dokument_id,
            fassung_id: eingabe.fassung_id,
            basis: eingabe.textstand.basis(),
            abschnitt_von: von,
            abschnitt_bis: bis,
            anbieter_id: wahl.anbieter_id(),
            anbieter_adresse: wahl.provider().base_url.as_str(),
            modell: wahl.modell(),
            ziel_art: if wahl.art() == ZielArt::Lokal {
                "lokal"
            } else {
                "cloud"
            },
        });
        if let Ok(Some(eintrag)) = eingabe.cache.lesen(&schluessel) {
            if abbruch.ist_abgebrochen() {
                bericht.abgebrochen = true;
                break;
            }
            bericht.aus_cache += 1;
            bericht.verworfen.addieren(&eintrag.zaehler());
            for korrektur in eintrag.korrekturen {
                korrekturen.insert(korrektur.nr, korrektur.text);
            }
            fertig(&mut bericht, fortschritt);
            continue;
        }

        let abschnittstext = abschnitt
            .zeilen
            .iter()
            .map(|zeile| zeilen_text(basis, zeile))
            .collect::<Vec<_>>()
            .join("\n");
        let Some(begrenzer) = waehle_begrenzer(&abschnittstext) else {
            bericht.maskenkollision += 1;
            bericht.verworfen.erhoehen(GrundCode::MaskeBeschaedigt, 1);
            fertig(&mut bericht, fortschritt);
            continue;
        };
        let mut masken = BTreeMap::new();
        let mut quell_pipes = BTreeMap::new();
        let mut maskierte_zeilen = Vec::with_capacity(abschnitt.zeilen.len());
        for zeile in &abschnitt.zeilen {
            let schutz = schutzbereiche_der_zeile(basis, zeile, &global);
            let maske = maskiere_zeile(zeilen_text(basis, zeile), &schutz, begrenzer);
            quell_pipes.insert(zeile.nr, maske.text.matches('|').count());
            maskierte_zeilen.push((zeile.nr, maske.text.clone()));
            masken.insert(zeile.nr, maske);
        }
        let auftrag = baue_auftrag(&maskierte_zeilen);
        if abbruch.vor_anfrage() {
            bericht.abgebrochen = true;
            break;
        }
        let antwort = versand.anfragen(wahl, &auftrag).await;
        let antwort = match antwort {
            Ok(antwort) => antwort,
            Err(SprachFehler::Zeitlimit) => {
                bericht.teilfehler += 1;
                continue;
            }
            Err(SprachFehler::Abgebrochen) => {
                bericht.abgebrochen = true;
                break;
            }
            Err(fehler) => {
                bericht.fehler = Some(fehler);
                break;
            }
        };
        // Eine verspätete Antwort darf weder in den Cache noch in das Ergebnis gelangen.
        if abbruch.ist_abgebrochen() {
            bericht.abgebrochen = true;
            break;
        }
        if let Err(fehler) = pruefe_textstand(eingabe, aktueller_text) {
            return ohne_gruppen(bericht, fehler);
        }
        if abbruch.ist_abgebrochen() {
            bericht.abgebrochen = true;
            break;
        }
        if antwort.abgeschnitten {
            bericht.verworfen.erhoehen(GrundCode::AntwortZuLang, 1);
            bericht.teilfehler += 1;
            continue;
        }
        let abschnitt_zeilen = abschnitt.zeilen.iter().map(|z| z.nr).collect::<Vec<_>>();
        let geparst = parse_antwort(
            &antwort.text,
            &Erlaubt {
                abschnitt_zeilen: &abschnitt_zeilen,
                dokument_zeilen: zeilen.len(),
            },
            abschnitt.zeichen.saturating_mul(2).saturating_add(500),
            &quell_pipes,
        );
        let mut zaehler = geparst.verworfen;
        let mut neue_korrekturen = Vec::new();
        for (nr, neu) in geparst.zeilen {
            let Some(maske) = masken.get(&nr) else {
                continue;
            };
            if let Err(grund) = maske.pruefe(&neu) {
                zaehler.erhoehen(grund, 1);
                continue;
            }
            let neu_roh = maske.entmaskieren(&neu);
            let Some(zeile) = nr.checked_sub(1).and_then(|index| zeilen.get(index)) else {
                continue;
            };
            let original = zeilen_text(basis, zeile);
            if original == neu_roh {
                continue;
            }
            if zitatzeilen[nr - 1] {
                zaehler.erhoehen(GrundCode::AnfuehrungGeaendert, 1);
                continue;
            }
            if let Err(grund) = pruefe_zeile(original, &neu_roh) {
                zaehler.erhoehen(grund, 1);
                continue;
            }
            let Some(gruppe) = gruppe_fuer_zeile(zeile, original, &neu_roh) else {
                continue;
            };
            if beruehrt_schutz(&gruppe) {
                zaehler.erhoehen(GrundCode::GeschuetztBeruehrt, 1);
                continue;
            }
            neue_korrekturen.push(ZeilenKorrektur { nr, text: neu_roh });
        }
        let eintrag = CacheEintrag::aus_zaehler(neue_korrekturen, &zaehler);
        // Nur modellseitig vollständig geprüfte Korrekturen cachen. Regeln gehören nicht zum
        // Schlüssel; Regelkonflikte und Gesamtprüfung werden bei jedem Lauf neu entschieden.
        // Cachefehler verhindern keine geprüften Gruppen; Buchjobs werten sie separat aus.
        bericht.cache_schreibfehler |= eingabe.cache.schreiben(&schluessel, &eintrag).is_err();
        bericht.verworfen.addieren(&zaehler);
        for korrektur in eintrag.korrekturen {
            korrekturen.insert(korrektur.nr, korrektur.text);
        }
        fertig(&mut bericht, fortschritt);
    }

    if let Err(fehler) = pruefe_textstand(eingabe, aktueller_text) {
        return ohne_gruppen(bericht, fehler);
    }
    if abbruch.ist_abgebrochen() {
        bericht.abgebrochen = true;
    }
    let mut gruppen = Vec::new();
    for (nr, text) in korrekturen {
        let Some(zeile) = nr.checked_sub(1).and_then(|index| zeilen.get(index)) else {
            continue;
        };
        let Some(gruppe) = gruppe_fuer_zeile(zeile, zeilen_text(basis, zeile), &text) else {
            continue;
        };
        gruppen.push(gruppe);
    }
    let mut gruppen = verwerfe_regelkonflikte(gruppen, eingabe.regeln, &mut bericht.verworfen);
    if pruefe_gesamt(basis, eingabe.regeln, &gruppen).is_err() {
        gruppen.retain(|gruppe| {
            let gueltig =
                pruefe_gesamt(basis, eingabe.regeln, std::slice::from_ref(gruppe)).is_ok();
            if !gueltig {
                bericht.verworfen.erhoehen(GrundCode::GeschuetztBeruehrt, 1);
            }
            gueltig
        });
        if pruefe_gesamt(basis, eingabe.regeln, &gruppen).is_err() {
            bericht
                .verworfen
                .erhoehen(GrundCode::GeschuetztBeruehrt, gruppen.len() as u64);
            gruppen.clear();
        }
    }
    bericht.gruppen = gruppen.len();
    JobErgebnis { gruppen, bericht }
}

#[cfg(test)]
mod tests {
    use super::super::modell::mock::{test_versand, MockVersender};
    use super::super::modell::{Auftrag, ModellAntwort, VersandAusfall};
    use super::*;
    use crate::aufbereitung::{pruefe_aenderungen, AenderungArt, AenderungStatus};
    use crate::settings::{
        AppSettings, FunctionModel, LlmConnection, LlmModelConfig, FN_TEXT_AUFBEREITEN,
    };
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    fn test_einstellungen() -> AppSettings {
        let mut s = crate::settings::get_default_settings();
        s.llm_connections.clear();
        s.llm_models.clear();
        s.function_models.clear();
        s.llm_active_model_id = Some("local:eins".into());
        s.llm_connections.push(LlmConnection {
            id: "local".into(),
            kind: "local".into(),
            label: "local".into(),
            base_url: "http://127.0.0.1:0/v1".into(),
            enabled: true,
            monthly_budget_usd: None,
            budget_enforced: false,
            training_opt_out: true,
        });
        s.llm_models.push(LlmModelConfig {
            id: "local:eins".into(),
            connection_id: "local".into(),
            remote_id: "eins".into(),
            label: "eins".into(),
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
        s
    }

    fn antwort(text: &str) -> ModellAntwort {
        ModellAntwort {
            text: text.into(),
            abgeschnitten: false,
        }
    }

    fn korrigiere(auftrag: &Auftrag) -> ModellAntwort {
        let zeilen = auftrag
            .nutzer
            .lines()
            .filter_map(|zeile| {
                let (nr, text) = zeile.split_once('|')?;
                if nr.is_empty() || !nr.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                let neu = text.replace("Feler", "Fehler");
                (neu != text).then(|| format!("{nr}|{neu}"))
            })
            .collect::<Vec<_>>();
        antwort(&if zeilen.is_empty() {
            "OK".into()
        } else {
            zeilen.join("\n")
        })
    }

    fn korrektur_mock() -> Arc<MockVersender> {
        Arc::new(MockVersender::antworter(|auftrag, _| {
            Ok(korrigiere(auftrag))
        }))
    }

    fn eingabe<'a>(text: &'a str, stand: &'a Textstand, cache: &'a SprachCache) -> JobEingabe<'a> {
        JobEingabe {
            dokument_id: "dokument",
            fassung_id: Some("fassung"),
            textstand: stand,
            basis_text: text,
            regeln: &[],
            cache,
            grenzen: Grenzen {
                max_zeichen: 1000,
                max_zeilen: 1,
            },
        }
    }

    async fn lauf(
        eingabe: &JobEingabe<'_>,
        versand: &Sprachversand,
        abbruch: &Abbruch,
    ) -> JobErgebnis {
        sprache_job(
            eingabe,
            versand,
            &versand.vorschau().unwrap(),
            abbruch,
            &|| Ok(eingabe.basis_text.into()),
            &|_, _| {},
        )
        .await
    }

    fn alle_aenderungen(ergebnis: &JobErgebnis) -> Vec<Aenderung> {
        ergebnis
            .gruppen
            .iter()
            .flat_map(|g| g.aenderungen.iter().cloned())
            .collect()
    }

    #[tokio::test]
    async fn gueltige_unicode_gruppen_fortschritt_cache_und_angenommenes_ergebnis() {
        let text = "GEHEIMMARKE Ä🙂e\u{301}ß Feler.\nWeiterer Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let e = eingabe(text, &stand, &cache);
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let fortschritte = Mutex::new(Vec::new());
        let ergebnis = sprache_job(
            &e,
            &versand,
            &versand.vorschau().unwrap(),
            &Abbruch::neu(),
            &|| Ok(text.into()),
            &|fertig, gesamt| fortschritte.lock().unwrap().push((fertig, gesamt)),
        )
        .await;
        assert_eq!(ergebnis.gruppen.len(), 2);
        assert_eq!(ergebnis.bericht.abschnitte_fertig, 2);
        assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
        assert_eq!(*fortschritte.lock().unwrap(), [(0, 2), (1, 2), (2, 2)]);
        assert_eq!(mock.anzahl(), 2);
        assert_eq!(mock.lokal_aufrufe(), 2);
        assert_eq!(mock.cloud_aufrufe(), 0);
        assert!(mock
            .aufrufe()
            .iter()
            .all(|a| a.funktion == "text_aufbereiten"));
        let mut aenderungen = alle_aenderungen(&ergebnis);
        assert!(!aenderungen.is_empty());
        for a in &aenderungen {
            assert_eq!(a.art, AenderungArt::Sprache);
            assert_eq!(a.status, AenderungStatus::Offen);
            assert_eq!(a.alt, text[a.bereich[0]..a.bereich[1]]);
        }
        assert_eq!(pruefe_aenderungen(text, &aenderungen), Ok(()));
        let wieder = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(wieder.gruppen, ergebnis.gruppen);
        assert_eq!(wieder.bericht.aus_cache, 2);
        assert_eq!(mock.anzahl(), 2);
        for debug in [
            format!("{:?}", ergebnis.bericht),
            format!("{:?}", ergebnis.gruppen),
            format!("{e:?}"),
            SprachFehler::BasisGeaendert.to_string(),
        ] {
            assert!(!debug.contains("GEHEIMMARKE"));
            assert!(!debug.contains(text));
        }
        for a in &mut aenderungen {
            a.status = AenderungStatus::Angenommen;
        }
        let angenommen = crate::aufbereitung::ergebnis_text(text, &aenderungen).unwrap();
        assert_eq!(angenommen, text.replace("Feler", "Fehler"));
        let neuer_stand = Textstand::aus_text(&angenommen);
        let neu = lauf(
            &eingabe(&angenommen, &neuer_stand, &cache),
            &versand,
            &Abbruch::neu(),
        )
        .await;
        assert!(neu.gruppen.is_empty());
        assert_eq!(neu.bericht.aus_cache, 0);
        assert_eq!(mock.anzahl(), 4);
    }

    #[tokio::test]
    async fn unveraenderte_antworten_sind_zaehlerfrei_und_werden_gecacht() {
        for rueckgabe in ["OK", "1|Ä🙂e\u{301}ß bleibt."] {
            let text = "Ä🙂e\u{301}ß bleibt.";
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort(rueckgabe))]));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let e = eingabe(text, &stand, &cache);
            let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
            assert_eq!(cache.anzahl(), 1);
            assert_eq!(
                lauf(&e, &versand, &Abbruch::neu()).await.bericht.aus_cache,
                1
            );
            assert_eq!(mock.anzahl(), 1);
        }
    }

    #[tokio::test]
    async fn mehrere_inhaltszeilen_eines_abschnitts_behalten_globale_zeilennummern() {
        let text = "Ä🙂e\u{301}ß Feler.\n\nWeiterer Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let mut e = eingabe(text, &stand, &cache);
        e.grenzen.max_zeilen = 3;
        let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.bericht.abschnitte_gesamt, 1);
        assert_eq!(ergebnis.bericht.abschnitte_fertig, 1);
        assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
        assert_eq!(
            ergebnis.gruppen.iter().map(|g| g.zeile).collect::<Vec<_>>(),
            [1, 3]
        );
        assert_eq!(mock.anzahl(), 1);
        assert_eq!(
            pruefe_aenderungen(text, &alle_aenderungen(&ergebnis)),
            Ok(())
        );
    }

    #[tokio::test]
    async fn mehrzeilige_zitate_verwerfen_korrekturen_auch_vor_dem_cachen() {
        for (text, rueckgabe, erlaubt) in [
            (
                "„Anfang\nDie Ware ist rot.\nEnde“",
                "2|Die Ware ist blau.",
                vec![],
            ),
            (
                "Feler.\n„Anfang\nDie Ware ist rot.\nEnde“\nFeler.",
                "1|Fehler.\n3|Die Ware ist blau.\n5|Fehler.",
                vec![1, 5],
            ),
            (
                "Feler.\n“Anfang\nFeler Ende”\nFeler.",
                "1|Fehler.\n3|Fehler Ende”\n4|Fehler.",
                vec![1, 4],
            ),
        ] {
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort(rueckgabe))]));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let mut e = eingabe(text, &stand, &cache);
            e.grenzen.max_zeilen = 10;
            for aus_cache in [0, 1] {
                let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
                assert_eq!(
                    ergebnis.gruppen.iter().map(|g| g.zeile).collect::<Vec<_>>(),
                    erlaubt
                );
                assert_eq!(ergebnis.bericht.aus_cache, aus_cache);
                assert_eq!(
                    ergebnis
                        .bericht
                        .verworfen
                        .get(GrundCode::AnfuehrungGeaendert),
                    1
                );
                assert_eq!(ergebnis.bericht.verworfen.summe(), 1);
            }
            assert_eq!(mock.anzahl(), 1);
            assert_eq!(cache.anzahl(), 1);
        }
    }

    #[test]
    fn zitatmarkierung_beachtet_paare_und_begrenzt_nur_offene_zitate() {
        for (auf, zu) in [
            ('„', '“'),
            ('"', '"'),
            ('“', '”'),
            ('»', '«'),
            ('«', '»'),
            ('‘', '’'),
        ] {
            let text = format!("davor\n{auf}Anfang\nEnde{zu}\ndanach");
            assert_eq!(
                mehrzeilige_zitatzeilen(&text, 4),
                [false, true, true, false]
            );
            let text = format!("{auf}Anfang{zu}\nFeler.");
            assert_eq!(mehrzeilige_zitatzeilen(&text, 2), [false, false]);
            let text = format!("{auf}Anfang\n{}Ende{zu}\ndanach", "Feler.\n".repeat(22));
            let markiert = mehrzeilige_zitatzeilen(&text, 25);
            assert!(markiert[..24].iter().all(|markiert| *markiert));
            assert!(!markiert[24]);
        }
    }

    #[tokio::test]
    async fn offenes_zitat_sperrt_nur_oeffner_und_zwanzig_folgezeilen() {
        let mut zeilen = vec!["Feler.", "„Feler."];
        for _ in 0..22 {
            zeilen.push("Feler.");
        }
        let text = zeilen.join("\n");
        let stand = Textstand::aus_text(&text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let versand = test_versand(test_einstellungen(), korrektur_mock());
        let ergebnis = lauf(&eingabe(&text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert_eq!(
            ergebnis.gruppen.iter().map(|g| g.zeile).collect::<Vec<_>>(),
            [1, 23, 24]
        );
        assert_eq!(
            ergebnis
                .bericht
                .verworfen
                .get(GrundCode::AnfuehrungGeaendert),
            21
        );
        assert_eq!(ergebnis.bericht.verworfen.summe(), 21);
    }

    #[tokio::test]
    async fn parser_verwirft_fliesstext_mehrzeiler_und_falsche_ids() {
        for (rueckgabe, grund) in [
            ("Erklärung", GrundCode::FormatFremd),
            ("1|Ein Fehler.\nFortsetzung", GrundCode::FormatFremd),
            ("1|Ein Fehler.\n1|Ein Fehler.", GrundCode::IdDoppelt),
            ("2|Ein Fehler.", GrundCode::IdAbschnittsfremd),
            ("99|Ein Fehler.", GrundCode::IdUnbekannt),
            ("", GrundCode::AntwortLeer),
        ] {
            let text = "Ein Feler.\nAndere Zeile.";
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::skript(vec![
                Ok(antwort(rueckgabe)),
                Ok(antwort("OK")),
            ]));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let e = eingabe(text, &stand, &cache);
            let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(ergebnis.bericht.verworfen.get(grund), 1);
            assert_eq!(ergebnis.bericht.verworfen.summe(), 1);
            let wieder = lauf(&e, &versand, &Abbruch::neu()).await;
            assert_eq!(wieder.bericht.verworfen, ergebnis.bericht.verworfen);
            assert_eq!(wieder.bericht.aus_cache, 2);
            assert_eq!(mock.anzahl(), 2);
        }
    }

    #[tokio::test]
    async fn erfundene_inhalte_und_maskenschaeden_werden_abgewiesen() {
        for (text, rueckgabe, grund) in [
            (
                "Es sind 12 Dinge.",
                "1|Es sind 13 Dinge.",
                GrundCode::ZahlGeaendert,
            ),
            (
                "Das gilt nicht.",
                "1|Das gilt immer.",
                GrundCode::NegationGeaendert,
            ),
            (
                "Sie sagt „Feler“ hier.",
                "1|Sie sagt „Fehler“ hier.",
                GrundCode::AnfuehrungGeaendert,
            ),
            (
                "Siehe https://a.de hier.",
                "1|Siehe https://b.de hier.",
                GrundCode::MaskeBeschaedigt,
            ),
            (
                "Kurz.",
                "1|Ein sehr viel längerer erfundener Satz.",
                GrundCode::LaengeAbweichung,
            ),
            (
                "<Anna> hat Feler.",
                "1|Anna hat Fehler.",
                GrundCode::MaskeBeschaedigt,
            ),
        ] {
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::skript(vec![Ok(antwort(rueckgabe))]));
            let versand = test_versand(test_einstellungen(), mock);
            let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(ergebnis.bericht.verworfen.get(grund), 1);
        }
        // Eine zusätzliche URL lässt die vorhandene Maske intakt und erreicht die Inhaltsprüfung.
        let text = "Siehe https://a.de hier und dort im Text.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = Arc::new(MockVersender::antworter(|auftrag, _| {
            let mut a = korrigiere(&Auftrag::neu(None, auftrag.nutzer.replace("hier", "Feler")));
            a.text = a.text.replace("Fehler", "https://b.de");
            Ok(a)
        }));
        let versand = test_versand(test_einstellungen(), mock);
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.verworfen.get(GrundCode::UrlGeaendert), 1);
    }

    #[tokio::test]
    async fn marken_werden_nur_maskiert_versandt_und_unveraendert_erhalten() {
        let text = "<Anna> {{S.3}} [pause] Ä🙂e\u{301}ß hat einen Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = Arc::new(MockVersender::antworter(|auftrag, _| {
            for marke in ["<Anna>", "{{S.3}}", "[pause]"] {
                assert!(!auftrag.nutzer.contains(marke));
            }
            assert!(auftrag.nutzer.contains('⟦'));
            Ok(korrigiere(auftrag))
        }));
        let versand = test_versand(test_einstellungen(), mock);
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
        let mut aenderungen = alle_aenderungen(&ergebnis);
        for a in &mut aenderungen {
            a.status = AenderungStatus::Angenommen;
        }
        assert_eq!(
            crate::aufbereitung::ergebnis_text(text, &aenderungen).unwrap(),
            text.replace("Feler", "Fehler")
        );
    }

    #[tokio::test]
    async fn uebergroesse_und_maskenkollision_senden_nichts() {
        for (text, max_zeichen, uebergross, kollision) in [
            ("Ein sehr langer Feler.", 4, 1, 0),
            ("⟦ ⟧ ⟪ ⟫ ⦃ ⦄ Feler.", 1000, 0, 1),
        ] {
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = korrektur_mock();
            let versand = test_versand(test_einstellungen(), mock.clone());
            let mut e = eingabe(text, &stand, &cache);
            e.grenzen.max_zeichen = max_zeichen;
            let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(ergebnis.bericht.uebergross, uebergross);
            assert_eq!(ergebnis.bericht.maskenkollision, kollision);
            assert_eq!(ergebnis.bericht.abschnitte_fertig, 1);
            assert_eq!(
                ergebnis.bericht.verworfen.get(GrundCode::MaskeBeschaedigt),
                kollision as u64
            );
            assert_eq!(mock.anzahl(), 0);
            assert_eq!(cache.anzahl(), 0);
        }
    }

    #[tokio::test]
    async fn zeitlimit_und_abgeschnittene_antwort_werden_beim_fortsetzen_nachgeholt() {
        let text = "Erster Feler.\nZweiter Feler.\nDritter Feler.";
        for abgeschnitten in [false, true] {
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::antworter(move |auftrag, nr| {
                if nr == 1 {
                    if abgeschnitten {
                        let mut a = korrigiere(auftrag);
                        a.abgeschnitten = true;
                        return Ok(a);
                    }
                    return Err(VersandAusfall::Zeitlimit);
                }
                Ok(korrigiere(auftrag))
            }));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let e = eingabe(text, &stand, &cache);
            let erster = lauf(&e, &versand, &Abbruch::neu()).await;
            assert_eq!(erster.bericht.teilfehler, 1);
            assert_eq!(erster.bericht.fehler, None);
            assert_eq!(
                erster.gruppen.iter().map(|g| g.zeile).collect::<Vec<_>>(),
                [1, 3]
            );
            assert_eq!(cache.anzahl(), 2);
            assert_eq!(mock.anzahl(), 3);
            assert_eq!(
                erster.bericht.verworfen.get(GrundCode::AntwortZuLang),
                u64::from(abgeschnitten)
            );
            let weiter = lauf(&e, &versand, &Abbruch::neu()).await;
            assert_eq!(weiter.bericht.aus_cache, 2);
            assert_eq!(weiter.bericht.teilfehler, 0);
            assert_eq!(mock.anzahl(), 4);
            assert_eq!(cache.anzahl(), 3);
            let referenz_temp = tempfile::tempdir().unwrap();
            let referenz_cache = SprachCache::neu(referenz_temp.path());
            let referenz_versand = test_versand(test_einstellungen(), korrektur_mock());
            let referenz = lauf(
                &eingabe(text, &stand, &referenz_cache),
                &referenz_versand,
                &Abbruch::neu(),
            )
            .await;
            assert_eq!(weiter.gruppen, referenz.gruppen);
        }
    }

    #[tokio::test]
    async fn ueberlange_antwort_wird_vollstaendig_und_mit_zeichenlimit_verworfen() {
        let text = "Ä🙂e\u{301}ß Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let grenze = text.chars().count() * 2 + 500;
        let rueckgabe = format!("1|{}", "🙂".repeat(grenze));
        let mock = Arc::new(MockVersender::skript(vec![Ok(antwort(&rueckgabe))]));
        let versand = test_versand(test_einstellungen(), mock);
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.verworfen.get(GrundCode::AntwortZuLang), 1);
        assert_eq!(ergebnis.bericht.teilfehler, 0);
        assert_eq!(cache.anzahl(), 1);
    }

    #[tokio::test]
    async fn modell_abbruch_stoppt_ohne_fehler_und_ohne_cacheeintrag() {
        let text = "Ein Feler.\nWeiterer Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = Arc::new(MockVersender::skript(vec![Err(
            VersandAusfall::Abgebrochen,
        )]));
        let versand = test_versand(test_einstellungen(), mock.clone());
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert!(ergebnis.bericht.abgebrochen);
        assert_eq!(ergebnis.bericht.fehler, None);
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(cache.anzahl(), 0);
        assert_eq!(mock.anzahl(), 1);
    }

    #[tokio::test]
    async fn cachefehler_verhindern_weder_versand_noch_gruppen() {
        let temp = tempfile::tempdir().unwrap();
        // Eine Datei statt des Dokumentordners erzwingt Lese- und Schreibfehler im Cache.
        let datei = temp.path().join("datei");
        std::fs::write(&datei, b"kein Ordner").unwrap();
        let cache = SprachCache::neu(&datei);
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let e = eingabe(text, &stand, &cache);
        for _ in 0..2 {
            let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
            assert_eq!(ergebnis.gruppen.len(), 1);
            assert_eq!(ergebnis.bericht.fehler, None);
            assert_eq!(ergebnis.bericht.aus_cache, 0);
            assert!(ergebnis.bericht.cache_schreibfehler);
            assert_eq!(ergebnis.bericht.abschnitte_fertig, 1);
        }
        assert_eq!(mock.anzahl(), 2);
    }

    #[tokio::test]
    async fn fatale_versandfehler_stoppen_ohne_cloud_und_erhalten_fertige_gruppen() {
        for (ausfall, fehler) in [
            (VersandAusfall::Fehler, SprachFehler::LokalAusgefallen),
            (VersandAusfall::Gesperrt, SprachFehler::VersandGesperrt),
        ] {
            for vorher_fertig in [false, true] {
                let text = "Erster Feler.\nZweiter Feler.\nDritter Feler.";
                let stand = Textstand::aus_text(text);
                let temp = tempfile::tempdir().unwrap();
                let cache = SprachCache::neu(temp.path());
                let mock = Arc::new(MockVersender::antworter(move |auftrag, nr| {
                    if nr == usize::from(vorher_fertig) {
                        Err(ausfall)
                    } else {
                        Ok(korrigiere(auftrag))
                    }
                }));
                let versand = test_versand(test_einstellungen(), mock.clone());
                let ergebnis =
                    lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
                assert_eq!(ergebnis.bericht.fehler, Some(fehler));
                assert_eq!(ergebnis.gruppen.len(), usize::from(vorher_fertig));
                assert_eq!(mock.anzahl(), 1 + usize::from(vorher_fertig));
                assert_eq!(mock.cloud_aufrufe(), 0);
            }
        }
    }

    #[tokio::test]
    async fn expliziter_cloud_modellfehler_stoppt_nach_dem_ersten_versand() {
        let mut s = test_einstellungen();
        let mut verbindung = s.llm_connections[0].clone();
        verbindung.id = "openai".into();
        verbindung.kind = "openai".into();
        verbindung.label = "openai".into();
        verbindung.base_url = "https://api.openai.com/v1".into();
        s.llm_connections.push(verbindung);
        let mut modell = s.llm_models[0].clone();
        modell.id = "openai:fern".into();
        modell.connection_id = "openai".into();
        modell.remote_id = "fern".into();
        modell.label = "fern".into();
        s.llm_models.push(modell);
        s.function_models.insert(
            FN_TEXT_AUFBEREITEN.into(),
            FunctionModel {
                model_id: "openai:fern".into(),
                effort: None,
            },
        );
        let text = "Erster Feler.\nZweiter Feler.\nDritter Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = Arc::new(MockVersender::antworter(|_, _| Err(VersandAusfall::Fehler)));
        let versand = test_versand(s, mock.clone());
        assert_eq!(versand.vorschau().unwrap().art(), ZielArt::Cloud);
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.bericht.fehler, Some(SprachFehler::ModellFehler));
        assert_eq!(ergebnis.bericht.teilfehler, 0);
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(mock.anzahl(), 1);
        assert_eq!(mock.cloud_aufrufe(), 1);
        assert_eq!(cache.anzahl(), 0);
    }

    #[tokio::test]
    async fn abbruch_in_der_ersten_textstandpruefung_verhindert_den_versand() {
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let abbruch = Abbruch::neu();
        let abfragen = AtomicUsize::new(0);
        let ergebnis = sprache_job(
            &eingabe(text, &stand, &cache),
            &versand,
            &versand.vorschau().unwrap(),
            &abbruch,
            &|| {
                if abfragen.fetch_add(1, Ordering::SeqCst) == 0 {
                    abbruch.abbrechen();
                }
                Ok(text.into())
            },
            &|_, _| {},
        )
        .await;
        assert!(ergebnis.bericht.abgebrochen);
        assert_eq!(ergebnis.bericht.fehler, None);
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.abschnitte_fertig, 0);
        assert_eq!(cache.anzahl(), 0);
        assert_eq!(mock.anzahl(), 0);
    }

    #[tokio::test]
    async fn abbruch_in_der_ersten_textstandpruefung_verhindert_die_cacheuebernahme() {
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let e = eingabe(text, &stand, &cache);
        assert_eq!(lauf(&e, &versand, &Abbruch::neu()).await.gruppen.len(), 1);
        let abbruch = Abbruch::neu();
        let abfragen = AtomicUsize::new(0);
        let ergebnis = sprache_job(
            &e,
            &versand,
            &versand.vorschau().unwrap(),
            &abbruch,
            &|| {
                if abfragen.fetch_add(1, Ordering::SeqCst) == 0 {
                    abbruch.abbrechen();
                }
                Ok(text.into())
            },
            &|_, _| {},
        )
        .await;
        assert!(ergebnis.bericht.abgebrochen);
        assert_eq!(ergebnis.bericht.fehler, None);
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.gruppen, 0);
        assert_eq!(ergebnis.bericht.abschnitte_fertig, 0);
        assert_eq!(ergebnis.bericht.aus_cache, 0);
        assert_eq!(cache.anzahl(), 1);
        assert_eq!(mock.anzahl(), 1);
    }

    #[tokio::test]
    async fn abbruch_in_der_letzten_textstandpruefung_erhaelt_fertige_gruppen() {
        for aus_cache in [false, true] {
            let text = "Ein Feler.";
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = korrektur_mock();
            let versand = test_versand(test_einstellungen(), mock.clone());
            let e = eingabe(text, &stand, &cache);
            if aus_cache {
                assert_eq!(lauf(&e, &versand, &Abbruch::neu()).await.gruppen.len(), 1);
            }
            let abbruch = Abbruch::neu();
            let abfragen = AtomicUsize::new(0);
            let letzte_abfrage = if aus_cache { 2 } else { 3 };
            let ergebnis = sprache_job(
                &e,
                &versand,
                &versand.vorschau().unwrap(),
                &abbruch,
                &|| {
                    if abfragen.fetch_add(1, Ordering::SeqCst) + 1 == letzte_abfrage {
                        abbruch.abbrechen();
                    }
                    Ok(text.into())
                },
                &|_, _| {},
            )
            .await;
            assert_eq!(abfragen.load(Ordering::SeqCst), letzte_abfrage);
            assert!(ergebnis.bericht.abgebrochen);
            assert_eq!(ergebnis.bericht.fehler, None);
            assert_eq!(ergebnis.gruppen.len(), 1);
            assert_eq!(ergebnis.bericht.gruppen, 1);
            assert_eq!(ergebnis.bericht.abschnitte_fertig, 1);
            assert_eq!(ergebnis.bericht.aus_cache, usize::from(aus_cache));
            assert_eq!(cache.anzahl(), 1);
            assert_eq!(mock.anzahl(), 1);
        }
    }

    #[tokio::test]
    async fn abbruch_in_der_textstandpruefung_nach_antwort_verhindert_den_cacheeintrag() {
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let abbruch = Abbruch::neu();
        let abfragen = AtomicUsize::new(0);
        let ergebnis = sprache_job(
            &eingabe(text, &stand, &cache),
            &versand,
            &versand.vorschau().unwrap(),
            &abbruch,
            &|| {
                if abfragen.fetch_add(1, Ordering::SeqCst) == 1 {
                    abbruch.abbrechen();
                }
                Ok(text.into())
            },
            &|_, _| {},
        )
        .await;
        assert!(ergebnis.bericht.abgebrochen);
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.abschnitte_fertig, 0);
        assert_eq!(cache.anzahl(), 0);
        assert_eq!(mock.anzahl(), 1);
    }

    #[tokio::test]
    async fn letzte_textstandpruefung_nach_fortschritt_gilt_auch_fuer_cachetreffer() {
        for aus_cache in [false, true] {
            for loeschen in [false, true] {
                let text = "Ein Feler.";
                let stand = Textstand::aus_text(text);
                let temp = tempfile::tempdir().unwrap();
                let cache = SprachCache::neu(temp.path());
                let mock = korrektur_mock();
                let versand = test_versand(test_einstellungen(), mock.clone());
                let e = eingabe(text, &stand, &cache);
                if aus_cache {
                    assert_eq!(lauf(&e, &versand, &Abbruch::neu()).await.gruppen.len(), 1);
                }
                let fertig = AtomicBool::new(false);
                let abfragen = AtomicUsize::new(0);
                let ergebnis = sprache_job(
                    &e,
                    &versand,
                    &versand.vorschau().unwrap(),
                    &Abbruch::neu(),
                    &|| {
                        abfragen.fetch_add(1, Ordering::SeqCst);
                        if fertig.load(Ordering::SeqCst) {
                            if loeschen {
                                Err(SprachFehler::DokumentGeloescht)
                            } else {
                                Ok("Anderer Text".into())
                            }
                        } else {
                            Ok(text.into())
                        }
                    },
                    &|anzahl, gesamt| {
                        fertig.store(anzahl == gesamt, Ordering::SeqCst);
                    },
                )
                .await;
                assert!(ergebnis.gruppen.is_empty());
                assert_eq!(ergebnis.bericht.gruppen, 0);
                assert_eq!(ergebnis.bericht.abschnitte_fertig, 1);
                assert_eq!(ergebnis.bericht.aus_cache, usize::from(aus_cache));
                assert_eq!(
                    ergebnis.bericht.fehler,
                    Some(if loeschen {
                        SprachFehler::DokumentGeloescht
                    } else {
                        SprachFehler::BasisGeaendert
                    })
                );
                assert_eq!(
                    abfragen.load(Ordering::SeqCst),
                    if aus_cache { 2 } else { 3 }
                );
                assert_eq!(mock.anzahl(), 1);
            }
        }
    }

    #[tokio::test]
    async fn letzte_textstandpruefung_verwirft_gruppen_auch_nach_abbruch_oder_hartem_fehler() {
        for hart in [false, true] {
            let text = "Erster Feler.\nZweiter Feler.\nDritter Feler.";
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let beendet = Arc::new(AtomicBool::new(false));
            let signal = beendet.clone();
            let mock = Arc::new(MockVersender::antworter(move |auftrag, nr| {
                if nr == 1 {
                    signal.store(true, Ordering::SeqCst);
                    Err(if hart {
                        VersandAusfall::Fehler
                    } else {
                        VersandAusfall::Abgebrochen
                    })
                } else {
                    Ok(korrigiere(auftrag))
                }
            }));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let ergebnis = sprache_job(
                &eingabe(text, &stand, &cache),
                &versand,
                &versand.vorschau().unwrap(),
                &Abbruch::neu(),
                &|| {
                    if beendet.load(Ordering::SeqCst) {
                        Err(SprachFehler::DokumentGeloescht)
                    } else {
                        Ok(text.into())
                    }
                },
                &|_, _| {},
            )
            .await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(
                ergebnis.bericht.fehler,
                Some(SprachFehler::DokumentGeloescht)
            );
            assert_eq!(ergebnis.bericht.abgebrochen, !hart);
            assert_eq!(mock.anzahl(), 2);
            assert_eq!(cache.anzahl(), 1);
        }
    }

    #[tokio::test]
    async fn abbruch_vor_start_und_verspaetete_antwort_werden_nicht_gecacht() {
        let text = "Erster Feler.\nZweiter Feler.\nDritter Feler.";
        for abbruch_bei in [0, 1, 2] {
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let abbruch = Abbruch::neu();
            if abbruch_bei == 0 {
                abbruch.abbrechen();
            }
            let signal = abbruch.clone();
            let mock = Arc::new(MockVersender::antworter(move |auftrag, nr| {
                if nr + 1 == abbruch_bei {
                    signal.abbrechen();
                }
                Ok(korrigiere(auftrag))
            }));
            let versand = test_versand(test_einstellungen(), mock.clone());
            let e = eingabe(text, &stand, &cache);
            let erster = lauf(&e, &versand, &abbruch).await;
            let fertig = abbruch_bei.saturating_sub(1);
            assert!(erster.bericht.abgebrochen);
            assert_eq!(erster.gruppen.len(), fertig);
            assert_eq!(mock.anzahl(), abbruch_bei);
            assert_eq!(cache.anzahl(), fertig);
            let weiter = lauf(&e, &versand, &Abbruch::neu()).await;
            assert_eq!(weiter.bericht.aus_cache, fertig);
            assert_eq!(weiter.gruppen.len(), 3);
            assert_eq!(mock.anzahl(), abbruch_bei + 3 - fertig);
            assert_eq!(cache.anzahl(), 3);
            assert_eq!(
                weiter.gruppen.iter().map(|g| g.zeile).collect::<Vec<_>>(),
                [1, 2, 3]
            );
            let referenz_temp = tempfile::tempdir().unwrap();
            let referenz_cache = SprachCache::neu(referenz_temp.path());
            let referenz_versand = test_versand(test_einstellungen(), korrektur_mock());
            let referenz = lauf(
                &eingabe(text, &stand, &referenz_cache),
                &referenz_versand,
                &Abbruch::neu(),
            )
            .await;
            assert_eq!(weiter.gruppen, referenz.gruppen);
        }
    }

    #[tokio::test]
    async fn abbruch_zwischen_abschnitten_startet_den_naechsten_nicht() {
        let text = "Erster Feler.\nZweiter Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let abbruch = Abbruch::neu();
        let ergebnis = sprache_job(
            &eingabe(text, &stand, &cache),
            &versand,
            &versand.vorschau().unwrap(),
            &abbruch,
            &|| Ok(text.into()),
            &|fertig, _| {
                if fertig == 1 {
                    abbruch.abbrechen();
                }
            },
        )
        .await;
        assert!(ergebnis.bericht.abgebrochen);
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(cache.anzahl(), 1);
        assert_eq!(mock.anzahl(), 1);
    }

    #[tokio::test]
    async fn dokumentwechsel_oder_loeschung_verwirft_auch_bereits_fertige_gruppen() {
        for loeschen in [false, true] {
            for wechsel_bei in [1, 2, 3] {
                let text = "Erster Feler.\nZweiter Feler.";
                let stand = Textstand::aus_text(text);
                let temp = tempfile::tempdir().unwrap();
                let cache = SprachCache::neu(temp.path());
                let mock = korrektur_mock();
                let versand = test_versand(test_einstellungen(), mock.clone());
                let abfragen = AtomicUsize::new(0);
                let ergebnis = sprache_job(
                    &eingabe(text, &stand, &cache),
                    &versand,
                    &versand.vorschau().unwrap(),
                    &Abbruch::neu(),
                    &|| {
                        if abfragen.fetch_add(1, Ordering::SeqCst) >= wechsel_bei {
                            if loeschen {
                                Err(SprachFehler::DokumentGeloescht)
                            } else {
                                Ok("Anderer Text".into())
                            }
                        } else {
                            Ok(text.into())
                        }
                    },
                    &|_, _| {},
                )
                .await;
                assert!(ergebnis.gruppen.is_empty());
                assert_eq!(ergebnis.bericht.gruppen, 0);
                assert_eq!(
                    ergebnis.bericht.fehler,
                    Some(if loeschen {
                        SprachFehler::DokumentGeloescht
                    } else {
                        SprachFehler::BasisGeaendert
                    })
                );
                assert_eq!(cache.anzahl(), usize::from(wechsel_bei >= 2));
                assert_eq!(mock.anzahl(), if wechsel_bei == 3 { 2 } else { 1 });
            }
        }
    }

    #[tokio::test]
    async fn unpassender_textstand_stoppt_bereits_vor_dem_versand() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let stand = Textstand::aus_text("Andere Basis");
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let ergebnis = lauf(
            &eingabe("Ein Feler.", &stand, &cache),
            &versand,
            &Abbruch::neu(),
        )
        .await;
        assert_eq!(ergebnis.bericht.fehler, Some(SprachFehler::BasisGeaendert));
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(mock.anzahl(), 0);
    }

    #[tokio::test]
    async fn anderes_modell_oder_neue_basis_verwendet_keine_alten_cachetreffer() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let e = eingabe(text, &stand, &cache);
        assert_eq!(lauf(&e, &versand, &Abbruch::neu()).await.gruppen.len(), 1);
        let mut s = test_einstellungen();
        s.llm_models[0].id = "local:zwei".into();
        s.llm_models[0].remote_id = "zwei".into();
        s.llm_active_model_id = Some("local:zwei".into());
        let anders = test_versand(s, mock.clone());
        let ergebnis = lauf(&e, &anders, &Abbruch::neu()).await;
        assert_eq!(ergebnis.bericht.aus_cache, 0);
        assert_eq!(ergebnis.gruppen.len(), 1);
        let neu = "Weiterer Feler.";
        let neuer_stand = Textstand::aus_text(neu);
        let ergebnis = lauf(
            &eingabe(neu, &neuer_stand, &cache),
            &versand,
            &Abbruch::neu(),
        )
        .await;
        assert_eq!(ergebnis.bericht.aus_cache, 0);
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(mock.anzahl(), 3);
    }

    #[tokio::test]
    async fn andere_anbieteradresse_verwendet_keinen_alten_cachetreffer() {
        let text = "Ein Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let e = eingabe(text, &stand, &cache);
        assert_eq!(lauf(&e, &versand, &Abbruch::neu()).await.gruppen.len(), 1);
        let mut s = test_einstellungen();
        s.llm_connections[0].base_url = "http://127.0.0.1:1/v1".into();
        let anders = test_versand(s, mock.clone());
        let ergebnis = lauf(&e, &anders, &Abbruch::neu()).await;
        assert_eq!(ergebnis.bericht.aus_cache, 0);
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(cache.anzahl(), 2);
        assert_eq!(mock.anzahl(), 2);
        assert_eq!(
            lauf(&e, &anders, &Abbruch::neu()).await.bericht.aus_cache,
            1
        );
        assert_eq!(mock.anzahl(), 2);
    }

    #[tokio::test]
    async fn regelkonflikt_verwirft_die_ganze_ki_gruppe_auch_aus_dem_cache() {
        let text = "Ein Feler hier.\nWeiterer Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = korrektur_mock();
        let versand = test_versand(test_einstellungen(), mock.clone());
        let mut e = eingabe(text, &stand, &cache);
        let referenz = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(referenz.gruppen.len(), 2);
        assert_eq!(cache.anzahl(), 2);
        let mut regel = referenz.gruppen[0].aenderungen[0].clone();
        regel.id = "regel".into();
        regel.art = AenderungArt::Regel("test".into());
        regel.gruppe = None;
        let regeln = [regel];
        e.regeln = &regeln;
        let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(ergebnis.gruppen[0].zeile, 2);
        assert_eq!(ergebnis.bericht.verworfen.get(GrundCode::RegelKonflikt), 1);
        assert_eq!(ergebnis.bericht.aus_cache, 2);
        assert_eq!(mock.anzahl(), 2);
        assert_eq!(cache.anzahl(), 2);
        let mut frei = regeln[0].clone();
        frei.bereich = [0, 3];
        frei.alt = "Ein".into();
        frei.neu = "Der".into();
        let freie_regeln = [frei];
        e.regeln = &freie_regeln;
        let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.gruppen.len(), 2);
        assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
        assert_eq!(ergebnis.bericht.aus_cache, 2);
        assert_eq!(cache.anzahl(), 2);
        assert_eq!(mock.anzahl(), 2);
    }

    #[tokio::test]
    async fn gesamtpruefung_behaelt_bei_ungueltigen_regeln_keine_ki_gruppen() {
        let text = "Ein Feler.\nWeiterer Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let versand = test_versand(test_einstellungen(), korrektur_mock());
        let mut e = eingabe(text, &stand, &cache);
        let ohne_regel = lauf(&e, &versand, &Abbruch::neu()).await;
        assert_eq!(ohne_regel.gruppen.len(), 2);
        let mut regel = ohne_regel.gruppen[0].aenderungen[0].clone();
        regel.id = "ungueltige_regel".into();
        regel.art = AenderungArt::Regel("test".into());
        regel.gruppe = None;
        regel.bereich = [text.len() + 1, text.len() + 1];
        regel.alt.clear();
        let regeln = [regel];
        e.regeln = &regeln;
        let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(ergebnis.bericht.gruppen, 0);
        assert_eq!(
            ergebnis
                .bericht
                .verworfen
                .get(GrundCode::GeschuetztBeruehrt),
            2
        );
        assert_eq!(ergebnis.bericht.verworfen.get(GrundCode::RegelKonflikt), 0);
    }

    #[tokio::test]
    async fn verschobener_inline_code_wird_auch_in_spaeteren_zeilen_verworfen() {
        for vorspann in [false, true] {
            let text = if vorspann {
                "Vorspann.\nNutze `x` danach alpha beta gamma."
            } else {
                "Nutze `x` danach alpha beta gamma."
            };
            let stand = Textstand::aus_text(text);
            let temp = tempfile::tempdir().unwrap();
            let cache = SprachCache::neu(temp.path());
            let mock = Arc::new(MockVersender::antworter(move |_, nr| {
                if vorspann && nr == 0 {
                    Ok(antwort("OK"))
                } else {
                    Ok(antwort(&format!(
                        "{}|Nutze danach alpha beta gamma ⟦0⟧.",
                        if vorspann { 2 } else { 1 }
                    )))
                }
            }));
            let versand = test_versand(test_einstellungen(), mock);
            let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
            assert!(ergebnis.gruppen.is_empty());
            assert_eq!(
                ergebnis
                    .bericht
                    .verworfen
                    .get(GrundCode::GeschuetztBeruehrt),
                1
            );
            assert_eq!(ergebnis.bericht.verworfen.summe(), 1);
        }
    }

    #[tokio::test]
    async fn einfuegung_an_der_schutzgrenze_wird_als_ganze_gruppe_verworfen() {
        let text = "[pause] Feler.";
        let stand = Textstand::aus_text(text);
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let mock = Arc::new(MockVersender::antworter(|auftrag, _| {
            let a = korrigiere(auftrag);
            Ok(antwort(&a.text.replace("⟧ ", "⟧x ")))
        }));
        let versand = test_versand(test_einstellungen(), mock.clone());
        let e = eingabe(text, &stand, &cache);
        let ergebnis = lauf(&e, &versand, &Abbruch::neu()).await;
        assert!(ergebnis.gruppen.is_empty());
        assert_eq!(
            ergebnis
                .bericht
                .verworfen
                .get(GrundCode::GeschuetztBeruehrt),
            1
        );
        let wahl = versand.vorschau().unwrap();
        let schluessel = CacheSchluessel::bilden(&SchluesselTeile {
            dokument_id: e.dokument_id,
            fassung_id: e.fassung_id,
            basis: stand.basis(),
            abschnitt_von: 1,
            abschnitt_bis: 1,
            anbieter_id: wahl.anbieter_id(),
            anbieter_adresse: wahl.provider().base_url.as_str(),
            modell: wahl.modell(),
            ziel_art: "lokal",
        });
        let eintrag = cache.lesen(&schluessel).unwrap().unwrap();
        assert!(eintrag.korrekturen.is_empty());
        assert_eq!(eintrag.zaehler().get(GrundCode::GeschuetztBeruehrt), 1);
        let wieder = lauf(&e, &versand, &Abbruch::neu()).await;
        assert!(wieder.gruppen.is_empty());
        assert_eq!(wieder.bericht.aus_cache, 1);
        assert_eq!(wieder.bericht.verworfen, ergebnis.bericht.verworfen);
        assert_eq!(mock.anzahl(), 1);
        // Ohne Einfügung an der Grenze bleibt dieselbe Tippfehlerkorrektur zulässig.
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let versand = test_versand(test_einstellungen(), korrektur_mock());
        let ergebnis = lauf(&eingabe(text, &stand, &cache), &versand, &Abbruch::neu()).await;
        assert_eq!(ergebnis.gruppen.len(), 1);
        assert_eq!(ergebnis.bericht.verworfen.summe(), 0);
    }
}
