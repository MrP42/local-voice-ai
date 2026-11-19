//! Tauri-Anbindung mit getrennten Sitzungssperren und seriellen Arbeitern.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter, Manager, State};

use super::cache::{Ablage, Cache, DateiAblage};
use super::events::Ereignis;
use super::planner::Modus;
use super::sitzung::{self, SatzInhalt, Sitzung, Status};
use super::{MediaSegmentSource, QuellenFehler, Uebersetzer, UebersetzungsFehler, ZeitGuete};
use crate::media::mediendienst::{FfmpegDekoder, LokalerErkenner, Mediendienst};
use crate::media::zeitachse::{pruefe_medienquelle, MedienQuelle};
use crate::settings::AppSettings;

pub const EREIGNIS: &str = "medien-uebersetzung-ereignis";
const CACHE_LIMIT: u64 = 512 * 1024 * 1024;
const LEERLAUF: Duration = Duration::from_millis(150);
const STOPP_FRIST: Duration = Duration::from_secs(2);
// Cache-Transaktionen umfassen mehrere Ablageaufrufe, einschließlich Wiederaufnahme.
static CACHE_SPERRE: Mutex<()> = Mutex::new(());

fn sperren<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|fehler| fehler.into_inner())
}

fn darf_lokal_uebersetzen(is_local: bool) -> Result<(), UebersetzungsFehler> {
    if is_local {
        Ok(())
    } else {
        Err(UebersetzungsFehler("kein_lokales_modell".into()))
    }
}

fn ist_streng_lokal(url_oder_none: Option<&str>, ist_eingebaut: bool) -> bool {
    ist_eingebaut || url_oder_none.is_some_and(crate::llm_client::is_loopback_url)
}

fn uebersetzer_kennung(anbieter: &str, modell: &str, base_url: &str) -> String {
    format!("{anbieter}|{modell}|{base_url}")
}

fn uebersetzung_kuerzen(ergebnis: Result<String, String>) -> Result<String, UebersetzungsFehler> {
    ergebnis.map_err(|_| UebersetzungsFehler("uebersetzung".into()))
}

#[derive(Clone)]
pub struct LokalerUebersetzer {
    pub settings: AppSettings,
    pub ziel_sprache: String,
    pub handle: tokio::runtime::Handle,
}

impl Uebersetzer for LokalerUebersetzer {
    fn uebersetzen(&mut self, satz: &str) -> Result<String, UebersetzungsFehler> {
        // Streng: massgeblich ist die aufgeloeste Adresse, nicht die Anbieter-ID (ein Anbieter
        // "ollama" kann auf einen entfernten Server zeigen).
        let lokal = self
            .settings
            .active_post_process_provider()
            .is_some_and(|anbieter| {
                ist_streng_lokal(
                    Some(anbieter.base_url.as_str()),
                    crate::managers::llm::is_local(anbieter),
                )
            });
        darf_lokal_uebersetzen(lokal)?;
        uebersetzung_kuerzen(self.handle.block_on(crate::translator::translate_on(
            &self.settings,
            satz,
            &self.ziel_sprache,
            false,
        )))
    }

