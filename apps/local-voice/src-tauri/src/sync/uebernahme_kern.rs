//! Lokale Daten bei Anmeldung, Abmeldung und Kontowechsel (lva#400), Kernlogik ohne Tauri.
//!
//! Verbindlich: `docs/superpowers/specs/2026-11-08-lokale-daten-bei-anmeldung.md` (U1–U11).
//! Alles hier arbeitet allein über den Datenordner (`&Path`), damit es ohne App testbar ist.
//! Sperren (Seiten, Bibliothek, Sync-Lauf) hält der Aufrufer.
//!
//! Bausteine:
//! - **Herkunftsliste** (U6): Schlüssel und Hashes, die das zuletzt verbundene Konto bestätigt hatte.
//! - **Bestand** (U2): Sync-Objekte, die das Konto nicht nachweisbar schon so kennt.
//! - **Nur auf diesem Gerät** (U5): exakte Schlüssel und Einheiten, die nie hochgeladen werden.
//! - **Sicherung** (U4): Kopien, Manifest, vollständige Prüfung.
//! - **Journal** (U11) und **Umsetzung**, idempotent wiederholbar.
//! - **Wiederherstellen** (U8), das nie überschreibt.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const SICHERUNGEN: &str = "sicherungen";
pub const PRAEFIX: &str = "konto-";
pub const MANIFEST: &str = "manifest.json";
pub const INFO: &str = "info.json";
pub const EINSTELLUNGEN: &str = "einstellungen.json";
pub const HERKUNFT: &str = "sync-herkunft.json";
pub const NUR_LOKAL: &str = "sync-nur-lokal.json";
pub const JOURNAL: &str = "sync-uebergang.json";
pub const PROJEKTE: &str = "projects";
pub const BIBLIOTHEK: &str = "library";
pub const INDEX: &str = "index.json";
pub const FORTSCHRITT: &str = "fortschritt.json";
const PAPIERKORB_SEITEN: &str = "_geloescht";
const MANIFEST_VERSION: u32 = 1;

pub fn jetzt_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn gueltige_seite(id: &str) -> bool {
    id.strip_prefix("page_")
        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric()))
}

pub fn gueltiges_buch(id: &str) -> bool {
    id.strip_prefix("book_").is_some_and(|r| {
        !r.is_empty() && r.len() <= 64 && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

pub fn schluessel(sammlung: &str, id: &str) -> String {
    format!("{sammlung}/{id}")
}

fn unterordner(dir: &Path) -> Vec<String> {
    let Ok(eintraege) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut namen: Vec<String> = eintraege
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .collect();
    namen.sort();
    namen
}

/// Bücher im Datenordner (`library/book_*`).
pub fn buecher(data: &Path) -> Vec<String> {
    unterordner(&data.join(BIBLIOTHEK))
        .into_iter()
        .filter(|id| gueltiges_buch(id))
        .collect()
}

fn ordner_sync(dir: &Path) {
    // Unter Windows lässt sich ein Ordner nicht öffnen; dort genügt der fsync der Dateien.
    if let Ok(f) = fs::File::open(dir) {
        let _ = f.sync_all();
    }
}

/// Schreiben über eine Nachbardatei mit fsync und Umbenennen.
pub fn schreibe_atomar(pfad: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = pfad.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Ordner nicht anlegbar: {e}"))?;
    }
    let mut name = pfad
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    let tmp = pfad.with_file_name(name);
    let geschrieben = (|| -> Result<(), String> {
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())
    })();
    if let Err(e) = geschrieben {
        let _ = fs::remove_file(&tmp);
        return Err(format!("{} nicht schreibbar: {e}", pfad.display()));
    }
    fs::rename(&tmp, pfad).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("{} nicht ersetzbar: {e}", pfad.display())
    })?;
    if let Some(dir) = pfad.parent() {
        ordner_sync(dir);
    }
    Ok(())
}

fn lies_json<T: for<'de> Deserialize<'de>>(pfad: &Path) -> Option<T> {
    let roh = fs::read(pfad).ok()?;
    serde_json::from_slice(&roh).ok()
}

fn entferne_datei(pfad: &Path) -> Result<(), String> {
    match fs::remove_file(pfad) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{} nicht entfernbar: {e}", pfad.display())),
    }
}

// ---------------------------------------------------------------------------
// Bezüge von Fortschritten und Lesezeichen (strukturiert, kein Teilwort-Vergleich)
// ---------------------------------------------------------------------------

/// Seite oder Buch, zu dem ein Eintrag aus `fortschritt.json` gehört. Schlüssel:
/// `progress.text:<seite>:<reiter>`, `progress.audio:<buch>`, `progress.doc:<buch>:<dok>`,
/// `tts.resume.<seite>[:<reiter>]`, `tts.bookmarks.<seite>[:<reiter>]`,
/// `tts.voice.book.<buch>.<sprache>`. Andere (z. B. `tts.voice.lang.<sprache>`) gehören
/// zu keiner Einheit.
pub fn bezug(map_schluessel: &str) -> Option<&str> {
    let gueltig = |id: &str| gueltige_seite(id) || gueltiges_buch(id);
    if let Some(rest) = map_schluessel.strip_prefix("progress.") {
        let mut t = rest.split(':');
        let art = t.next()?;
        let id = t.next()?;
        return (matches!(art, "text" | "audio" | "doc") && gueltig(id)).then_some(id);
    }
    for p in ["tts.resume.", "tts.bookmarks."] {
        if let Some(rest) = map_schluessel.strip_prefix(p) {
            let id = rest.split(':').next()?;
            return gueltig(id).then_some(id);
        }
    }
    if let Some(rest) = map_schluessel.strip_prefix("tts.voice.book.") {
        let id = rest.rsplit_once('.').map(|(b, _)| b)?;
        return gueltiges_buch(id).then_some(id);
    }
    None
}

/// Schlüssel in `fortschritt.json` zu einem `progress`- oder `bookmark`-Objekt.
pub fn map_schluessel(sammlung: &str, wert: &Value) -> Option<String> {
    let key = wert.get("key").and_then(Value::as_str)?;
    match sammlung {
        "progress" => Some(format!("progress.{key}")),
        "bookmark" => Some(key.to_string()),
        _ => None,
    }
}

