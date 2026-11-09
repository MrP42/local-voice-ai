//! D4: Zugriff des Abgleichs (`sync::ablage`) auf den Stand der Ablage (Spec §2, §6; ADR-0029 E4/E9).
//!
//! Alles läuft unter der Ablage-Sperre und liest den Stand frisch: Zusammenführen mit entfernten Ständen
//! (nie überschreiben, immer [`zf::zusammenfuehren_datei`] bzw. [`zf::zusammenfuehren_ordner`]), Grabsteine
//! (Original, Texte, Baum, Freigaben über den Löschauftrag im Journal), heruntergeladene Originale und
//! Textfassungen einlegen. Entfernte Stände werden vor dem Einspielen geprüft wie ein geladener Stand
//! ([`super::stand_gueltig`]); ein ungültiger Datensatz wird verworfen und nie geschrieben.

use super::{
    meta_ohne_folgen, name_lesbar, nicht_gefunden, ohne_umweg, original_name, typ_ok, Ablage,
    Eintrag, DATEIEN, TEXT,
};
use crate::ablage::zusammenfuehren::{self as zf, Datei, Ordner};
use crate::ablage::{
    datei_id_pruefen, id_gueltig, ist_umweg, neue_id, sperren, Fehler, Res, PRAEFIX_DATEI,
    PRAEFIX_FASSUNG, PRAEFIX_ORDNER, PRAEFIX_WAHL,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Ordner für Downloads des Abgleichs (Zwischendateien, nie im Baum).
pub const EINGANG: &str = "eingang";

/// Der Stand, wie ihn der Abgleich sieht (eine Momentaufnahme unter der Sperre).
#[derive(Debug, Clone, Default)]
pub struct SyncBestand {
    pub geraet: String,
    pub ordner: BTreeMap<String, Ordner>,
    pub dateien: BTreeMap<String, Datei>,
    /// Endgültig gelöschte Kennungen (und solche, die ein offener Löschauftrag gleich entfernt).
    pub begraben: BTreeSet<String>,
    /// `(dat, fas)` mit vorhandenem Original auf diesem Gerät.
    pub originale: BTreeSet<(String, String)>,
    /// `(dat, textrev)` mit vorhandener Textfassung auf diesem Gerät.
    pub texte: BTreeSet<(String, String)>,
}

/// Ein entfernter Stand zum Einspielen.
#[derive(Debug, Clone)]
pub enum Eingang {
    Ordner(Ordner),
    Datei(Datei),
    /// Grabstein (endgültig gelöscht oder auf einem anderen Gerät „nur dieses Gerät“ geworden).
    Grab(String),
}

/// Ergebnis des Einspielens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Einspielung {
    /// Kennungen, deren Stand sich hier geändert hat.
    pub geaendert: Vec<String>,
    /// Ungültige Datensätze (nicht geschrieben).
    pub verworfen: Vec<String>,
    /// Begrabene Kennungen (lokal entfernt oder schon weg).
    pub begraben: Vec<String>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn ordner_gueltig(o: &Ordner) -> bool {
    id_gueltig(PRAEFIX_ORDNER, &o.id)
        && name_lesbar(&o.name.wert)
        && o.eltern
            .wert
            .as_deref()
            .is_none_or(|p| id_gueltig(PRAEFIX_ORDNER, p) && p != o.id)
}

/// Prüft einen entfernten Datensatz `ablage_datei` wie einen geladenen Stand, dazu die Hex-Felder.
pub fn datei_gueltig(d: &Datei) -> bool {
    id_gueltig(PRAEFIX_DATEI, &d.id)
        && name_lesbar(&d.name.wert)
        && id_gueltig(PRAEFIX_ORDNER, &d.ordner.wert)
        && d.fassungen.iter().all(|f| {
            id_gueltig(PRAEFIX_FASSUNG, &f.fas)
                && typ_ok(&f.typ)
                && hex64(&f.blob)
                && hex64(&f.original_sha256)
                && f.blob == f.original_sha256
                && f.texte
                    .iter()
                    .all(|t| hex64(&t.textrev) && (t.blob.is_empty() || hex64(&t.blob)))
        })
        && d.entfernt.iter().all(|f| id_gueltig(PRAEFIX_FASSUNG, f))
        && d.wahlen.iter().all(|w| {
            id_gueltig(PRAEFIX_FASSUNG, &w.gewaehlt)
                && w.gegen.iter().all(|g| id_gueltig(PRAEFIX_FASSUNG, g))
        })
}

impl Ablage {
    fn original_rel(dat: &str, fas: &str, typ: &str) -> String {
        format!("{DATEIEN}/{dat}/{}", original_name(fas, typ))
    }

    /// Gibt es die Datei unter `rel` als echte Datei (ohne Verknüpfung auf dem Weg)?
    fn echte_datei(&self, rel: &str) -> bool {
        ohne_umweg(&self.dir, rel).is_ok()
            && meta_ohne_folgen(&self.dir.join(rel)).is_ok_and(|m| m.is_file() && !ist_umweg(&m))
    }

    /// Momentaufnahme für den Abgleich (führt ein offenes Journal vorher zu Ende).
    pub fn sync_bestand(&self) -> Res<SyncBestand> {
        let _g = sperren();
        let mut st = self.laden()?;
        let _ = self.offene_zuerst(&mut st);
        let (offen_dat, offen_ord) = st.offen_geloescht();
        let mut begraben: BTreeSet<String> = st.begraben.iter().cloned().collect();
        begraben.extend(offen_dat.iter().map(|s| s.to_string()));
        begraben.extend(offen_ord.iter().map(|s| s.to_string()));
        let mut originale = BTreeSet::new();
        let mut texte = BTreeSet::new();
        for d in st.dateien.values() {
            for f in &d.fassungen {
                if self.echte_datei(&Self::original_rel(&d.id, &f.fas, &f.typ)) {
                    originale.insert((d.id.clone(), f.fas.clone()));
                }
                for t in &f.texte {
                    if !t.blob.is_empty()
                        && self.echte_datei(&format!("{TEXT}/{}/{}.json", d.id, t.textrev))
                    {
                        texte.insert((d.id.clone(), t.textrev.clone()));
                    }
                }
            }
        }
        Ok(SyncBestand {
            geraet: st.geraet.clone(),
            ordner: st.ordner.clone(),
            dateien: st.dateien.clone(),
            begraben,
            originale,
            texte,
        })
    }

    /// Spielt entfernte Stände ein: Ordner und Dateien werden mit dem frischen lokalen Stand
    /// zusammengeführt (nie ersetzt), Grabsteine entfernen Original, Texte, Baumeintrag und Freigaben
    /// über einen Löschauftrag im Journal (Wiederanlauf wie beim endgültigen Löschen). Begrabene
    /// Kennungen werden nie wiederbelebt. Fassungen in `entfernt` verlieren hier ihr Original.
    pub fn sync_einspielen(&self, eingaenge: &[Eingang]) -> Res<Einspielung> {
        let _g = sperren();
        let mut st = self.laden()?;
        self.offene_zuerst(&mut st)?;
        let mut aus = Einspielung::default();
        let mut graeber_dat: Vec<String> = Vec::new();
        let mut graeber_ord: Vec<String> = Vec::new();
        // Originale und Texte, die nach dem Schreiben des Baums entfernt werden (Aufbewahrung woanders).
        let mut weg_dateien: Vec<String> = Vec::new();
        let mut geaendert = false;
        for e in eingaenge {
            match e {
                Eingang::Ordner(o) => {
                    if !ordner_gueltig(o) {
                        aus.verworfen.push(o.id.clone());
                        continue;
                    }
                    if st.begraben.contains(&o.id) {
                        continue;
                    }
                    let neu = match st.ordner.get(&o.id) {
                        Some(alt) => zf::zusammenfuehren_ordner(alt, o),
                        None => o.clone(),
                    };
                    if st.ordner.get(&o.id) != Some(&neu) {
                        st.ordner.insert(o.id.clone(), neu);
                        aus.geaendert.push(o.id.clone());
                        geaendert = true;
                    }
                }
                Eingang::Datei(d) => {
                    if !datei_gueltig(d) {
                        aus.verworfen.push(d.id.clone());
                        continue;
                    }
                    if st.begraben.contains(&d.id) {
                        continue;
                    }
                    let alt = st.dateien.get(&d.id).cloned();
                    let mut neu = match &alt {
                        Some(a) => zf::zusammenfuehren_datei(a, d),
                        None => zf::zusammenfuehren_datei(d, d),
                    };
                    neu.blob_refs = zf::blob_refs_aus(&neu.fassungen);
                    if let Some(a) = &alt {
                        let bleibt: BTreeSet<&str> =
                            neu.fassungen.iter().map(|f| f.fas.as_str()).collect();
                        for f in a
                            .fassungen
                            .iter()
                            .filter(|f| !bleibt.contains(f.fas.as_str()))
                        {
                            weg_dateien.push(Self::original_rel(&a.id, &f.fas, &f.typ));
                            for t in &f.texte {
                                weg_dateien.push(format!("{TEXT}/{}/{}.json", a.id, t.textrev));
                            }
                        }
                    }
                    if alt.as_ref() != Some(&neu) {
                        st.dateien.insert(d.id.clone(), neu);
                        aus.geaendert.push(d.id.clone());
                        geaendert = true;
                    }
                }
                Eingang::Grab(id) => {
                    if id_gueltig(PRAEFIX_DATEI, id) {
                        graeber_dat.push(id.clone());
                    } else if id_gueltig(PRAEFIX_ORDNER, id) {
                        graeber_ord.push(id.clone());
                    } else {
                        aus.verworfen.push(id.clone());
                    }
                }
            }
        }
        graeber_dat.sort();
        graeber_dat.dedup();
        graeber_ord.sort();
        graeber_ord.dedup();
        // Nur, was hier noch etwas hinterlässt, braucht einen Löschauftrag.
        let graeber_dat: Vec<String> = graeber_dat
            .into_iter()
            .filter(|d| {
                st.dateien.contains_key(d)
                    || self.dir.join(DATEIEN).join(d).exists()
                    || self.dir.join(TEXT).join(d).exists()
                    || !st.begraben.contains(d)
            })
            .collect();
        let graeber_ord: Vec<String> = graeber_ord
            .into_iter()
            .filter(|o| st.ordner.contains_key(o) || !st.begraben.contains(o))
            .collect();
        if !graeber_dat.is_empty() || !graeber_ord.is_empty() {
            st.journal.push(Eintrag::Loeschen {
                dat: graeber_dat.clone(),
                ordner: graeber_ord.clone(),
            });
            self.sichern(&mut st)?;
            self.loeschen_ausfuehren(&mut st, &graeber_dat, &graeber_ord)?;
            st.journal.retain(|e| {
                !matches!(e, Eintrag::Loeschen { dat: d, ordner: o } if *d == graeber_dat && *o == graeber_ord)
            });
            aus.begraben.extend(graeber_dat);
            aus.begraben.extend(graeber_ord);
            geaendert = true;
        }
        if geaendert {
            self.sichern(&mut st)?;
        }
        for rel in weg_dateien {
            if ohne_umweg(&self.dir, &rel).is_ok() {
                let _ = fs::remove_file(self.dir.join(rel));
            }
        }
        Ok(aus)
    }

    /// Entfernt liegengebliebene Downloads in `eingang/` (nie über eine Verknüpfung).
    pub fn sync_eingang_raeumen(&self) {
        if ohne_umweg(&self.dir, EINGANG).is_err() {
            return;
        }
        let Ok(r) = fs::read_dir(self.dir.join(EINGANG)) else {
            return;
        };
        for e in r.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with(".sync-")
                && fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_file() && !ist_umweg(&m))
            {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    /// Zwischenpfad für einen Download (im Ordner `eingang/`, nie im Baum).
    pub fn sync_zwischenpfad(&self) -> Res<PathBuf> {
        ohne_umweg(&self.dir, EINGANG)?;
        fs::create_dir_all(self.dir.join(EINGANG))
            .map_err(|_| Fehler::speicher("Eingangsordner nicht anlegbar."))?;
        Ok(self
            .dir
            .join(EINGANG)
            .join(format!(".sync-{}", neue_id(""))))
    }

    /// Legt ein heruntergeladenes Original (Inhalt bereits gegen die Blob-Kennung geprüft) ohne
    /// Ersetzen an seinen Platz. `Ok(false)`: Datei oder Fassung gibt es nicht mehr (Zwischendatei weg).
    pub fn original_einlegen(&self, dat: &str, fas: &str, zwischen: &Path) -> Res<bool> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let st = self.laden()?;
        let Some(f) = st
            .dateien
            .get(dat)
            .and_then(|d| d.fassungen.iter().find(|f| f.fas == fas))
        else {
            let _ = fs::remove_file(zwischen);
            return Ok(false);
        };
        let groesse = fs::metadata(zwischen).map(|m| m.len()).unwrap_or(u64::MAX);
        if groesse != f.size {
            let _ = fs::remove_file(zwischen);
            return Err(Fehler::speicher("Heruntergeladene Datei passt nicht."));
        }
        let rel = Self::original_rel(dat, fas, &f.typ);
        ohne_umweg(&self.dir, &rel)?;
        if self.dir.join(&rel).exists() {
            let _ = fs::remove_file(zwischen);
            return Ok(true);
        }
        fs::create_dir_all(self.dir.join(DATEIEN).join(dat))
            .map_err(|_| Fehler::speicher("Ordner der Datei nicht anlegbar."))?;
        ohne_umweg(&self.dir, &rel)?;
        crate::buch_dateien::verschieben_ohne_ersetzen(zwischen, &self.dir.join(&rel)).map_err(
            |_| {
                let _ = fs::remove_file(zwischen);
                Fehler::speicher("Datei nicht ablegbar.")
            },
        )?;
        crate::ablage::ordner_sync(&self.dir.join(DATEIEN).join(dat));
        Ok(true)
    }

    /// Pfad des Originals einer Fassung (zum Hochladen), wenn es hier als echte Datei liegt.
    pub fn original_quelle(&self, dat: &str, fas: &str) -> Res<Option<PathBuf>> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let st = self.laden()?;
        let f = st
            .dateien
            .get(dat)
            .and_then(|d| d.fassungen.iter().find(|f| f.fas == fas))
            .ok_or_else(nicht_gefunden)?;
        let rel = Self::original_rel(dat, fas, &f.typ);
        Ok(self.echte_datei(&rel).then(|| self.dir.join(rel)))
    }

    /// Legt eine heruntergeladene Textfassung ab, wenn sie zu ihrem Eintrag passt: SHA-256 der Bytes =
    /// Text-Blob, `textrev` und `original_sha256` im Markenformat = Eintrag bzw. Fassung (D4b).
    /// `Ok(false)`: kein passender Eintrag (mehr).
    pub fn text_einlegen(&self, dat: &str, fas: &str, textrev: &str, bytes: &[u8]) -> Res<bool> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let st = self.laden()?;
        let Some(f) = st
            .dateien
            .get(dat)
            .and_then(|d| d.fassungen.iter().find(|f| f.fas == fas))
        else {
            return Ok(false);
        };
        let Some(t) = f.texte.iter().find(|t| t.textrev == textrev) else {
            return Ok(false);
        };
        let sha = crate::ablage::extrakt::hex(&super::sha2_digest(bytes));
        #[derive(serde::Deserialize)]
        struct Kopf {
            textrev: String,
            original_sha256: String,
        }
        let kopf: Kopf = serde_json::from_slice(bytes)
            .map_err(|_| Fehler::ungueltig("Textfassung unlesbar."))?;
        if sha != t.blob || kopf.textrev != t.textrev || kopf.original_sha256 != f.original_sha256 {
            return Err(Fehler::ungueltig(
                "Textfassung passt nicht zu ihrer Fassung.",
            ));
        }
        self.text_schreiben(dat, textrev, bytes)?;
        Ok(true)
    }

    /// Pfad einer Textfassung (zum Hochladen), wenn sie hier liegt.
    pub fn text_quelle(&self, dat: &str, textrev: &str) -> Option<PathBuf> {
        let rel = format!("{TEXT}/{dat}/{textrev}.json");
        (id_gueltig(PRAEFIX_DATEI, dat) && hex64(textrev) && self.echte_datei(&rel))
            .then(|| self.dir.join(rel))
    }

    /// Konflikt auflösen: eine neue Wahl für `fas` (Spec §2.1; Lamport-Zähler aus dem Stand).
    pub fn fassung_waehlen(&self, dat: &str, fas: &str) -> Res<()> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let mut st = self.laden()?;
        st.nicht_offen_geloescht(Some(dat), None)?;
        let z = st.zaehler();
        let geraet = st.geraet.clone();
        let d = st.dateien.get_mut(dat).ok_or_else(nicht_gefunden)?;
        if !d.fassungen.iter().any(|f| f.fas == fas) {
            return Err(nicht_gefunden());
        }
        let w = zf::neue_wahl(d, &neue_id(PRAEFIX_WAHL), fas, z, &geraet);
        d.wahlen.push(w);
        d.wahlen.sort();
        self.sichern(&mut st)
    }
}
