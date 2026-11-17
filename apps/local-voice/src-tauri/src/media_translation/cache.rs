use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::hash;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Syntheseparameter {
    pub stimme: String,
    pub tempo_promille: u32,
    pub tonhoehe: i32,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheArt {
    Stt,
    Uebersetzung,
    Tts(Syntheseparameter),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schluessel {
    pub art: CacheArt,
    pub spur: String,
    pub modellrevision: String,
    pub quell_sprache: String,
    pub ziel_sprache: String,
    pub kontext: String,
    pub prompt_version: String,
    pub pipeline_version: String,
    /// Medienbereich oder Satzinhalt; niemals nur die Modellkonfiguration cachen.
    pub eingabe: String,
}

impl Schluessel {
    pub fn kanonisch(&self) -> String {
        let mut teile = vec![
            "lva620-cache-v1".to_owned(),
            self.spur.clone(),
            self.modellrevision.clone(),
            self.quell_sprache.clone(),
            self.ziel_sprache.clone(),
            self.kontext.clone(),
            self.prompt_version.clone(),
            self.pipeline_version.clone(),
            self.eingabe.clone(),
        ];
        match &self.art {
            CacheArt::Stt => teile.push("stt".to_owned()),
            CacheArt::Uebersetzung => teile.push("uebersetzung".to_owned()),
            CacheArt::Tts(parameter) => {
                teile.extend([
                    "tts".to_owned(),
                    parameter.stimme.clone(),
                    parameter.tempo_promille.to_string(),
                    parameter.tonhoehe.to_string(),
                    parameter.seed.to_string(),
                ]);
            }
        }
        // Längenpräfixe verhindern Mehrdeutigkeit durch Trennzeichen im Inhalt.
        teile
            .iter()
            .map(|teil| format!("{}:{}", teil.len(), teil))
            .collect()
    }

    pub fn dateiname(&self) -> String {
        format!("{:016x}.entry", hash(self.kanonisch().as_bytes()))
    }
}

/// Eine Ablage gehört ausschließlich einem Cache; Umbenennen veröffentlicht atomar.
pub trait Ablage {
    fn namen(&self) -> io::Result<Vec<String>>;
    fn lesen(&self, name: &str) -> io::Result<Vec<u8>>;
    fn schreiben(&mut self, name: &str, daten: &[u8]) -> io::Result<()>;
    fn umbenennen(&mut self, von: &str, nach: &str) -> io::Result<()>;
    fn entfernen(&mut self, name: &str) -> io::Result<()>;
}

#[derive(Default)]
pub struct SpeicherAblage {
    dateien: BTreeMap<String, Vec<u8>>,
}

impl Ablage for SpeicherAblage {
    fn namen(&self) -> io::Result<Vec<String>> {
        Ok(self.dateien.keys().cloned().collect())
    }

    fn lesen(&self, name: &str) -> io::Result<Vec<u8>> {
        self.dateien
            .get(name)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn schreiben(&mut self, name: &str, daten: &[u8]) -> io::Result<()> {
        self.dateien.insert(name.to_owned(), daten.to_vec());
        Ok(())
    }

    fn umbenennen(&mut self, von: &str, nach: &str) -> io::Result<()> {
        let daten = self
            .dateien
            .remove(von)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        self.dateien.insert(nach.to_owned(), daten);
        Ok(())
    }

    fn entfernen(&mut self, name: &str) -> io::Result<()> {
        self.dateien.remove(name);
        Ok(())
    }
}

pub struct DateiAblage {
    wurzel: PathBuf,
}

impl DateiAblage {
    pub fn neu(wurzel: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&wurzel)?;
        Ok(Self { wurzel })
    }

    fn pfad(&self, name: &str) -> io::Result<PathBuf> {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
            || name == "."
            || name == ".."
        {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        Ok(self.wurzel.join(name))
    }
}

impl Ablage for DateiAblage {
    fn namen(&self) -> io::Result<Vec<String>> {
        let mut namen = Vec::new();
        for eintrag in fs::read_dir(&self.wurzel)? {
            let eintrag = eintrag?;
            if eintrag.file_type()?.is_file() {
                if let Some(name) = eintrag.file_name().to_str() {
                    namen.push(name.to_owned());
                }
            }
        }
        namen.sort();
        Ok(namen)
    }

    fn lesen(&self, name: &str) -> io::Result<Vec<u8>> {
        fs::read(self.pfad(name)?)
    }

    fn schreiben(&mut self, name: &str, daten: &[u8]) -> io::Result<()> {
        let mut datei = fs::File::create(self.pfad(name)?)?;
        datei.write_all(daten)?;
        datei.sync_all()
    }

    fn umbenennen(&mut self, von: &str, nach: &str) -> io::Result<()> {
        fs::rename(self.pfad(von)?, self.pfad(nach)?)
    }

    fn entfernen(&mut self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.pfad(name)?) {
            Err(fehler) if fehler.kind() == io::ErrorKind::NotFound => Ok(()),
            ergebnis => ergebnis,
        }
    }
}

pub struct Cache<A: Ablage> {
    ablage: A,
    limit_bytes: u64,
}

impl<A: Ablage> Cache<A> {
    pub fn neu(ablage: A, limit_bytes: u64) -> Self {
        Self {
            ablage,
            limit_bytes,
        }
    }

    pub fn in_ablage(self) -> A {
        self.ablage
    }

    /// Tempdateien und unvollständige Einträge werden vor jeder Veröffentlichung entfernt.
    pub fn wiederaufnehmen(&mut self) -> io::Result<()> {
        self.bereinigen(self.limit_bytes).map(|_| ())
    }

    pub fn holen(&mut self, schluessel: &Schluessel) -> io::Result<Option<Vec<u8>>> {
        let name = schluessel.dateiname();
        let daten = match self.ablage.lesen(&name) {
            Ok(daten) => daten,
            Err(fehler) if fehler.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(fehler) => return Err(fehler),
        };
        match entpacken(&daten) {
            Some((_, key, inhalt)) if key == schluessel.kanonisch() => Ok(Some(inhalt)),
            Some(_) => Ok(None), // Ein Hashkonflikt darf keinen fremden Inhalt liefern.
            None => {
                self.ablage.entfernen(&name)?;
                Ok(None)
            }
        }
    }

    /// Gleiche Uhrwerte werden logisch fortgezählt; Zugriffe ändern das Alter nicht.
    pub fn ablegen(
        &mut self,
        schluessel: &Schluessel,
        inhalt: &[u8],
        jetzt_ms: u64,
    ) -> io::Result<bool> {
        let letztes_alter = self.bereinigen(self.limit_bytes)?;
        let alter = match letztes_alter {
            Some(alter) => jetzt_ms.max(
                alter
                    .checked_add(1)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?,
            ),
            None => jetzt_ms,
        };
        let daten = verpacken(&schluessel.kanonisch(), inhalt, alter);
        if daten.len() as u64 > self.limit_bytes {
            return Ok(false);
        }
        let name = schluessel.dateiname();
        let temporaer = format!("{}.tmp", name);
        self.ablage.schreiben(&temporaer, &daten)?;
        self.ablage.umbenennen(&temporaer, &name)?;
        self.bereinigen(self.limit_bytes)?;
        Ok(true)
    }

    fn bereinigen(&mut self, limit_bytes: u64) -> io::Result<Option<u64>> {
        let mut eintraege = Vec::new();
        let mut gesamt = 0_u64;
        let mut letztes_alter: Option<u64> = None;
        for name in self.ablage.namen()? {
            if name.ends_with(".tmp") {
                self.ablage.entfernen(&name)?;
            } else if name.ends_with(".entry") {
                let daten = self.ablage.lesen(&name)?;
                if let Some((alter, key, _)) = entpacken(&daten) {
                    if name != format!("{:016x}.entry", hash(key.as_bytes())) {
                        self.ablage.entfernen(&name)?;
                        continue;
                    }
                    let groesse = daten.len() as u64;
                    letztes_alter = Some(letztes_alter.map_or(alter, |wert| wert.max(alter)));
                    gesamt = gesamt.saturating_add(groesse);
                    eintraege.push((alter, name, groesse));
                } else {
                    self.ablage.entfernen(&name)?;
                }
            }
        }
        eintraege.sort();
        for (_, name, groesse) in eintraege {
            if gesamt <= limit_bytes {
                break;
            }
            self.ablage.entfernen(&name)?;
            gesamt = gesamt.saturating_sub(groesse);
        }
        Ok(letztes_alter)
    }
}

fn verpacken(key: &str, inhalt: &[u8], jetzt_ms: u64) -> Vec<u8> {
    let mut daten = b"LVA62001".to_vec();
    daten.extend(jetzt_ms.to_le_bytes());
    daten.extend((key.len() as u64).to_le_bytes());
    daten.extend((inhalt.len() as u64).to_le_bytes());
    daten.extend(key.as_bytes());
    daten.extend(inhalt);
    let pruefsumme = hash(&daten);
    daten.extend(pruefsumme.to_le_bytes());
    daten
}

fn nehmen<'a>(daten: &mut &'a [u8], anzahl: usize) -> Option<&'a [u8]> {
    if anzahl > daten.len() {
        return None;
    }
    let (teil, rest) = daten.split_at(anzahl);
    *daten = rest;
    Some(teil)
}