    /// Anbieter und Modell gehen in den Cache-Schluessel ein: ein anderes Modell uebersetzt neu.
    fn kennung(&self) -> String {
        let Some(anbieter) = self.settings.active_post_process_provider() else {
            return String::new();
        };
        let modell = self
            .settings
            .post_process_models
            .get(&anbieter.id)
            .map(String::as_str)
            .unwrap_or("");
        uebersetzer_kennung(&anbieter.id, modell, &anbieter.base_url)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct StartAntwort {
    pub session_id: String,
    pub dauer_ms: u64,
    pub hat_ton: bool,
    pub hat_video: bool,
}

pub trait Ereignissenke: Send + Sync {
    fn senden(&self, ereignis: Ereignis);
}

struct TauriSenke(AppHandle);

impl Ereignissenke for TauriSenke {
    fn senden(&self, ereignis: Ereignis) {
        let _ = self.0.emit(EREIGNIS, ereignis);
    }
}

struct AbbruchStand {
    aktuell: Arc<AtomicBool>,
    fenster: Arc<AtomicBool>,
}

#[derive(Clone)]
struct Abbruch(Arc<Mutex<AbbruchStand>>);

impl Abbruch {
    fn neu() -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        Self(Arc::new(Mutex::new(AbbruchStand {
            aktuell: Arc::clone(&flag),
            fenster: flag,
        })))
    }

    // Unter der Sitzungssperre dem Auftrag zuordnen, bevor die Werkbank hinausgeht.
    fn vorbereiten(&self) {
        let mut stand = sperren(&self.0);
        stand.fenster = Arc::clone(&stand.aktuell);
    }

    fn fenster(&self) -> Arc<AtomicBool> {
        Arc::clone(&sperren(&self.0).fenster)
    }

    fn erneuern(&self) {
        let mut stand = sperren(&self.0);
        stand.aktuell.store(true, Ordering::Release);
        stand.aktuell = Arc::new(AtomicBool::new(false));
    }

    fn setzen(&self) {
        sperren(&self.0).aktuell.store(true, Ordering::Release);
    }
}

/// Der Dienst behält seinen Lieferstand bis zum nächsten Seek.
struct LokaleQuelle {
    app: AppHandle,
    quelle: MedienQuelle,
    pfad: PathBuf,
    abbruch: Abbruch,
    dienst: Option<(
        Arc<AtomicBool>,
        Mediendienst<FfmpegDekoder, LokalerErkenner>,
    )>,
}

impl MediaSegmentSource for LokaleQuelle {
    fn segmente(
        &mut self,
        von_ms: u64,
        bis_ms: u64,
    ) -> Result<(Vec<super::Segment>, ZeitGuete), QuellenFehler> {
        let flag = self.abbruch.fenster();
        if self
            .dienst
            .as_ref()
            .is_none_or(|(alt, _)| !Arc::ptr_eq(alt, &flag))
        {
            let dienst = Mediendienst::neu(
                self.quelle.clone(),
                self.pfad.clone(),
                FfmpegDekoder,
                LokalerErkenner::neu(self.app.clone()),
                Arc::clone(&flag),
            );
            self.dienst = Some((flag, dienst));
        }
        match &mut self.dienst {
            Some((_, dienst)) => dienst.segmente(von_ms, bis_ms),
            None => Err(QuellenFehler("quelle".into())),
        }
    }
}

trait SitzungsZugang: Send + Sync {
    fn starten(&self);
    fn abspielen(&self, spielt: bool, position_ms: u64);
    fn spulen(&self, position_ms: u64);
    fn stoppen(&self);
    fn status(&self) -> Status;
    fn satz_text(&self, satz_id: &str) -> Option<SatzInhalt>;
    fn durchlauf(&self, jetzt_ms: u64) -> (bool, Vec<Ereignis>);
}

struct SitzungsAdapter<Q, U, A: Ablage> {
    sitzung: Mutex<Sitzung<Q, U, A>>,
    abbruch: Abbruch,
    cache_sperre: Option<&'static Mutex<()>>,
}

impl<Q, U, A> SitzungsZugang for SitzungsAdapter<Q, U, A>
where
    Q: MediaSegmentSource + Send,
    U: Uebersetzer + Send,
    A: Ablage + Send,
{
    fn starten(&self) {
        sperren(&self.sitzung).starten(0, false);
    }

    fn abspielen(&self, spielt: bool, position_ms: u64) {
        let mut sitzung = sperren(&self.sitzung);
        if position_ms.abs_diff(sitzung.status().bereit.von_ms) > 1_500 {
            self.abbruch.erneuern();
            sitzung.spulen(position_ms);
        }
        if spielt {
            sitzung.fortsetzen();
        } else {
            sitzung.pausieren();
        }
    }

    fn spulen(&self, position_ms: u64) {
        let mut sitzung = sperren(&self.sitzung);
        self.abbruch.erneuern();
        sitzung.spulen(position_ms);
    }

    fn stoppen(&self) {
        let mut sitzung = sperren(&self.sitzung);
        self.abbruch.setzen();
        sitzung.stoppen();
    }

    fn status(&self) -> Status {
        sperren(&self.sitzung).status()
    }

    fn satz_text(&self, satz_id: &str) -> Option<SatzInhalt> {
        sperren(&self.sitzung).satz_text(satz_id)
    }

    fn durchlauf(&self, jetzt_ms: u64) -> (bool, Vec<Ereignis>) {
        let (arbeit, mut ereignisse) = {
            let mut sitzung = sperren(&self.sitzung);
            let ereignisse = sitzung.tick(jetzt_ms).ereignisse;
            let arbeit = sitzung.naechster_auftrag().and_then(|auftrag| {
                sitzung.werkbank_entnehmen().map(|werkbank| {
                    self.abbruch.vorbereiten();
                    (auftrag, werkbank)
                })
            });
            (arbeit, ereignisse)
        };
        let Some((auftrag, mut werkbank)) = arbeit else {
            return (false, ereignisse);
        };
        let _cache_sperre = self.cache_sperre.map(sperren);
        let ergebnis = sitzung::ausfuehren(&mut werkbank, auftrag);
        let mut sitzung = sperren(&self.sitzung);
        ereignisse.extend(sitzung.uebernehmen(auftrag, ergebnis));
        let _ = sitzung.werkbank_zurueckgeben(werkbank);
        (true, ereignisse)
    }
}

struct VorbereiteteSitzung {
    sitzung: Arc<dyn SitzungsZugang>,
    dauer_ms: u64,
    hat_ton: bool,
    hat_video: bool,
}

trait SitzungsFabrik {
    fn erstellen(&self, session_id: &str, abbruch: Abbruch) -> Result<VorbereiteteSitzung, String>;
}

struct LokaleFabrik {
    app: AppHandle,
    pfad: PathBuf,
    ziel_sprache: String,
    handle: tokio::runtime::Handle,
}

impl SitzungsFabrik for LokaleFabrik {
    fn erstellen(&self, session_id: &str, abbruch: Abbruch) -> Result<VorbereiteteSitzung, String> {
        let quelle = pruefe_medienquelle(&self.pfad).map_err(|_| "medienquelle".to_owned())?;
        if !quelle.hat_ton {
            return Err("keine_tonspur".into());
        }
        let wurzel = self
            .app
            .path()
            .app_data_dir()
            .map_err(|_| "cache".to_owned())?
            .join("medien-uebersetzung-cache");
        let _cache_sperre = sperren(&CACHE_SPERRE);
        let ablage = DateiAblage::neu(wurzel).map_err(|_| "cache".to_owned())?;
        let mut cache = Cache::neu(ablage, CACHE_LIMIT);
        cache.wiederaufnehmen().map_err(|_| "cache".to_owned())?;
        let sitzung = Sitzung::neu(
            session_id.to_owned(),
            quelle.dauer_ms,
            Modus::Untertitel,
            self.ziel_sprache.clone(),
            LokaleQuelle {
                app: self.app.clone(),
                quelle: quelle.clone(),
                pfad: self.pfad.clone(),
                abbruch: abbruch.clone(),
                dienst: None,
            },
            LokalerUebersetzer {
                settings: crate::settings::get_settings(&self.app),
                ziel_sprache: self.ziel_sprache.clone(),
                handle: self.handle.clone(),
            },
            cache,
        );
        Ok(VorbereiteteSitzung {
            sitzung: Arc::new(SitzungsAdapter {
                sitzung: Mutex::new(sitzung),
                abbruch,
                cache_sperre: Some(&CACHE_SPERRE),
            }),
            dauer_ms: quelle.dauer_ms,
            hat_ton: quelle.hat_ton,
            hat_video: quelle.hat_video,
        })
    }
}

struct Eintrag {
    sitzung: Arc<dyn SitzungsZugang>,
    ende: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

#[derive(Default)]
pub struct SitzungsVerwalter {
    eintraege: Arc<Mutex<HashMap<String, Eintrag>>>,
}

fn session_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl SitzungsVerwalter {
    pub fn starten(
        &self,
        app: AppHandle,
        datei: String,
        ziel_sprache: String,
    ) -> Result<StartAntwort, String> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| "laufzeit".to_owned())?;
        let senke = Arc::new(TauriSenke(app.clone()));
        self.starten_mit(
            &LokaleFabrik {
                app,
                pfad: PathBuf::from(datei),
                ziel_sprache,
                handle,
            },
            senke,
        )
    }

