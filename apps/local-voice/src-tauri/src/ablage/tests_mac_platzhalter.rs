//! lva#472 (Teil von #395, D7 macOS): Cloud-Platzhalter von OneDrive und iCloud („dataless“, File Provider)
//! erkennen, BEVOR gelesen wird, und nie einen Download auslösen. Nur synthetische Daten.
//!
//! Auf dem Gerät lässt sich ein echter Platzhalter nicht künstlich herstellen (`SF_DATALESS` setzt nur der
//! Kernel für einen File Provider). Deshalb prüfen die plattformneutralen Tests die Logik über die
//! Test-Naht (`naht::stat_flags_setzen` speist `st_flags` ein, wie macOS sie für einen Platzhalter liefert).
//! Was nur macOS kann (Öffnen ohne Lesezugriff, Thread-Richtlinie gegen den Abruf, `O_NOFOLLOW_ANY`), prüfen
//! die macOS-Tests gegen echte Dateien; der echte Platzhalter wird einmal von Hand in einem eigenen
//! OneDrive-Testordner geprüft (`echter_platzhalter_probe`, mit `--ignored`).

use super::eintrag::{
    self, attribute_aus_stat_flags, online_urteil, reparse_urteil, Gattung, Roh, Urteil,
    ATTR_OFFLINE, SF_DATALESS,
};
use super::speicher::Ablage;
use super::verknuepft;
use std::fs;
use std::path::PathBuf;

/// Weitere echte `st_flags` von macOS (`sys/stat.h`), die nichts mit „nur online“ zu tun haben.
const UF_COMPRESSED: u32 = 0x20;
const UF_TRACKED: u32 = 0x40;
const UF_HIDDEN: u32 = 0x8000;
const SF_ARCHIVED: u32 = 0x1_0000;
const SF_RESTRICTED: u32 = 0x8_0000;
const SF_FIRMLINK: u32 = 0x80_0000;
/// Echte Maske einer ausgelagerten Datei (OneDrive, gemessen 10.11.2026, macOS 15.8.1): `SF_DATALESS` mit
/// `UF_TRACKED` und `UF_COMPRESSED`.
const OFFLINE_GEMESSEN: u32 = 0x4000_0060;

struct NahtWaechter;

impl Drop for NahtWaechter {
    fn drop(&mut self) {
        eintrag::naht::leeren();
    }
}

fn naht_sauber() -> NahtWaechter {
    eintrag::naht::leeren();
    NahtWaechter
}

struct Env {
    _d: tempfile::TempDir,
    a: Ablage,
    quelle: PathBuf,
}

fn env() -> Env {
    let d = tempfile::tempdir().unwrap();
    let a = Ablage::neu(d.path().join("daten").join("ablage"));
    assert_eq!(a.wartung(), None);
    let quelle = d.path().join("Dokumente");
    fs::create_dir_all(&quelle).unwrap();
    // Kanonisch (macOS: /var → /private/var), damit der Pfad keine Verknüpfung enthält.
    let quelle = fs::canonicalize(quelle).unwrap();
    Env { _d: d, a, quelle }
}

impl Env {
    fn schreiben(&self, rel: &str, inhalt: &[u8]) -> PathBuf {
        let p = self.quelle.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, inhalt).unwrap();
        p
    }

    fn verknuepfen(&self) -> String {
        let id = verknuepft::verknuepfen(&self.a, &self.quelle).unwrap();
        verknuepft::abtasten(&self.a, &id).unwrap();
        id
    }
}

// ---------------------------------------------------------------------------
// Reine Logik (plattformneutral)
// ---------------------------------------------------------------------------