/// Einheit (Seite oder Buch), zu der ein Sync-Objekt gehört.
pub fn einheit_von(sammlung: &str, id: &str, wert: &Value) -> Option<String> {
    match sammlung {
        "page" => Some(id.to_string()),
        "book" => Some(id.to_string()),
        "book_file" => wert.get("book").and_then(Value::as_str).map(str::to_owned),
        "progress" | "bookmark" => {
            let k = map_schluessel(sammlung, wert)?;
            bezug(&k).map(str::to_owned)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Herkunft (U6)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Herkunft {
    pub user_id: i64,
    pub hub: String,
    pub email: String,
    /// Ledger-Schlüssel → zuletzt bestätigter Hash (nur nicht gelöschte Objekte).
    pub eintraege: BTreeMap<String, String>,
}

impl Herkunft {
    pub fn gleiches_konto(&self, user_id: i64, hub: &str) -> bool {
        self.user_id == user_id && hub_gleich(&self.hub, hub)
    }
}

pub fn hub_gleich(a: &str, b: &str) -> bool {
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

pub fn konto_kennung(user_id: i64, hub: &str) -> String {
    format!(
        "{user_id}@{}",
        hub.trim_end_matches('/').to_ascii_lowercase()
    )
}

pub fn herkunft_laden(data: &Path) -> Option<Herkunft> {
    lies_json(&data.join(HERKUNFT))
}

pub fn herkunft_speichern(data: &Path, h: &Herkunft) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(h).map_err(|e| e.to_string())?;
    schreibe_atomar(&data.join(HERKUNFT), &bytes)
}

// ---------------------------------------------------------------------------
// Nur auf diesem Gerät (U5)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct NurLokal {
    /// Exakte Sync-Schlüssel nur für Objekte ohne Einheit (`settings/app`, globale
    /// Stimmwahl `tts.voice.lang.*`); alles andere ist über seine Einheit markiert.
    #[serde(default)]
    pub schluessel: BTreeSet<String>,
    /// Seiten und Bücher, deren abhängige Objekte (auch künftige) lokal bleiben.
    #[serde(default)]
    pub einheiten: BTreeSet<String>,
}

impl NurLokal {
    pub fn ist_leer(&self) -> bool {
        self.schluessel.is_empty() && self.einheiten.is_empty()
    }

    /// Zahl für die Anzeige: Einheiten plus Einzelobjekte ohne Einheit.
    pub fn anzahl(&self) -> u32 {
        let einzeln = self
            .schluessel
            .iter()
            .filter(|k| {
                k.split_once('/')
                    .is_none_or(|(_, id)| !self.einheiten.contains(id))
            })
            .count();
        (self.einheiten.len() + einzeln) as u32
    }

    /// Bleibt ein lokales Sync-Objekt auf diesem Gerät?
    pub fn betrifft(&self, sammlung: &str, id: &str, wert: &Value) -> bool {
        if self.ist_leer() {
            return false;
        }
        self.schluessel.contains(&schluessel(sammlung, id))
            || einheit_von(sammlung, id, wert).is_some_and(|e| self.einheiten.contains(&e))
    }

    /// Ist die Einheit `u` nur lokal, direkt oder über ihr Buch (Fassungsseite → Buch)?
    pub fn einheit_gesperrt(&self, u: &str, seite_buch: &BTreeMap<String, String>) -> bool {
        self.einheiten.contains(u)
            || seite_buch
                .get(u)
                .is_some_and(|b| self.einheiten.contains(b))
    }

    /// Wie [`Self::betrifft`], dazu abhängige Objekte (Fortschritt, Lesezeichen,
    /// Buchdateien) und Fassungsseiten eines nur lokalen Buchs (Push und Pull).
    pub fn betrifft_mit_buch(
        &self,
        sammlung: &str,
        id: &str,
        wert: &Value,
        seite_buch: &BTreeMap<String, String>,
    ) -> bool {
        if self.ist_leer() {
            return false;
        }
        self.betrifft(sammlung, id, wert)
            || einheit_von(sammlung, id, wert)
                .is_some_and(|u| self.einheit_gesperrt(&u, seite_buch))
    }

    /// Ein Objekt vom Hub mit einem markierten Schlüssel wird weder angewendet noch verbucht.
    pub fn sperrt_eingang(&self, sammlung: &str, id: &str) -> bool {
        self.schluessel.contains(&schluessel(sammlung, id))
            || (matches!(sammlung, "page" | "book") && self.einheiten.contains(id))
    }

    /// Eine Einheit freigeben (Kollision: die Kontofassung soll ankommen). Abhängige
    /// Objekte sind nur über die Einheit markiert, nie über exakte Schlüssel.
    pub fn einheit_freigeben(&mut self, id: &str) {
        self.einheiten.remove(id);
        self.schluessel.remove(&schluessel("page", id));
        self.schluessel.remove(&schluessel("book", id));
    }

    pub fn vereinigen(&mut self, andere: &NurLokal) {
        self.schluessel.extend(andere.schluessel.iter().cloned());
        self.einheiten.extend(andere.einheiten.iter().cloned());
    }
}

/// Strikt lesen: fehlt die Datei, ist nichts markiert; ist sie unlesbar oder beschädigt,
/// ist das ein Fehler (sonst würden gesperrte Daten hochladbar, Codex lva#412).
fn lies_streng<T: for<'de> Deserialize<'de>>(pfad: &Path) -> Result<Option<T>, String> {
    match fs::read(pfad) {
        Ok(roh) => serde_json::from_slice(&roh)
            .map(Some)
            .map_err(|e| format!("{} ist beschädigt: {e}", pfad.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{} ist nicht lesbar: {e}", pfad.display())),
    }
}

pub fn nur_lokal_laden(data: &Path) -> Result<NurLokal, String> {
    Ok(lies_streng(&data.join(NUR_LOKAL))?.unwrap_or_default())
}

pub fn nur_lokal_speichern(data: &Path, n: &NurLokal) -> Result<(), String> {
    if n.ist_leer() {
        return entferne_datei(&data.join(NUR_LOKAL));
    }
    let bytes = serde_json::to_vec_pretty(n).map_err(|e| e.to_string())?;
    schreibe_atomar(&data.join(NUR_LOKAL), &bytes)
}

// ---------------------------------------------------------------------------
// Bestand (U2)
// ---------------------------------------------------------------------------

/// Ein lokales Sync-Objekt mit seinem aktuellen Hash (`None`: Hash nicht fertig).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Objekt {
    pub sammlung: String,
    pub id: String,
    pub hash: Option<String>,
    pub einheit: Option<String>,
    pub map_schluessel: Option<String>,
}

/// Objekte, die das Konto nicht nachweisbar mit genau diesem Inhalt kennt. Nur die
/// Herkunft desselben Kontos zählt; ein unfertiger Hash zählt immer als Bestand.
/// Bereits nur lokale Objekte zählen nicht.
pub fn bestand<'a>(
    objekte: &'a [Objekt],
    herkunft_desselben_kontos: Option<&Herkunft>,
    nur_lokal: &NurLokal,
) -> Vec<&'a Objekt> {
    objekte
        .iter()
        .filter(|o| {
            let k = schluessel(&o.sammlung, &o.id);
            if nur_lokal.schluessel.contains(&k)
                || o.einheit
                    .as_ref()
                    .is_some_and(|e| nur_lokal.einheiten.contains(e))
            {
                return false;
            }
            let bekannt = herkunft_desselben_kontos
                .and_then(|h| h.eintraege.get(&k))
                .zip(o.hash.as_ref())
                .is_some_and(|(alt, neu)| alt == neu);
            !bekannt
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Sicherung (U4)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ManifestDatei {
    pub pfad: String,
    pub groesse: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: u32,
    pub erstellt_ms: u64,
    pub dateien: Vec<ManifestDatei>,
}

/// Was in `info.json` steht und die Oberfläche zeigt.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct SicherungInfo {
    #[serde(default)]
    pub name: String,
    pub erstellt_ms: u64,
    /// `anmeldung`, `abmeldung`, `freigabe`, `kollision`, `vor_wiederherstellen`.
    pub anlass: String,
    /// E-Mail des beteiligten Kontos (falls bekannt).
    #[serde(default)]
    pub konto: Option<String>,
    pub seiten: u32,
    pub buecher: u32,
    pub dateien: u32,
    pub bytes: u64,
}

fn sicherer_rel(rel: &str) -> Result<(), String> {
    let p = Path::new(rel);
    if rel.is_empty()
        || p.is_absolute()
        || p.components().any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("Ungültiger Pfad in der Sicherung: {rel}"));
    }
    Ok(())
}

pub fn rel_text(data: &Path, pfad: &Path) -> String {
    pfad.strip_prefix(data)
        .unwrap_or(pfad)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn dateien_unter(data: &Path, dir: &Path, aus: &mut Vec<String>) -> Result<(), String> {
    let eintraege = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{} nicht lesbar: {e}", rel_text(data, dir))),
    };
    for eintrag in eintraege {
        let eintrag = eintrag.map_err(|e| e.to_string())?;
        let art = eintrag.file_type().map_err(|e| e.to_string())?;
        let pfad = eintrag.path();
        if art.is_dir() {
            dateien_unter(data, &pfad, aus)?;
        } else if art.is_file() {
            aus.push(rel_text(data, &pfad));
        }
    }
    Ok(())
}

/// Umfang (U4): Seitenordner und Index unter `projects/` (ohne Papierkorb), alle Bücher
/// vollständig, Fortschritt, Herkunft und Nur-lokal-Liste. Verknüpfungen werden nicht
/// verfolgt.
pub fn umfang(data: &Path) -> Result<Vec<String>, String> {
    let mut dateien = Vec::new();
    let projekte = data.join(PROJEKTE);
    for name in unterordner(&projekte) {
        if name == PAPIERKORB_SEITEN || name.starts_with('.') {
            continue;
        }
        dateien_unter(data, &projekte.join(name), &mut dateien)?;
    }
    if projekte.join(INDEX).is_file() {
        dateien.push(format!("{PROJEKTE}/{INDEX}"));
    }
    for buch in buecher(data) {
        dateien_unter(data, &data.join(BIBLIOTHEK).join(buch), &mut dateien)?;
    }
    if data.join(BIBLIOTHEK).join(FORTSCHRITT).is_file() {
        dateien.push(format!("{BIBLIOTHEK}/{FORTSCHRITT}"));
    }
    for name in [HERKUNFT, NUR_LOKAL] {
        if data.join(name).is_file() {
            dateien.push(name.to_string());
        }
    }
    dateien.sort();
    dateien.dedup();
    Ok(dateien)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn pruefsumme(pfad: &Path) -> Result<(u64, String), String> {
    let mut f = fs::File::open(pfad).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut puffer = vec![0u8; 64 * 1024];
    let mut n_ges = 0u64;
    loop {
        let n = f.read(&mut puffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&puffer[..n]);
        n_ges += n as u64;
    }
    Ok((n_ges, hex(&h.finalize())))
}

/// Datei kopieren (nie verknüpfen: eine spätere Änderung am Original darf die Sicherung
/// nicht ändern). Das Ziel wird nie überschrieben; die Kopie wird mit fsync gesichert.
fn sichere_datei(von: &Path, nach: &Path) -> Result<(u64, String), String> {
    if let Some(dir) = nach.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("Ordner nicht anlegbar: {e}"))?;
    }
    let mut quelle =
        fs::File::open(von).map_err(|e| format!("{} nicht lesbar: {e}", von.display()))?;
    let mut ziel = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(nach)
        .map_err(|e| format!("{} nicht anlegbar: {e}", nach.display()))?;
    let kopiert = (|| -> Result<(u64, String), String> {
        let mut h = Sha256::new();
        let mut puffer = vec![0u8; 64 * 1024];
        let mut n_ges = 0u64;
        loop {
            let n = quelle.read(&mut puffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            h.update(&puffer[..n]);
            ziel.write_all(&puffer[..n]).map_err(|e| e.to_string())?;
            n_ges += n as u64;
        }
        ziel.sync_all().map_err(|e| e.to_string())?;
        Ok((n_ges, hex(&h.finalize())))
    })();
    if kopiert.is_err() {
        drop(ziel);
        let _ = fs::remove_file(nach);
    }
    kopiert.map_err(|e| format!("{} nicht gesichert: {e}", von.display()))
}

/// Platzbedarf der Kopien: plus zehn Prozent und ein MiB.
pub fn platz_bedarf(data: &Path, dateien: &[String]) -> u64 {
    let kopie: u64 = dateien
        .iter()
        .filter_map(|rel| fs::metadata(data.join(rel)).ok())
        .map(|m| m.len())
        .sum();
    kopie + kopie / 10 + (1 << 20)
}

pub struct Auftrag<'a> {
    pub anlass: &'a str,
    pub konto: Option<&'a str>,
    /// Synchronisierte Einstellungen (Teilmenge) zum späteren Zurückspielen.
    pub einstellungen: &'a Value,
    /// Freier Platz auf dem Datenträger (`None`: unbekannt).
    pub platz_frei: Option<u64>,
}