    fn starten_mit(
        &self,
        fabrik: &dyn SitzungsFabrik,
        senke: Arc<dyn Ereignissenke>,
    ) -> Result<StartAntwort, String> {
        let id = session_id();
        let vorbereitet = fabrik.erstellen(&id, Abbruch::neu())?;
        if !vorbereitet.hat_ton {
            return Err("keine_tonspur".into());
        }
        let sitzung = vorbereitet.sitzung;
        sitzung.starten();
        let ende = Arc::new(AtomicBool::new(false));
        let worker_ende = Arc::clone(&ende);
        let worker_sitzung = Arc::clone(&sitzung);
        let mut eintraege = sperren(&self.eintraege);
        if eintraege.contains_key(&id) {
            return Err("sitzungskennung".into());
        }
        let worker = thread::Builder::new()
            .name("medien-uebersetzung".into())
            .spawn(move || {
                let beginn = Instant::now();
                while !worker_ende.load(Ordering::Acquire) {
                    let jetzt_ms = beginn.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                    let (gearbeitet, ereignisse) = worker_sitzung.durchlauf(jetzt_ms);
                    for ereignis in ereignisse {
                        if worker_ende.load(Ordering::Acquire) {
                            break;
                        }
                        senke.senden(ereignis);
                    }
                    if !gearbeitet {
                        thread::park_timeout(LEERLAUF);
                    }
                }
            })
            .map_err(|_| "worker".to_owned())?;
        eintraege.insert(
            id.clone(),
            Eintrag {
                sitzung,
                ende,
                worker,
            },
        );
        Ok(StartAntwort {
            session_id: id,
            dauer_ms: vorbereitet.dauer_ms,
            hat_ton: vorbereitet.hat_ton,
            hat_video: vorbereitet.hat_video,
        })
    }

