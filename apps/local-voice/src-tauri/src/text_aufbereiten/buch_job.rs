//! Persistenter Buchjob. Der Dokumenttext wird niemals ersetzt oder protokolliert.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use crate::aufbereitung::{Aenderung, Basis};
use crate::llm_versand::ZielArt;
use super::buch::kapitel;
use super::sprache::{cache::SprachCache, job::{sprache_job, Abbruch, JobEingabe},
    modell::Sprachversand, politik::Wahl, vertrag::{abschnitte_bilden, Grenzen, Textstand}};

const GRENZEN: Grenzen = Grenzen { max_zeichen: 12_000, max_zeilen: 120 };
const MAX_BYTES: usize = 8 * 1024 * 1024;
static JOBS: LazyLock<Mutex<HashMap<String, Arc<Lauf>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Serialize, Deserialize, specta::Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuchJobPhase { Laeuft, Pausiert, Gestoppt, Fertig, Fehler }

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct BuchJobStatus {
    pub id: String,
    pub phase: BuchJobPhase,
    pub kapitel_fertig: usize,
    pub kapitel_gesamt: usize,
    pub prozent: f64,
    pub rest_sekunden: Option<u64>,
    pub aenderungen: Vec<Aenderung>,
    pub fehler: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
pub struct BuchUmfang {
    pub zeichen: usize,
    pub kapitel: usize,
    pub anfragen: usize,
    pub ziel: String,
    pub cloud: bool,
    pub ki_explizit_noetig: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Stand {
    version: u32,
    dokument_id: String,
    text: String,
    ziel: String,
    status: BuchJobStatus,
}

struct Lauf {
    stand: Mutex<Stand>,
    // 0 = weiter, 1 = nach Anfrage pausieren, 2 = nach Anfrage stoppen.
    steuerung: AtomicU8,
}

type Meldung<'a> = dyn Fn(&BuchJobStatus, Option<(usize, Vec<Aenderung>)>) + Send + Sync + 'a;

fn sperre<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn ziel(w: &Wahl) -> String {
    // Keine Zugangsdaten im Schluessel; Provideradresse, Modell und Zielart binden die Wahl.
    Basis::aus_text(&format!("{}\n{}\n{}\n{}", w.anbieter_id(), w.provider().base_url,
        w.modell(), if w.art() == ZielArt::Lokal { "lokal" } else { "cloud" })).sha256
}

pub fn umfang(text: &str, w: &Wahl) -> Result<BuchUmfang, String> {
    if text.len() > MAX_BYTES { return Err("text_too_large".into()); }
    let teile = kapitel(text);
    let anfragen = teile.iter().map(|&[a, b]| abschnitte_bilden(&text[a..b], GRENZEN).len()).sum();
    let zeichen = text.chars().count();
    Ok(BuchUmfang { zeichen, kapitel: teile.len(), anfragen, ziel: ziel(w),
        cloud: w.art() != ZielArt::Lokal, ki_explizit_noetig: zeichen > 200_000 })
}

fn id(dokument: &str) -> String { Basis::aus_text(dokument).sha256 }

fn pruefe_start(text: &str, wahl: &Wahl, ziel_key: &str, ki_freigegeben: bool) -> Result<BuchUmfang, String> {
    let umfang = umfang(text, wahl)?;
    if umfang.ziel != ziel_key { return Err("ziel_gewechselt".into()); }
    if !ki_freigegeben { return Err("ki_opt_in_required".into()); }
    Ok(umfang)
}

fn pruefe_neuen_stand(alt: &Stand, text: &str, aktiv: bool) -> Result<(), String> {
    if aktiv && alt.status.phase == BuchJobPhase::Laeuft { return Err("job_running".into()); }
    if alt.text == text { return Err("job_exists".into()); }
    Ok(())
}

fn ordner(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    crate::portable::app_data_dir(app).map(|p| p.join("aufbereitung_buch_jobs"))
        .map_err(|_| "storage".into())
}

fn speichern(root: &Path, stand: &Stand) -> Result<(), String> {
    std::fs::create_dir_all(root).map_err(|_| "storage".to_string())?;
    let path = root.join(format!("{}.json", stand.status.id));
    let temp = root.join(format!("{}.tmp", stand.status.id));
    let bytes = serde_json::to_vec(stand).map_err(|_| "storage".to_string())?;
    // Alle Zugriffe auf denselben Job sind durch seine Stand-Sperre serialisiert.
    use std::io::Write;
    {
        let mut f = std::fs::File::create(&temp).map_err(|_| "storage".to_string())?;
        f.write_all(&bytes).and_then(|_| f.sync_all()).map_err(|_| "storage".to_string())?;
    }
    std::fs::rename(temp, path).map_err(|_| "storage".to_string())
}

fn holen(root: &Path, dokument: &str) -> Result<Arc<Lauf>, String> {
    let id = id(dokument);
    let mut jobs = sperre(&JOBS);
    if let Some(job) = jobs.get(&id) { return Ok(job.clone()); }
    let bytes = std::fs::read(root.join(format!("{id}.json"))).map_err(|_| "job_not_found".to_string())?;
    let mut stand: Stand = serde_json::from_slice(&bytes).map_err(|_| "storage".to_string())?;
    if stand.version != 1 || stand.dokument_id != dokument || stand.status.id != id {
        return Err("storage".into());
    }
    if stand.status.phase == BuchJobPhase::Laeuft {
        stand.status.phase = BuchJobPhase::Pausiert;
        stand.status.rest_sekunden = None;
        speichern(root, &stand)?;
    }
    let lauf = Arc::new(Lauf { stand: Mutex::new(stand), steuerung: AtomicU8::new(0) });
    jobs.insert(id, lauf.clone());
    Ok(lauf)
}

/// Globale Bytepositionen und eindeutige Gruppen auch bei lokalen Kapitelzeilennummern.
fn verschieben(a: &mut Aenderung, offset: usize, kapitel: usize) {
    a.bereich[0] += offset;
    a.bereich[1] += offset;
    a.id = format!("buch-k{kapitel}-{}", a.id);
    a.gruppe = a.gruppe.as_ref().map(|g| format!("buch-k{kapitel}-{g}"));
}

async fn ausfuehren(root: PathBuf, lauf: Arc<Lauf>, versand: Sprachversand, wahl: Wahl,
    melde: impl Fn(&BuchJobStatus, Option<(usize, Vec<Aenderung>)>) + Send + Sync) {
    let (text, dokument, job_id) = {
        let s = sperre(&lauf.stand);
        (s.text.clone(), s.dokument_id.clone(), s.status.id.clone())
    };
    let cache_root = root.join(&job_id);
    let result = std::fs::create_dir_all(&cache_root).map_err(|_| "storage".to_string());
    if let Err(e) = result { beenden(&root, &lauf, Some(e), &melde); return; }
    let cache = SprachCache::neu(&cache_root);
    let teile = kapitel(&text);
    let start = Instant::now();
    let mut erledigt = 0usize;
    for (k, &[von, bis]) in teile.iter().enumerate() {
        if k < sperre(&lauf.stand).status.kapitel_fertig { continue; }
        if lauf.steuerung.load(Ordering::Acquire) != 0 { break; }
        let basis = &text[von..bis];
        let stand = Textstand::aus_text(basis);
        let signal = lauf.clone();
        let abbruch = Abbruch::mit_anfragegrenze(Arc::new(move || signal.steuerung.load(Ordering::Acquire) != 0));
        // Kapitel bleiben getrennt; grosse Kapitel laufen in begrenzten A3-Anfragen.
        let ergebnis = sprache_job(&JobEingabe {
            dokument_id: &dokument, fassung_id: Some(&job_id), textstand: &stand,
            basis_text: basis, regeln: &[], cache: &cache, grenzen: GRENZEN,
        }, &versand, &wahl, &abbruch, &|| Ok(basis.to_string()), &|_, _| {}).await;
        let mut aenderungen: Vec<_> = ergebnis.gruppen.into_iter().flat_map(|g| g.aenderungen).collect();
        for a in &mut aenderungen { verschieben(a, von, k + 1); }
        let fehler = ergebnis.bericht.fehler.map(|e| e.code().to_string())
            .or_else(|| ergebnis.bericht.cache_schreibfehler.then(|| "cache".to_string()));
        let komplett = ergebnis.bericht.abschnitte_fertig == ergebnis.bericht.abschnitte_gesamt && fehler.is_none()
            && ergebnis.bericht.teilfehler == 0 && ergebnis.bericht.uebergross == 0
            && ergebnis.bericht.maskenkollision == 0;
        if komplett {
            erledigt += 1;
            let snapshot = {
                let mut s = sperre(&lauf.stand);
                s.status.aenderungen.extend(aenderungen.clone());
                s.status.kapitel_fertig = k + 1;
                s.status.prozent = (k + 1) as f64 * 100.0 / teile.len() as f64;
                s.status.rest_sekunden = Some((start.elapsed().as_secs_f64() / erledigt as f64
                    * (teile.len() - k - 1) as f64).ceil() as u64);
                if speichern(&root, &s).is_err() { drop(s); beenden(&root, &lauf, Some("storage".into()), &melde); return; }
                s.status.clone()
            };
            melde(&snapshot, Some((k + 1, aenderungen)));
        } else if !ergebnis.bericht.abgebrochen {
            beenden(&root, &lauf, Some(fehler.unwrap_or_else(|| "chapter_incomplete".into())), &melde);
            return;
        }
        if lauf.steuerung.load(Ordering::Acquire) != 0 { break; }
    }
    beenden(&root, &lauf, None, &melde);
}

fn beenden(root: &Path, lauf: &Lauf, fehler: Option<String>, melde: &Meldung<'_>) {
    let snapshot = {
        let mut s = sperre(&lauf.stand);
        s.status.phase = if fehler.is_some() { BuchJobPhase::Fehler }
            else if s.status.kapitel_fertig == s.status.kapitel_gesamt { BuchJobPhase::Fertig }
            else if lauf.steuerung.load(Ordering::Acquire) == 2 { BuchJobPhase::Gestoppt }
            else { BuchJobPhase::Pausiert };
        s.status.fehler = fehler;
        s.status.rest_sekunden = None;
        if speichern(root, &s).is_err() {
            s.status.phase = BuchJobPhase::Fehler;
            s.status.fehler = Some("storage".into());
        }
        s.status.clone()
    };
    melde(&snapshot, None);
}

pub fn starten(app: tauri::AppHandle, dokument: String, text: String, ziel_key: String,
    ki_freigegeben: bool, fortsetzen: bool) -> Result<BuchJobStatus, String> {
    let versand = Sprachversand::aus_app(&app).map_err(|e| e.code().to_string())?;
    let wahl = versand.vorschau().map_err(|e| e.code().to_string())?;
    let umfang = pruefe_start(&text, &wahl, &ziel_key, ki_freigegeben)?;
    let root = ordner(&app)?;
    let lauf = if fortsetzen { holen(&root, &dokument)? } else {
        let mut jobs = sperre(&JOBS);
        // Gleiche Basis wird fortgesetzt; nur ein ausdruecklicher Start mit neuer Basis ersetzt den Stand.
        let key = id(&dokument);
        if let Some(alt) = jobs.get(&key) {
            pruefe_neuen_stand(&sperre(&alt.stand), &text, true)?;
        } else {
            match std::fs::read(root.join(format!("{key}.json"))) {
                Ok(bytes) => {
                    let alt: Stand = serde_json::from_slice(&bytes).map_err(|_| "storage".to_string())?;
                    if alt.version != 1 || alt.dokument_id != dokument { return Err("storage".into()); }
                    pruefe_neuen_stand(&alt, &text, false)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("storage".into()),
            }
        }
        let status = BuchJobStatus { id: key.clone(), phase: BuchJobPhase::Pausiert,
            kapitel_fertig: 0, kapitel_gesamt: umfang.kapitel, prozent: 0.0,
            rest_sekunden: None, aenderungen: Vec::new(), fehler: None };
        let stand = Stand { version: 1, dokument_id: dokument, text: text.clone(), ziel: ziel_key.clone(), status };
        speichern(&root, &stand)?;
        let lauf = Arc::new(Lauf { stand: Mutex::new(stand), steuerung: AtomicU8::new(0) });
        jobs.insert(key, lauf.clone());
        lauf
    };
    let snapshot = {
        let mut s = sperre(&lauf.stand);
        if s.status.phase == BuchJobPhase::Laeuft { return Err("job_running".into()); }
        if s.text != text { return Err("basis_geaendert".into()); }
        if s.ziel != ziel_key { return Err("ziel_gewechselt".into()); }
        lauf.steuerung.store(0, Ordering::Release);
        s.status.phase = BuchJobPhase::Laeuft;
        s.status.fehler = None;
        if let Err(e) = speichern(&root, &s) {
            s.status.phase = BuchJobPhase::Fehler;
            s.status.fehler = Some(e.clone());
            return Err(e);
        }
        s.status.clone()
    };
    let _ = app.emit("aufbereitung-buch-fortschritt", &snapshot);
    tauri::async_runtime::spawn(ausfuehren(root, lauf, versand, wahl, move |status, kapitel| {
        if let Some((nummer, aenderungen)) = kapitel {
            let _ = app.emit("aufbereitung-buch-kapitel", BuchKapitelVorschlaege {
                id: status.id.clone(), kapitel: nummer, aenderungen });
        }
        let _ = app.emit("aufbereitung-buch-fortschritt", status);
    }));
    Ok(snapshot)
}

#[derive(Clone, Serialize, specta::Type)]
pub struct BuchKapitelVorschlaege {
    pub id: String,
    pub kapitel: usize,
    pub aenderungen: Vec<Aenderung>,
}

pub fn status(app: &tauri::AppHandle, dokument: &str) -> Result<BuchJobStatus, String> {
    let lauf = holen(&ordner(app)?, dokument)?;
    let result = sperre(&lauf.stand).status.clone();
    Ok(result)
}

pub fn vorschlaege(app: &tauri::AppHandle, dokument: &str, text: &str) -> Result<Vec<Aenderung>, String> {
    let lauf = holen(&ordner(app)?, dokument)?;
    let s = sperre(&lauf.stand);
    if s.text != text { return Err("basis_geaendert".into()); }
    Ok(s.status.aenderungen.clone())
}

pub fn steuern(app: &tauri::AppHandle, dokument: &str, stoppen: bool) -> Result<BuchJobStatus, String> {
    let root = ordner(app)?;
    let lauf = holen(&root, dokument)?;
    let mut s = sperre(&lauf.stand);
    lauf.steuerung.store(if stoppen { 2 } else { 1 }, Ordering::Release);
    if s.status.phase != BuchJobPhase::Laeuft && s.status.phase != BuchJobPhase::Fertig {
        s.status.phase = if stoppen { BuchJobPhase::Gestoppt } else { BuchJobPhase::Pausiert };
    }
    speichern(&root, &s)?;
    Ok(s.status.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::sprache::modell::{mock::{test_versand, MockVersender}, ModellAntwort, VersandAusfall};
    use super::super::sprache::politik::tests::einstellungen;

    fn lauf(text: &str, dokument: &str) -> Arc<Lauf> {
        Arc::new(Lauf { steuerung: AtomicU8::new(0), stand: Mutex::new(Stand {
            version: 1, dokument_id: dokument.into(), text: text.into(), ziel: "test".into(),
            status: BuchJobStatus { id: id(dokument), phase: BuchJobPhase::Laeuft,
                kapitel_fertig: 0, kapitel_gesamt: kapitel(text).len(), prozent: 0.0,
                rest_sekunden: None, aenderungen: Vec::new(), fehler: None },
        }) })
    }

    fn antwort(auftrag: &super::super::sprache::modell::Auftrag) -> ModellAntwort {
        let korrekturen = auftrag.nutzer.lines().filter_map(|l| {
            let (nr, text) = l.split_once('|')?;
            if !nr.bytes().all(|b| b.is_ascii_digit()) || !text.contains("Feler") { return None; }
            Some(format!("{nr}|{}", text.replace("Feler", "Fehler")))
        }).collect::<Vec<_>>();
        ModellAntwort { text: if korrekturen.is_empty() { "OK".into() } else { korrekturen.join("\n") }, abgeschnitten: false }
    }

    #[tokio::test]
    async fn stopp_nach_kapitel_drei_neustart_setzt_ab_vier_fort() {
        // Gegenprobe: Ohne gespeicherten Kapitelstand werden 1-3 erneut angefragt/dupliziert.
        let temp = tempfile::tempdir().unwrap();
        let dokument = temp.path().to_string_lossy().to_string();
        let text: String = (1..=5).map(|n| format!("## Kapitel {n}\nEin Feler bleibt.\n\n")).collect();
        let job = lauf(&text, &dokument);
        let mock = Arc::new(MockVersender::antworter(|a, _| Ok(antwort(a))));
        let versand = test_versand(einstellungen(true, false), mock.clone());
        let wahl = versand.vorschau().unwrap();
        ausfuehren(temp.path().into(), job.clone(), versand, wahl, |status, kapitel| {
            if kapitel.is_some() && status.kapitel_fertig == 3 { job.steuerung.store(2, Ordering::Release); }
        }).await;
        assert_eq!(sperre(&job.stand).status.kapitel_fertig, 3);
        assert!(sperre(&job.stand).status.phase == BuchJobPhase::Gestoppt);
        assert_eq!(sperre(&job.stand).status.aenderungen.len(), 3);
        assert_eq!(mock.anzahl(), 3);
        let wieder = holen(temp.path(), &dokument).unwrap();
        let versand = test_versand(einstellungen(true, false), mock.clone());
        let wahl = versand.vorschau().unwrap();
        let gemeldet = Mutex::new(Vec::new());
        ausfuehren(temp.path().into(), wieder.clone(), versand, wahl, |_, kapitel| {
            if let Some((n, _)) = kapitel { sperre(&gemeldet).push(n); }
        }).await;
        assert_eq!(*sperre(&gemeldet), vec![4, 5]);
        assert_eq!(mock.anzahl(), 5);
        let s = sperre(&wieder.stand);
        assert!(s.status.phase == BuchJobPhase::Fertig);
        assert_eq!(s.status.aenderungen.len(), 5);
        crate::aufbereitung::pruefe_aenderungen(&text, &s.status.aenderungen).unwrap();
        assert!(s.status.aenderungen.iter().all(|a| a.status == crate::aufbereitung::AenderungStatus::Offen));
    }

    #[tokio::test]
    async fn pause_nach_laufender_anfrage_auch_bei_zeitlimit() {
        for zeitlimit in [false, true] {
            // Gegenprobe: Sofortabbruch verliert die Antwort; ohne Anfragegrenze startet Anfrage 2.
            let temp = tempfile::tempdir().unwrap();
            let dokument = temp.path().to_string_lossy().to_string();
            let text = "Ein Feler bleibt.\n".repeat(121);
            let job = lauf(&text, &dokument);
            let signal = job.clone();
            let mock = Arc::new(MockVersender::antworter(move |a, nr| {
                if nr == 0 {
                    signal.steuerung.store(1, Ordering::Release);
                    if zeitlimit { return Err(VersandAusfall::Zeitlimit); }
                }
                Ok(antwort(a))
            }));
            let versand = test_versand(einstellungen(true, false), mock.clone());
            let wahl = versand.vorschau().unwrap();
            ausfuehren(temp.path().into(), job.clone(), versand, wahl, |_, _| {}).await;
            assert_eq!(mock.anzahl(), 1);
            assert!(sperre(&job.stand).status.phase == BuchJobPhase::Pausiert);
            let cache = SprachCache::neu(temp.path().join(id(&dokument)));
            assert_eq!(cache.anzahl(), usize::from(!zeitlimit));
            let wieder = holen(temp.path(), &dokument).unwrap();
            let versand = test_versand(einstellungen(true, false), mock.clone());
            let wahl = versand.vorschau().unwrap();
            ausfuehren(temp.path().into(), wieder.clone(), versand, wahl, |_, _| {}).await;
            assert_eq!(mock.anzahl(), if zeitlimit { 3 } else { 2 });
            let s = sperre(&wieder.stand);
            assert!(s.status.phase == BuchJobPhase::Fertig);
            assert_eq!(s.status.aenderungen.len(), 121);
        }
    }

    #[tokio::test]
    async fn unnummerierte_ganztextantwort_wird_nicht_zum_vorschlag() {
        // Gegenprobe: Ein permissiver Parser wuerde die komplette Modellantwort uebernehmen.
        let temp = tempfile::tempdir().unwrap();
        let job = lauf("{{S. 1}} Ein Feler bleibt.", "parser-test");
        let mock = Arc::new(MockVersender::antworter(|_, _| Ok(ModellAntwort {
            text: "Ein komplett neu geschriebenes Buch.".into(), abgeschnitten: false,
        })));
        let versand = test_versand(einstellungen(true, false), mock);
        let wahl = versand.vorschau().unwrap();
        ausfuehren(temp.path().into(), job.clone(), versand, wahl, |_, _| {}).await;
        let s = sperre(&job.stand);
        assert!(s.status.aenderungen.is_empty());
        assert_eq!(s.text, "{{S. 1}} Ein Feler bleibt.");
    }

    #[tokio::test]
    async fn cache_schreibfehler_ist_kein_fertiges_kapitel() {
        // Gegenprobe: Ein verschluckter Cachefehler wuerde dauerhaften Fortschritt vortaeuschen.
        let temp = tempfile::tempdir().unwrap();
        let job = lauf("Ein Feler bleibt.", "cache-test");
        let cache_root = temp.path().join(id("cache-test"));
        std::fs::create_dir_all(&cache_root).unwrap();
        std::fs::write(cache_root.join("aufbereitung"), b"synthetisch").unwrap();
        let mock = Arc::new(MockVersender::antworter(|a, _| Ok(antwort(a))));
        let versand = test_versand(einstellungen(true, false), mock);
        let wahl = versand.vorschau().unwrap();
        ausfuehren(temp.path().into(), job.clone(), versand, wahl, |_, _| {}).await;
        let s = sperre(&job.stand);
        assert!(s.status.phase == BuchJobPhase::Fehler);
        assert_eq!(s.status.fehler.as_deref(), Some("cache"));
        assert_eq!(s.status.kapitel_fertig, 0);
    }

    #[test]
    fn neustart_haelt_laufenden_job_an_und_umfang_zaehlt_anfragen() {
        // Gegenprobe: Ein nach Neustart weiterhin laufender Status laesst sich nicht fortsetzen.
        let temp = tempfile::tempdir().unwrap();
        let dokument = temp.path().to_string_lossy().to_string();
        let text = "## Kapitel 1\nEin Satz.\n\n## Kapitel 2\nAnderer Satz.";
        let job = lauf(text, &dokument);
        speichern(temp.path(), &sperre(&job.stand)).unwrap();
        let wieder = holen(temp.path(), &dokument).unwrap();
        assert!(sperre(&wieder.stand).status.phase == BuchJobPhase::Pausiert);
        let wahl = super::super::sprache::politik::waehle(&einstellungen(true, false)).unwrap();
        let u = umfang(text, &wahl).unwrap();
        assert_eq!(u.zeichen, text.chars().count());
        assert_eq!(u.kapitel, 2);
        assert_eq!(u.anfragen, 2);
        assert!(!u.cloud);
        assert!(!u.ki_explizit_noetig);
        assert!(umfang(&"x".repeat(200_001), &wahl).unwrap().ki_explizit_noetig);
        assert!(umfang(&"x".repeat(MAX_BYTES + 1), &wahl).is_err());
    }

    #[test]
    fn start_braucht_opt_in_und_bindet_ziel_und_basis() {
        // Gegenprobe: Ohne Startpruefung koennte ein unbestaetigtes/anderes Cloudziel anlaufen.
        let wahl = super::super::sprache::politik::waehle(&einstellungen(true, false)).unwrap();
        let key = ziel(&wahl);
        assert_eq!(pruefe_start("Text", &wahl, &key, false).err().as_deref(), Some("ki_opt_in_required"));
        assert_eq!(pruefe_start("Text", &wahl, "anderes_ziel", true).err().as_deref(), Some("ziel_gewechselt"));
        assert!(pruefe_start("Text", &wahl, &key, true).is_ok());
        let job = lauf("Text", "test");
        let mut s = sperre(&job.stand);
        assert_eq!(pruefe_neuen_stand(&s, "Neu", true).err().as_deref(), Some("job_running"));
        s.status.phase = BuchJobPhase::Pausiert;
        assert_eq!(pruefe_neuen_stand(&s, "Text", true).err().as_deref(), Some("job_exists"));
        assert!(pruefe_neuen_stand(&s, "Neu", true).is_ok());
    }
}