/// Sicherung anlegen und vollständig prüfen. Scheitert etwas, bleibt kein halber Ordner
/// zurück und der Aufrufer darf nichts ändern.
pub fn sichern(data: &Path, auftrag: &Auftrag) -> Result<SicherungInfo, String> {
    let dateien = umfang(data)?;
    let noetig = platz_bedarf(data, &dateien);
    if let Some(frei) = auftrag.platz_frei {
        if frei < noetig {
            return Err(format!(
                "Zu wenig freier Speicher für die Sicherung: nötig {} MB, frei {} MB. Es wurde nichts geändert.",
                noetig.div_ceil(1 << 20),
                frei / (1 << 20)
            ));
        }
    }
    let basis = data.join(SICHERUNGEN);
    fs::create_dir_all(&basis).map_err(|e| format!("Sicherungsordner nicht anlegbar: {e}"))?;
    let zeit = jetzt_ms();
    let mut ms = zeit;
    let (dir, name) = loop {
        let name = format!("{PRAEFIX}{ms}");
        let dir = basis.join(&name);
        match fs::create_dir(&dir) {
            Ok(()) => break (dir, name),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => ms += 1,
            Err(e) => return Err(format!("Sicherungsordner nicht anlegbar: {e}")),
        }
    };
    let ergebnis = (|| -> Result<SicherungInfo, String> {
        let mut eintraege = Vec::with_capacity(dateien.len());
        for rel in &dateien {
            sicherer_rel(rel)?;
            let (groesse, sha256) = sichere_datei(&data.join(rel), &dir.join(rel))?;
            eintraege.push(ManifestDatei {
                pfad: rel.clone(),
                groesse,
                sha256,
            });
        }
        let einst = serde_json::to_vec_pretty(auftrag.einstellungen).map_err(|e| e.to_string())?;
        schreibe_atomar(&dir.join(EINSTELLUNGEN), &einst)?;
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            erstellt_ms: zeit,
            dateien: eintraege,
        };
        let info = SicherungInfo {
            name: name.clone(),
            erstellt_ms: zeit,
            anlass: auftrag.anlass.to_string(),
            konto: auftrag.konto.map(str::to_owned),
            seiten: einheiten(&manifest, PROJEKTE, gueltige_seite).len() as u32,
            buecher: einheiten(&manifest, BIBLIOTHEK, gueltiges_buch).len() as u32,
            dateien: manifest.dateien.len() as u32,
            bytes: manifest.dateien.iter().map(|d| d.groesse).sum(),
        };
        schreibe_atomar(
            &dir.join(MANIFEST),
            &serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )?;
        pruefen(&dir)?;
        // `info.json` zuletzt: nur eine geprüfte Sicherung erscheint in der Liste.
        schreibe_atomar(
            &dir.join(INFO),
            &serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?,
        )?;
        ordner_sync(&basis);
        Ok(info)
    })();
    if ergebnis.is_err() {
        let _ = fs::remove_dir_all(&dir);
    }
    ergebnis.map_err(|e| format!("Sicherung nicht erstellt: {e}"))
}

