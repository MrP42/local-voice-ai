"""Offline-Verkabelungspruefung; ersetzt nicht die Rust-Verhaltenstests."""
from pathlib import Path
import re
import unittest

SRC = Path(__file__).resolve().parents[1] / "src-tauri" / "src"

# Jede entfernte Sperre muss ihre Gegenprobe ausloesen, ohne Quellen zu aendern.
STELLEN = {
    "lib.rs": ["richtlinie::plugin", "richtlinie::autostart_schreiben_mit"],
    "settings.rs": ["richtlinie::einstellungen_pruefen", "richtlinie::einstellungen_begrenzen"],
    "shortcut/handy_keys.rs": ["richtlinie::Vorgang::TastaturHook"],
    "shortcut/tauri_impl.rs": ["!crate::richtlinie::aktuell().keine_tastatur_hooks"],
    "managers/meeting_detect/mod.rs": ["richtlinie::Vorgang::Prozessabfrage"],
    "managers/llm/app_usage.rs": ["richtlinie::Vorgang::Prozessabfrage"],
    "managers/meetings/search/indexer.rs": ["richtlinie::Vorgang::Prozessabfrage"],
    "managers/meetings/recorder.rs": ["richtlinie::systemton_erlaubt"],
    "audio_toolkit/audio/loopback.rs": ["richtlinie::Vorgang::Systemton"],
    "managers/llm/runtime.rs": ["richtlinie::programm_download_pruefen"],
    "managers/tts/phonon_models.rs": ["richtlinie::programm_download_pruefen"],
    "managers/tts/models.rs": ["richtlinie::programm_download_pruefen"],
    "managers/llm/cli_install.rs": ["richtlinie::programm_download_pruefen"],
    "managers/model/download.rs": ["richtlinie::katalog_download_pruefen", "richtlinie::daten_pruefen"],
    "managers/model.rs": ["richtlinie::daten_verankern", "richtlinie::daten_ordner_pruefen"],
    "managers/model/netz_wege.rs": ["richtlinie::ist_gesperrt"],
    "managers/tts/portable.rs": ["richtlinie::daten_verankern", "richtlinie::daten_ordner_pruefen"],
}