#[test]
fn stat_flags_werden_zum_selben_nur_online_zustand_wie_unter_windows() {
    assert_eq!(SF_DATALESS, 0x4000_0000, "`SF_DATALESS` aus sys/stat.h");
    // Das einzige Bit, das zählt; der Zustand ist derselbe wie Windows `OFFLINE`.
    assert_eq!(attribute_aus_stat_flags(SF_DATALESS), ATTR_OFFLINE);
    assert!(online_urteil(attribute_aus_stat_flags(OFFLINE_GEMESSEN)));
    assert_eq!(
        reparse_urteil(0, attribute_aus_stat_flags(OFFLINE_GEMESSEN)),
        Urteil::NurOnline
    );
    // Mit jeder Kombination anderer Flags bleibt es „nur online“ (auch alle Bits gesetzt).
    for f in [
        SF_DATALESS | UF_HIDDEN,
        SF_DATALESS | SF_ARCHIVED | SF_RESTRICTED | SF_FIRMLINK,
        u32::MAX,
    ] {
        assert!(online_urteil(attribute_aus_stat_flags(f)), "{f:#x}");
    }
    // Alles andere ist lesbar: lokale Dateien, komprimierte (decmpfs), verfolgte, versteckte.
    for f in [
        0,
        UF_COMPRESSED,
        UF_TRACKED,
        UF_COMPRESSED | UF_TRACKED,
        UF_HIDDEN,
        SF_ARCHIVED,
        SF_RESTRICTED,
        SF_FIRMLINK,
        !SF_DATALESS,
    ] {
        assert_eq!(attribute_aus_stat_flags(f), 0, "{f:#x}");
        assert!(!online_urteil(attribute_aus_stat_flags(f)));
        assert_eq!(
            reparse_urteil(0, attribute_aus_stat_flags(f)),
            Urteil::Lesbar
        );
    }
}

#[test]
fn dataless_gilt_fuer_dateien_wie_fuer_ordner_und_ueberstimmt_nichts_anderes() {
    let a = attribute_aus_stat_flags(OFFLINE_GEMESSEN);
    for gattung in [Gattung::Datei, Gattung::Ordner] {
        let roh = Roh {
            gattung,
            attribute: a,
            tag: 0,
            groesse: 57,
            zeit: 0,
            nlink: 1,
        };
        assert_eq!(roh.urteil(), Urteil::NurOnline, "{gattung:?}");
    }
    // Ein Symlink bleibt ein Umweg, auch wenn die Flags des Links selbst etwas melden.
    let link = Roh {
        gattung: Gattung::Link,
        attribute: a,
        tag: 0,
        groesse: 0,
        zeit: 0,
        nlink: 1,
    };
    assert_eq!(link.urteil(), Urteil::Umweg);
    // Die Ordner-Beobachtung (Workflows) und der Import urteilen über dieselben Attribute.
    let datei = Roh {
        gattung: Gattung::Datei,
        attribute: a,
        tag: 0,
        groesse: 57,
        zeit: 0,
        nlink: 1,
    };
    assert!(crate::managers::workflows::trigger::folder::attributes_cloud_only(a));
    assert_eq!(
        crate::managers::workflows::trigger::folder::entry_state(&datei),
        crate::managers::workflows::trigger::folder::EntryState::CloudOnly
    );
}

#[test]
fn abruf_gesperrt_ist_nur_ein_macos_fehler() {
    // Sonst nie: ein beliebiger Fehler ist kein „Abruf verweigert“.
    assert!(!eintrag::ist_abruf_gesperrt(&std::io::Error::from(
        std::io::ErrorKind::NotFound
    )));
    assert!(!eintrag::ist_abruf_gesperrt(&std::io::Error::other("x")));
    assert!(!eintrag::ist_abruf_gesperrt(
        &std::io::Error::from_raw_os_error(2)
    ));
}

// ---------------------------------------------------------------------------
// Über die Naht: Abtasten, Öffnen, Momentkopie (plattformneutral, dieselbe Folge wie unter Windows)
// ---------------------------------------------------------------------------