fn einheiten(manifest: &Manifest, wurzel: &str, gueltig: fn(&str) -> bool) -> BTreeSet<String> {
    manifest
        .dateien
        .iter()
        .filter_map(|d| {
            let mut teile = d.pfad.split('/');
            (teile.next() == Some(wurzel)).then(|| teile.next())?
        })
        .filter(|n| gueltig(n))
        .map(str::to_owned)
        .collect()
}

/// Jede Datei gegen das Manifest prüfen.
pub fn pruefen(dir: &Path) -> Result<Manifest, String> {
    let manifest: Manifest =
        lies_json(&dir.join(MANIFEST)).ok_or("Manifest der Sicherung fehlt oder ist unlesbar")?;
    if manifest.version != MANIFEST_VERSION {
        return Err(format!("Unbekannte Manifest-Version {}", manifest.version));
    }
    for d in &manifest.dateien {
        sicherer_rel(&d.pfad)?;
        let (groesse, sha) = pruefsumme(&dir.join(&d.pfad))
            .map_err(|_| format!("Sicherung unvollständig: {} fehlt", d.pfad))?;
        if groesse != d.groesse || sha != d.sha256 {
            return Err(format!("Sicherung beschädigt: {}", d.pfad));
        }
    }
    Ok(manifest)
}

/// Geprüfte Sicherungen (mit `info.json`), neueste zuerst.
pub fn auflisten(data: &Path) -> Vec<SicherungInfo> {
    let basis = data.join(SICHERUNGEN);
    let mut liste: Vec<SicherungInfo> = unterordner(&basis)
        .into_iter()
        .filter(|n| n.starts_with(PRAEFIX))
        .filter_map(|n| {
            let mut info: SicherungInfo = lies_json(&basis.join(&n).join(INFO))?;
            info.name = n;
            Some(info)
        })
        .collect();
    liste.sort_by_key(|s| std::cmp::Reverse(s.erstellt_ms));
    liste
}

pub fn sicherungs_ordner(data: &Path, name: &str) -> Result<PathBuf, String> {
    let gueltig = name
        .strip_prefix(PRAEFIX)
        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()));
    if !gueltig {
        return Err(format!("Ungültiger Name einer Sicherung: {name}"));
    }
    let dir = data.join(SICHERUNGEN).join(name);
    let meta =
        fs::symlink_metadata(&dir).map_err(|_| format!("Sicherung nicht gefunden: {name}"))?;
    if !meta.is_dir() {
        return Err(format!("Sicherung nicht gefunden: {name}"));
    }
    Ok(dir)
}

/// Einstellungen einer Sicherung (für das Zurückspielen auf Wunsch).
pub fn einstellungen_der_sicherung(data: &Path, name: &str) -> Result<Value, String> {
    let dir = sicherungs_ordner(data, name)?;
    lies_json(&dir.join(EINSTELLUNGEN)).ok_or_else(|| "Einstellungen der Sicherung unlesbar".into())
}

/// Eine Sicherung auf ausdrücklichen Wunsch löschen; nicht, solange ein Übergang sie braucht.
pub fn loeschen(data: &Path, name: &str) -> Result<(), String> {
    let dir = sicherungs_ordner(data, name)?;
    if journal_vorhanden(data) {
        return Err("Solange ein Übergang offen ist, lässt sich keine Sicherung löschen.".into());
    }
    fs::remove_dir_all(&dir).map_err(|e| format!("Sicherung nicht löschbar: {e}"))
}

// ---------------------------------------------------------------------------
// Seitenindex (rohes JSON, Schema gehört `commands::pages`; unbekannte Felder bleiben)
// ---------------------------------------------------------------------------

fn index_laden(pfad: &Path) -> Value {
    lies_json(pfad).unwrap_or_else(|| serde_json::json!({ "pages": [] }))
}

fn index_seiten(index: &mut Value) -> &mut Vec<Value> {
    if !index.get("pages").is_some_and(Value::is_array) {
        index["pages"] = Value::Array(Vec::new());
    }
    index["pages"].as_array_mut().expect("pages ist ein Array")
}

