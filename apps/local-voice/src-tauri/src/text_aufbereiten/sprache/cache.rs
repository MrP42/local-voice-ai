//! Dokumentgebundener Cache ausschließlich für geprüfte Zeilenkorrekturen und Zähler.

use super::fehler::{GrundCode, SprachFehler, Zaehler};
use super::vertrag::{MASKIERUNG_VERSION, PROMPT_VERSION, PRUEF_VERSION};
use crate::aufbereitung::Basis;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CacheSchluessel(String);

impl fmt::Debug for CacheSchluessel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("CacheSchluessel")
            .field(&self.0.chars().take(8).collect::<String>())
            .finish()
    }
}

pub struct SchluesselTeile<'a> {
    pub dokument_id: &'a str,
    pub fassung_id: Option<&'a str>,
    pub basis: &'a Basis,
    pub abschnitt_von: usize,
    pub abschnitt_bis: usize,
    pub anbieter_id: &'a str,
    pub modell: &'a str,
    pub ziel_art: &'a str,
}

impl fmt::Debug for SchluesselTeile<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SchluesselTeile")
            .field("dokument_id_laenge", &self.dokument_id.len())
            .field("fassung_id_laenge", &self.fassung_id.map(str::len))
            .field("basis_laenge", &self.basis.laenge)
            .field("abschnitt_von", &self.abschnitt_von)
            .field("abschnitt_bis", &self.abschnitt_bis)
            .field("anbieter_id_laenge", &self.anbieter_id.len())
            .field("modell_laenge", &self.modell.len())
            .field("ziel_art_laenge", &self.ziel_art.len())
            .finish()
    }
}

impl CacheSchluessel {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn bilden(teile: &SchluesselTeile<'_>) -> Self {
        Self::bilden_mit_versionen(teile, PROMPT_VERSION, MASKIERUNG_VERSION, PRUEF_VERSION)
    }

    fn bilden_mit_versionen(
        teile: &SchluesselTeile<'_>,
        prompt: u32,
        maske: u32,
        pruef: u32,
    ) -> Self {
        let mut hash = Sha256::new();
        // Auch Zahlen werden als längenpräfixierte Felder kodiert.
        let mut feld = |bytes: &[u8]| {
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        };
        feld(b"lva712-sprache-cache-v1");
        feld(teile.dokument_id.as_bytes());
        match teile.fassung_id {
            Some(wert) => {
                feld(&[1]);
                feld(wert.as_bytes());
            }
            None => feld(&[0]),
        }
        feld(teile.basis.sha256.as_bytes());
        feld(&teile.basis.laenge.to_le_bytes());
        feld(&(teile.abschnitt_von as u64).to_le_bytes());
        feld(&(teile.abschnitt_bis as u64).to_le_bytes());
        feld(teile.anbieter_id.as_bytes());
        feld(teile.modell.as_bytes());
        feld(teile.ziel_art.as_bytes());
        feld(&prompt.to_le_bytes());
        feld(&maske.to_le_bytes());
        feld(&pruef.to_le_bytes());
        Self(format!("{:x}", hash.finalize()))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ZeilenKorrektur {
    pub nr: usize,
    pub text: String,
}

impl fmt::Debug for ZeilenKorrektur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ZeilenKorrektur")
            .field("nr", &self.nr)
            .field("text_laenge", &self.text.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEintrag {
    pub korrekturen: Vec<ZeilenKorrektur>,
    pub verworfen: Vec<(String, u64)>,
}

impl fmt::Debug for CacheEintrag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let summe = self
            .verworfen
            .iter()
            .fold(0u64, |summe, (_, n)| summe.saturating_add(*n));
        f.debug_struct("CacheEintrag")
            .field("korrekturen_anzahl", &self.korrekturen.len())
            .field("verworfen_anzahl", &self.verworfen.len())
            .field("verworfen_summe", &summe)
            .finish()
    }
}

const GRUND_CODES: [GrundCode; 14] = [
    GrundCode::FormatFremd,
    GrundCode::IdUnbekannt,
    GrundCode::IdDoppelt,
    GrundCode::IdAbschnittsfremd,
    GrundCode::AntwortZuLang,
    GrundCode::LaengeAbweichung,
    GrundCode::ZahlGeaendert,
    GrundCode::UrlGeaendert,
    GrundCode::AnfuehrungGeaendert,
    GrundCode::NegationGeaendert,
    GrundCode::MaskeBeschaedigt,
    GrundCode::GeschuetztBeruehrt,
    GrundCode::RegelKonflikt,
    GrundCode::AntwortLeer,
];

impl CacheEintrag {
    pub fn aus_zaehler(korrekturen: Vec<ZeilenKorrektur>, z: &Zaehler) -> Self {
        Self {
            korrekturen,
            verworfen: z
                .iter()
                .map(|(grund, n)| (grund.code().into(), n))
                .collect(),
        }
    }

