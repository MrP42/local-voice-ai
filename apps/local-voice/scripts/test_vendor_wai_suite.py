"""Offline-Regressions- und Mutationstests für vendor-wai-suite.py (Python 3.11).

Aufruf: python scripts/test_vendor_wai_suite.py
Optional: WAI_SDK auf einen lokalen Checkout mit den gepinnten Git-Objekten setzen.
Der Pintest liest ausschließlich; weder Netzwerk noch vendorte Dateien werden verändert.
"""

import copy
import importlib.util
import io
import os
import shutil
import subprocess
import tomllib
import types
import unittest
from pathlib import Path
from unittest.mock import patch

HIER = Path(__file__).resolve().parent
SKRIPT = HIER / "vendor-wai-suite.py"
spec = importlib.util.spec_from_file_location("vendor_suite", SKRIPT)
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)
COMMIT = "0123456789abcdef0123456789abcdef01234567"
CRATE = "wai-projekte"
PAKET = '''[package]
name = "wai-projekte"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true
description = "Suite"
'''
WORKSPACE = '''[workspace.package]
edition = "2024"
rust-version = "1.97"
license = "LicenseRef-WAI-Proprietary"
publish = false
[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
sha2 = "0.11"
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls"] }
tempfile = "3"
wai-links = { path = "crates/wai-links", version = "0.1.0" }
'''
HEUTE = PAKET + '''[dependencies]
sha2.workspace = true
serde.workspace = true
reqwest.workspace = true
wai-links.workspace = true
'''
HEUTE_ERWARTET = '''# Erzeugt von scripts/vendor-wai-suite.py aus https://github.com/MrP42/wai-sdk (crates/wai-projekte, Commit 0123456789ab). Nicht von Hand ändern.
[package]
name = "wai-projekte"
version = "0.1.0"
edition = "2024"
rust-version = "1.97"
license = "LicenseRef-WAI-Proprietary"
publish = false
description = "Suite"

[lib]
test = false
doctest = false

[dependencies]
sha2 = "0.10"
serde = { version = "1", features = ["derive"] }
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls-no-provider"] }
wai-links = { path = "../wai-links", version = "0.1.0" }
'''


def erzeuge(modul, quelle, crate=CRATE, workspace=WORKSPACE):
    """Auch am alten Skript nutzbar: echte cargo_toml(), nur Git-Lesen ersetzt."""
    def lesen(sdk, *argumente):
        if argumente == ("show", f"{COMMIT}:Cargo.toml"):
            return workspace.encode("utf-8")
        if argumente == ("show", f"{COMMIT}:crates/{crate}/Cargo.toml"):
            return quelle.encode("utf-8")
        raise AssertionError(argumente)

    with patch.object(modul, "git", lesen):
        return modul.cargo_toml(Path("nicht-gelesen"), COMMIT, crate).decode("utf-8")


