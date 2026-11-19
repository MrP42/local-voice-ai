use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::{
    abgelaufen, ergebnis_text, konflikt, pruefe_aenderungen, Aufbereitung, SpeicherFehler,
};

pub const SCHEMA_VERSION: u32 = 1;

static SCHREIBSPERRE: Mutex<()> = Mutex::new(());

#[derive(Serialize, Deserialize)]
struct Datei {
    schema_version: u32,
    basis_text: String,
    aufbereitung: Aufbereitung,
}

impl std::fmt::Debug for Datei {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Datei")
            .field("schema_version", &self.schema_version)
            .field("basis_text_laenge", &self.basis_text.len())
            .field("aufbereitung", &self.aufbereitung)
            .finish()
    }
}

/// `ordner` ist der vertrauenswürdige Dokumentordner, keine Benutzereingabe.
pub struct AufbereitungsSpeicher {
    ordner: PathBuf,
}

impl AufbereitungsSpeicher {
    pub fn neu(ordner: impl AsRef<Path>) -> Self {
        Self {
            ordner: ordner.as_ref().join("aufbereitung"),
        }
    }

    pub fn original_einfrieren(&self, text: &str) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        fs::create_dir_all(&self.ordner).map_err(|_| SpeicherFehler::Schreiben)?;
        // Die exklusive Sperre verhindert zwei gleichzeitige Erstschreibvorgänge.
        let sperre = self.ordner.join("original.lock");
        let datei = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sperre)
            .map_err(|_| SpeicherFehler::Schreiben)?;
        let ergebnis = self.original_schreiben(text);
        drop(datei);
        let entfernt = fs::remove_file(sperre).map_err(|_| SpeicherFehler::Schreiben);
        ergebnis.and(entfernt)
    }

    fn original_schreiben(&self, text: &str) -> Result<(), SpeicherFehler> {
        let pfad = self.ordner.join("original.txt");
        match fs::read(&pfad) {
            Ok(bytes) if bytes == text.as_bytes() => Ok(()),
            Ok(_) => Err(SpeicherFehler::OriginalFest),
            Err(fehler) if fehler.kind() == std::io::ErrorKind::NotFound => {
                crate::commands::pages::write_atomic(&pfad, text.as_bytes())
                    .map_err(|_| SpeicherFehler::Schreiben)
            }
            Err(_) => Err(SpeicherFehler::Lesen),
        }
    }

    pub fn speichern(&self, aufbereitung: &Aufbereitung) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        self.pfad(&aufbereitung.id)?;
        // Ein vorhandener Snapshot erlaubt auch das Speichern veralteter Entwürfe.
        let basis_text = if self
            .pfad(&aufbereitung.id)?
            .try_exists()
            .map_err(|_| SpeicherFehler::Lesen)?
        {
            let vorhanden = self.datei_laden(&aufbereitung.id)?;
            vorhanden.basis_text
        } else if let Some(id) = &aufbereitung.vorgaenger_aktiv {
            let vorgaenger = self.datei_laden(id)?;
            if vorgaenger.aufbereitung.dokument != aufbereitung.dokument {
                return Err(SpeicherFehler::Basis);
            }
            ergebnis_text(&vorgaenger.basis_text, &vorgaenger.aufbereitung.aenderungen)
                .map_err(|_| SpeicherFehler::Aenderungen)?
        } else {
            fs::read_to_string(self.ordner.join("original.txt"))
                .map_err(|_| SpeicherFehler::Lesen)?
        };
        self.speichern_intern(aufbereitung, &basis_text)
    }

    /// Übergabe einer externen Seitenversion; deren CAS-Prüfung bleibt beim Aufrufer.
    pub fn speichern_mit_basis(
        &self,
        aufbereitung: &Aufbereitung,
        basis_text: &str,
    ) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        self.speichern_intern(aufbereitung, basis_text)
    }

    // Der Aufrufer hält die prozessweite Schreibsperre.
    fn speichern_intern(
        &self,
        aufbereitung: &Aufbereitung,
        basis_text: &str,
    ) -> Result<(), SpeicherFehler> {
        let pfad = self.pfad(&aufbereitung.id)?;
        self.identitaet_pruefen(aufbereitung, basis_text)?;
        self.metadaten_pruefen(aufbereitung)?;
        if konflikt(&aufbereitung.basis, basis_text) {
            return Err(SpeicherFehler::Basis);
        }
        pruefe_aenderungen(basis_text, &aufbereitung.aenderungen)
            .map_err(|_| SpeicherFehler::Aenderungen)?;
        // Das Original muss vor jeder ersten Aufbereitung eingefroren sein.
        fs::read(self.ordner.join("original.txt")).map_err(|_| SpeicherFehler::Lesen)?;
        let datei = Datei {
            schema_version: SCHEMA_VERSION,
            basis_text: basis_text.to_owned(),
            aufbereitung: aufbereitung.clone(),
        };
        let bytes = serde_json::to_vec(&datei).map_err(|_| SpeicherFehler::Schreiben)?;
        crate::commands::pages::write_atomic(&pfad, &bytes).map_err(|_| SpeicherFehler::Schreiben)
    }

    fn identitaet_pruefen(
        &self,
        aufbereitung: &Aufbereitung,
        basis_text: &str,
    ) -> Result<(), SpeicherFehler> {
        if !self
            .pfad(&aufbereitung.id)?
            .try_exists()
            .map_err(|_| SpeicherFehler::Lesen)?
        {
            return Ok(());
        }
        let vorhanden = self.datei_laden(&aufbereitung.id)?;
        let mut erlaubt = vorhanden.aufbereitung;
        erlaubt.status = aufbereitung.status;
        erlaubt.fortschritt = aufbereitung.fortschritt.clone();
        erlaubt.geaendert = aufbereitung.geaendert;
        for (alt, neu) in erlaubt
            .aenderungen
            .iter_mut()
            .zip(&aufbereitung.aenderungen)
        {
            alt.status = neu.status;
        }
        if erlaubt != *aufbereitung || vorhanden.basis_text != basis_text {
            return Err(SpeicherFehler::Basis);
        }
        Ok(())
    }

    pub fn laden(&self, id: &str) -> Result<Aufbereitung, SpeicherFehler> {
        Ok(self.datei_laden(id)?.aufbereitung)
    }

    fn datei_laden(&self, id: &str) -> Result<Datei, SpeicherFehler> {
        let pfad = self.pfad(id)?;
        let bytes = fs::read(pfad).map_err(|_| SpeicherFehler::Lesen)?;
        let wert: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| SpeicherFehler::Lesen)?;
        if wert.get("schema_version").and_then(|v| v.as_u64()) != Some(u64::from(SCHEMA_VERSION)) {
            return Err(SpeicherFehler::Schema);
        }
        let datei: Datei = serde_json::from_value(wert).map_err(|_| SpeicherFehler::Schema)?;
        if datei.aufbereitung.id != id {
            return Err(SpeicherFehler::Schema);
        }
        self.metadaten_pruefen(&datei.aufbereitung)?;
        if konflikt(&datei.aufbereitung.basis, &datei.basis_text) {
            return Err(SpeicherFehler::Basis);
        }
        pruefe_aenderungen(&datei.basis_text, &datei.aufbereitung.aenderungen)
            .map_err(|_| SpeicherFehler::Aenderungen)?;
        Ok(datei)
    }

    fn metadaten_pruefen(&self, aufbereitung: &Aufbereitung) -> Result<(), SpeicherFehler> {
        if let Some(id) = &aufbereitung.vorgaenger_aktiv {
            self.pfad(id)?;
            if id == &aufbereitung.id {
                return Err(SpeicherFehler::Schema);
            }
        }
        if aufbereitung.fortschritt.abschnitte_fertig > aufbereitung.fortschritt.abschnitte_gesamt
            || aufbereitung.geaendert < aufbereitung.erstellt
        {
            return Err(SpeicherFehler::Schema);
        }
        let mut ids = HashSet::new();
        if aufbereitung.aenderungen.iter().any(|a| !ids.insert(&a.id)) {
            return Err(SpeicherFehler::Schema);
        }
        Ok(())
    }

    pub fn liste(&self) -> Result<Vec<Aufbereitung>, SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        let eintraege = match fs::read_dir(&self.ordner) {
            Ok(eintraege) => eintraege,
            Err(fehler) if fehler.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(_) => return Err(SpeicherFehler::Lesen),
        };
        let mut ids = Vec::new();
        for eintrag in eintraege {
            let eintrag = eintrag.map_err(|_| SpeicherFehler::Lesen)?;
            let name = eintrag.file_name();
            let name = name.to_str().ok_or(SpeicherFehler::Schema)?;
            if matches!(name, "original.txt" | "original.lock") {
                continue;
            }
            // Temporärdateien des atomaren Schreibers gehören nicht zur Liste.
            if let Some(id) = name.strip_suffix(".json") {
                self.pfad(id)?;
                ids.push(id.to_owned());
            }
        }
        ids.sort();
        ids.iter().map(|id| self.laden(id)).collect()
    }

    pub fn loeschen(&self, id: &str) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        self.loeschen_intern(id)
    }

    // Der Aufrufer hält die prozessweite Schreibsperre.
    fn loeschen_intern(&self, id: &str) -> Result<(), SpeicherFehler> {
        let pfad = self.pfad(id)?;
        match fs::remove_file(pfad) {
            Ok(()) => Ok(()),
            Err(fehler) if fehler.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SpeicherFehler::Schreiben),
        }
    }

    pub fn alles_loeschen(&self) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        match fs::remove_dir_all(&self.ordner) {
            Ok(()) => Ok(()),
            Err(fehler) if fehler.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SpeicherFehler::Schreiben),
        }
    }

    pub fn aufraeumen(&self, jetzt_ms: u64) -> Result<(), SpeicherFehler> {
        self.aufraeumen_mit(self.liste()?, jetzt_ms)
    }

    fn aufraeumen_mit(
        &self,
        kandidaten: Vec<Aufbereitung>,
        jetzt_ms: u64,
    ) -> Result<(), SpeicherFehler> {
        let _sperre = SCHREIBSPERRE
            .lock()
            .unwrap_or_else(|fehler| fehler.into_inner());
        for kandidat in kandidaten {
            if !self
                .pfad(&kandidat.id)?
                .try_exists()
                .map_err(|_| SpeicherFehler::Lesen)?
            {
                continue;
            }
            // Erst der aktuelle Stand unter der Sperre entscheidet über das Löschen.
            let aufbereitung = self.laden(&kandidat.id)?;
            if abgelaufen(&aufbereitung, jetzt_ms) {
                self.loeschen_intern(&aufbereitung.id)?;
            }
        }
        Ok(())
    }

    fn pfad(&self, id: &str) -> Result<PathBuf, SpeicherFehler> {
        if id.len() != 26
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
        {
            return Err(SpeicherFehler::Id);
        }
        Ok(self.ordner.join(format!("{id}.json")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aufbereitung::ergebnis::tests::{aenderung, fassung};
    use crate::aufbereitung::{Fortschritt, Status, AUFBEWAHRUNG_MS};

    fn umgebung() -> (tempfile::TempDir, AufbereitungsSpeicher) {
        let temp = tempfile::tempdir().unwrap();
        let speicher = AufbereitungsSpeicher::neu(temp.path());
        speicher.original_einfrieren("ab").unwrap();
        (temp, speicher)
    }

    fn json_schreiben(speicher: &AufbereitungsSpeicher, wert: &serde_json::Value) {
        fs::write(
            speicher.pfad("00000000000000000000000001").unwrap(),
            serde_json::to_vec(wert).unwrap(),
        )
        .unwrap();
    }

    fn json_lesen(speicher: &AufbereitungsSpeicher) -> serde_json::Value {
        serde_json::from_slice(
            &fs::read(speicher.pfad("00000000000000000000000001").unwrap()).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn original_einmal_gleich_ok_anders_fest() {
        let (_temp, speicher) = umgebung();
        assert_eq!(speicher.original_einfrieren("ab"), Ok(()));
        assert_eq!(
            speicher.original_einfrieren("anders"),
            Err(SpeicherFehler::OriginalFest)
        );
        assert_eq!(
            fs::read_to_string(speicher.ordner.join("original.txt")).unwrap(),
            "ab"
        );
        assert!(!speicher.ordner.join("original.lock").exists());
    }

    #[test]
    fn abbruch_fortsetzen_roundtrip() {
        let (_temp, speicher) = umgebung();
        let mut a = fassung("ab");
        a.status = Status::Abgebrochen;
        a.fortschritt = Fortschritt {
            abschnitte_fertig: 2,
            abschnitte_gesamt: 5,
        };
        speicher.speichern(&a).unwrap();
        let mut geladen = speicher.laden(&a.id).unwrap();
        assert_eq!(geladen, a);
        let fortsetzen_ab = geladen.fortschritt.abschnitte_fertig;
        assert_eq!(fortsetzen_ab, 2);
        geladen.status = Status::Laeuft;
        geladen.fortschritt.abschnitte_fertig = fortsetzen_ab + 1;
        speicher.speichern(&geladen).unwrap();
        assert_eq!(
            speicher.laden(&a.id).unwrap().fortschritt.abschnitte_fertig,
            3
        );
        geladen.fortschritt.abschnitte_fertig = 6;
        assert_eq!(speicher.speichern(&geladen), Err(SpeicherFehler::Schema));
        assert_eq!(
            speicher.laden(&a.id).unwrap().fortschritt.abschnitte_fertig,
            3
        );
    }

    #[test]
    fn alte_neue_und_fremde_schemata_abgelehnt() {
        let (_temp, speicher) = umgebung();
        let a = fassung("ab");
        speicher.speichern(&a).unwrap();
        let mut wert = json_lesen(&speicher);
        for version in [0, 2] {
            wert["schema_version"] = version.into();
            json_schreiben(&speicher, &wert);
            assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Schema));
        }
        json_schreiben(&speicher, &serde_json::json!({"fremd": "GEHEIMTEXT"}));
        assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Schema));
    }

    #[test]
    fn unbekannte_felder_toleriert() {
        let (_temp, speicher) = umgebung();
        let a = fassung("ab");
        speicher.speichern(&a).unwrap();
        let mut wert = json_lesen(&speicher);
        wert["zukunft"] = true.into();
        wert["aufbereitung"]["zukunft"] = true.into();
        json_schreiben(&speicher, &wert);
        assert_eq!(speicher.laden(&a.id).unwrap(), a);
    }

    #[test]
    fn kaputtes_und_leeres_json_ohne_inhalt_im_fehler() {
        let (_temp, speicher) = umgebung();
        let a = fassung("ab");
        for bytes in [b"GEHEIMTEXT{".as_slice(), b"".as_slice()] {
            fs::write(speicher.pfad(&a.id).unwrap(), bytes).unwrap();
            let fehler = speicher.laden(&a.id).unwrap_err();
            assert_eq!(fehler, SpeicherFehler::Lesen);
            assert!(!fehler.to_string().contains("GEHEIMTEXT"));
        }
    }

    #[test]
    fn pfade_werden_vor_jedem_zugriff_geprueft() {
        let (temp, speicher) = umgebung();
        let aussen = temp.path().join("GEHEIMTEXT.json");
        fs::write(&aussen, "unveraendert").unwrap();
        for id in [
            "..",
            "../GEHEIMTEXT",
            "/GEHEIMTEXT",
            "000000000000000000000000001",
            "0000000000000000000000000a",
            "000000000000000000000000/1",
        ] {
            assert_eq!(speicher.laden(id), Err(SpeicherFehler::Id));
            assert_eq!(speicher.loeschen(id), Err(SpeicherFehler::Id));
            let mut a = fassung("ab");
            a.id = id.into();
            assert_eq!(speicher.speichern(&a), Err(SpeicherFehler::Id));
        }
        assert_eq!(fs::read_to_string(aussen).unwrap(), "unveraendert");
    }

    #[test]
    fn laden_prueft_bereiche_alt_gruppen_und_marken() {
        let (_temp, speicher) = umgebung();
        let mut a = fassung("ab");
        a.aenderungen.push(aenderung(0, "a", "A"));
        speicher.speichern(&a).unwrap();
        let original = json_lesen(&speicher);
        for feld in ["bereich", "alt"] {
            let mut wert = original.clone();
            wert["aufbereitung"]["aenderungen"][0][feld] = if feld == "bereich" {
                serde_json::json!([0, 99])
            } else {
                serde_json::json!("GEHEIMTEXT")
            };
            json_schreiben(&speicher, &wert);
            assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Aenderungen));
        }
        let mut gruppiert = vec![aenderung(0, "a", "A"), aenderung(1, "b", "B")];
        for aenderung in &mut gruppiert {
            aenderung.gruppe = Some("gemeinsam".into());
        }
        gruppiert[1].status = super::super::AenderungStatus::Abgelehnt;
        for (basis, aenderungen) in [
            ("ab", vec![aenderung(0, "ab", ""), aenderung(1, "b", "")]),
            ("[Tags]", vec![aenderung(0, "[Tags]", "")]),
            ("ab", gruppiert),
        ] {
            let mut wert = original.clone();
            wert["basis_text"] = basis.into();
            wert["aufbereitung"]["basis"] =
                serde_json::to_value(super::super::Basis::aus_text(basis)).unwrap();
            wert["aufbereitung"]["aenderungen"] = serde_json::to_value(aenderungen).unwrap();
            json_schreiben(&speicher, &wert);
            assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Aenderungen));
        }
    }

    #[test]
    fn basis_snapshot_und_datei_id_werden_geprueft() {
        let (_temp, speicher) = umgebung();
        let a = fassung("ab");
        speicher.speichern(&a).unwrap();
        let original = json_lesen(&speicher);
        let mut wert = original.clone();
        wert["basis_text"] = "ba".into();
        json_schreiben(&speicher, &wert);
        assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Basis));
        wert = original;
        wert["aufbereitung"]["id"] = "00000000000000000000000002".into();
        json_schreiben(&speicher, &wert);
        assert_eq!(speicher.laden(&a.id), Err(SpeicherFehler::Schema));
    }

    #[test]
    fn kette_und_externe_basis_bleiben_selbststaendig_lesbar() {
        let (_temp, speicher) = umgebung();
        let mut erste = fassung("ab");
        erste.aenderungen.push(aenderung(0, "a", "A"));
        speicher.speichern(&erste).unwrap();
        let mut zweite = fassung("Ab");
        zweite.id = "00000000000000000000000002".into();
        zweite.vorgaenger_aktiv = Some(erste.id.clone());
        speicher.speichern(&zweite).unwrap();
        speicher.loeschen(&erste.id).unwrap();
        assert_eq!(speicher.laden(&zweite.id).unwrap(), zweite);
        zweite.status = Status::Veraltet;
        speicher.speichern(&zweite).unwrap();
        let mut externe = fassung("Seitenversion");
        externe.id = "00000000000000000000000003".into();
        assert_eq!(speicher.speichern(&externe), Err(SpeicherFehler::Basis));
        speicher
            .speichern_mit_basis(&externe, "Seitenversion")
            .unwrap();
        assert_eq!(speicher.laden(&externe.id).unwrap(), externe);
    }

    #[test]
    fn vorhandene_identitaet_bleibt_auf_beiden_speicherwegen_fest() {
        let (_temp, speicher) = umgebung();
        let mut a = fassung("ab");
        a.aenderungen.push(aenderung(0, "a", "A"));
        speicher.speichern(&a).unwrap();
        let vorher = fs::read(speicher.pfad(&a.id).unwrap()).unwrap();
        let mutationen: [fn(&mut Aufbereitung); 10] = [
            |a| a.dokument.art = super::super::DokumentArt::Buch,
            |a| a.dokument.id = "anders".into(),
            |a| a.dokument.fassung_id = Some("anders".into()),
            |a| a.vorgaenger_aktiv = Some("00000000000000000000000002".into()),
            |a| a.schritte.push(super::super::Schritt::Sprache),
            |a| {
                a.modell = Some(super::super::ModellArt {
                    funktion: super::super::Schritt::Sprache,
                    ziel_art: super::super::ZielArt::Lokal,
                });
            },
            |a| a.erstellt = 99,
            |a| a.aenderungen[0].neu = "anders".into(),
            |a| a.aenderungen[0].grund_code = "anders".into(),
            |a| a.aenderungen.clear(),
        ];
        for mutation in mutationen {
            let mut andere = a.clone();
            mutation(&mut andere);
            assert_eq!(speicher.speichern(&andere), Err(SpeicherFehler::Basis));
            assert_eq!(
                speicher.speichern_mit_basis(&andere, "ab"),
                Err(SpeicherFehler::Basis)
            );
            assert_eq!(fs::read(speicher.pfad(&a.id).unwrap()).unwrap(), vorher);
        }
        let mut andere = a.clone();
        andere.basis = super::super::Basis::aus_text("ba");
        andere.aenderungen.clear();
        assert_eq!(
            speicher.speichern_mit_basis(&andere, "ba"),
            Err(SpeicherFehler::Basis)
        );
        assert_eq!(speicher.speichern(&andere), Err(SpeicherFehler::Basis));
        assert_eq!(fs::read(speicher.pfad(&a.id).unwrap()).unwrap(), vorher);

        // Nur die ausdrücklich veränderlichen Felder dürfen aktualisiert werden.
        a.status = Status::Verworfen;
        a.aenderungen[0].status = super::super::AenderungStatus::Abgelehnt;
        a.fortschritt.abschnitte_gesamt = 1;
        a.geaendert = 200;
        speicher.speichern_mit_basis(&a, "ab").unwrap();
        assert_eq!(speicher.laden(&a.id).unwrap(), a);
        a.status = Status::Veraltet;
        a.geaendert = 300;
        speicher.speichern(&a).unwrap();
        assert_eq!(speicher.laden(&a.id).unwrap(), a);
    }

    #[test]
    fn aufraeumen_prueft_veraltete_kandidaten_erneut() {
        let (_temp, speicher) = umgebung();
        let jetzt = 100 + AUFBEWAHRUNG_MS + 24 * 60 * 60 * 1000;
        let mut a = fassung("ab");
        a.status = Status::Veraltet;
        speicher.speichern(&a).unwrap();
        let kandidaten = speicher.liste().unwrap();
        assert!(abgelaufen(&kandidaten[0], jetzt));
        a.geaendert = jetzt;
        speicher.speichern(&a).unwrap();
        speicher.aufraeumen_mit(kandidaten, jetzt).unwrap();
        assert_eq!(speicher.laden(&a.id).unwrap(), a);
        // Ohne Aktualisierung wird dieselbe Fassung nach weiteren 30 Tagen gelöscht.
        speicher.aufraeumen(jetzt + AUFBEWAHRUNG_MS).unwrap();
        assert!(speicher.liste().unwrap().is_empty());
    }

    #[test]
    fn debug_maskiert_dokumentinhalte() {
        let mut a = fassung("GEHEIMTEXT");
        a.aenderungen.push(aenderung(0, "GEHEIMTEXT", "GEHEIMTEXT"));
        assert!(!format!("{:?}", a.aenderungen[0]).contains("GEHEIMTEXT"));
        assert!(!format!("{a:?}").contains("GEHEIMTEXT"));
        let datei = Datei {
            schema_version: SCHEMA_VERSION,
            basis_text: "GEHEIMTEXT".into(),
            aufbereitung: a,
        };
        assert!(!format!("{datei:?}").contains("GEHEIMTEXT"));
    }

    #[test]
    fn aufraeumen_und_dokument_loeschen() {
        let (temp, speicher) = umgebung();
        let jetzt = 100 + AUFBEWAHRUNG_MS + 24 * 60 * 60 * 1000;
        let status = [
            Status::Verworfen,
            Status::Veraltet,
            Status::Aktiv,
            Status::Entwurf,
        ];
        for (index, status) in status.into_iter().enumerate() {
            let mut a = fassung("ab");
            a.id = format!("{:026}", index + 1);
            a.status = status;
            speicher.speichern(&a).unwrap();
        }
        speicher
            .aufraeumen(100 + AUFBEWAHRUNG_MS - 24 * 60 * 60 * 1000)
            .unwrap();
        assert_eq!(speicher.liste().unwrap().len(), 4);
        speicher.aufraeumen(jetzt).unwrap();
        let liste = speicher.liste().unwrap();
        assert_eq!(liste.len(), 2);
        assert_eq!(liste[0].status, Status::Aktiv);
        speicher.loeschen(&liste[0].id).unwrap();
        speicher.loeschen(&liste[0].id).unwrap();
        assert_eq!(speicher.liste().unwrap().len(), 1);
        speicher.alles_loeschen().unwrap();
        assert!(!temp.path().join("aufbereitung").exists());
        assert!(speicher.liste().unwrap().is_empty());
        speicher.alles_loeschen().unwrap();
    }

    #[test]
    fn fehlermeldungen_enthalten_keinen_text() {
        let (_temp, speicher) = umgebung();
        let fehler = speicher.original_einfrieren("GEHEIMTEXT").unwrap_err();
        assert!(!format!("{fehler} {fehler:?}").contains("GEHEIMTEXT"));
        let a = fassung("GEHEIMTEXT");
        assert_eq!(speicher.speichern(&a), Err(SpeicherFehler::Basis));
        for fehler in [
            SpeicherFehler::Id,
            SpeicherFehler::OriginalFest,
            SpeicherFehler::Lesen,
            SpeicherFehler::Schreiben,
            SpeicherFehler::Schema,
            SpeicherFehler::Basis,
            SpeicherFehler::Aenderungen,
        ] {
            assert!(!format!("{fehler} {fehler:?}").contains("GEHEIMTEXT"));
        }
        for fehler in [
            super::super::ErgebnisFehler::Status,
            super::super::ErgebnisFehler::Bereich,
            super::super::ErgebnisFehler::Ueberlappung,
            super::super::ErgebnisFehler::Geschuetzt,
            super::super::ErgebnisFehler::Alt,
            super::super::ErgebnisFehler::Gruppe,
            super::super::ErgebnisFehler::Konflikt,
        ] {
            assert!(!format!("{fehler} {fehler:?}").contains("GEHEIMTEXT"));
        }
    }

    #[test]
    fn schreibfehler_lassen_keine_halbe_datei_zurueck() {
        let (_temp, speicher) = umgebung();
        let a = fassung("ab");
        speicher.speichern(&a).unwrap();
        let vorher = fs::read(speicher.pfad(&a.id).unwrap()).unwrap();
        let mut kaputt = a.clone();
        kaputt.aenderungen.push(aenderung(0, "GEHEIMTEXT", ""));
        assert_eq!(speicher.speichern(&kaputt), Err(SpeicherFehler::Basis));
        assert_eq!(fs::read(speicher.pfad(&a.id).unwrap()).unwrap(), vorher);
        // Ein fehlender Elternordner ist auf allen Plattformen deterministisch.
        let fehlend = speicher.ordner.join("fehlend");
        let ziel = fehlend.join("fassung.json");
        assert!(crate::commands::pages::write_atomic(&ziel, b"GEHEIMTEXT").is_err());
        assert!(!ziel.exists());
        assert!(!fehlend.exists());
        let zweites_ziel = speicher.pfad("00000000000000000000000002").unwrap();
        fs::create_dir(&zweites_ziel).unwrap();
        let mut zweite = a;
        zweite.id = "00000000000000000000000002".into();
        assert!(speicher.speichern_mit_basis(&zweite, "ab").is_err());
        assert!(zweites_ziel.is_dir());
        let namen: Vec<_> = fs::read_dir(&speicher.ordner)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(namen.len(), 3);
        assert!(namen
            .iter()
            .all(|name| name == "original.txt" || name.ends_with(".json")));
    }
}