pub fn neue_seiten_kennung(data: &Path, index: &Value) -> String {
    let mut ms = jetzt_ms();
    loop {
        let id = format!("page_{ms}");
        let im_index = index
            .get("pages")
            .and_then(Value::as_array)
            .is_some_and(|p| p.iter().any(|e| e["id"] == id.as_str()));
        if !im_index && fs::symlink_metadata(data.join(PROJEKTE).join(&id)).is_err() {
            return id;
        }
        ms += 1;
    }
}

/// Einen Seitenordner unter neuer Kennung nach `projects/` kopieren und den Index um einen
/// Eintrag ergänzen (Vorlage `eintrag`, Titel mit `zusatz`). Gibt die neue Kennung zurück.
/// Mit `neue_id` (aus dem Journal) ist der Schritt wiederholbar.
pub fn seite_kopieren(
    data: &Path,
    quelle: &Path,
    eintrag: &Value,
    zusatz: &str,
    neue_id: Option<&str>,
) -> Result<String, String> {
    let pfad = data.join(PROJEKTE).join(INDEX);
    let mut index = index_laden(&pfad);
    let id = match neue_id {
        Some(id) if gueltige_seite(id) => id.to_string(),
        Some(id) => return Err(format!("Ungültige Seiten-Kennung: {id}")),
        None => neue_seiten_kennung(data, &index),
    };
    let ziel = data.join(PROJEKTE).join(&id);
    if !ziel.is_dir() {
        let tmp = data.join(PROJEKTE).join(format!(".kopie-{id}"));
        let _ = fs::remove_dir_all(&tmp);
        let kopiert = (|| -> Result<(), String> {
            let mut dateien = Vec::new();
            dateien_unter(quelle, quelle, &mut dateien)?;
            for rel in dateien {
                sichere_datei(&quelle.join(&rel), &tmp.join(&rel))?;
            }
            fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
            fs::rename(&tmp, &ziel).map_err(|e| e.to_string())
        })();
        if let Err(e) = kopiert {
            let _ = fs::remove_dir_all(&tmp);
            return Err(format!("Seite nicht kopiert: {e}"));
        }
    }
    let seiten = index_seiten(&mut index);
    if !seiten.iter().any(|e| e["id"] == id.as_str()) {
        let mut neu = eintrag.clone();
        let titel = neu
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Seite")
            .to_string();
        neu["id"] = Value::String(id.clone());
        neu["title"] = Value::String(format!("{titel} {zusatz}").trim().to_string());
        if let Value::Object(m) = &mut neu {
            m.remove("buch");
        }
        seiten.push(neu);
        schreibe_atomar(
            &pfad,
            &serde_json::to_vec(&index).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(id)
}

fn index_eintrag(index: &Value, id: &str) -> Option<Value> {
    index
        .get("pages")?
        .as_array()?
        .iter()
        .find(|e| e["id"] == id)
        .cloned()
}

fn index_entferne(
    data: &Path,
    seiten: &BTreeSet<String>,
    buecher: &BTreeSet<String>,
) -> Result<(), String> {
    let pfad = data.join(PROJEKTE).join(INDEX);
    if !pfad.is_file() {
        return Ok(());
    }
    let mut index = index_laden(&pfad);
    let liste = index_seiten(&mut index);
    let vorher = liste.len();
    liste.retain(|e| {
        let id = e["id"].as_str().unwrap_or_default();
        let buch = e.get("buch").and_then(Value::as_str).unwrap_or_default();
        !seiten.contains(id) && !buecher.contains(buch)
    });
    if liste.len() == vorher {
        return Ok(());
    }
    schreibe_atomar(
        &pfad,
        &serde_json::to_vec(&index).map_err(|e| e.to_string())?,
    )
}

// ---------------------------------------------------------------------------
// Journal und Umsetzung (U11)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Wahl {
    Zusammenfuehren,
    GetrenntHalten,
    Entfernen,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Entscheidung,
    /// Bei `wiederherstellen`: Markierungen gesetzt, das Ledger bereinigt (vor dem Schreiben
    /// der Dateien); sonst: Sicherung vollständig geprüft.
    Gesichert,
    Umgesetzt,
}

/// Was nach der Sicherung geschieht. Alle Felder stehen fest, bevor die Phase
/// `gesichert` geschrieben wird; die Umsetzung leitet nichts mehr neu ab.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Lose Seiten, die unter neuer Kennung lokal bleiben (alt → neu).
    #[serde(default)]
    pub kopien: BTreeMap<String, String>,
    /// Einheiten (Seiten, Bücher), die lokal entfernt werden.
    #[serde(default)]
    pub entfernen: BTreeSet<String>,
    /// Einträge aus `fortschritt.json`, die lokal entfernt werden.
    #[serde(default)]
    pub fortschritt: BTreeSet<String>,
    /// Was danach nur lokal bleibt (bei Kopien die NEUEN Kennungen).
    #[serde(default)]
    pub nur_lokal: NurLokal,
    /// Einheiten, deren Markierung entfällt, damit die Kontofassung ankommt.
    #[serde(default)]
    pub freigeben: BTreeSet<String>,
    /// Exakte Schlüssel, deren Markierung entfällt (Übernahme ins Konto).
    #[serde(default)]
    pub frei_schluessel: BTreeSet<String>,
    /// Ledger-Einträge, die beim Abschluss entfallen (Push mit Basis 0, U5).
    #[serde(default)]
    pub ledger_weg: BTreeSet<String>,
}

/// Wozu der Übergang gehört; alle laufen über dasselbe Journal (U11).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Art {
    #[default]
    Anmeldung,
    Abmeldung,
    Kollision,
    Uebernahme,
    /// Sicherung zurückspielen (U8): erst markieren, dann schreiben.
    Wiederherstellen,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Journal {
    #[serde(default)]
    pub art: Art,
    pub hub: String,
    pub user_id: i64,
    pub email: String,
    pub wahl: Option<Wahl>,
    pub phase: Phase,
    #[serde(default)]
    pub sicherung: Option<String>,
    #[serde(default)]
    pub plan: Plan,
}

/// Journal lesen; beschädigt → Fehler (Übergänge und Sync bleiben dann gesperrt).
pub fn journal_laden(data: &Path) -> Result<Option<Journal>, String> {
    lies_streng(&data.join(JOURNAL))
}

/// Ein Journal liegt vor (auch ein beschädigtes): kein Zyklus, kein neuer Übergang.
pub fn journal_vorhanden(data: &Path) -> bool {
    fs::symlink_metadata(data.join(JOURNAL)).is_ok()
}

pub fn journal_speichern(data: &Path, j: &Journal) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(j).map_err(|e| e.to_string())?;
    schreibe_atomar(&data.join(JOURNAL), &bytes)
}