fn zahl(daten: &mut &[u8]) -> Option<u64> {
    Some(u64::from_le_bytes(nehmen(daten, 8)?.try_into().ok()?))
}

fn entpacken(daten: &[u8]) -> Option<(u64, String, Vec<u8>)> {
    let mut rest = daten;
    if nehmen(&mut rest, 8)? != b"LVA62001" {
        return None;
    }
    let alter = zahl(&mut rest)?;
    let key_laenge = usize::try_from(zahl(&mut rest)?).ok()?;
    let inhalt_laenge = usize::try_from(zahl(&mut rest)?).ok()?;
    let key = std::str::from_utf8(nehmen(&mut rest, key_laenge)?).ok()?;
    let inhalt = nehmen(&mut rest, inhalt_laenge)?;
    let pruefsumme = zahl(&mut rest)?;
    if !rest.is_empty() || hash(daten.get(..daten.len().checked_sub(8)?)?) != pruefsumme {
        return None;
    }
    Some((alter, key.to_owned(), inhalt.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schluessel() -> Schluessel {
        Schluessel {
            art: CacheArt::Tts(Syntheseparameter {
                stimme: "lokal".into(),
                tempo_promille: 1000,
                tonhoehe: 0,
                seed: 1,
            }),
            spur: "spur".into(),
            modellrevision: "rev1".into(),
            quell_sprache: "de".into(),
            ziel_sprache: "en".into(),
            kontext: "kontext".into(),
            prompt_version: "p1".into(),
            pipeline_version: "v1".into(),
            eingabe: "satz".into(),
        }
    }

    #[test]
    fn jedes_feld_invalidiert() {
        let basis = schluessel();
        let mut varianten = Vec::new();
        for feld in 0..8 {
            let mut key = basis.clone();
            match feld {
                0 => key.spur.push('x'),
                1 => key.modellrevision.push('x'),
                2 => key.quell_sprache.push('x'),
                3 => key.ziel_sprache.push('x'),
                4 => key.kontext.push('x'),
                5 => key.prompt_version.push('x'),
                6 => key.pipeline_version.push('x'),
                _ => key.eingabe.push('x'),
            }
            varianten.push(key);
        }
        for feld in 0..4 {
            let mut key = basis.clone();
            if let CacheArt::Tts(parameter) = &mut key.art {
                match feld {
                    0 => parameter.stimme.push('x'),
                    1 => parameter.tempo_promille += 1,
                    2 => parameter.tonhoehe += 1,
                    _ => parameter.seed += 1,
                }
            }
            varianten.push(key);
        }
        for art in [CacheArt::Stt, CacheArt::Uebersetzung] {
            let mut key = basis.clone();
            key.art = art;
            varianten.push(key);
        }
        let mut cache = Cache::neu(SpeicherAblage::default(), 100_000);
        assert!(cache.ablegen(&basis, b"inhalt", 1).expect("ablegen"));
        assert_eq!(basis.kanonisch(), basis.clone().kanonisch());
        for key in varianten {
            // Gegenprobe: Ein ausgelassenes Feld ergibt denselben Schlüssel und einen Treffer.
            assert_ne!(basis.kanonisch(), key.kanonisch());
            assert_ne!(basis.dateiname(), key.dateiname());
            assert_eq!(cache.holen(&key).expect("holen"), None);
        }
    }

    #[test]
    fn wiederanlauf_und_pruefsumme() {
        let key = schluessel();
        let mut ablage = SpeicherAblage::default();
        ablage.schreiben("rest.tmp", b"halb").expect("temp");
        ablage
            .schreiben(&key.dateiname(), b"LVA62001")
            .expect("halb");
        let mut cache = Cache::neu(ablage, 10_000);
        cache.wiederaufnehmen().expect("wiederaufnehmen");
        // Gegenprobe: Ohne Wiederaufnahme bleiben temporäre und halbe Einträge liegen.
        assert!(cache.ablage.namen().expect("namen").is_empty());
        cache.ablegen(&key, b"inhalt", 1).expect("ablegen");
        let mut daten = cache.ablage.lesen(&key.dateiname()).expect("lesen");
        daten[32] ^= 1;
        cache
            .ablage
            .schreiben(&key.dateiname(), &daten)
            .expect("defekt");
        // Gegenprobe: Ohne Prüfsumme würde ein beschädigter Eintrag als Treffer gelten.
        assert_eq!(cache.holen(&key).expect("holen"), None);
        cache.ablegen(&key, b"neu", 2).expect("neu");
        assert_eq!(cache.holen(&key).expect("holen"), Some(b"neu".to_vec()));
    }

    #[test]
    fn limit_verdraengt_aeltesten_auch_nach_neustart() {
        let a = schluessel();
        let mut b = a.clone();
        b.eingabe = "sat2".into();
        let groesse = verpacken(&a.kanonisch(), b"a", 1).len() as u64;
        let mut cache = Cache::neu(SpeicherAblage::default(), groesse * 2);
        cache.ablegen(&a, b"a", 1).expect("a");
        cache.ablegen(&b, b"b", 2).expect("b");
        let mut cache = Cache::neu(cache.in_ablage(), groesse);
        cache.wiederaufnehmen().expect("neustart");
        // Gegenprobe: Ohne gespeichertes Alter könnte der neuere Eintrag verdrängt werden.
        assert_eq!(cache.holen(&a).expect("a"), None);
        assert_eq!(cache.holen(&b).expect("b"), Some(b"b".to_vec()));
        assert!(!cache
            .ablegen(&a, &vec![0; groesse as usize], 3)
            .expect("zu gross"));
    }

    #[test]
    fn gleiche_uhrwerte_bleiben_fifo() {
        let mut keys: Vec<_> = (0..3)
            .map(|nummer| {
                let mut key = schluessel();
                key.eingabe = nummer.to_string();
                key
            })
            .collect();
        // Die neuere Datei würde bei lexikalischem Gleichstand zuerst verdrängt.
        keys.sort_by_key(Schluessel::dateiname);
        keys.reverse();
        let groesse = verpacken(&keys[0].kanonisch(), b"a", 0).len() as u64;
        let mut cache = Cache::neu(SpeicherAblage::default(), groesse * 2);
        for key in &keys {
            cache.ablegen(key, b"a", 0).expect("ablegen");
        }
        // Gegenprobe: Nur Uhrzeit plus Dateiname würde hier den neuesten Eintrag entfernen.
        assert_eq!(cache.holen(&keys[0]).expect("alt"), None);
        assert_eq!(cache.holen(&keys[1]).expect("mittel"), Some(b"a".to_vec()));
        assert_eq!(cache.holen(&keys[2]).expect("neu"), Some(b"a".to_vec()));
    }

    #[test]
    fn veroeffentlichung_erst_nach_umbenennen() {
        struct Abbruch(SpeicherAblage);
        impl Ablage for Abbruch {
            fn namen(&self) -> io::Result<Vec<String>> {
                self.0.namen()
            }
            fn lesen(&self, name: &str) -> io::Result<Vec<u8>> {
                self.0.lesen(name)
            }
            fn schreiben(&mut self, name: &str, daten: &[u8]) -> io::Result<()> {
                self.0.schreiben(name, daten)
            }
            fn umbenennen(&mut self, _von: &str, _nach: &str) -> io::Result<()> {
                Err(io::Error::from(io::ErrorKind::Interrupted))
            }
            fn entfernen(&mut self, name: &str) -> io::Result<()> {
                self.0.entfernen(name)
            }
        }
        let key = schluessel();
        let mut cache = Cache::neu(Abbruch(SpeicherAblage::default()), 10_000);
        assert!(cache.ablegen(&key, b"neu", 1).is_err());
        // Gegenprobe: Direktes Schreiben unter dem Zielnamen macht den Eintrag schon sichtbar.
        assert_eq!(cache.holen(&key).expect("holen"), None);
        cache.wiederaufnehmen().expect("bereinigen");
        assert!(cache.ablage.namen().expect("namen").is_empty());
    }
}