class ManifestTest(unittest.TestCase):
    modul = v

    def erzeuge(self, quelle, **kwargs):
        return erzeuge(self.modul, quelle, **kwargs)

    def abbruch(self, quelle, *teile, **kwargs):
        with self.assertRaises(SystemExit) as c:
            self.erzeuge(quelle, **kwargs)
        for teil in teile:
            self.assertIn(teil, str(c.exception.code))

    def test_regression_features_fielen_weg(self):
        # Erst rot am bisherigen cargo_toml(): [features] fehlt in der Ausgabe.
        text = self.erzeuge(PAKET + '[features]\nz = ["a"]\na = []\n')
        self.assertEqual(tomllib.loads(text).get("features"), {"a": [], "z": ["a"]})
        self.assertTrue(text.endswith('[features]\na = []\nz = ["a"]\n'))

    def test_regression_zieltabellen_fielen_weg(self):
        text = self.erzeuge(PAKET + '[target.\'cfg(unix)\'.dependencies]\nserde.workspace = true\n')
        self.assertEqual(tomllib.loads(text).get("target"), {
            "cfg(unix)": {"dependencies": {"serde": {"version": "1", "features": ["derive"]}}}
        })

    def test_regression_erlaubt_liess_fremden_paketnamen_durch(self):
        self.abbruch(PAKET + '[dependencies]\nserde = { package = "anderes", version = "1" }\n', "anderes")

    def test_bisherige_bytes_und_sonderregeln_bleiben(self):
        self.assertEqual(self.erzeuge(HEUTE).encode("utf-8"), HEUTE_ERWARTET.encode("utf-8"))

    def test_package_eigene_werte_werden_nicht_durch_workspace_ersetzt(self):
        for feld, wert in (("edition", "2021"), ("rust-version", "1.85"), ("license", "MIT")):
            with self.subTest(feld=feld):
                quelle = PAKET.replace(f'{feld}.workspace = true', f'{feld} = "{wert}"')
                self.assertEqual(tomllib.loads(self.erzeuge(quelle))["package"][feld], wert)
                # Ein eigener Wert braucht keinen entsprechenden Workspace-Eintrag.
                ws = tomllib.loads(WORKSPACE)
                zeile = f'{feld} = "{ws["workspace"]["package"][feld]}"\n'
                self.assertEqual(tomllib.loads(self.erzeuge(
                    quelle, workspace=WORKSPACE.replace(zeile, "")
                ))["package"][feld], wert)
                self.abbruch(PAKET, CRATE, feld, "auflösen",
                             workspace=WORKSPACE.replace(zeile, ""))

    def test_package_pflichtfelder_fehlen_mit_klarer_meldung(self):
        for feld in ("name", "version", "description"):
            with self.subTest(feld=feld):
                quelle = "\n".join(z for z in PAKET.splitlines() if not z.startswith(f"{feld} =")) + "\n"
                self.abbruch(quelle, CRATE, "[package]", feld, "fehlt")

    def test_package_vererbung_und_optionale_felder_wie_vorbild(self):
        ws = WORKSPACE.replace('[workspace.package]\n',
                               '[workspace.package]\nversion = "0.2.0"\ndescription = "Geerbt"\n')
        quelle = PAKET.replace('version = "0.1.0"', 'version.workspace = true').replace(
            'description = "Suite"', 'description.workspace = true')
        paket = tomllib.loads(self.erzeuge(quelle, workspace=ws))["package"]
        self.assertEqual(paket["version"], "0.2.0")
        self.assertEqual(paket["description"], "Geerbt")
        self.abbruch(PAKET.replace('edition.workspace = true\n', ""), CRATE, "edition", "fehlt")
        for feld in ("rust-version", "license"):
            with self.subTest(feld=feld):
                paket = tomllib.loads(self.erzeuge(PAKET.replace(f'{feld}.workspace = true\n', "")))["package"]
                self.assertNotIn(feld, paket)

    def test_unbekanntes_wai_paket_verlangt_crates_erweitern(self):
        for kopf in ("dependencies", "target.'cfg(unix)'.dependencies"):
            for name in ("wai-fremd", "alias"):
                with self.subTest(kopf=kopf, name=name):
                    self.abbruch(PAKET + f'[{kopf}]\n{name} = {{ package = "wai-fremd", path = "../wai-fremd" }}\n',
                                 CRATE, "wai-fremd", "CRATES erweitern")
            self.abbruch(PAKET + f'[{kopf}]\nfremd = "1"\n',
                         "fremd", "erst Lizenz prüfen", "ERLAUBT erweitern")

    def test_gemeinsame_hilfsfunktionen_werden_wiederverwendet(self):
        for name in ("pruefe_features", "toml_wert", "toml_schluessel"):
            self.assertIs(getattr(self.modul, name), getattr(self.modul._dienst, name))

    def test_tabellen_falltabelle(self):
        # Tabellenname, Inhalt, Ausgabe-Tabelle (None: bewusst ausgelassen).
        for tabelle, inhalt, erwartet in (
            ("dependencies", 'serde = "1"', "dependencies"),
            ("features", 'a = []', "features"),
            ("target.'cfg(unix)'.dependencies", 'serde = "1"', "target"),
            ("dev-dependencies", 'fremd = "1"', None),
            ("target.'cfg(unix)'.dev-dependencies", 'fremd = "1"', None),
            ("lints.rust", 'unsafe_code = "forbid"', None),
        ):
            with self.subTest(tabelle=tabelle):
                text = self.erzeuge(PAKET + f"[{tabelle}]\n{inhalt}\n")
                if erwartet is None:
                    self.assertEqual(text, self.erzeuge(PAKET))
                else:
                    self.assertIn(erwartet, tomllib.loads(text))
        for tabelle, inhalt, name in (
            ("build-dependencies", 'serde = "1"', "build-dependencies"),
            ("lib", 'name = "x"', "lib"),
            ("[bin]", 'name = "x"', "bin"),
            ("[example]", 'name = "x"', "example"),
            ("workspace", 'members = []', "workspace"),
            ("profile.release", 'opt-level = 3', "profile"),
            ("patch.crates-io", 'serde = { path = "x" }', "patch"),
            ("neu", 'x = 1', "neu"),
            ("target.'cfg(unix)'.build-dependencies", 'serde = "1"', 'target."cfg(unix)".build-dependencies'),
            ("target.'cfg(unix)'.neu", 'x = 1', 'target."cfg(unix)".neu'),
        ):
            with self.subTest(tabelle=tabelle):
                self.abbruch(PAKET + f"[{tabelle}]\n{inhalt}\n", name)
        self.abbruch(PAKET + '[target.\'cfg(unix)\']\nrunner = "x"\n', "runner")
        self.abbruch(PAKET + '[package.metadata]\nx = 1\n', "package", "metadata")
        self.abbruch(PAKET.replace('edition.workspace = true',
                                  'edition = { workspace = true, neu = { x = 1 } }'), "package.edition")

    def test_dev_nur_fuer_mit_tests_und_nach_paketnamen(self):
        for kopf in ("dev-dependencies", "target.'cfg(unix)'.dev-dependencies"):
            with self.subTest(kopf=kopf):
                quelle = PAKET + f'[{kopf}]\nt = {{ package = "tempfile", version = "3" }}\n'
                text = self.erzeuge(quelle, crate="wai-notizbuch-zugang")
                daten = tomllib.loads(text)
                tabelle = daten if kopf == "dev-dependencies" else daten["target"]["cfg(unix)"]
                self.assertEqual(tabelle["dev-dependencies"]["t"], {"package": "tempfile", "version": "3"})
                self.assertTrue(daten["lib"]["test"])
                self.abbruch(PAKET + f'[{kopf}]\ntempfile = {{ package = "serde", version = "1" }}\n',
                             "serde", crate="wai-notizbuch-zugang")
                self.abbruch(PAKET + f'[{kopf}]\nwai-links = {{ path = "../wai-links" }}\n',
                             "wai-links", crate="wai-notizbuch-zugang")

    def test_abhaengigkeiten_quellen_und_alias(self):
        for kopf in ("dependencies", "target.'cfg(unix)'.dependencies"):
            for quelle in ('path = "../serde"', 'git = "https://example.org/x"', 'registry = "privat"',
                           'registry-index = "https://example.org/index"'):
                with self.subTest(kopf=kopf, quelle=quelle):
                    self.abbruch(PAKET + f'[{kopf}]\nserde = {{ version = "1", {quelle} }}\n',
                                 "serde", quelle.split(" ")[0])
        self.abbruch(PAKET + '[dependencies]\nserde.workspace = true\n', "path",
                     workspace=WORKSPACE.replace('serde = { version = "1",', 'serde = { path = "../serde", version = "1",'))
        self.abbruch(PAKET + '[dependencies]\nserde.workspace = true\n', "anderes",
                     workspace=WORKSPACE.replace('serde = { version = "1",', 'serde = { package = "anderes", version = "1",'))
        for angabe in ('git = "https://example.org/x"', 'registry = "privat"'):
            self.abbruch(PAKET + f'[dependencies]\nwai-links = {{ {angabe} }}\n', angabe.split(" ")[0])
        self.abbruch(PAKET + '[dependencies]\nwai-fremd = { path = "../wai-fremd" }\n', "wai-fremd")
        self.abbruch(PAKET + '[dependencies]\nwai-links = { package = "anderes", path = "../wai-links" }\n', "anderes")
        daten = tomllib.loads(self.erzeuge(PAKET + '[dependencies]\nhash = { package = "sha2", version = "0.11" }\n'))
        self.assertEqual(daten["dependencies"]["hash"], {"package": "sha2", "version": "0.10"})
        daten = tomllib.loads(self.erzeuge(PAKET + '[dependencies]\nlinks = { package = "wai-links", path = "../wai-links", optional = true }\n[features]\nx = ["dep:links"]\n'))
        self.assertEqual(daten["dependencies"]["links"], {
            "path": "../wai-links", "version": "0.1.0", "package": "wai-links", "optional": True
        })

    def test_features_beruecksichtigen_optionale_zielabhaengigkeiten(self):
        basis = PAKET + '''[dependencies]
serde = "1"
[target.'cfg(unix)'.dependencies]
serde = { version = "1", optional = true }
[features]
'''
        for rest, gueltig in (
            ('a = ["dep:serde"]', True), ('a = ["serde?/rc"]', True),
            ('a = ["serde"]', True), ('a = ["serde/rc"]', True),
            ('a = ["dep:serde"]\nb = ["serde"]', False),
            ('serde = []', False), ('serde = ["serde/rc"]', True),
            ('a = ["fehlt"]', False), ('a = ["dep:fehlt"]', False),
            ('a = ["serde/rc/extra"]', False), ('a = ["dep:serde/rc"]', False),
            ('a = "serde"', False), ('a = [1]', False),
            ('"c++" = []\n"a.b-c" = []\n"\U0001e4d0" = []', True),
        ):
            with self.subTest(rest=rest):
                if gueltig:
                    self.assertIn("features", tomllib.loads(self.erzeuge(basis + rest)))
                else:
                    self.abbruch(basis + rest)
        self.abbruch(PAKET + '[dependencies]\nserde = "1"\n[features]\na = ["dep:serde"]\n', "a", "serde")
        self.abbruch(PAKET + '[dev-dependencies]\ntempfile = "3"\n[features]\na = ["tempfile/x"]\n', "a")

    def test_features_sind_sortiert_und_toml_schluessel_escaped(self):
        text = self.erzeuge(PAKET + '''[features]
z = ["b", "a"]
b = []
a = []
"c++" = []
[target.'cfg(target_os = "linux")'.dependencies]
serde = "1"
''')
        self.assertTrue(text.endswith('[features]\na = []\nb = []\n"c++" = []\nz = ["a", "b"]\n'))
        self.assertIn('cfg(target_os = "linux")', tomllib.loads(text)["target"])