pub fn journal_loeschen(data: &Path) -> Result<(), String> {
    entferne_datei(&data.join(JOURNAL))
}

// ---------------------------------------------------------------------------
// Anmeldung abschließen und verwerfen, Widerrufe (U10/U11)
// ---------------------------------------------------------------------------

/// Gehört eine wartende Anmeldung zum Journal (gleiches Konto am gleichen Hub)?
pub fn wartend_passt(j: &Journal, user_id: i64, hub: &str) -> bool {
    j.user_id == user_id && hub_gleich(&j.hub, hub)
}

/// Ist die Anmeldung des Journals vollständig gebunden: gespeichertes Konto UND Ledger
/// gehören zu ihm? Erst dann darf das Journal ohne wartende Anmeldung fallen.
pub fn anmeldung_gebunden(
    j: &Journal,
    konto: Option<(i64, &str)>,
    ledger_konto: Option<&str>,
) -> bool {
    konto.is_some_and(|(u, h)| wartend_passt(j, u, h))
        && ledger_konto == Some(konto_kennung(j.user_id, &j.hub).as_str())
}

/// Ausstehender Token-Widerruf am Hub.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Widerruf {
    pub url: String,
    pub token: String,
}

/// Widerrufsliste lesen. Fehlt sie, ist sie leer; ist sie beschädigt oder nicht lesbar,
/// ist das ein Fehler (nie als leer behandeln: sonst würden Aufträge überschrieben).
/// Liest auch das frühere Einzelformat.
pub fn widerrufe_laden(pfad: &Path) -> Result<Vec<Widerruf>, String> {
    let roh = match fs::read(pfad) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{} ist nicht lesbar: {e}", pfad.display())),
    };
    serde_json::from_slice::<Vec<Widerruf>>(&roh)
        .or_else(|_| serde_json::from_slice::<Widerruf>(&roh).map(|w| vec![w]))
        .map_err(|e| format!("{} ist beschädigt: {e}", pfad.display()))
}

/// Widerrufe dauerhaft vormerken (bestehende bleiben). `schreiben` schreibt die Datei
/// (privat, atomar). Eine beschädigte Liste bricht ab, ohne etwas zu überschreiben.
pub fn widerrufe_vormerken(
    pfad: &Path,
    neu: &[Widerruf],
    schreiben: impl FnOnce(&Path, &[u8]) -> Result<(), String>,
) -> Result<(), String> {
    let mut liste = widerrufe_laden(pfad)?;
    let vorher = liste.len();
    for w in neu {
        if !liste.contains(w) {
            liste.push(w.clone());
        }
    }
    if liste.len() == vorher && pfad.is_file() {
        return Ok(());
    }
    let roh = serde_json::to_vec(&liste).map_err(|e| e.to_string())?;
    schreiben(pfad, &roh)
}

/// Gescheiterte Anmeldung verwerfen (U10): ERST die Tokens dauerhaft sichern
/// (`vormerken`), dann wartende Anmeldung und Journal löschen. Scheitert das Vormerken,
/// bleibt beides liegen: die Tokenquelle geht nie vor der Vormerkung verloren.
pub fn anmeldung_verwerfen(
    vormerken: impl FnOnce() -> Result<(), String>,
    wartend: &Path,
    data: &Path,
) -> Result<(), String> {
    vormerken()?;
    entferne_datei(wartend)?;
    journal_loeschen(data)
}