#[test]
fn dataless_dateien_sind_sichtbar_aber_nie_gelesen_und_ordner_nie_betreten() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("Lokal.txt", b"lokaler Text");
    let wolke = e.schreiben("Wolke.docx", b"niemals lesen");
    let tief = e.schreiben("Akte/Nur online.txt", b"Wolkenwort Kuckuck");
    e.schreiben("Offen/innen.txt", b"darin");
    eintrag::naht::stat_flags_setzen(&wolke, OFFLINE_GEMESSEN);
    eintrag::naht::stat_flags_setzen(&tief, OFFLINE_GEMESSEN);
    eintrag::naht::stat_flags_setzen(&e.quelle.join("Offen"), SF_DATALESS);
    let id = e.verknuepfen();
    let b = verknuepft::bestand(e.a.dir(), &id);
    let rels: Vec<&str> = b.dateien.keys().map(String::as_str).collect();
    assert_eq!(
        rels,
        ["Akte/Nur online.txt", "Lokal.txt", "Wolke.docx"],
        "der Ordner „Offen“ wird nicht betreten"
    );
    assert_eq!((b.nur_online_ordner, b.ausgelassen), (1, 0));
    for rel in ["Akte/Nur online.txt", "Wolke.docx"] {
        let ein = &b.dateien[rel];
        assert_eq!(ein.stand, verknuepft::NUR_ONLINE, "{rel}");
        assert_eq!(
            ein.groesse,
            fs::metadata(e.quelle.join(rel)).unwrap().len(),
            "Größe bleibt sichtbar"
        );
        assert_eq!(
            (ein.sha256.as_deref(), ein.textrev.as_deref()),
            (None, None)
        );
    }
    assert_eq!(b.dateien["Lokal.txt"].stand, "ausstehend");
    let auf: Vec<String> = verknuepft::auftraege(&e.a, None)
        .unwrap()
        .into_iter()
        .map(|a| a.rel)
        .collect();
    assert_eq!(auf, ["Lokal.txt"], "nie ein Auftrag für einen Platzhalter");
}

#[test]
fn dataless_wurzel_wird_nicht_verknuepft() {
    let _n = naht_sauber();
    let e = env();
    e.schreiben("a.txt", b"x");
    eintrag::naht::stat_flags_setzen(&e.quelle, SF_DATALESS);
    assert_eq!(
        verknuepft::verknuepfen(&e.a, &e.quelle).unwrap_err().code,
        "ablage.nur_online"
    );
}

#[test]
fn dataless_am_handle_wird_vor_dem_lesezugriff_abgewiesen() {
    let _n = naht_sauber();
    let e = env();
    let datei = e.schreiben("Akte/brief.txt", b"vertraulich");
    e.verknuepfen();
    let auf = verknuepft::auftraege(&e.a, None).unwrap().remove(0);
    // Der Pfad sieht eine lokale Datei, das Handle (fstat) meldet `SF_DATALESS`: Wettlauf mit „Speicher
    // freigeben“. Stufe 1 urteilt, Stufe 2 (Lesezugriff) wird nie erreicht.
    eintrag::naht::handle_stat_flags_setzen(&datei, OFFLINE_GEMESSEN);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert_eq!(eintrag::naht::stufe2_aufrufe(&datei), 0);
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    eintrag::naht::leeren();
    // Wird sie erst zwischen Stufe 1 und Stufe 2 ausgelagert, urteilt Stufe 2 am neuen Handle: nichts
    // wird kopiert.
    eintrag::naht::handle_folge(
        &datei,
        &[(0, 0), (attribute_aus_stat_flags(SF_DATALESS), 0)],
    );
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert_eq!(eintrag::naht::stufe2_aufrufe(&datei), 1);
    assert!(!e.a.dir().join("verknuepft").join("zwischen").exists());
    eintrag::naht::leeren();
    // Ausgelagert und wieder geladen (Flag weg): wird gelesen.
    let (kopie, _sha) = verknuepft::momentkopie(&e.a, &auf).unwrap();
    assert!(kopie.is_file());
    fs::remove_file(kopie).unwrap();
}

