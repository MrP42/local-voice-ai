//! D4: Zugriff des Abgleichs (`sync::ablage`) auf den Stand der Ablage (Spec §2, §6; ADR-0029 E4/E9).
//!
//! Alles läuft unter der Ablage-Sperre und liest den Stand frisch: Zusammenführen mit entfernten Ständen
//! (nie überschreiben, immer [`zf::zusammenfuehren_datei`] bzw. [`zf::zusammenfuehren_ordner`]), Grabsteine
//! (Original, Texte, Baum, Freigaben über den Löschauftrag im Journal), heruntergeladene Originale und
//! Textfassungen einlegen. Entfernte Stände werden vor dem Einspielen geprüft wie ein geladener Stand
//! ([`super::stand_gueltig`]); ein ungültiger Datensatz wird verworfen und nie geschrieben.

use super::{
    meta_ohne_folgen, name_lesbar, nicht_gefunden, ohne_umweg, original_name, typ_ok, Ablage,
    Eintrag, SyncBlob, DATEIEN, STANDARD_ORDNER, TEXT,
};
use crate::ablage::freigabe;
use crate::ablage::zusammenfuehren::{self as zf, Datei, Ordner, Register};
use crate::ablage::{
    datei_id_pruefen, id_gueltig, ist_umweg, neue_id, sperren, Fehler, Res, PRAEFIX_DATEI,
    PRAEFIX_FASSUNG, PRAEFIX_ORDNER, PRAEFIX_WAHL,
};
use crate::sync::ablage::Teilnahme;
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
    /// Festgehaltene Kennungen („nur dieses Gerät“, nie wieder senden, nie begraben).
    pub lokal: BTreeSet<String>,
    /// Befreite Kennungen (synchron trotz festgehaltenem Ordner).
    pub befreit: BTreeSet<String>,
    /// Von diesem Gerät hochgeladene Blobs (Wartung „Blobs abgleichen“).
    pub sync_blobs: BTreeMap<String, SyncBlob>,
    /// Alte Kennungen nach dem Rückweg lokal → sync (alt → neu): am Hub noch lebende werden begraben.
    pub weiterleitung: BTreeMap<String, String>,
}

impl SyncBestand {
    /// Teilnahme je Kennung ([`crate::sync::ablage::teilnahmen`]).
    pub fn teilnahmen(&self) -> BTreeMap<String, Teilnahme> {
        crate::sync::ablage::teilnahmen(&self.ordner, &self.dateien, &self.lokal, &self.befreit)
    }
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
    /// Begrabene Kennungen (lokal in den Papierkorb gelegt oder schon weg).
    pub begraben: Vec<String>,
    /// Grabsteine, die hier nicht gelten (festgehalten, „nur dieses Gerät“ oder unklar).
    pub gehalten: Vec<String>,
}

fn hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn ordner_gueltig(o: &Ordner) -> bool {
    id_gueltig(PRAEFIX_ORDNER, &o.id)
        && zf::ordner_zaehler_ok(o)
        && name_lesbar(&o.name.wert)
        && o.eltern
            .wert
            .as_deref()
            .is_none_or(|p| id_gueltig(PRAEFIX_ORDNER, p) && p != o.id)
}