    pub fn zaehler(&self) -> Zaehler {
        let mut z = Zaehler::default();
        // Auch beschädigte Zähler dürfen beim Einlesen keinen Überlauf auslösen.
        let mut rest = u64::MAX;
        for (code, n) in &self.verworfen {
            if let Some(grund) = GRUND_CODES
                .iter()
                .find(|grund| grund.code() == code.as_str())
            {
                let anzahl = (*n).min(rest);
                z.erhoehen(*grund, anzahl);
                rest -= anzahl;
            }
        }
        z
    }
}

#[derive(Serialize, Deserialize)]
struct CacheDatei {
    schema_version: u32,
    schluessel: String,
    eintrag: CacheEintrag,
}

impl fmt::Debug for CacheDatei {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheDatei")
            .field("schema_version", &self.schema_version)
            .field("schluessel_laenge", &self.schluessel.len())
            .field("eintrag", &self.eintrag)
            .finish()
    }
}

pub struct SprachCache {
    ordner: PathBuf,
}

impl fmt::Debug for SprachCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SprachCache").finish_non_exhaustive()
    }
}

impl SprachCache {
    pub fn neu(dokument_ordner: impl AsRef<Path>) -> Self {
        Self {
            ordner: dokument_ordner
                .as_ref()
                .join("aufbereitung")
                .join("sprache-cache"),
        }
    }

    fn pfad(&self, s: &CacheSchluessel) -> PathBuf {
        self.ordner.join(format!("{}.json", s.as_str()))
    }