/// Ordner einer Einheit im Datenordner (lose Seite oder Buch). Seiten im Buch-Ordner
/// gehen mit ihrem Buch.
pub fn einheit_ordner(data: &Path, einheit: &str) -> Option<PathBuf> {
    if gueltiges_buch(einheit) {
        return Some(data.join(BIBLIOTHEK).join(einheit));
    }
    if gueltige_seite(einheit) {
        return Some(data.join(PROJEKTE).join(einheit));
    }
    None
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Umsetzung {
    /// Aus `fortschritt.json` entfernte Schlüssel (für die Oberfläche: localStorage leeren).
    pub entfernte_fortschritte: Vec<String>,
    pub entfernte_einheiten: u32,
}

/// Den Plan nach geprüfter Sicherung umsetzen. Idempotent: bereits Erledigtes wird
/// übersprungen, Kopien entstehen unter der Kennung aus dem Plan. Entfernt wird nur, was
/// vollständig in der Sicherung liegt; sonst bricht der Schritt ab, ohne zu löschen.
/// Kennzeichen eines Fehlers „lokal seit der Sicherung geändert“: der Aufrufer sichert
/// neu und setzt dann um (nichts wurde entfernt).
pub const ABWEICHUNG: &str = "hat sich seit der Sicherung geändert";

pub fn ist_abweichung(fehler: &str) -> bool {
    fehler.contains(ABWEICHUNG)
}

pub fn umsetzen(data: &Path, sicherung: &str, plan: &Plan) -> Result<Umsetzung, String> {
    let dir = sicherungs_ordner(data, sicherung)?;
    let manifest = pruefen(&dir)?;
    let gesichert: BTreeMap<&str, &str> = manifest
        .dateien
        .iter()
        .map(|d| (d.pfad.as_str(), d.sha256.as_str()))
        .collect();
    let mut u = Umsetzung::default();

    // 1. Kopien (aus dem Original, sonst aus der Sicherung).
    let index_sicherung: Value = lies_json(&dir.join(PROJEKTE).join(INDEX)).unwrap_or(Value::Null);
    let index_jetzt = index_laden(&data.join(PROJEKTE).join(INDEX));
    for (alt, neu) in &plan.kopien {
        if data.join(PROJEKTE).join(neu).is_dir() {
            // Ordner da; Index-Eintrag ggf. nachtragen (seite_kopieren prüft beides).
        }
        let quelle_lokal = data.join(PROJEKTE).join(alt);
        let quelle = if quelle_lokal.is_dir() {
            quelle_lokal
        } else {
            dir.join(PROJEKTE).join(alt)
        };
        let eintrag = index_eintrag(&index_jetzt, alt)
            .or_else(|| index_eintrag(&index_sicherung, alt))
            .unwrap_or_else(|| serde_json::json!({ "id": alt, "title": "Seite" }));
        seite_kopieren(data, &quelle, &eintrag, "(nur auf diesem Gerät)", Some(neu))?;
    }

    // 1b. Fortschritte und Lesezeichen der kopierten Seiten unter der neuen Kennung
    //     (die alten gehen mit dem Original, sie liegen in der Sicherung).
    if !plan.kopien.is_empty() {
        let pfad = data.join(BIBLIOTHEK).join(FORTSCHRITT);
        if let Some(mut map) = lies_json::<BTreeMap<String, String>>(&pfad) {
            let neu: Vec<(String, String)> = map
                .iter()
                .filter_map(|(k, v)| {
                    let alt = bezug(k)?;
                    let ziel = plan.kopien.get(alt)?;
                    Some((k.replacen(alt, ziel, 1), v.clone()))
                })
                .filter(|(k, _)| !map.contains_key(k))
                .collect();
            if !neu.is_empty() {
                map.extend(neu);
                schreibe_atomar(&pfad, &serde_json::to_vec(&map).map_err(|e| e.to_string())?)?;
            }
        }
    }

    // 2. Prüfen, dass jede zu entfernende Datei unverändert in der Sicherung liegt.
    let mut faellig = Vec::new();
    for einheit in &plan.entfernen {
        let Some(ordner) = einheit_ordner(data, einheit) else {
            return Err(format!("Ungültige Einheit: {einheit}"));
        };
        if !ordner.is_dir() {
            continue;
        }
        let mut dateien = Vec::new();
        dateien_unter(data, &ordner, &mut dateien)?;
        for rel in &dateien {
            let (_, sha) = pruefsumme(&data.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
            if gesichert.get(rel.as_str()) != Some(&sha.as_str()) {
                return Err(format!(
                    "{rel} {ABWEICHUNG} oder fehlt darin. Es wurde nichts entfernt."
                ));
            }
        }
        faellig.push(ordner);
    }
    let fortschritt_pfad = data.join(BIBLIOTHEK).join(FORTSCHRITT);
    let fortschritt: Option<BTreeMap<String, String>> = lies_json(&fortschritt_pfad);
    let gesicherter_fortschritt: BTreeMap<String, String> =
        lies_json(&dir.join(BIBLIOTHEK).join(FORTSCHRITT)).unwrap_or_default();
    // Nur Werte entfernen, die so gesichert sind; geprüft, bevor irgendetwas weggeht.
    let fortschritt_weg = |map: &BTreeMap<String, String>| -> Vec<String> {
        map.keys()
            .filter(|k| {
                plan.fortschritt.contains(*k)
                    || bezug(k).is_some_and(|b| plan.entfernen.contains(b))
            })
            .cloned()
            .collect()
    };
    if let Some(map) = &fortschritt {
        for k in fortschritt_weg(map) {
            if gesicherter_fortschritt.get(&k) != map.get(&k) {
                return Err(format!(
                    "Fortschritt {k} {ABWEICHUNG}. Es wurde nichts entfernt."
                ));
            }
        }
    }

    // 3. Entfernen.
    for ordner in faellig {
        fs::remove_dir_all(&ordner)
            .map_err(|e| format!("{} nicht entfernbar: {e}", ordner.display()))?;
        u.entfernte_einheiten += 1;
    }
    let seiten: BTreeSet<String> = plan
        .entfernen
        .iter()
        .filter(|e| gueltige_seite(e))
        .cloned()
        .collect();
    let buecher: BTreeSet<String> = plan
        .entfernen
        .iter()
        .filter(|e| gueltiges_buch(e))
        .cloned()
        .collect();
    index_entferne(data, &seiten, &buecher)?;
    if let Some(mut map) = fortschritt {
        let weg = fortschritt_weg(&map);
        for k in &weg {
            map.remove(k);
        }
        if !weg.is_empty() {
            schreibe_atomar(
                &fortschritt_pfad,
                &serde_json::to_vec(&map).map_err(|e| e.to_string())?,
            )?;
        }
        u.entfernte_fortschritte = weg;
    }

    // 4. Nur-lokal-Liste: alte Kennungen freigeben, neue markieren.
    if !plan.nur_lokal.ist_leer() || !plan.freigeben.is_empty() || !plan.frei_schluessel.is_empty()
    {
        let mut n = nur_lokal_laden(data)?;
        for id in &plan.freigeben {
            n.einheit_freigeben(id);
        }
        for k in &plan.frei_schluessel {
            n.schluessel.remove(k);
        }
        n.vereinigen(&plan.nur_lokal);
        nur_lokal_speichern(data, &n)?;
    }
    Ok(u)
}

// ---------------------------------------------------------------------------
// Wiederherstellen (U8)
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Wiederhergestellt {
    /// Zurückkopierte Seiten und Bücher (fehlten lokal).
    pub zurueck: Vec<String>,
    /// Abweichende lose Seiten, als zusätzlicher Eintrag angelegt (neue Kennungen).
    pub zusaetzlich: Vec<String>,
    /// Gleich wie lokal, nichts zu tun.
    pub gleich: Vec<String>,
    /// Abweichende Bücher (oder Buchseiten): bleiben in der Sicherung.
    pub in_sicherung: Vec<String>,
    pub fortschritte_ergaenzt: u32,
    /// Ergänzte Schlüssel aus `fortschritt.json` (bleiben nur lokal, U8).
    pub ergaenzte_fortschritte: Vec<String>,
    /// Abweichende Fortschrittswerte: aktuelle bleiben, die Sicherung behält ihre.
    pub fortschritte_abweichend: u32,
}

fn einheit_gleich(data: &Path, manifest: &Manifest, wurzel: &str, einheit: &str) -> bool {
    let praefix = format!("{wurzel}/{einheit}/");
    let gesichert: BTreeMap<&str, &str> = manifest
        .dateien
        .iter()
        .filter(|d| d.pfad.starts_with(&praefix))
        .map(|d| (d.pfad.as_str(), d.sha256.as_str()))
        .collect();
    let mut lokal = Vec::new();
    if dateien_unter(data, &data.join(wurzel).join(einheit), &mut lokal).is_err() {
        return false;
    }
    lokal.len() == gesichert.len()
        && lokal.iter().all(|rel| {
            pruefsumme(&data.join(rel))
                .is_ok_and(|(_, sha)| gesichert.get(rel.as_str()) == Some(&sha.as_str()))
        })
}

/// Seiten und Bücher aus einer Sicherung zurückholen, ohne je zu überschreiben.
/// Plan einer Wiederherstellung, ohne etwas zu schreiben (U8): fehlende Einheiten kommen
/// zurück (`nur_lokal.einheiten`), abweichende lose Seiten erhalten eine neue Kennung
/// (`kopien`, neue Kennung ebenfalls nur lokal), fehlende Fortschrittsschlüssel werden
/// ergänzt (`fortschritt`). Abweichende Bücher bleiben in der Sicherung.
pub fn wiederherstellen_plan(data: &Path, name: &str) -> Result<Plan, String> {
    let dir = sicherungs_ordner(data, name)?;
    let manifest = pruefen(&dir)?;
    let mut plan = Plan::default();
    let mut probe = index_laden(&data.join(PROJEKTE).join(INDEX));
    for (wurzel, gueltig) in [
        (PROJEKTE, gueltige_seite as fn(&str) -> bool),
        (BIBLIOTHEK, gueltiges_buch as fn(&str) -> bool),
    ] {
        for einheit in einheiten(&manifest, wurzel, gueltig) {
            let ziel = data.join(wurzel).join(&einheit);
            if fs::symlink_metadata(&ziel).is_err() {
                plan.nur_lokal.einheiten.insert(einheit);
            } else if wurzel == PROJEKTE && !einheit_gleich(data, &manifest, wurzel, &einheit) {
                let neu = neue_seiten_kennung(data, &probe);
                index_seiten(&mut probe).push(serde_json::json!({ "id": neu }));
                plan.nur_lokal.einheiten.insert(neu.clone());
                plan.kopien.insert(einheit, neu);
            }
        }
    }
    let alt: BTreeMap<String, String> =
        lies_json(&dir.join(BIBLIOTHEK).join(FORTSCHRITT)).unwrap_or_default();
    let jetzt: BTreeMap<String, String> =
        lies_json(&data.join(BIBLIOTHEK).join(FORTSCHRITT)).unwrap_or_default();
    plan.fortschritt = alt
        .keys()
        .filter(|k| !jetzt.contains_key(*k))
        .cloned()
        .collect();
    Ok(plan)
}

/// Wiederherstellung nach Plan ausführen; wiederholbar. Überschreibt nie: was seit dem
/// Plan lokal entstanden ist oder nicht im Plan steht, bleibt in der Sicherung.
pub fn wiederherstellen(data: &Path, name: &str, plan: &Plan) -> Result<Wiederhergestellt, String> {
    let dir = sicherungs_ordner(data, name)?;
    let manifest = pruefen(&dir)?;
    let mut w = Wiederhergestellt::default();
    let index_sicherung: Value = lies_json(&dir.join(PROJEKTE).join(INDEX)).unwrap_or(Value::Null);
    let zusatz = format!("(aus Sicherung {name})");

    for (wurzel, gueltig) in [
        (PROJEKTE, gueltige_seite as fn(&str) -> bool),
        (BIBLIOTHEK, gueltiges_buch as fn(&str) -> bool),
    ] {
        for einheit in einheiten(&manifest, wurzel, gueltig) {
            let ziel = data.join(wurzel).join(&einheit);
            if let Some(neu) = plan.kopien.get(&einheit) {
                let eintrag = index_eintrag(&index_sicherung, &einheit)
                    .unwrap_or_else(|| serde_json::json!({ "id": einheit, "title": "Seite" }));
                seite_kopieren(
                    data,
                    &dir.join(wurzel).join(&einheit),
                    &eintrag,
                    &zusatz,
                    Some(neu),
                )?;
                w.zusaetzlich.push(neu.clone());
                continue;
            }
            let geplant = plan.nur_lokal.einheiten.contains(&einheit);
            let tmp = data
                .join(wurzel)
                .join(format!(".wiederherstellen-{einheit}"));
            if fs::symlink_metadata(&ziel).is_ok() {
                if geplant && fs::symlink_metadata(&tmp).is_err() {
                    // Schon zurückgekopiert (Wiederanlauf); Index unten nachtragen.
                } else if einheit_gleich(data, &manifest, wurzel, &einheit) {
                    w.gleich.push(einheit);
                    continue;
                } else {
                    w.in_sicherung.push(einheit);
                    continue;
                }
            } else if !geplant {
                // Erst nach dem Plan verschwunden: ohne Markierung nicht zurückschreiben.
                w.in_sicherung.push(einheit);
                continue;
            } else {
                let praefix = format!("{wurzel}/{einheit}/");
                let _ = fs::remove_dir_all(&tmp);
                let kopiert = (|| -> Result<(), String> {
                    for d in manifest
                        .dateien
                        .iter()
                        .filter(|d| d.pfad.starts_with(&praefix))
                    {
                        let rest = &d.pfad[praefix.len()..];
                        sichere_datei(&dir.join(&d.pfad), &tmp.join(rest))?;
                    }
                    fs::rename(&tmp, &ziel).map_err(|e| e.to_string())
                })();
                if let Err(e) = kopiert {
                    let _ = fs::remove_dir_all(&tmp);
                    return Err(format!("{wurzel}/{einheit} nicht wiederhergestellt: {e}"));
                }
            }
            index_nachtragen(data, &index_sicherung, wurzel, &einheit)?;
            w.zurueck.push(einheit);
        }
    }

    let alt: BTreeMap<String, String> =
        lies_json(&dir.join(BIBLIOTHEK).join(FORTSCHRITT)).unwrap_or_default();
    let pfad = data.join(BIBLIOTHEK).join(FORTSCHRITT);
    let mut jetzt: BTreeMap<String, String> = lies_json(&pfad).unwrap_or_default();
    let mut geaendert = false;
    for (k, v) in alt {
        match jetzt.get(&k) {
            None if plan.fortschritt.contains(&k) => {
                w.ergaenzte_fortschritte.push(k.clone());
                jetzt.insert(k, v);
                w.fortschritte_ergaenzt += 1;
                geaendert = true;
            }
            Some(aktuell) if *aktuell != v => w.fortschritte_abweichend += 1,
            _ => {}
        }
    }
    if geaendert {
        schreibe_atomar(
            &pfad,
            &serde_json::to_vec(&jetzt).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(w)
}

/// Index-Eintrag einer zurückgekopierten Seite bzw. die Fassungsseiten eines Buchs nachtragen.
fn index_nachtragen(
    data: &Path,
    index_sicherung: &Value,
    wurzel: &str,
    einheit: &str,
) -> Result<(), String> {
    let pfad = data.join(PROJEKTE).join(INDEX);
    let mut index = index_laden(&pfad);
    let neu: Vec<Value> = if wurzel == PROJEKTE {
        vec![index_eintrag(index_sicherung, einheit)
            .unwrap_or_else(|| serde_json::json!({ "id": einheit, "title": "Seite" }))]
    } else {
        index_sicherung
            .get("pages")
            .and_then(Value::as_array)
            .map(|p| {
                p.iter()
                    .filter(|e| e.get("buch").and_then(Value::as_str) == Some(einheit))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    let seiten = index_seiten(&mut index);
    let vorher = seiten.len();
    for e in neu {
        if !seiten.iter().any(|x| x["id"] == e["id"]) {
            seiten.push(e);
        }
    }
    if seiten.len() != vorher {
        schreibe_atomar(
            &pfad,
            &serde_json::to_vec(&index).map_err(|e| e.to_string())?,
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "uebernahme_kern_tests.rs"]
mod tests;