class Vertrag(unittest.TestCase):
    def test_pe_bereinigung_hat_nur_den_handleweg(self):
        text = (SRC / "commands/meeting_diarization.rs").read_text(encoding="utf-8")
        zweig = text.split("if e.downcast_ref::<crate::richtlinie::PeInDaten>()", 1)[1].split("Err(net_msg", 1)[0]
        self.assertNotIn("anchor.remove(", zweig)
        self.assertIn("safe::remove_handle(&handle)", zweig)

    def test_delete_recht_erst_im_loeschhelfer(self):
        anker = (SRC / "managers/model/download/anchor.rs").read_text(encoding="utf-8")
        pruefung = anker.split("pub(crate) fn check_data(", 1)[1].split("/// Oeffnet `root`", 1)[0]
        self.assertNotIn("| DELETE.0", pruefung)
        safe = (SRC / "managers/model/download/safe.rs").read_text(encoding="utf-8")
        helfer = safe.split("pub(crate) fn remove_handle(file:", 1)[1].split("#[cfg(not(windows))]", 1)[0]
        self.assertIn("ReOpenFile(", helfer)
        self.assertLess(helfer.index("ReOpenFile("), helfer.index("SetFileInformationByHandle("))

    def test_stimmenimport_veroeffentlicht_erst_nach_pruefung(self):
        text = (SRC / "managers/tts/portable.rs").read_text(encoding="utf-8")
        self.assertIn("fn import_checked(", text)
        kern = text.split("fn import_checked(", 1)[1].split("/// Die bekannten Dateien", 1)[0]
        self.assertLess(kern.index("daten_ordner_pruefen("), kern.index("std::fs::rename("))
        self.assertIn("cleanup_imports(", kern)
        self.assertNotIn("remove_dir_all", kern)

    def test_review_b_sichtbarkeit_und_fehlerweitergabe(self):
        stellen = [
            ("richtlinie/mod.rs", "pub(crate) fn streng()"),
            ("richtlinie/wirkung.rs", "Err(e) => Err(e),"),
            ("ablage/befehle.rs", "map_err(semantik_download_fehler)"),
        ]
        for datei, stelle in stellen:
            with self.subTest(datei=datei):
                text = (SRC / datei).read_text(encoding="utf-8")
                self.assertTrue(stelle in text, datei)
                self.assertFalse(stelle in text.replace(stelle, "ENTFERNT"), datei)

    def test_wiederaufnahme_und_plattformwege(self):
        text = (SRC / "shortcut/handy_keys.rs").read_text(encoding="utf-8")
        resume = text.split("if manager.is_none()", 1)[1]
        self.assertLess(resume.index("require_hook_opt_in(&app)"), resume.index("HotkeyManager::new_with_blocking"))
        register = text.split("pub fn register_shortcut(", 1)[1]
        self.assertTrue("require_hook_opt_in(app)?;" in register.split("state.register", 1)[0])
        loopback = (SRC / "audio_toolkit/audio/loopback.rs").read_text(encoding="utf-8")
        for zweig in loopback.split('Err(anyhow::anyhow!("loopback capture is windows-only in M8"))')[:2]:
            self.assertTrue("systemton_starten_mit(" in zweig)

    def test_cli_oberflaeche_reicht_eigene_fehler_variante_durch(self):
        text = (SRC.parents[1] / "src/components/settings/post-processing/LlmConnectionsSettings.tsx").read_text(encoding="utf-8")
        for stelle in ['installFailure.code === "firmenrichtlinie"', 'result.error.code === "firmenrichtlinie"']:
            self.assertTrue(stelle in text, "Richtlinienvariante muss vor Uebersetzung abgefangen werden")
            self.assertFalse(stelle in text.replace(stelle, "ENTFERNT"))

    def test_fixrunde_schreibschutz_und_fehlerwege(self):
        stellen = [
            ("richtlinie/wirkung.rs", 'format!("richtlinie:{schalter}: {meldung}")'),
            ("settings.rs", "richtlinie::rohwerte_beibehalten"),
            ("settings.rs", "!_richtlinie.keine_tastatur_hooks"),
            ("lib.rs", "if richtlinie::autostart_entfernen(app.handle()).is_err()"),
            ("managers/tts/models.rs", "richtlinie::piper_stimme_pruefen"),
            ("managers/tts/phonon_models.rs", "richtlinie::fehlermeldung(err)"),
            ("commands/models.rs", "richtlinie::fehlermeldung(fehler)"),
            ("commands/meeting_diarization.rs", "richtlinie::fehlermeldung(e)"),
        ]
        for datei, stelle in stellen:
            with self.subTest(datei=datei):
                text = (SRC / datei).read_text(encoding="utf-8")
                self.assertTrue(stelle in text, f"{datei}: Wirkstelle fehlt")
                self.assertFalse(stelle in text.replace(stelle, "ENTFERNT"))

    def test_rohwerte_sind_der_gemeinsame_serialisierungsabschluss(self):
        text = (SRC / "settings.rs").read_text(encoding="utf-8")
        kern = text.split("fn to_store_value_mit(", 1)[1].split("\n}", 1)[0]
        self.assertLess(kern.index("store_value_with"), kern.index("rohwerte_beibehalten"))
        for start in ["pub(crate) fn laden(", "pub(crate) fn schreiben_in(", "pub fn try_write_settings("]:
            weg = text.split(start, 1)[1].split("\n}", 1)[0]
            self.assertTrue("to_store_value(" in weg, start)
            self.assertFalse("to_store_value(" in weg.replace("to_store_value(", "ENTFERNT("))
        wirkung = (SRC / "richtlinie/wirkung.rs").read_text(encoding="utf-8")
        self.assertFalse("richtlinie_gesperrt:" in wirkung)
        for schalter in ["keine_programm_downloads", "kein_selbstupdate", "kein_autostart",
                         "keine_tastatur_hooks", "keine_besprechungserkennung", "kein_systemton"]:
            self.assertTrue(f'"{schalter}"' in wirkung)

    def test_wirkstellen_und_gegenprobe(self):
        for datei, sperren in STELLEN.items():
            text = (SRC / datei).read_text(encoding="utf-8")
            for sperre in sperren:
                with self.subTest(datei=datei, sperre=sperre):
                    self.assertTrue(sperre in text, "Wirkstelle fehlt")
                    self.assertFalse(sperre in text.replace(sperre, "ENTFERNT"))

    def test_sperren_liegen_vor_nebenwirkungen(self):
        stellen = [
            ("managers/llm/runtime.rs", "async fn download_runtime(", "programm_download_pruefen", ".run_download("),
            ("managers/tts/models.rs", "async fn download_runtime(", "programm_download_pruefen", ".run_download("),
            ("managers/tts/phonon_models.rs", "async fn install_runtime_entry(", "programm_download_pruefen", "ModelManager::download_http"),
            ("managers/llm/cli_install.rs", "pub async fn download(", "programm_download_pruefen", "let source = Http"),
            ("managers/meeting_detect/mod.rs", "pub fn start(", "start_mit(", "start_freigegeben(app)"),
            ("managers/meeting_detect/mod.rs", "fn start_mit(", "Vorgang::Prozessabfrage", "starten();"),
            ("managers/meeting_detect/mod.rs", "pub fn run_cli(", "Vorgang::Prozessabfrage", "LiveProcesses::new"),
            ("audio_toolkit/audio/loopback.rs", "fn start_inner(", "systemton_starten_mit(", "Self::start_freigegeben("),
            ("audio_toolkit/audio/loopback.rs", "fn systemton_starten_mit<T>(", "Vorgang::Systemton", "starten()"),
            ("shortcut/handy_keys.rs", "pub fn new(app:", "require_hook_opt_in(&app)?", "thread::spawn"),
            ("shortcut/handy_keys.rs", "pub fn start_recording(", "require_hook_opt_in(app)?", "KeyboardListener::new"),
            ("richtlinie/wirkung.rs", "fn extend_api(", "plugin_aufrufen(", "self.0.extend_api"),
            ("richtlinie/wirkung.rs", "fn plugin_aufrufen<T>(", "plugin_pruefen", "weiter(anfrage)"),
        ]
        for datei, start, sperre, wirkung in stellen:
            with self.subTest(datei=datei, start=start):
                text = (SRC / datei).read_text(encoding="utf-8").split(start, 1)[1]

                def korrekt(quelltext):
                    return sperre in quelltext and quelltext.index(sperre) < quelltext.index(wirkung)

                self.assertTrue(korrekt(text), "Sperre muss vor Nebenwirkung stehen")
                self.assertFalse(korrekt(text.replace(sperre, "ENTFERNT")))
                self.assertFalse(korrekt(wirkung + text), "Gegenprobe: Nebenwirkung vor Sperre")

    def test_durchgesetzt_und_felder_stimmen_ueberein(self):
        text = (SRC / "richtlinie/mod.rs").read_text(encoding="utf-8")
        namen = re.findall(r'"([a-z_]+)"', text.split("pub const DURCHGESETZT:", 1)[1].split("];", 1)[0])
        felder = re.findall(r"pub ([a-z_]+): bool", text)
        self.assertEqual(sorted(namen), sorted(felder))
        self.assertEqual(len(namen), 6)
        self.assertNotEqual(sorted(namen[:-1]), sorted(felder))

    def test_neue_dateien_und_wirkstellen_haben_lf(self):
        for datei in [*STELLEN, "richtlinie/mod.rs", "richtlinie/tests.rs", "richtlinie/wirkung.rs",
                      "commands/models.rs", "managers/tts/phonon_models/tests.rs",
                      "managers/model/download/anchor.rs", "managers/model/download/anchor/tests.rs",
                      "managers/model/download/safe.rs", "managers/model/download/safe/tests.rs"]:
            self.assertFalse(b"\r" in (SRC / datei).read_bytes(), datei)
        app = SRC.parents[1]
        for datei in ["AGENTS.md", "src/bindings.ts", "src-tauri/portable/LIESMICH-FIRMENRECHNER.txt",
                      "scripts/test_richtlinie_contract.py", "src-tauri/src/commands/llm.rs",
                      "src-tauri/src/commands/meeting_diarization.rs",
                      "src/components/settings/post-processing/LlmConnectionsSettings.tsx"]:
            self.assertFalse(b"\r" in (app / datei).read_bytes(), datei)


if __name__ == "__main__":
    unittest.main()