/// Prüft einen entfernten Datensatz `ablage_datei` wie einen geladenen Stand, dazu die Hex-Felder und
/// die Zählerobergrenze `ZAEHLER_MAX`.
pub fn datei_gueltig(d: &Datei) -> bool {
    id_gueltig(PRAEFIX_DATEI, &d.id)
        && zf::datei_zaehler_ok(d)
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
            lokal: st.lokal_menge(),
            befreit: st.befreit_menge(),
            sync_blobs: st.sync_blobs.clone(),
            weiterleitung: st.weiterleitung.clone(),
        })
    }

    /// Spielt entfernte Stände ein: Ordner und Dateien werden mit dem frischen lokalen Stand
    /// zusammengeführt (nie ersetzt), Grabsteine entfernen Original, Texte, Baumeintrag und Freigaben
    /// über einen Löschauftrag im Journal (Wiederanlauf wie beim endgültigen Löschen). Begrabene
    /// Kennungen werden nie wiederbelebt. Fassungen in `entfernt` verlieren hier ihr Original.
    pub fn sync_einspielen(&self, eingaenge: &[Eingang]) -> Res<Einspielung> {
        let _g = sperren();
        let mut st = self.laden()?;
        // Entfernte Änderungen halten nichts fest (nur Handlungen auf diesem Gerät tun das).
        st.vorher = None;
        self.offene_zuerst(&mut st)?;
        st.vorher = None;
        // Teilnahme vor dem Einspielen: ein Grabstein gilt nur für synchrone Einträge.
        let teilnahme_vorher = st.teilnahmen();
        let lokal = st.lokal_menge();
        // Cloud-Ausnahmen verfallen bei jedem Klassenwechsel, auch wenn er von einem anderen Gerät kommt.
        let klassen_vorher = self.klassenstand(&st);
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
                    // Stand zu einer alten Kennung (Rückweg lokal → sync): auf die neue abbilden und
                    // zusammenführen, nie verwerfen (ohne lebendes Ziel bleibt er draußen).
                    let umgeleitet;
                    let o = match st.weiterleitung.get(&o.id).map(|_| st.aufgeloest(&o.id)) {
                        Some(ziel) if st.ordner.contains_key(&ziel) => {
                            let mut x = o.clone();
                            x.id = ziel;
                            umgeleitet = x;
                            &umgeleitet
                        }
                        Some(_) => continue,
                        None => o,
                    };
                    if st.begraben.contains(&o.id) {
                        continue;
                    }
                    let mut neu = match st.ordner.get(&o.id) {
                        Some(alt) => zf::zusammenfuehren_ordner(alt, o),
                        None => o.clone(),
                    };
                    // Eltern mit alter Kennung (Rückweg lokal → sync): der neuen folgen.
                    if let Some(n) = neu.eltern.wert.as_ref().map(|p| st.aufgeloest(p)) {
                        if neu.eltern.wert.as_deref() != Some(n.as_str()) {
                            neu.eltern = Register::neu(Some(n), st.zaehler()?, &st.geraet);
                        }
                    }
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
                    let umgeleitet;
                    let d = match st.weiterleitung.get(&d.id).map(|_| st.aufgeloest(&d.id)) {
                        Some(ziel) if st.dateien.contains_key(&ziel) => {
                            let mut x = d.clone();
                            x.id = ziel;
                            umgeleitet = x;
                            &umgeleitet
                        }
                        Some(_) => continue,
                        None => d,
                    };
                    if st.begraben.contains(&d.id) {
                        continue;
                    }
                    let alt = st.dateien.get(&d.id).cloned();
                    let mut neu = match &alt {
                        Some(a) => zf::zusammenfuehren_datei(a, d),
                        None => zf::zusammenfuehren_datei(d, d),
                    };
                    neu.blob_refs = zf::blob_refs_aus(&neu.fassungen);
                    let ziel = st.aufgeloest(&neu.ordner.wert);
                    if ziel != neu.ordner.wert {
                        neu.ordner = Register::neu(ziel, st.zaehler()?, &st.geraet);
                    }
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
                    if st.weiterleitung.contains_key(id) {
                        continue; // alte Kennung nach dem Rückweg: der eigene Grabstein
                    }
                    let gilt = !lokal.contains(id)
                        && teilnahme_vorher
                            .get(id)
                            .is_none_or(|t| *t == Teilnahme::Sync);
                    if !gilt {
                        aus.gehalten.push(id.clone());
                    } else if id_gueltig(PRAEFIX_DATEI, id) {
                        graeber_dat.push(id.clone());
                    } else if id_gueltig(PRAEFIX_ORDNER, id) {
                        graeber_ord.push(id.clone());
                    } else {
                        aus.verworfen.push(id.clone());
                    }
                }
            }
        }
        // Später eingetroffene Einträge unter einem festgehaltenen Ordner: befreit (bleiben synchron).
        if !lokal.is_empty() && !aus.geaendert.is_empty() {
            let eltern = st.eltern();
            for id in &aus.geaendert {
                if lokal.contains(id) || st.befreit.contains(id) {
                    continue;
                }
                let start = match (st.dateien.get(id), st.ordner.get(id)) {
                    (Some(d), _) => Some(d.ordner.wert.clone()),
                    (None, Some(o)) => o.eltern.wert.clone(),
                    _ => None,
                };
                let unter_lokal = start.is_some_and(|s| {
                    zf::strenge_kette(&st.ordner, &eltern, &s)
                        .iter()
                        .any(|g| lokal.contains(g))
                });
                if unter_lokal {
                    st.befreit.push(id.clone());
                    geaendert = true;
                }
            }
            st.befreit.sort();
            st.befreit.dedup();
        }
        // Vor jedem Speichern des neuen Stands: ändert sich die wirksame Klasse eines Ordners mit Ausnahme,
        // wird sie entzogen. Schlägt das fehl, wird nichts gespeichert; der Abgleich liefert den Stand
        // beim nächsten Lauf erneut.
        self.ausnahmen_nach_klassenwechsel(&st, &klassen_vorher)?;
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
        // Ein Ordnergrab, unter dem hier noch lebende, nicht begrabene Einträge liegen (z. B. eine Datei,
        // die ein Gerät vor dem Umschalten eines anderen hineinlegte), wird nicht angewendet: der Ordner
        // bleibt als Behälter, seine Einträge bleiben synchron und werden nie mitgelöscht.
        let eltern = st.eltern();
        let bleibt_unter = |o: &String| {
            st.dateien.values().any(|d| {
                !graeber_dat.contains(&d.id) && zf::kette(&eltern, &d.ordner.wert).contains(o)
            }) || st.ordner.values().any(|k| {
                k.id != *o && !graeber_ord.contains(&k.id) && zf::kette(&eltern, &k.id).contains(o)
            })
        };
        let (gehalten_ord, graeber_ord): (Vec<String>, Vec<String>) =
            graeber_ord.iter().cloned().partition(|o| bleibt_unter(o));
        aus.gehalten.extend(gehalten_ord);
        if !graeber_dat.is_empty() || !graeber_ord.is_empty() {
            // Inhalte gehen für die Papierkorbfrist beiseite (`aufheben`), nicht sofort weg.
            st.journal.push(Eintrag::Loeschen {
                dat: graeber_dat.clone(),
                ordner: graeber_ord.clone(),
                aufheben: true,
            });
            self.sichern(&mut st)?;
            self.loeschen_ausfuehren(&mut st, &graeber_dat, &graeber_ord, true)?;
            st.journal.retain(|e| {
                !matches!(e, Eintrag::Loeschen { dat: d, ordner: o, .. } if *d == graeber_dat && *o == graeber_ord)
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

    /// Frische Prüfung unter der Sperre (unmittelbar vor Upload oder Push): welche der Kennungen
    /// nehmen noch am Abgleich teil (nicht begraben, nicht festgehalten, Teilnahme `Sync`)?
    pub fn sync_noch_synchron(&self, ids: &[String]) -> Res<BTreeSet<String>> {
        let _g = sperren();
        let st = self.laden()?;
        let t = st.teilnahmen();
        let (offen_dat, offen_ord) = st.offen_geloescht();
        Ok(ids
            .iter()
            .filter(|id| {
                t.get(*id) == Some(&Teilnahme::Sync)
                    && !st.begraben.contains(*id)
                    && !offen_dat.contains(id.as_str())
                    && !offen_ord.contains(id.as_str())
            })
            .cloned()
            .collect())
    }

    /// Vermerkt einen von diesem Gerät hochgeladenen Blob (dauerhaft, Wartung „Blobs abgleichen“).
    pub fn sync_blob_vermerken(&self, blob: &str, konto: &str) -> Res<()> {
        let _g = sperren();
        let mut st = self.laden()?;
        if st.sync_blobs.get(blob).is_some_and(|b| b.konto == konto) {
            return Ok(());
        }
        st.sync_blobs.insert(
            blob.to_string(),
            SyncBlob {
                ms: crate::ablage::jetzt_ms(),
                konto: konto.to_string(),
            },
        );
        self.sichern(&mut st)
    }

    /// Vergisst Blobs (am Hub gelöscht oder schon weg).
    pub fn sync_blobs_vergessen(&self, blobs: &[String]) -> Res<()> {
        if blobs.is_empty() {
            return Ok(());
        }
        let _g = sperren();
        let mut st = self.laden()?;
        let vorher = st.sync_blobs.len();
        st.sync_blobs.retain(|b, _| !blobs.contains(b));
        if st.sync_blobs.len() == vorher {
            return Ok(());
        }
        self.sichern(&mut st)
    }

    /// D4: Zieht den unveränderten Standardordner „Allgemein“ auf die kontoweit gleiche Kennung `ziel`
    /// um (ein atomarer Schreibvorgang: Ordner, Dateien und Unterordner umgehängt), damit zwei Geräte
    /// nach dem ersten Abgleich ein einziges „Allgemein“ haben. Eine Kennung, die der Hub schon kennt
    /// (`am_hub`), bleibt. Liefert, ob umgezogen wurde.
    pub fn standard_umziehen(&self, ziel: &str, am_hub: &dyn Fn(&str) -> bool) -> Res<bool> {
        if !id_gueltig(PRAEFIX_ORDNER, ziel) {
            return Err(Fehler::ungueltig("Kennung des Ordners ungültig."));
        }
        let _g = sperren();
        let mut st = self.laden()?;
        if st.begraben.iter().any(|b| b == ziel) || !st.journal.is_empty() {
            return Ok(false);
        }
        let freigaben = match freigabe::laden(&self.dir) {
            freigabe::Lage::Gueltig(f) => Some(f),
            _ => None,
        };
        let geraet = st.geraet.clone();
        let alt = st
            .ordner
            .values()
            .find(|o| {
                o.id != ziel
                    && o.name.wert == STANDARD_ORDNER
                    && o.name.geraet == geraet
                    && o.eltern.wert.is_none()
                    && o.datenklasse.wert == "erbt"
                    && !o.nur_geraet.wert
                    && !o.zielordner_ki.wert
                    && o.papierkorb.wert.is_none()
                    && [
                        o.eltern.zaehler,
                        o.datenklasse.zaehler,
                        o.nur_geraet.zaehler,
                        o.zielordner_ki.zaehler,
                        o.papierkorb.zaehler,
                    ]
                    .iter()
                    .all(|z| *z == o.name.zaehler)
                    && !am_hub(&o.id)
                    && !st.lokal.contains(&o.id)
                    && freigaben
                        .as_ref()
                        .is_some_and(|f| !f.ordner.contains_key(&o.id))
            })
            .map(|o| o.id.clone());
        let Some(alt) = alt else {
            return Ok(false);
        };
        let z = st.zaehler()?;
        if !st.ordner.contains_key(ziel) {
            let mut o = st.ordner[&alt].clone();
            o.id = ziel.to_string();
            st.ordner.insert(ziel.to_string(), o);
        }
        for d in st.dateien.values_mut() {
            if d.ordner.wert == alt {
                d.ordner = Register::neu(ziel.to_string(), z, &geraet);
            }
        }
        for o in st.ordner.values_mut() {
            if o.eltern.wert.as_deref() == Some(alt.as_str()) {
                o.eltern = Register::neu(Some(ziel.to_string()), z, &geraet);
            }
        }
        st.ordner.remove(&alt);
        self.sichern(&mut st)?;
        Ok(true)
    }

    /// Konflikt auflösen: eine neue Wahl für `fas` (Spec §2.1; Lamport-Zähler aus dem Stand).
    pub fn fassung_waehlen(&self, dat: &str, fas: &str) -> Res<()> {
        datei_id_pruefen(dat)?;
        let _g = sperren();
        let mut st = self.laden()?;
        st.nicht_offen_geloescht(Some(dat), None)?;
        let z = st.zaehler()?;
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