    pub fn abspielen(
        &self,
        session_id: &str,
        spielt: bool,
        position_ms: u64,
    ) -> Result<(), String> {
        let eintraege = sperren(&self.eintraege);
        let eintrag = eintraege
            .get(session_id)
            .ok_or_else(|| "unbekannte_sitzung".to_owned())?;
        eintrag.sitzung.abspielen(spielt, position_ms);
        eintrag.worker.thread().unpark();
        Ok(())
    }

    pub fn spulen(&self, session_id: &str, position_ms: u64) -> Result<(), String> {
        let eintraege = sperren(&self.eintraege);
        let eintrag = eintraege
            .get(session_id)
            .ok_or_else(|| "unbekannte_sitzung".to_owned())?;
        eintrag.sitzung.spulen(position_ms);
        eintrag.worker.thread().unpark();
        Ok(())
    }

    pub fn stoppen(&self, session_id: &str) -> Result<(), String> {
        self.stoppen_mit_frist(session_id, STOPP_FRIST)
    }

    fn stoppen_mit_frist(&self, session_id: &str, frist: Duration) -> Result<(), String> {
        let Some(eintrag) = sperren(&self.eintraege).remove(session_id) else {
            return Ok(());
        };
        eintrag.ende.store(true, Ordering::Release);
        eintrag.sitzung.stoppen();
        eintrag.worker.thread().unpark();
        let beginn = Instant::now();
        while !eintrag.worker.is_finished() {
            let Some(rest) = frist.checked_sub(beginn.elapsed()) else {
                // Rust kann fremde Threads nicht töten; der Abbruch bleibt dauerhaft gesetzt.
                return Err("worker_zeitueberschreitung".into());
            };
            thread::sleep(rest.min(Duration::from_millis(10)));
        }
        eintrag.worker.join().map_err(|_| "worker".to_owned())
    }

    pub fn status(&self, session_id: &str) -> Result<Status, String> {
        let eintraege = sperren(&self.eintraege);
        eintraege
            .get(session_id)
            .map(|eintrag| eintrag.sitzung.status())
            .ok_or_else(|| "unbekannte_sitzung".to_owned())
    }

    pub fn satz_text(&self, session_id: &str, satz_id: &str) -> Result<Option<SatzInhalt>, String> {
        let eintraege = sperren(&self.eintraege);
        eintraege
            .get(session_id)
            .map(|eintrag| eintrag.sitzung.satz_text(satz_id))
            .ok_or_else(|| "unbekannte_sitzung".to_owned())
    }
}

impl Drop for SitzungsVerwalter {
    fn drop(&mut self) {
        for (_, eintrag) in sperren(&self.eintraege).drain() {
            eintrag.ende.store(true, Ordering::Release);
            eintrag.sitzung.stoppen();
            eintrag.worker.thread().unpark();
        }
    }
}