class MutationenTest(unittest.TestCase):
    def test_sieben_mutationen_machen_die_verhaltenstests_rot(self):
        quelltext = SKRIPT.read_text(encoding="utf-8")
        mutationen = (
            ("Features weg", 'if features:', 'if False:', 'test_regression_features_fielen_weg'),
            ("Ziele weg", 'for ziel in sorted(ziele):', 'for ziel in []:', 'test_regression_zieltabellen_fielen_weg'),
            ("ERLAUBT nach Schlüssel", 'paketname = basis.get("package", name)', 'paketname = name',
             'test_regression_erlaubt_liess_fremden_paketnamen_durch'),
            ("unbekannte Tabelle still", 'raise SystemExit(f"{crate}: Tabelle [{tabelle}] wird nicht unterstützt")',
             'continue', 'test_tabellen_falltabelle'),
            ("path für Fremdes", 'if "path" in basis:', 'if False:', 'test_abhaengigkeiten_quellen_und_alias'),
            ("package still aus Workspace", 'wert = _dienst.geerbt(p, ws.get("package", {}), feld)',
             'wert = ws.get("package", {}).get(feld) if feld in ("edition", "rust-version", "license") else _dienst.geerbt(p, ws.get("package", {}), feld)',
             'test_package_eigene_werte_werden_nicht_durch_workspace_ersetzt'),
            ("falscher wai-Hinweis", 'CRATES erweitern', 'erst Lizenz prüfen und melden, dann ERLAUBT erweitern',
             'test_unbekanntes_wai_paket_verlangt_crates_erweitern'),
        )
        for name, alt, neu, nachweis in mutationen:
            with self.subTest(mutation=name):
                self.assertEqual(quelltext.count(alt), 1, f"Mutationsstelle verändert: {name}")
                mutant = types.ModuleType("vendor_suite_mutant")
                mutant.__file__ = str(SKRIPT)
                exec(compile(quelltext.replace(alt, neu, 1), str(SKRIPT), "exec"), mutant.__dict__)
                klasse = type("MutiertesManifestTest", (ManifestTest,), {"modul": mutant})
                suite = unittest.defaultTestLoader.loadTestsFromTestCase(klasse)
                protokoll = io.StringIO()
                ergebnis = unittest.TextTestRunner(stream=protokoll).run(suite)
                self.assertFalse(ergebnis.wasSuccessful(), protokoll.getvalue())
                # Zusätzlich ausgelöste SystemExit in anderen Fällen sind möglich; der gezielte
                # Regressionstest muss aber durch eine Assertion scheitern, nicht am Laden.
                self.assertTrue(any(nachweis in test.id() for test, _ in ergebnis.failures), protokoll.getvalue())