#[test]
fn dataless_pfadglied_oder_ausgelagerte_quelle_werden_vor_dem_oeffnen_abgewiesen() {
    let _n = naht_sauber();
    let e = env();
    let datei = e.schreiben("Akte/brief.txt", b"vertraulich");
    e.verknuepfen();
    let auf = verknuepft::auftraege(&e.a, None).unwrap().remove(0);
    eintrag::naht::stat_flags_setzen(&e.quelle.join("Akte"), SF_DATALESS);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    eintrag::naht::leeren();
    eintrag::naht::stat_flags_setzen(&datei, SF_DATALESS);
    assert_eq!(
        verknuepft::momentkopie(&e.a, &auf).unwrap_err(),
        "verknuepft.cloud"
    );
    assert_eq!(
        eintrag::naht::stufe2_aufrufe(&datei),
        0,
        "nicht einmal geöffnet"
    );
}

// ---------------------------------------------------------------------------
// Nur macOS: echte Dateien, echtes Öffnen, Thread-Richtlinie
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod mac {
    use super::*;
    use crate::ablage::eintrag::{OeffnenArt, OeffnenFehler};
    use std::io::Read;
    use std::os::raw::c_int;

    extern "C" {
        fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
    }

    /// `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES`, `IOPOL_SCOPE_THREAD` (sys/resource.h).
    fn thread_richtlinie() -> c_int {
        // SAFETY: reiner Lesezugriff auf die E/A-Richtlinie des aufrufenden Threads.
        unsafe { getiopolicy_np(3, 1) }
    }

    const AUS: c_int = 1;
    const STANDARD: c_int = 0;

    fn art(folgen: bool) -> OeffnenArt {
        OeffnenArt { folgen, teilen: 7 }
    }

    #[test]
    fn echte_lokale_datei_hat_keine_flags_und_wird_zweistufig_gelesen() {
        let _n = naht_sauber();
        let e = env();
        let p = e.schreiben("brief.txt", b"Inhalt Zebrafink");
        let r = eintrag::roh(&p).unwrap();
        assert_eq!((r.attribute, r.tag), (0, 0));
        assert_eq!(r.urteil(), Urteil::Lesbar);
        assert!(r.ist_datei());
        assert_eq!(r.groesse, 16);
        let mut f = eintrag::oeffnen_ohne_abruf(&p, art(false), eintrag::reparse_urteil)
            .unwrap_or_else(|_| panic!("lokale Datei muss lesbar sein"));
        assert_eq!(eintrag::offen_daten(&f, &p), Some((0, 0)));
        assert_eq!(eintrag::offen_urteil(&f, &p), Urteil::Lesbar);
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "Inhalt Zebrafink");
        // Ein Ordner: dieselben Rohdaten ohne Flag.
        let o = eintrag::roh(&e.quelle).unwrap();
        assert_eq!((o.gattung, o.attribute), (Gattung::Ordner, 0));
    }

    #[test]
    fn stufe_1_oeffnet_ohne_lesezugriff_stufe_2_mit() {
        let e = env();
        let p = e.schreiben("brief.txt", b"x");
        let eins = eintrag::naht::stufe1_fuer_test(&p, art(false)).unwrap();
        // SAFETY: gültiger Dateideskriptor, nur Abfrage der Öffnungsflags.
        let fl = unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&eins), libc::F_GETFL) };
        assert!(
            fl & libc::O_EVTONLY != 0,
            "Stufe 1 öffnet nur für Ereignisse (löst keinen Abruf aus), Flags {fl:#x}"
        );
        // Stufe 2 (der Rückgabewert von `oeffnen_ohne_abruf`) ist ein normales Lesehandle.
        let zwei = eintrag::oeffnen_ohne_abruf(&p, art(false), eintrag::reparse_urteil)
            .unwrap_or_else(|_| panic!("lesbar"));
        // SAFETY: wie oben.
        let fl2 = unsafe { libc::fcntl(std::os::fd::AsRawFd::as_raw_fd(&zwei), libc::F_GETFL) };
        assert_eq!(fl2 & libc::O_EVTONLY, 0);
    }

    #[test]
    fn symlinks_werden_nicht_gefolgt_auch_nicht_in_einem_zwischenglied() {
        let _n = naht_sauber();
        let e = env();
        let echt = e.schreiben("echt/f.txt", b"geheim");
        std::os::unix::fs::symlink(&echt, e.quelle.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(e.quelle.join("echt"), e.quelle.join("ordnerlink")).unwrap();
        // Der Link selbst.
        let r = eintrag::roh(&e.quelle.join("link.txt")).unwrap();
        assert_eq!(r.urteil(), Urteil::Umweg);
        assert!(matches!(
            eintrag::oeffnen_ohne_abruf(
                &e.quelle.join("link.txt"),
                art(false),
                eintrag::reparse_urteil
            ),
            Err(OeffnenFehler::Urteil(Urteil::Umweg))
        ));
        // Ein Symlink mitten im Pfad (zwischen Prüfung und Öffnen ausgetauscht): `O_NOFOLLOW_ANY`, ein
        // Umweg und nie gelesen (Stufe 2 wird nicht erreicht).
        let ueber = e.quelle.join("ordnerlink").join("f.txt");
        assert!(matches!(
            eintrag::oeffnen_ohne_abruf(&ueber, art(false), eintrag::reparse_urteil),
            Err(OeffnenFehler::Urteil(Urteil::Umweg))
        ));
        assert_eq!(eintrag::naht::stufe2_aufrufe(&ueber), 0);
        // Wer folgen will (Ordner-Beobachtung), darf es.
        assert!(
            eintrag::oeffnen_ohne_abruf(&ueber, art(true), eintrag::reparse_urteil).is_ok(),
            "folgen: true bleibt wie bisher"
        );
    }

    #[test]
    fn die_abrufsperre_gilt_nur_im_faden_und_stellt_den_alten_wert_wieder_her() {
        assert_eq!(thread_richtlinie(), STANDARD);
        {
            let aussen = eintrag::abruf_sperren();
            assert!(aussen.wirksam());
            assert_eq!(thread_richtlinie(), AUS);
            // Andere Fäden bleiben unberührt.
            std::thread::spawn(|| assert_eq!(thread_richtlinie(), STANDARD))
                .join()
                .unwrap();
            {
                let innen = eintrag::abruf_sperren();
                assert!(innen.wirksam());
                assert_eq!(thread_richtlinie(), AUS);
            }
            assert_eq!(thread_richtlinie(), AUS, "die äußere Sperre gilt weiter");
        }
        assert_eq!(thread_richtlinie(), STANDARD);
    }

    #[test]
    fn die_abrufsperre_wird_auch_bei_einer_panik_aufgehoben() {
        let r = std::thread::spawn(|| {
            let _s = eintrag::abruf_sperren();
            assert_eq!(thread_richtlinie(), AUS);
            panic!("Abbruch mitten im Lesen");
        })
        .join();
        assert!(r.is_err());
        // Der Faden ist weg; im Test-Faden war nie etwas gesetzt.
        assert_eq!(thread_richtlinie(), STANDARD);
        // Dieselbe Prüfung im selben Faden: Panik abfangen, danach ist die Richtlinie wieder wie vorher.
        let r = std::panic::catch_unwind(|| {
            let _s = eintrag::abruf_sperren();
            assert_eq!(thread_richtlinie(), AUS);
            panic!("Abbruch");
        });
        assert!(r.is_err());
        assert_eq!(thread_richtlinie(), STANDARD);
    }

    #[test]
    fn oeffnen_ohne_abruf_stellt_die_richtlinie_nach_jedem_ausgang_wieder_her() {
        let _n = naht_sauber();
        let e = env();
        let p = e.schreiben("brief.txt", b"x");
        let _ = eintrag::oeffnen_ohne_abruf(&p, art(false), eintrag::reparse_urteil);
        assert_eq!(thread_richtlinie(), STANDARD, "Erfolg");
        eintrag::naht::handle_stat_flags_setzen(&p, SF_DATALESS);
        assert!(eintrag::oeffnen_ohne_abruf(&p, art(false), eintrag::reparse_urteil).is_err());
        assert_eq!(thread_richtlinie(), STANDARD, "nur online");
        assert!(eintrag::oeffnen_ohne_abruf(
            &e.quelle.join("fehlt.txt"),
            art(false),
            eintrag::reparse_urteil
        )
        .is_err());
        assert_eq!(thread_richtlinie(), STANDARD, "Datei fehlt");
    }

    #[test]
    fn abruf_gesperrt_erkennt_nur_edeadlk() {
        // EDEADLK: so antwortet macOS auf einen Lesezugriff auf einen Platzhalter bei ausgeschalteter
        // Materialisierung (gemessen), statt die Datei herunterzuladen.
        assert!(eintrag::ist_abruf_gesperrt(
            &std::io::Error::from_raw_os_error(libc::EDEADLK)
        ));
        assert!(!eintrag::ist_abruf_gesperrt(
            &std::io::Error::from_raw_os_error(libc::EACCES)
        ));
        assert!(!eintrag::ist_abruf_gesperrt(&std::io::Error::from(
            std::io::ErrorKind::NotFound
        )));
    }

    #[test]
    fn ein_dataless_ordner_wird_nicht_aufgelistet_und_nie_teilweise() {
        let _n = naht_sauber();
        let e = env();
        e.schreiben("Akte/a.txt", b"a");
        e.schreiben("Akte/b.txt", b"b");
        let dir = e.quelle.join("Akte");
        assert_eq!(eintrag::verzeichnis(&dir).unwrap().len(), 2);
        // Wird der Ordner zwischen Urteil und Auflisten ausgelagert: Fehler für den ganzen Ordner.
        eintrag::naht::stat_flags_setzen(&dir, SF_DATALESS);
        assert!(eintrag::verzeichnis(&dir).is_err());
        assert_eq!(thread_richtlinie(), STANDARD);
    }

    #[test]
    fn jeder_eintrag_eines_echten_ordners_traegt_die_flags_seiner_selbst() {
        let _n = naht_sauber();
        let e = env();
        e.schreiben("a.txt", b"a");
        let p = e.quelle.join("a.txt");
        // Die Flags kommen aus `lstat` und werden pro Eintrag abgebildet; ein eingespeistes Flag erscheint
        // genau bei dem einen Eintrag.
        eintrag::naht::stat_flags_setzen(&p, SF_DATALESS);
        let liste = eintrag::verzeichnis(&e.quelle).unwrap();
        let roh = liste[0].roh.unwrap();
        assert_eq!(roh.urteil(), Urteil::NurOnline);
        assert_eq!(roh.attribute, ATTR_OFFLINE);
    }

    /// Der einzige Test gegen einen echten Platzhalter. Von Hand: in einem EIGENEN Testordner (nie Daten
    /// des Nutzers) synthetische Dateien anlegen, hochladen lassen, im Finder „Speicher freigeben“ wählen
    /// (eine Datei hydriert lassen), dann `LVA_472_PROBE_DIR=<Testordner> cargo test … -- --ignored
    /// echter_platzhalter_probe`. Der Test liest nur Metadaten und versucht jeden Lesezugriff nur unter
    /// der Abrufsperre; er prüft zuletzt, dass nichts hydriert wurde.
    #[test]
    #[ignore = "braucht einen echten File-Provider-Testordner (LVA_472_PROBE_DIR)"]
    fn echter_platzhalter_probe() {
        let Ok(dir) = std::env::var("LVA_472_PROBE_DIR") else {
            panic!("LVA_472_PROBE_DIR fehlt");
        };
        let dir = PathBuf::from(dir);
        assert!(
            dir.to_string_lossy().contains("LVA-Testordner-472"),
            "nur der eigene Testordner"
        );
        let vorher: Vec<(String, bool)> = eintrag::verzeichnis(&dir)
            .unwrap()
            .into_iter()
            .map(|e| {
                let n = e.name.to_string_lossy().into_owned();
                let online = e.roh.unwrap().urteil() == Urteil::NurOnline;
                (n, online)
            })
            .collect();
        println!("Eintraege: {vorher:?}");
        let mut online_n = 0;
        for (name, online) in &vorher {
            let p = dir.join(name);
            let r = eintrag::roh(&p).unwrap();
            if !*online {
                continue;
            }
            online_n += 1;
            if r.ist_datei() {
                // Stufe 1 meldet „nur online“, Stufe 2 wird nie erreicht.
                match eintrag::oeffnen_ohne_abruf(&p, art(false), eintrag::reparse_urteil) {
                    Err(OeffnenFehler::Urteil(Urteil::NurOnline)) => {}
                    Ok(_) => panic!("Platzhalter wurde geoeffnet"),
                    Err(_) => panic!("falscher Fehler"),
                }
                assert_eq!(eintrag::naht::stufe2_aufrufe(&p), 0);
                // Auch die Ordner-Beobachtung der Workflows liest keinen Platzhalter: Urteil über die
                // Rohdaten, Hashen scheitert, ohne zu laden.
                use crate::managers::workflows::trigger::folder::{
                    hash_file, FileProbe, SystemProbe,
                };
                assert!(SystemProbe.cloud_only(&p, &r));
                assert!(hash_file(&p).is_err());
                assert_eq!(eintrag::roh(&p).unwrap().urteil(), Urteil::NurOnline);
                // Der Lesezugriff unter der Abrufsperre: Fehler statt Download.
                let _s = eintrag::abruf_sperren();
                let mut f = fs::File::open(&p).unwrap();
                let mut b = [0u8; 8];
                let fehler = f.read(&mut b).unwrap_err();
                assert!(eintrag::ist_abruf_gesperrt(&fehler), "{fehler:?}");
            }
        }
        assert!(online_n > 0, "der Testordner braucht einen Platzhalter");
        // Der Ablage-Weg über den echten Ordner: verknüpfen, abtasten, Momentkopie je Platzhalter.
        let d = tempfile::tempdir().unwrap();
        let a = Ablage::neu(d.path().join("daten").join("ablage"));
        assert_eq!(a.wartung(), None);
        let kanonisch = fs::canonicalize(&dir).unwrap();
        let id = verknuepft::verknuepfen(&a, &kanonisch).unwrap();
        verknuepft::abtasten(&a, &id).unwrap();
        let b = verknuepft::bestand(a.dir(), &id);
        let mut online_rels = Vec::new();
        for (rel, e) in &b.dateien {
            let echt_online = e.stand == verknuepft::NUR_ONLINE;
            println!("Bestand: {rel} stand={} groesse={}", e.stand, e.groesse);
            if echt_online {
                online_rels.push(rel.clone());
                assert_eq!((e.sha256.as_deref(), e.textrev.as_deref()), (None, None));
                let auf = verknuepft::Auftrag {
                    ordner: id.clone(),
                    rel: rel.clone(),
                    dat: e.dat.clone(),
                    typ: e.typ.clone(),
                    groesse: e.groesse,
                    zeit: e.zeit,
                    wurzel: kanonisch.clone(),
                    quelle: kanonisch.join(rel),
                    art: verknuepft::Art::Text,
                    versuche: 0,
                };
                assert_eq!(
                    verknuepft::momentkopie(&a, &auf).unwrap_err(),
                    "verknuepft.cloud",
                    "{rel}"
                );
            }
        }
        assert_eq!(
            online_rels.len(),
            vorher.iter().filter(|(_, o)| *o).count()
                + usize::from(
                    b.dateien.contains_key("unterordner/tief.txt")
                        && b.dateien["unterordner/tief.txt"].stand == verknuepft::NUR_ONLINE
                ),
            "alle Platzhalter stehen als nur online im Bestand"
        );
        assert!(verknuepft::auftraege(&a, None)
            .unwrap()
            .iter()
            .all(|x| !online_rels.contains(&x.rel)));
        // Nichts wurde hydriert.
        for (name, online) in &vorher {
            let r = eintrag::roh(&dir.join(name)).unwrap();
            assert_eq!(r.urteil() == Urteil::NurOnline, *online, "{name}");
        }
        let tief = eintrag::roh(&dir.join("unterordner").join("tief.txt")).unwrap();
        assert_eq!(tief.urteil(), Urteil::NurOnline, "tief.txt nicht hydriert");
    }
}