async fn blockierend<T: Send + 'static>(
    arbeit: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(arbeit)
        .await
        .map_err(|_| "worker".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_starten(
    app: AppHandle,
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    datei: String,
    ziel_sprache: String,
) -> Result<StartAntwort, String> {
    let verwalter = Arc::clone(verwalter.inner());
    blockierend(move || verwalter.starten(app, datei, ziel_sprache)).await
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_abspielen(
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    session_id: String,
    spielt: bool,
    position_ms: u64,
) -> Result<(), String> {
    verwalter.abspielen(&session_id, spielt, position_ms)
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_spulen(
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    session_id: String,
    position_ms: u64,
) -> Result<(), String> {
    verwalter.spulen(&session_id, position_ms)
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_stoppen(
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    session_id: String,
) -> Result<(), String> {
    let verwalter = Arc::clone(verwalter.inner());
    blockierend(move || verwalter.stoppen(&session_id)).await
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_status(
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    session_id: String,
) -> Result<Status, String> {
    verwalter.status(&session_id)
}

#[tauri::command]
#[specta::specta]
pub async fn medien_uebersetzung_satz_text(
    verwalter: State<'_, Arc<SitzungsVerwalter>>,
    session_id: String,
    satz_id: String,
) -> Result<Option<SatzInhalt>, String> {
    verwalter.satz_text(&session_id, &satz_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::{self, Receiver, Sender};

    use super::super::cache::SpeicherAblage;
    use super::super::events::{EreignisArt, FehlerCode};
    use super::super::Segment;

    #[derive(Default)]
    struct VecSenke(Mutex<Vec<Ereignis>>);

    impl Ereignissenke for VecSenke {
        fn senden(&self, ereignis: Ereignis) {
            sperren(&self.0).push(ereignis);
        }
    }

    struct TestQuelle {
        abbruch: Abbruch,
        aufrufe: Arc<AtomicUsize>,
        beendet: Arc<AtomicBool>,
        eingang: Sender<Arc<AtomicBool>>,
        wartet: bool,
    }

    impl Drop for TestQuelle {
        fn drop(&mut self) {
            self.beendet.store(true, Ordering::Release);
        }
    }

    impl MediaSegmentSource for TestQuelle {
        fn segmente(
            &mut self,
            von_ms: u64,
            _bis_ms: u64,
        ) -> Result<(Vec<Segment>, ZeitGuete), QuellenFehler> {
            self.aufrufe.fetch_add(1, Ordering::AcqRel);
            let flag = self.abbruch.fenster();
            let _ = self.eingang.send(Arc::clone(&flag));
            if self.wartet {
                while !flag.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                return Err(QuellenFehler("abgebrochen".into()));
            }
            Ok((
                vec![Segment {
                    start_ms: von_ms,
                    end_ms: von_ms + 100,
                    text: "GEHEIMER SATZ.".into(),
                    woerter: None,
                }],
                ZeitGuete::Satz,
            ))
        }
    }

    struct TestUebersetzer {
        modell_fehlt: Arc<AtomicBool>,
        dauerfehler: Arc<AtomicBool>,
    }

    impl Uebersetzer for TestUebersetzer {
        fn uebersetzen(&mut self, _satz: &str) -> Result<String, UebersetzungsFehler> {
            darf_lokal_uebersetzen(!self.modell_fehlt.load(Ordering::Acquire))?;
            if self.dauerfehler.load(Ordering::Acquire) {
                return uebersetzung_kuerzen(Err("GEHEIMER FEHLERPFAD".into()));
            }
            Ok("GEHEIMES ZIEL".into())
        }
    }

    struct TestFabrik {
        aufrufe: Arc<AtomicUsize>,
        beendet: Arc<AtomicBool>,
        modell_fehlt: Arc<AtomicBool>,
        dauerfehler: Arc<AtomicBool>,
        eingang: Sender<Arc<AtomicBool>>,
        wartet: bool,
        hat_ton: bool,
        dauer_ms: u64,
    }

    impl TestFabrik {
        fn neu(wartet: bool) -> (Self, Receiver<Arc<AtomicBool>>) {
            let (eingang, empfaenger) = mpsc::channel();
            (
                Self {
                    aufrufe: Arc::new(AtomicUsize::new(0)),
                    beendet: Arc::new(AtomicBool::new(false)),
                    modell_fehlt: Arc::new(AtomicBool::new(false)),
                    dauerfehler: Arc::new(AtomicBool::new(false)),
                    eingang,
                    wartet,
                    hat_ton: true,
                    dauer_ms: 30_000,
                },
                empfaenger,
            )
        }
    }

    impl SitzungsFabrik for TestFabrik {
        fn erstellen(
            &self,
            session_id: &str,
            abbruch: Abbruch,
        ) -> Result<VorbereiteteSitzung, String> {
            let sitzung = Sitzung::neu(
                session_id.into(),
                self.dauer_ms,
                Modus::Untertitel,
                "en".into(),
                TestQuelle {
                    abbruch: abbruch.clone(),
                    aufrufe: Arc::clone(&self.aufrufe),
                    beendet: Arc::clone(&self.beendet),
                    eingang: self.eingang.clone(),
                    wartet: self.wartet,
                },
                TestUebersetzer {
                    modell_fehlt: Arc::clone(&self.modell_fehlt),
                    dauerfehler: Arc::clone(&self.dauerfehler),
                },
                Cache::neu(SpeicherAblage::default(), CACHE_LIMIT),
            );
            Ok(VorbereiteteSitzung {
                sitzung: Arc::new(SitzungsAdapter {
                    sitzung: Mutex::new(sitzung),
                    abbruch,
                    cache_sperre: None,
                }),
                dauer_ms: self.dauer_ms,
                hat_ton: self.hat_ton,
                hat_video: true,
            })
        }
    }

    fn warten(mut bedingung: impl FnMut() -> bool) {
        let beginn = Instant::now();
        while !bedingung() {
            assert!(beginn.elapsed() < Duration::from_secs(2), "Testfrist");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn nur_loopback_oder_eingebautes_modell_darf_uebersetzen() {
        for url in [
            "http://127.0.0.1:11434/v1",
            "http://localhost:11434/v1",
            "http://[::1]:11434/v1",
        ] {
            assert!(ist_streng_lokal(Some(url), false));
        }
        for url in [
            "http://entfernter-server:11434/v1",
            "https://api.example.com",
            "http://127.0.0.1.evil.example/",
        ] {
            // Gegenprobe: Eine Anbieter-ID wie ollama oder vllm darf die URL nicht freigeben.
            assert!(!ist_streng_lokal(Some(url), false));
        }
        assert!(!ist_streng_lokal(None, false));
        assert!(ist_streng_lokal(None, true));
        assert!(ist_streng_lokal(Some("https://api.example.com"), true));
        let mut aufgerufen = false;
        let ergebnis = darf_lokal_uebersetzen(ist_streng_lokal(
            Some("http://entfernter-server:11434/v1"),
            false,
        ))
        .map(|()| aufgerufen = true);
        // Gegenprobe: Ohne lokale Freigabe würde der nachgelagerte Aufruf stattfinden.
        assert_eq!(
            ergebnis,
            Err(UebersetzungsFehler("kein_lokales_modell".into()))
        );
        assert!(!aufgerufen);
        assert_eq!(darf_lokal_uebersetzen(true), Ok(()));
    }

    #[test]
    fn cachekennung_unterscheidet_lokale_endpunkte() {
        let erste = uebersetzer_kennung("ollama", "modell", "http://127.0.0.1:11434/v1");
        let zweite = uebersetzer_kennung("ollama", "modell", "http://127.0.0.1:11435/v1");
        // Gegenprobe: Anbieter und Modell allein würden beide Endpunkte gleich behandeln.
        assert_ne!(erste, zweite);
        assert_eq!(erste, "ollama|modell|http://127.0.0.1:11434/v1");
    }

    #[test]
    fn uebersetzungsfehler_enthalten_keinen_fremden_text() {
        // Gegenprobe: Durchreichen des Fehlers verrät den privaten Marker.
        assert_eq!(
            uebersetzung_kuerzen(Err("GEHEIMER PFAD".into())),
            Err(UebersetzungsFehler("uebersetzung".into()))
        );
        assert_eq!(uebersetzung_kuerzen(Ok("Ziel".into())), Ok("Ziel".into()));
    }

    #[test]
    fn start_liefert_zufaellige_kennung_und_metadaten() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        // Gegenprobe: Ein konstanter oder abgeschnittener Bezeichner erfüllt dies nicht.
        assert_eq!(antwort.session_id.len(), 32);
        assert!(antwort
            .session_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(antwort.session_id, session_id());
        assert_eq!(antwort.dauer_ms, 30_000);
        assert!(antwort.hat_ton && antwort.hat_video);
        assert!(verwalter.status(&antwort.session_id).unwrap().laeuft);
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn ohne_ton_startet_kein_worker() {
        let verwalter = SitzungsVerwalter::default();
        let (mut fabrik, _) = TestFabrik::neu(false);
        fabrik.hat_ton = false;
        // Gegenprobe: Ohne Tonprüfung würde der Worker die Quelle aufrufen.
        assert_eq!(
            verwalter.starten_mit(&fabrik, Arc::new(VecSenke::default())),
            Err("keine_tonspur".into())
        );
        assert!(sperren(&verwalter.eintraege).is_empty());
        assert_eq!(fabrik.aufrufe.load(Ordering::Acquire), 0);
    }

    #[test]
    fn doppelter_stopp_ist_harmlos() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        verwalter.stoppen(&antwort.session_id).unwrap();
        // Gegenprobe: Fehlende Einträge als Stoppfehler zu behandeln bricht Idempotenz.
        assert_eq!(verwalter.stoppen(&antwort.session_id), Ok(()));
        assert_eq!(
            verwalter.status(&antwort.session_id),
            Err("unbekannte_sitzung".into())
        );
    }

    #[test]
    fn unbekannte_kennung_liefert_nur_kurzform() {
        let verwalter = SitzungsVerwalter::default();
        // Gegenprobe: Ein Fallback auf eine andere Sitzung würde Daten freigeben.
        assert_eq!(verwalter.status("GEHEIM"), Err("unbekannte_sitzung".into()));
        assert_eq!(
            verwalter.abspielen("GEHEIM", true, 0),
            Err("unbekannte_sitzung".into())
        );
        assert_eq!(
            verwalter.spulen("GEHEIM", 1),
            Err("unbekannte_sitzung".into())
        );
        assert_eq!(
            verwalter.satz_text("GEHEIM", "satz"),
            Err("unbekannte_sitzung".into())
        );
    }

    #[test]
    fn abspielen_laesst_position_und_auftraege_weiterlaufen() {
        let verwalter = SitzungsVerwalter::default();
        let (mut fabrik, _) = TestFabrik::neu(false);
        fabrik.dauer_ms = 180_000;
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        warten(|| verwalter.status(&antwort.session_id).unwrap().bereit.bis_ms == 90_000);
        assert_eq!(fabrik.aufrufe.load(Ordering::Acquire), 6);
        verwalter.abspielen(&antwort.session_id, true, 0).unwrap();
        // Gegenprobe: Ohne Wiedergabefreigabe bleiben Position und Vorsprung bei 0 bzw. 90 s.
        warten(|| {
            let status = verwalter.status(&antwort.session_id).unwrap();
            status.position_ms > 0 && status.bereit.bis_ms > 90_000
        });
        assert!(fabrik.aufrufe.load(Ordering::Acquire) > 6);
        assert_eq!(verwalter.status(&antwort.session_id).unwrap().epoch, 1);
        let position = verwalter.status(&antwort.session_id).unwrap().position_ms;
        verwalter
            .abspielen(&antwort.session_id, false, position)
            .unwrap();
        let position = verwalter.status(&antwort.session_id).unwrap().position_ms;
        thread::sleep(LEERLAUF + LEERLAUF);
        assert_eq!(
            verwalter.status(&antwort.session_id).unwrap().position_ms,
            position
        );
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn abspielen_sucht_nur_bei_mehr_als_1500_ms_abweichung() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, eingang) = TestFabrik::neu(true);
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        let alt = eingang.recv_timeout(STOPP_FRIST).unwrap();
        verwalter
            .abspielen(&antwort.session_id, false, 1_500)
            .unwrap();
        // Gegenprobe: Ein unbedingter Seek würde schon an der Toleranzgrenze abbrechen.
        assert!(!alt.load(Ordering::Acquire));
        let status = verwalter.status(&antwort.session_id).unwrap();
        assert_eq!(status.position_ms, 0);
        assert_eq!(status.epoch, 1);
        verwalter
            .abspielen(&antwort.session_id, false, 1_501)
            .unwrap();
        let neu = eingang.recv_timeout(STOPP_FRIST).unwrap();
        // Gegenprobe: Nur die Wiedergabe zu setzen lässt das alte Fenster weiterlaufen.
        assert!(alt.load(Ordering::Acquire));
        assert!(!neu.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&alt, &neu));
        let status = verwalter.status(&antwort.session_id).unwrap();
        assert_eq!(status.position_ms, 1_501);
        assert_eq!(status.epoch, 2);
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn seek_bricht_altes_fenster_ab_und_erneuert_flag() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, eingang) = TestFabrik::neu(true);
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        let alt = eingang.recv_timeout(STOPP_FRIST).unwrap();
        verwalter.spulen(&antwort.session_id, 20_000).unwrap();
        let neu = eingang.recv_timeout(STOPP_FRIST).unwrap();
        // Gegenprobe: Dasselbe Flag zurückzusetzen lässt laufende Arbeit den Abbruch verpassen.
        assert!(alt.load(Ordering::Acquire));
        assert!(!neu.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&alt, &neu));
        let status = verwalter.status(&antwort.session_id).unwrap();
        assert_eq!(status.position_ms, 20_000);
        assert_eq!(status.epoch, 2);
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn stopp_beendet_worker_und_gibt_werkbank_frei() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, eingang) = TestFabrik::neu(true);
        let antwort = verwalter
            .starten_mit(&fabrik, Arc::new(VecSenke::default()))
            .unwrap();
        let flag = eingang.recv_timeout(STOPP_FRIST).unwrap();
        let beginn = Instant::now();
        verwalter.stoppen(&antwort.session_id).unwrap();
        // Gegenprobe: Nur den Eintrag zu entfernen beendet die ausgelagerte Werkbank nicht.
        assert!(beginn.elapsed() < STOPP_FRIST);
        assert!(flag.load(Ordering::Acquire));
        assert!(fabrik.beendet.load(Ordering::Acquire));
    }

    #[test]
    fn ereignissenke_erhaelt_metadaten_und_texte_bleiben_abrufbar() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        let senke = Arc::new(VecSenke::default());
        let antwort = verwalter.starten_mit(&fabrik, senke.clone()).unwrap();
        warten(|| {
            sperren(&senke.0)
                .iter()
                .any(|e| matches!(e.art, EreignisArt::Satz { .. }))
        });
        let ereignisse = sperren(&senke.0).clone();
        let satz_id = ereignisse
            .iter()
            .find_map(|e| match &e.art {
                EreignisArt::Satz { satz_id, .. } => Some(satz_id),
                _ => None,
            })
            .unwrap();
        let inhalt = verwalter
            .satz_text(&antwort.session_id, satz_id)
            .unwrap()
            .unwrap();
        assert_eq!(inhalt.original, "GEHEIMER SATZ.");
        assert_eq!(inhalt.uebersetzung, "GEHEIMES ZIEL");
        // Gegenprobe: Satztexte oder Übersetzungen im Ereignis verraten diese Marker.
        assert!(!serde_json::to_string(&ereignisse)
            .unwrap()
            .contains("GEHEIM"));
        assert!(ereignisse
            .iter()
            .all(|e| e.session_id == antwort.session_id));
        assert_eq!(
            verwalter.satz_text(&antwort.session_id, "fehlt").unwrap(),
            None
        );
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn modellfehler_pausiert_bis_seek() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        fabrik.modell_fehlt.store(true, Ordering::Release);
        let senke = Arc::new(VecSenke::default());
        let antwort = verwalter.starten_mit(&fabrik, senke.clone()).unwrap();
        warten(|| {
            verwalter.status(&antwort.session_id).unwrap().fehler
                == Some(FehlerCode::KeinLokalesModell)
        });
        thread::sleep(LEERLAUF + LEERLAUF);
        // Gegenprobe: Wiederholte Aufträge würden die Quelle trotz Sperre erneut aufrufen.
        assert_eq!(fabrik.aufrufe.load(Ordering::Acquire), 1);
        fabrik.modell_fehlt.store(false, Ordering::Release);
        verwalter.spulen(&antwort.session_id, 20_000).unwrap();
        warten(|| {
            sperren(&senke.0)
                .iter()
                .any(|e| matches!(e.art, EreignisArt::Satz { .. }))
        });
        assert_eq!(verwalter.status(&antwort.session_id).unwrap().fehler, None);
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn dauerfehler_pausiert_ohne_private_fehlertexte() {
        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        fabrik.dauerfehler.store(true, Ordering::Release);
        let senke = Arc::new(VecSenke::default());
        let antwort = verwalter.starten_mit(&fabrik, senke.clone()).unwrap();
        warten(|| {
            sperren(&senke.0).iter().any(|e| {
                matches!(
                    e.art,
                    EreignisArt::Fehler {
                        code: FehlerCode::Uebersetzung
                    }
                )
            })
        });
        thread::sleep(LEERLAUF + LEERLAUF);
        // Gegenprobe: Ohne Dauersperre würden weitere Fenster und Fehlerereignisse entstehen.
        assert_eq!(fabrik.aufrufe.load(Ordering::Acquire), 1);
        assert_eq!(
            sperren(&senke.0)
                .iter()
                .filter(|e| matches!(e.art, EreignisArt::Fehler { .. }))
                .count(),
            1
        );
        assert!(!serde_json::to_string(&*sperren(&senke.0))
            .unwrap()
            .contains("GEHEIM"));
        verwalter.stoppen(&antwort.session_id).unwrap();
    }

    #[test]
    fn stopp_wartet_nur_bis_zur_frist() {
        struct SperrSenke {
            eingang: Sender<()>,
            frei: Arc<AtomicBool>,
        }

        impl Ereignissenke for SperrSenke {
            fn senden(&self, _ereignis: Ereignis) {
                let _ = self.eingang.send(());
                while !self.frei.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }

        struct Freigabe(Arc<AtomicBool>);
        impl Drop for Freigabe {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let verwalter = SitzungsVerwalter::default();
        let (fabrik, _) = TestFabrik::neu(false);
        let (sender, empfaenger) = mpsc::channel();
        let frei = Freigabe(Arc::new(AtomicBool::new(false)));
        let antwort = verwalter
            .starten_mit(
                &fabrik,
                Arc::new(SperrSenke {
                    eingang: sender,
                    frei: Arc::clone(&frei.0),
                }),
            )
            .unwrap();
        empfaenger.recv_timeout(STOPP_FRIST).unwrap();
        let beginn = Instant::now();
        let ergebnis = verwalter.stoppen_mit_frist(&antwort.session_id, Duration::from_millis(25));
        drop(frei);
        // Gegenprobe: Ein unbedingtes join würde an der gesperrten Senke hängen bleiben.
        assert_eq!(ergebnis, Err("worker_zeitueberschreitung".into()));
        assert!(beginn.elapsed() < Duration::from_secs(1));
        assert!(sperren(&verwalter.eintraege).is_empty());
        warten(|| fabrik.beendet.load(Ordering::Acquire));
    }
}