@unittest.skipUnless(os.environ.get("WAI_SDK") and shutil.which("git"), "WAI_SDK oder git fehlt; keine Netzabfrage")
class PinsTest(unittest.TestCase):
    def test_heutige_pins_erzeugen_bytegleiche_manifeste(self):
        sdk = Path(os.environ["WAI_SDK"])
        for crate in v.CRATES:
            with self.subTest(crate=crate):
                ordner = v.VENDOR / crate
                pin = tomllib.loads((ordner / "VENDOR.toml").read_text(encoding="utf-8"))["commit"]
                def lesen(*argumente):
                    r = subprocess.run(["git", "-C", str(sdk), *argumente], capture_output=True,
                                       text=True, encoding="utf-8")
                    self.assertEqual(r.returncode, 0, r.stderr)
                    return r.stdout
                quelle = tomllib.loads(lesen("show", f"{pin}:crates/{crate}/Cargo.toml"))
                ws = tomllib.loads(lesen("show", f"{pin}:Cargo.toml"))["workspace"]
                print(f"{crate} {pin[:12]}: features={list(quelle.get('features', {}))}, "
                      f"target={list(quelle.get('target', {}))}")
                ist = (ordner / "Cargo.toml").read_bytes().replace(b"\r\n", b"\n")
                self.assertEqual(v.manifest(copy.deepcopy(quelle), ws, pin, crate).encode("utf-8"), ist)


if __name__ == "__main__":
    unittest.main()