    pub fn lesen(&self, s: &CacheSchluessel) -> Result<Option<CacheEintrag>, SprachFehler> {
        let bytes = match fs::read(self.pfad(s)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SprachFehler::Cache),
        };
        let datei: CacheDatei = match serde_json::from_slice(&bytes) {
            Ok(datei) => datei,
            Err(_) => return Ok(None),
        };
        if datei.schema_version != 1 || datei.schluessel != s.as_str() {
            return Ok(None);
        }
        Ok(Some(datei.eintrag))
    }

    pub fn schreiben(&self, s: &CacheSchluessel, e: &CacheEintrag) -> Result<(), SprachFehler> {
        let bytes = serde_json::to_vec(&CacheDatei {
            schema_version: 1,
            schluessel: s.as_str().into(),
            eintrag: e.clone(),
        })
        .map_err(|_| SprachFehler::Cache)?;
        fs::create_dir_all(&self.ordner).map_err(|_| SprachFehler::Cache)?;
        let pfad = self.pfad(s);
        match fs::read(&pfad) {
            Ok(vorhanden) if vorhanden == bytes => return Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(_) => return Err(SprachFehler::Cache),
        }
        crate::commands::pages::write_atomic(&pfad, &bytes).map_err(|_| SprachFehler::Cache)
    }

    pub fn anzahl(&self) -> usize {
        let Ok(eintraege) = fs::read_dir(&self.ordner) else {
            return 0;
        };
        eintraege
            .filter_map(Result::ok)
            .filter(|e| {
                e.path().extension().is_some_and(|endung| endung == "json")
                    && e.file_type().is_ok_and(|art| art.is_file())
            })
            .count()
    }

    pub fn loeschen_alle(&self) -> Result<(), SprachFehler> {
        match fs::remove_dir_all(&self.ordner) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SprachFehler::Cache),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn teile(basis: &Basis) -> SchluesselTeile<'_> {
        SchluesselTeile {
            dokument_id: "dokument",
            fassung_id: Some("fassung"),
            basis,
            abschnitt_von: 1,
            abschnitt_bis: 4,
            anbieter_id: "anbieter",
            modell: "modell",
            ziel_art: "lokal",
        }
    }

    fn eintrag(text: &str) -> CacheEintrag {
        let mut z = Zaehler::default();
        z.erhoehen(GrundCode::RegelKonflikt, 2);
        CacheEintrag::aus_zaehler(
            vec![ZeilenKorrektur {
                nr: 3,
                text: text.into(),
            }],
            &z,
        )
    }

    #[test]
    fn jedes_schluesselfeld_und_jede_version_ist_wirksam() {
        let basis = Basis::aus_text("Ä🙂e\u{301}");
        let original = CacheSchluessel::bilden(&teile(&basis));
        assert_eq!(original.as_str().len(), 64);
        assert!(original
            .as_str()
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        for feld in 0..12 {
            let mut andere_basis = basis.clone();
            if feld == 3 {
                andere_basis.sha256 = "0".repeat(64);
            }
            if feld == 4 {
                andere_basis.laenge += 1;
            }
            let mut t = teile(&andere_basis);
            match feld {
                0 => t.dokument_id = "anders",
                1 => t.fassung_id = None,
                2 => t.fassung_id = Some("andere"),
                3 | 4 => {}
                5 => t.abschnitt_von += 1,
                6 => t.abschnitt_bis += 1,
                7 => t.anbieter_id = "anders",
                8 => t.modell = "anders",
                9 => t.ziel_art = "cloud",
                10 => t.fassung_id = Some(""),
                11 => t.dokument_id = "",
                _ => unreachable!(),
            }
            assert_ne!(CacheSchluessel::bilden(&t), original, "Feld {feld}");
        }
        let t = teile(&basis);
        assert_eq!(
            original,
            CacheSchluessel::bilden_mit_versionen(
                &t,
                PROMPT_VERSION,
                MASKIERUNG_VERSION,
                PRUEF_VERSION
            )
        );
        for versionen in [
            (PROMPT_VERSION + 1, MASKIERUNG_VERSION, PRUEF_VERSION),
            (PROMPT_VERSION, MASKIERUNG_VERSION + 1, PRUEF_VERSION),
            (PROMPT_VERSION, MASKIERUNG_VERSION, PRUEF_VERSION + 1),
        ] {
            assert_ne!(
                original,
                CacheSchluessel::bilden_mit_versionen(&t, versionen.0, versionen.1, versionen.2)
            );
        }
    }

    #[test]
    fn laengenpraefixe_und_optionsmarker_verhindern_kollisionen() {
        let basis = Basis::aus_text("");
        let mut a = teile(&basis);
        a.dokument_id = "ab";
        a.fassung_id = Some("c");
        let mut b = teile(&basis);
        b.dokument_id = "a";
        b.fassung_id = Some("bc");
        assert_ne!(CacheSchluessel::bilden(&a), CacheSchluessel::bilden(&b));
        a.fassung_id = None;
        b.dokument_id = "ab";
        b.fassung_id = Some("");
        assert_ne!(CacheSchluessel::bilden(&a), CacheSchluessel::bilden(&b));
    }

    #[test]
    fn rundreise_ist_idempotent_und_andere_inhalte_ersetzen_den_eintrag() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let s = CacheSchluessel::bilden(&teile(&Basis::aus_text("basis")));
        assert!(!cache.ordner.exists());
        assert_eq!(cache.lesen(&s), Ok(None));
        assert_eq!(cache.anzahl(), 0);
        let e = eintrag("Ä🙂e\u{301}");
        cache.schreiben(&s, &e).unwrap();
        assert_eq!(cache.lesen(&s), Ok(Some(e.clone())));
        let bytes = fs::read(cache.pfad(&s)).unwrap();
        let zeit = fs::metadata(cache.pfad(&s)).unwrap().modified().unwrap();
        cache.schreiben(&s, &e).unwrap();
        assert_eq!(fs::read(cache.pfad(&s)).unwrap(), bytes);
        assert_eq!(
            fs::metadata(cache.pfad(&s)).unwrap().modified().unwrap(),
            zeit
        );
        let anders = eintrag("anderer Inhalt");
        cache.schreiben(&s, &anders).unwrap();
        assert_eq!(cache.lesen(&s), Ok(Some(anders)));
        assert_ne!(fs::read(cache.pfad(&s)).unwrap(), bytes);
        assert_eq!(cache.anzahl(), 1);
    }

    #[test]
    fn beschaedigte_fremde_und_umbenannte_dateien_sind_fehltreffer() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let basis = Basis::aus_text("basis");
        let s = CacheSchluessel::bilden(&teile(&basis));
        cache.schreiben(&s, &eintrag("korrekt")).unwrap();
        let bytes = fs::read(cache.pfad(&s)).unwrap();
        fs::write(cache.pfad(&s), b"{kaputt").unwrap();
        assert_eq!(cache.lesen(&s), Ok(None));
        let mut fremd: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        fremd["schema_version"] = serde_json::json!(2);
        fs::write(cache.pfad(&s), serde_json::to_vec(&fremd).unwrap()).unwrap();
        assert_eq!(cache.lesen(&s), Ok(None));
        fs::write(cache.pfad(&s), &bytes).unwrap();
        let mut t = teile(&basis);
        t.modell = "anderes";
        let anderes = CacheSchluessel::bilden(&t);
        fs::rename(cache.pfad(&s), cache.pfad(&anderes)).unwrap();
        assert_eq!(cache.lesen(&anderes), Ok(None));
        assert_eq!(cache.lesen(&s), Ok(None));
    }

    #[test]
    fn echte_ea_fehler_werden_als_fester_code_gemeldet() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        let s = CacheSchluessel::bilden(&teile(&Basis::aus_text("")));
        fs::create_dir_all(cache.pfad(&s)).unwrap();
        assert_eq!(cache.lesen(&s), Err(SprachFehler::Cache));
        assert_eq!(
            cache.schreiben(&s, &eintrag("text")),
            Err(SprachFehler::Cache)
        );
        cache.loeschen_alle().unwrap();
        fs::write(&cache.ordner, b"Datei statt Ordner").unwrap();
        assert_eq!(
            cache.schreiben(&s, &eintrag("text")),
            Err(SprachFehler::Cache)
        );
        assert_eq!(cache.loeschen_alle(), Err(SprachFehler::Cache));
    }

    #[test]
    fn loeschen_entfernt_auch_tempreste_und_speicher_loescht_cache_mit() {
        let temp = tempfile::tempdir().unwrap();
        let cache = SprachCache::neu(temp.path());
        assert_eq!(
            cache.ordner,
            temp.path().join("aufbereitung").join("sprache-cache")
        );
        cache.loeschen_alle().unwrap();
        let s = CacheSchluessel::bilden(&teile(&Basis::aus_text("basis")));
        cache.schreiben(&s, &eintrag("korrekt")).unwrap();
        fs::write(cache.ordner.join("liegengeblieben.tmp"), b"Rest").unwrap();
        fs::create_dir(cache.ordner.join("verzeichnis.json")).unwrap();
        assert_eq!(cache.anzahl(), 1);
        cache.loeschen_alle().unwrap();
        assert!(!cache.ordner.exists());
        cache.loeschen_alle().unwrap();
        cache.schreiben(&s, &eintrag("korrekt")).unwrap();
        crate::aufbereitung::AufbereitungsSpeicher::neu(temp.path())
            .alles_loeschen()
            .unwrap();
        assert!(!cache.ordner.exists());
        assert_eq!(cache.lesen(&s), Ok(None));
    }

    #[test]
    fn alle_zaehler_kehren_zurueck_unbekannte_codes_werden_ignoriert() {
        let mut z = Zaehler::default();
        for (i, grund) in GRUND_CODES.iter().enumerate() {
            z.erhoehen(*grund, i as u64 + 1);
        }
        let mut e = CacheEintrag::aus_zaehler(vec![], &z);
        assert_eq!(e.zaehler(), z);
        e.verworfen.push(("unbekannt".into(), 500));
        assert_eq!(e.zaehler(), z);
        e.verworfen = vec![
            (GrundCode::FormatFremd.code().into(), u64::MAX),
            (GrundCode::FormatFremd.code().into(), 1),
        ];
        assert_eq!(e.zaehler().summe(), u64::MAX);
    }

    #[test]
    fn debug_enthaelt_keine_texte_und_keinen_vollstaendigen_schluessel() {
        let marke = "EINDEUTIGE_GEHEIMMARKE_712";
        let mut e = eintrag(marke);
        e.verworfen.push((marke.into(), 1));
        assert!(!format!("{e:?}").contains(marke));
        assert!(!format!("{:?}", e.korrekturen[0]).contains(marke));
        let basis = Basis::aus_text(marke);
        let mut t = teile(&basis);
        t.dokument_id = marke;
        t.fassung_id = Some(marke);
        t.anbieter_id = marke;
        t.modell = marke;
        t.ziel_art = marke;
        assert!(!format!("{t:?}").contains(marke));
        let s = CacheSchluessel::bilden(&t);
        let debug = format!("{s:?}");
        assert!(debug.contains(&s.as_str()[..8]));
        assert!(!debug.contains(s.as_str()));
        let datei = CacheDatei {
            schema_version: 1,
            schluessel: marke.into(),
            eintrag: e,
        };
        assert!(!format!("{datei:?}").contains(marke));
        assert!(!format!("{:?}", SprachCache::neu(marke)).contains(marke));
    }
}
