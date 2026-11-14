"""Tests für vendor-wai-voice-dienst.py: was aus der `Cargo.toml` der Crate in die vendorte Kopie kommt.

Kein Netz und kein wai-sdk-Checkout nötig. Die Manifest-Tests rufen die reine Funktion `manifest` mit TOML-Text
auf; der Durchlauf über `git` baut sich ein kleines Repository in einem Temp-Ordner. Der Test mit `cargo` wird
übersprungen, wenn `cargo` fehlt.

Aufruf:  python scripts/test_vendor_wai_voice_dienst.py
"""

import contextlib
import importlib.util
import io
import json
import shutil
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

HIER = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("vendor_dienst", HIER / "vendor-wai-voice-dienst.py")
v = importlib.util.module_from_spec(spec)
spec.loader.exec_module(v)

COMMIT = "0123456789abcdef0123456789abcdef01234567"
KOPF = (
    "# Erzeugt von scripts/vendor-wai-voice-dienst.py aus https://github.com/MrP42/wai-sdk "
    "(crates/wai-voice-dienst, Commit 0123456789ab). Nicht von Hand ändern.\n"
)
WORKSPACE = """
[workspace]
members = ["crates/*"]

[workspace.package]
edition = "2024"
rust-version = "1.97"
license = "LicenseRef-WAI-Proprietary"
publish = false

[workspace.dependencies]
sha2 = "0.11"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tempfile = "3"
libc = "0.2"
windows-sys = "0.60"
"""
PAKET = """
[package]
name = "wai-voice-dienst"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish.workspace = true
description = "Vertrag des Sprachdienstes"
"""
# So sieht die Crate im SDK heute aus (ohne Features).
HEUTE = (
    PAKET
    + """
[dependencies]
# Kopplung
ring = "0.17"
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
thiserror.workspace = true
"""
)
HEUTE_ERWARTET = (
    KOPF
    + """[package]
name = "wai-voice-dienst"
version = "0.1.0"
edition = "2024"
rust-version = "1.97"
license = "LicenseRef-WAI-Proprietary"
publish = false
description = "Vertrag des Sprachdienstes"

[lib]
test = false
doctest = false

[dependencies]
ring = "0.17"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
"""
)
# Die Form, die der SDK-Zweig mit dem Verbindungsaufbau schon hat: Features, optionale Zielabhängigkeiten,
# Testabhängigkeiten.
MIT_FEATURES = (
    HEUTE
    + """
[features]
# Verbindungsaufbau
lokal = ["dep:libc", "dep:windows-sys"]
nachbau = []

[target.'cfg(unix)'.dependencies]
libc = { workspace = true, optional = true }

[target.'cfg(windows)'.dependencies]
windows-sys = { workspace = true, optional = true, features = [
    "Win32_System_Pipes",
    "Win32_Foundation",
] }

[target.'cfg(unix)'.dev-dependencies]
libc.workspace = true

[target.'cfg(windows)'.dev-dependencies]
windows-sys = { workspace = true, features = ["Win32_Foundation"] }

[dev-dependencies]
tempfile.workspace = true
"""
)
MIT_FEATURES_ERWARTET = (
    HEUTE_ERWARTET
    + """
[target."cfg(unix)".dependencies]
libc = { version = "0.2", optional = true }

[target."cfg(windows)".dependencies]
windows-sys = { version = "0.60", features = ["Win32_Foundation", "Win32_System_Pipes"], optional = true }

[features]
lokal = ["dep:libc", "dep:windows-sys"]
nachbau = []
"""
)


# Feature-Verweise: (Name, Rest der Cargo.toml, nimmt Cargo das an?). Der Manifest-Test verlangt vom Skript
# dasselbe Urteil, der Cargo-Test legt jede Zeile Cargo selbst vor. Erhoben mit cargo 1.97.
OPT = 'serde = { version = "1", optional = true }\n'
HALB = "[dependencies]\nserde = \"1\"\n[target.'cfg(unix)'.dependencies]\n" + OPT + "[features]\n"
FEST = '[dependencies]\nserde = "1"\n[features]\n'
FREI = "[dependencies]\n" + OPT + "[features]\n"
OHNE = "[dependencies]\n[features]\n"
UMBENANNT = '[dependencies]\ns = { package = "serde", version = "1", optional = true }\n[features]\n'
FEATURE_FAELLE = [
    ("optional: dep:", FREI + 'a = ["dep:serde"]\n', True),
    ("optional: schwacher Verweis", FREI + 'a = ["serde?/rc"]\n', True),
    ("optional: implizites Feature", FREI + 'a = ["serde"]\n', True),
    ("optional: von keinem Feature genannt", FREI + "a = []\n", True),
    ("in einer von zwei Tabellen optional: dep:", HALB + 'a = ["dep:serde"]\n', True),
    ("in einer von zwei Tabellen optional: schwacher Verweis", HALB + 'a = ["serde?/rc"]\n', True),
    ("in einer von zwei Tabellen optional: implizites Feature", HALB + 'a = ["serde"]\n', True),
    ("nicht optional: dep:", FEST + 'a = ["dep:serde"]\n', False),
    ("nicht optional: schwacher Verweis", FEST + 'a = ["serde?/rc"]\n', False),
    ("nicht optional: bloßer Name", FEST + 'a = ["serde"]\n', False),
    ("nicht optional: Feature der Abhängigkeit", FEST + 'a = ["serde/rc"]\n', True),
    ("nicht optional: Feature gleichen Namens", FEST + "serde = []\n", True),
    ("dep: unterdrückt das implizite Feature", FREI + 'a = ["dep:serde"]\nb = ["serde"]\n', False),
    ("dep: unterdrückt es auch im selben Feature", FREI + 'a = ["dep:serde", "serde"]\n', False),
    ("eigenes Feature gleichen Namens mit dep:", FREI + 'serde = ["dep:serde"]\nb = ["serde"]\n', True),
    ("eigenes Feature gleichen Namens ohne dep:", FREI + "serde = []\n", False),
    ("eigenes Feature gleichen Namens, dep: in einem anderen", FREI + 'serde = []\nb = ["dep:serde"]\n', True),
    ("Feature der Abhängigkeit trotz dep: anderswo", FREI + 'a = ["dep:serde"]\nb = ["serde/rc"]\n', True),
    ("schwacher Verweis trotz dep: anderswo", FREI + 'a = ["dep:serde"]\nb = ["serde?/rc"]\n', True),
    ("Feature der Abhängigkeit, die auch ein Feature ist", FREI + 'serde = ["dep:serde"]\na = ["serde/rc"]\n', True),
    ("Feature nennt ein anderes", OHNE + 'a = ["b"]\nb = []\n', True),
    ("Feature nennt sich selbst", OHNE + 'a = ["a"]\n', True),
    ("unbekannter Name", OHNE + 'a = ["fehlt"]\n', False),
    ("leerer Eintrag", OHNE + 'a = [""]\n', False),
    ("Feature einer unbekannten Abhängigkeit", OHNE + 'a = ["fehlt/x"]\n', False),
    ("dep: auf Unbekanntes", OHNE + 'a = ["dep:fehlt"]\n', False),
    ("dep: und / zugleich", FREI + 'a = ["dep:serde/rc"]\n', False),
    ("Featurename mit /", OHNE + '"a/b" = []\n', False),
    ("Featurename mit dep:", OHNE + '"dep:a" = []\n', False),
    ("umbenannt: dep: nennt den Schlüssel", UMBENANNT + 'a = ["dep:s"]\n', True),
    ("umbenannt: dep: nennt den Paketnamen", UMBENANNT + 'a = ["dep:serde"]\n', False),
    # Ein Feature, das wie eine optionale Abhängigkeit heißt, verdeckt deren implizites Feature. Cargo verlangt
    # dann, dass irgendein Feature die Abhängigkeit nennt: mit dep:, mit / oder mit ?/.
    ("gleichnamiges Feature nennt die Abhängigkeit mit /", FREI + 'serde = ["serde/rc"]\n', True),
    ("gleichnamiges Feature nennt die Abhängigkeit mit ?/", FREI + 'serde = ["serde?/rc"]\n', True),
    ("gleichnamiges Feature leer, ein anderes nennt sie mit /", FREI + 'serde = []\na = ["serde/rc"]\n', True),
    ("gleichnamiges Feature leer, ein anderes nennt sie mit ?/", FREI + 'serde = []\na = ["serde?/rc"]\n', True),
    ("gleichnamiges Feature leer, ein anderes nennt nur den Namen", FREI + 'serde = []\na = ["serde"]\n', False),
    ("gleichnamiges Feature nennt nur sich selbst", FREI + 'serde = ["serde"]\n', False),
    ("in einer von zwei Tabellen optional: gleichnamiges Feature mit /", HALB + 'serde = ["serde/rc"]\n', True),
    ("in einer von zwei Tabellen optional: gleichnamiges Feature leer", HALB + "serde = []\n", False),
    ("mehrere Schrägstriche", FREI + 'a = ["serde/rc/extra"]\n', False),
    ("mehrere Schrägstriche, schwacher Verweis", FREI + 'a = ["serde?/rc/extra"]\n', False),
    ("mehrere Schrägstriche, nicht optional", FEST + 'a = ["serde/rc/extra"]\n', False),
    ("zwei Fragezeichen", FREI + 'a = ["serde??/rc"]\n', False),
    ("nichts vor dem Schrägstrich", FREI + 'a = ["/rc"]\n', False),
    ("nichts nach dem Schrägstrich", FREI + 'a = ["serde/"]\n', True),
    ("Featurename leer", OHNE + '"" = []\n', False),
    ("Featurename mit Leerzeichen", OHNE + '"a b" = []\n', False),
    ("Featurename mit Doppelpunkt", OHNE + '"a:b" = []\n', False),
    ("Featurename mit Fragezeichen", OHNE + '"a?" = []\n', False),
    ("Featurename beginnt mit -", OHNE + '"-a" = []\n', False),
    ("Featurename beginnt mit +", OHNE + '"+a" = []\n', False),
    ("Featurename beginnt mit .", OHNE + '".a" = []\n', False),
    ("Featurename beginnt mit Ziffer", OHNE + '"1a" = []\n', True),
    ("Featurename mit + und .", OHNE + '"c++" = []\n"a.b-c" = []\n', True),
    ("Featurename mit Umlaut", OHNE + '"grün" = []\n', True),
    ("Featurename _", OHNE + "_ = []\n", True),
]


def erzeuge(paket: str, workspace: str = WORKSPACE) -> str:
    return v.manifest(tomllib.loads(paket), tomllib.loads(workspace)["workspace"], COMMIT)


@contextlib.contextmanager
def erlaubt(*namen: str):
    """Für einen Test so tun, als wären diese Abhängigkeiten schon geprüft und freigegeben."""
    vorher = set(v.ERLAUBT)
    v.ERLAUBT |= set(namen)
    try:
        yield
    finally:
        v.ERLAUBT.clear()
        v.ERLAUBT |= vorher


class ManifestTest(unittest.TestCase):
    def abbruch(self, paket: str, *teile: str, workspace: str = WORKSPACE) -> str:
        with self.assertRaises(SystemExit) as c:
            erzeuge(paket, workspace)
        text = str(c.exception.code)
        for teil in teile:
            self.assertIn(teil, text)
        return text

    # ---- der heutige Stand bleibt Byte für Byte

    def test_heutige_crate_ergibt_dieselbe_datei_wie_bisher(self):
        self.assertEqual(erzeuge(HEUTE), HEUTE_ERWARTET)

    def test_die_eingecheckte_kopie_hat_genau_diese_form(self):
        # Die vendorte Datei im Repo unterscheidet sich vom erwarteten Text nur in Commit und Beschreibung.
        ist = (v.ZIEL / "Cargo.toml").read_text(encoding="utf-8").split("\n")
        soll = HEUTE_ERWARTET.split("\n")
        self.assertEqual(len(ist), len(soll))
        anders = [i for i, (a, b) in enumerate(zip(ist, soll)) if a != b]
        self.assertEqual([soll[i].split(" ")[0] for i in anders], ["#", "description"])

    # ---- Features

    def test_features_und_zieltabellen_werden_uebernommen(self):
        with erlaubt("libc", "windows-sys"):
            text = erzeuge(MIT_FEATURES)
        self.assertEqual(text, MIT_FEATURES_ERWARTET)
        gelesen = tomllib.loads(text)
        self.assertEqual(gelesen["features"], {"lokal": ["dep:libc", "dep:windows-sys"], "nachbau": []})
        self.assertEqual(gelesen["target"]["cfg(unix)"]["dependencies"], {"libc": {"version": "0.2", "optional": True}})
        self.assertEqual(sorted(gelesen), ["dependencies", "features", "lib", "package", "target"])

    def test_ohne_freigabe_scheitert_die_form_mit_features_an_der_ersten_neuen_abhaengigkeit(self):
        self.abbruch(MIT_FEATURES, "Neue Abhängigkeit 'libc'", "ERLAUBT")
        with erlaubt("libc"):
            self.abbruch(MIT_FEATURES, "Neue Abhängigkeit 'windows-sys'")

    def test_features_sind_sortiert_und_die_ausgabe_haengt_nicht_an_der_reihenfolge_der_quelle(self):
        a = HEUTE + '\n[features]\nz = ["b", "a"]\nb = []\na = ["serde/rc", "b"]\ndefault = ["z"]\n'
        b = '[features]\ndefault = ["z"]\na = ["b", "serde/rc"]\nb = []\nz = ["a", "b"]\n' + HEUTE
        text = erzeuge(a)
        self.assertEqual(text, erzeuge(b))
        self.assertTrue(text.endswith('\n[features]\na = ["b", "serde/rc"]\nb = []\ndefault = ["z"]\nz = ["a", "b"]\n'))

    def test_feature_mit_namen_der_anfuehrungszeichen_braucht(self):
        text = erzeuge(HEUTE + '\n[features]\n"c++" = []\n')
        self.assertIn('\n"c++" = []\n', text)
        self.assertEqual(tomllib.loads(text)["features"], {"c++": []})

    def test_leere_tabelle_features_schreibt_keinen_abschnitt(self):
        self.assertEqual(erzeuge(HEUTE + "\n[features]\n"), HEUTE_ERWARTET)

    def test_feature_verweise_urteilt_wie_in_der_tabelle(self):
        for name, fragment, gueltig in FEATURE_FAELLE:
            with self.subTest(name):
                if gueltig:
                    tomllib.loads(erzeuge(PAKET + fragment))
                else:
                    self.abbruch(PAKET + fragment, "crates/wai-voice-dienst/Cargo.toml")

    def test_feature_verweise_nennen_feature_und_ursache(self):
        fall = lambda eintrag: HEUTE + f'\n[features]\nx = ["{eintrag}"]\n'  # noqa: E731
        self.abbruch(fall("dep:serde"), "Feature 'x'", "'dep:serde'", "keine optionale Abhängigkeit")
        self.abbruch(fall("dep:fehlt"), "Feature 'x'", "'fehlt' ist keine optionale Abhängigkeit")
        self.abbruch(fall("fehlt/std"), "Feature 'x'", "'fehlt/std'")
        self.abbruch(fall("serde?/rc"), "Feature 'x'", "'serde?/rc'")
        self.abbruch(fall("fehlt"), "Feature 'x'", "'fehlt'", "weder ein Feature noch eine optionale Abhängigkeit")
        self.abbruch(fall("serde"), "Feature 'x'", "'serde'")
        self.abbruch(HEUTE + '\n[features]\nx = "serde"\n', "Feature 'x' ist keine Liste")
        self.abbruch(HEUTE + "\n[features]\nx = [1]\n", "Feature 'x' ist keine Liste")
        optional = PAKET + '[dependencies]\nserde = { version = "1", optional = true }\n[features]\n'
        self.abbruch(optional + 'a = ["dep:serde"]\nb = ["serde"]\n', "Feature 'b'", "'serde'", "'dep:serde'")
        self.abbruch(optional + "serde = []\n", "optionale Abhängigkeit 'serde'", "'dep:serde'", "'serde/…'", "'serde?/…'")
        self.abbruch(optional + 'a = ["serde/rc/extra"]\n', "Feature 'a'", "'serde/rc/extra'", "mehr als einem Schrägstrich")
        self.abbruch(optional + '"a b" = []\n', "Feature 'a b'", "Namen, den Cargo nicht annimmt")
        self.abbruch(optional + 'a = ["dep:serde/rc"]\n', "Feature 'a'", "'dep:serde/rc'", "`dep:` und `/` zugleich")

    def test_testabhaengigkeiten_zaehlen_fuer_features_nicht(self):
        # Cargo nimmt das an; die Kopie enthält die Testabhängigkeit aber nicht, also wäre der Verweis dort leer.
        quelle = PAKET + '[dependencies]\n[dev-dependencies]\nserde = "1"\n[features]\na = ["serde/rc"]\n'
        self.abbruch(quelle, "Feature 'a'", "'serde/rc'")

    def test_feature_das_eine_nicht_erlaubte_abhaengigkeit_einschaltet_scheitert_an_erlaubt(self):
        quelle = HEUTE + '\nfremd = { version = "1", optional = true }\n\n[features]\nx = ["dep:fremd"]\n'
        self.abbruch(quelle, "Neue Abhängigkeit 'fremd'", "Lizenz")
        ziel = HEUTE + "\n[features]\nx = [\"dep:fremd\"]\n[target.'cfg(unix)'.dependencies]\nfremd = { version = \"1\", optional = true }\n"
        self.abbruch(ziel, "Neue Abhängigkeit 'fremd'")
        # Nur genannt, nirgends erklärt: Das fällt beim Feature auf.
        self.abbruch(HEUTE + '\n[features]\nx = ["dep:fremd"]\n', "Feature 'x'", "'fremd'")

    # ---- Abhängigkeiten

    def test_optional_bleibt_erhalten(self):
        paket = PAKET + (
            '[dependencies]\nserde = { workspace = true, optional = true, features = ["rc"] }\n'
            'ring = { version = "0.17", optional = true, default-features = false }\n'
            "sha2 = { workspace = true, optional = true }\n"
        )
        self.assertEqual(
            tomllib.loads(erzeuge(paket))["dependencies"],
            {
                "ring": {"version": "0.17", "optional": True, "default-features": False},
                "serde": {"version": "1", "features": ["derive", "rc"], "optional": True},
                "sha2": {"version": "0.10", "optional": True},
            },
        )

    def test_felder_und_features_einer_abhaengigkeit_haengen_nicht_an_der_schreibweise_der_quelle(self):
        a = PAKET + '[dependencies]\nserde = { optional = true, features = ["rc", "derive", "rc"], default-features = false, version = "1" }\n'
        b = PAKET + '[dependencies]\nserde = { version = "1", default-features = false, features = ["derive", "rc"], optional = true }\n'
        c = PAKET + '[dependencies]\nserde = { workspace = true, optional = true, default-features = false, features = ["rc"] }\n'
        zeile = 'serde = { version = "1", features = ["derive", "rc"], default-features = false, optional = true }'
        for quelle in (a, b, c):
            self.assertIn(f"\n[dependencies]\n{zeile}\n", erzeuge(quelle))
        alias = erzeuge(PAKET + '[dependencies]\nhash = { optional = true, package = "sha2", version = "0.11" }\n')
        self.assertIn('\nhash = { version = "0.10", package = "sha2", optional = true }\n', alias)
        self.abbruch(PAKET + '[dependencies]\nserde = { version = "1", features = "derive" }\n', "'serde'", "features")

    def test_abhaengigkeiten_sind_nach_namen_sortiert(self):
        paket = PAKET + '[dependencies]\nthiserror = "2"\nring = "0.17"\nserde_json = "1"\n'
        self.assertIn('\n[dependencies]\nring = "0.17"\nserde_json = "1"\nthiserror = "2"\n', erzeuge(paket))

    def test_zieltabellen_sind_sortiert_und_schluessel_mit_anfuehrungszeichen_bleiben_lesbar(self):
        paket = PAKET + (
            "[target.x86_64-pc-windows-msvc.dependencies]\nserde_json.workspace = true\n"
            "[target.'cfg(target_os = \"linux\")'.dependencies]\nsha2.workspace = true\nring = \"0.17\"\n"
            "[target.'cfg(unix)'.dependencies]\n"
        )
        text = erzeuge(paket)
        self.assertTrue(
            text.endswith(
                '\n[dependencies]\n\n[target."cfg(target_os = \\"linux\\")".dependencies]\nring = "0.17"\nsha2 = "0.10"\n'
                '\n[target.x86_64-pc-windows-msvc.dependencies]\nserde_json = "1"\n'
            ),
            text,
        )
        self.assertEqual(
            tomllib.loads(text)["target"],
            {
                'cfg(target_os = "linux")': {"dependencies": {"ring": "0.17", "sha2": "0.10"}},
                "x86_64-pc-windows-msvc": {"dependencies": {"serde_json": "1"}},
            },
        )

    def test_testabhaengigkeiten_und_lints_entfallen_ohne_pruefung(self):
        paket = HEUTE + (
            '\n[dev-dependencies]\ntempfile.workspace = true\nnirgends-erlaubt = "9"\n'
            "[target.'cfg(unix)'.dev-dependencies]\nauch-nicht-erlaubt = \"1\"\n"
            '[lints.rust]\nunsafe_code = "forbid"\n'
        )
        self.assertEqual(erzeuge(paket), HEUTE_ERWARTET)

    def test_neue_abhaengigkeit_scheitert_auch_in_einer_zieltabelle(self):
        self.abbruch(HEUTE + '\nfremd = "1"\n', "Neue Abhängigkeit 'fremd'")
        self.abbruch(HEUTE + "\n[target.'cfg(windows)'.dependencies]\nfremd = \"1\"\n", "Neue Abhängigkeit 'fremd'")

    def test_geprueft_wird_der_paketname_nicht_der_schluessel(self):
        self.abbruch(PAKET + '[dependencies]\nserde = { package = "fremd", version = "1" }\n', "Neue Abhängigkeit 'fremd'")
        text = erzeuge(PAKET + '[dependencies]\nhash = { package = "sha2", version = "0.11" }\n')
        self.assertEqual(tomllib.loads(text)["dependencies"], {"hash": {"package": "sha2", "version": "0.10"}})

    def test_andere_quellen_als_crates_io_brechen_ab(self):
        for angabe in ('path = "../x"', 'git = "https://example.org/x"', 'registry = "eigen"'):
            self.abbruch(PAKET + f'[dependencies]\nserde = {{ version = "1", {angabe} }}\n', "'serde'", angabe.split(" ")[0])
        ws = WORKSPACE + 'ring = { path = "crates/ring", version = "0.17" }\n'
        self.abbruch(PAKET + "[dependencies]\nring.workspace = true\n", "'ring'", "path", workspace=ws)
        self.abbruch(PAKET + "[dependencies]\nserde = { optional = true }\n", "'serde'", "keine Version")
        self.abbruch(PAKET + "[dependencies]\nring.workspace = true\n", "'ring'", "Workspace")
        self.abbruch(PAKET + "[dependencies]\nserde = 1\n", "'serde'")

    def test_ausgelassene_tabellen_werden_benannt(self):
        self.assertEqual(v.ausgelassene_tabellen(tomllib.loads(HEUTE)), [])
        quelle = MIT_FEATURES + '\n[lints.rust]\nunsafe_code = "forbid"\n'
        self.assertEqual(
            v.ausgelassene_tabellen(tomllib.loads(quelle)),
            ["[dev-dependencies]", "[lints]", '[target."cfg(unix)".dev-dependencies]', '[target."cfg(windows)".dev-dependencies]'],
        )

    # ---- unbekannte Tabellen und Felder

    def test_unbekannte_tabellen_brechen_mit_namen_ab(self):
        for tabelle, inhalt in (
            ("build-dependencies", 'serde = "1"'),
            ("lib", 'name = "x"'),
            ("[bin]", 'name = "x"'),
            ("[example]", 'name = "x"'),
            ("profile.release", "opt-level = 3"),
            ("patch.crates-io", 'serde = { path = "x" }'),
            ("workspace", "members = []"),
            ("badges", "x = {}"),
            ("neu-in-cargo", "x = 1"),
        ):
            with self.subTest(tabelle):
                text = self.abbruch(HEUTE + f"\n[{tabelle}]\n{inhalt}\n", "wird nicht unterstützt", "still fallen")
                self.assertIn("[" + tabelle.strip("[]").split(".")[0] + "]", text)

    def test_unbekannte_tabelle_unter_einem_ziel_bricht_ab(self):
        text = self.abbruch(HEUTE + "\n[target.'cfg(unix)'.build-dependencies]\nserde = \"1\"\n", "wird nicht unterstützt")
        self.assertIn('[target."cfg(unix)".build-dependencies]', text)
        self.abbruch(HEUTE + "\n[target.'cfg(unix)']\nrunner = \"x\"\n", "runner")

    def test_unbekannte_felder_in_package_brechen_ab(self):
        for feld in ('build = "build.rs"', 'links = "x"', 'readme = "README.md"', "autotests = false", "metadata = { x = 1 }"):
            with self.subTest(feld):
                quelle = PAKET.replace("[package]\n", f"[package]\n{feld}\n")
                self.abbruch(quelle + "[dependencies]\n", "[package] nennt " + feld.split(" ")[0])

    def test_felder_in_package_eigen_oder_vom_workspace(self):
        eigen = PAKET.replace("edition.workspace = true", 'edition = "2021"').replace("license.workspace = true\n", "")
        text = erzeuge(eigen + "[dependencies]\n")
        self.assertIn('\nedition = "2021"\nrust-version = "1.97"\npublish = false\n', text)
        self.abbruch('[package]\nname = "x"\nversion = "1"\n', "[package] edition fehlt")
        self.abbruch(PAKET, "rust-version", workspace='[workspace.package]\nedition = "2024"\n')
        self.abbruch("[dependencies]\n", "[package] fehlt")

    def test_publish_ist_immer_false(self):
        self.assertIn("\npublish = false\n", erzeuge(PAKET.replace("publish.workspace = true", "publish = true")))

    # ---- Schreiben von TOML

    def test_texte_mit_anfuehrungszeichen_und_rueckstrich(self):
        quelle = PAKET.replace('"Vertrag des Sprachdienstes"', "'Pfad C:\\x und \"Wort\"'")
        text = erzeuge(quelle)
        self.assertEqual(tomllib.loads(text)["package"]["description"], 'Pfad C:\\x und "Wort"')
        self.abbruch(PAKET.replace('"Vertrag des Sprachdienstes"', '"a\\nb"'), "Steuerzeichen")


def git(ordner: Path, *argumente: str) -> str:
    fest = ["-c", "user.name=Test", "-c", "user.email=test@example.org", "-c", "commit.gpgsign=false", "-c", "core.autocrlf=false"]
    r = subprocess.run(["git", "-C", str(ordner), *fest, *argumente], capture_output=True, text=True)
    if r.returncode != 0:
        raise AssertionError(f"git {argumente}: {r.stderr}")
    return r.stdout.strip()


class DurchlaufTest(unittest.TestCase):
    """Übernehmen und Prüfen über ein echtes kleines Repository, in einen Temp-Ordner statt in den Baum."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="vendor-dienst-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.sdk = self.tmp / "sdk"
        crate = self.sdk / "crates" / "wai-voice-dienst"
        (crate / "src").mkdir(parents=True)
        (crate / "tests").mkdir()
        (self.sdk / "Cargo.toml").write_text(WORKSPACE, encoding="utf-8", newline="\n")
        (crate / "Cargo.toml").write_text(MIT_FEATURES, encoding="utf-8", newline="\n")
        (crate / "src" / "lib.rs").write_text('#[cfg(feature = "lokal")]\npub mod lokal;\n', encoding="utf-8", newline="\n")
        (crate / "src" / "lokal.rs").write_text("pub fn verbinde() {}\n", encoding="utf-8", newline="\n")
        (crate / "tests" / "t.rs").write_text("#[test]\nfn t() {}\n", encoding="utf-8", newline="\n")
        git(self.sdk, "init", "-q")
        git(self.sdk, "add", "-A")
        git(self.sdk, "commit", "-q", "-m", "fake")
        self.alt = (v.ZIEL, v.WURZEL)
        v.WURZEL = self.tmp / "app"
        v.ZIEL = v.WURZEL / "src-tauri" / "vendor" / "wai-voice-dienst"
        self.addCleanup(self.zurueck)

    def zurueck(self):
        v.ZIEL, v.WURZEL = self.alt

    def still(self, f, *argumente):
        aus, fehler = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(aus), contextlib.redirect_stderr(fehler):
            ergebnis = f(*argumente)
        return ergebnis, aus.getvalue() + fehler.getvalue()

    def test_uebernehmen_schreibt_features_und_pruefen_bemerkt_abweichungen(self):
        with erlaubt("libc", "windows-sys"):
            self.assertEqual(self.still(v.uebernehmen, self.sdk, "HEAD")[0], 0)
            commit = git(self.sdk, "rev-parse", "HEAD")
            self.assertEqual(sorted(p.name for p in v.ZIEL.iterdir()), ["Cargo.toml", "VENDOR.toml", "rustfmt.toml", "src"])
            self.assertEqual(sorted(p.name for p in (v.ZIEL / "src").iterdir()), ["lib.rs", "lokal.rs"])
            erzeugt = (v.ZIEL / "Cargo.toml").read_text(encoding="utf-8")
            self.assertEqual(erzeugt, MIT_FEATURES_ERWARTET.replace(COMMIT[:12], commit[:12]))
            vermerk = tomllib.loads((v.ZIEL / "VENDOR.toml").read_text(encoding="utf-8"))
            self.assertEqual(vermerk["commit"], commit)
            self.assertEqual(
                vermerk["ausgelassen"],
                ["Tests im SDK-Arbeitsbaum", "[dev-dependencies]", '[target."cfg(unix)".dev-dependencies]', '[target."cfg(windows)".dev-dependencies]'],
            )
            self.assertEqual(self.still(v.pruefen, self.sdk)[0], 0)

            # Wer [features] von Hand aus der Kopie nimmt, fällt bei --check auf.
            (v.ZIEL / "Cargo.toml").write_text(erzeugt.split("\n[features]")[0], encoding="utf-8", newline="\n")
            ergebnis, text = self.still(v.pruefen, self.sdk)
            self.assertEqual(ergebnis, 1)
            self.assertIn("geändert: Cargo.toml", text)

    def test_ohne_freigabe_wird_nichts_geschrieben(self):
        with self.assertRaises(SystemExit) as c:
            self.still(v.uebernehmen, self.sdk, "HEAD")
        self.assertIn("Neue Abhängigkeit 'libc'", str(c.exception.code))
        self.assertFalse(v.ZIEL.exists())

    def test_ein_fehler_laesst_die_vorhandene_kopie_stehen(self):
        v.ZIEL.mkdir(parents=True)
        (v.ZIEL / "merker.txt").write_text("alt", encoding="utf-8")
        with self.assertRaises(SystemExit):
            self.still(v.uebernehmen, self.sdk, "HEAD")
        self.assertEqual((v.ZIEL / "merker.txt").read_text(encoding="utf-8"), "alt")

    @unittest.skipUnless(shutil.which("cargo"), "cargo fehlt")
    def test_cargo_liest_die_erzeugte_datei_mit_features_und_zielen(self):
        with erlaubt("libc", "windows-sys"):
            self.still(v.uebernehmen, self.sdk, "HEAD")
        r = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path", str(v.ZIEL / "Cargo.toml")],
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        paket = json.loads(r.stdout)["packages"][0]
        self.assertEqual(paket["features"], {"lokal": ["dep:libc", "dep:windows-sys"], "nachbau": []})
        ziele = {(d["name"], d["target"], d["optional"]) for d in paket["dependencies"] if d["target"]}
        self.assertEqual(ziele, {("libc", "cfg(unix)", True), ("windows-sys", "cfg(windows)", True)})
        self.assertEqual(sorted(d["name"] for d in paket["dependencies"] if d["kind"] == "dev"), [])


@unittest.skipUnless(shutil.which("cargo"), "cargo fehlt")
class CargoVergleichTest(unittest.TestCase):
    """Legt Cargo jede Zeile aus FEATURE_FAELLE vor: Die Tabelle, nach der das Skript urteilt, stimmt mit Cargo."""

    KOPF = '[package]\nname = "probe"\nversion = "0.1.0"\nedition = "2021"\n'

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="vendor-cargo-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        (self.tmp / "src").mkdir()
        (self.tmp / "src" / "lib.rs").write_text("", encoding="utf-8")

    def cargo(self, manifest: str) -> subprocess.CompletedProcess:
        (self.tmp / "Cargo.toml").write_text(manifest, encoding="utf-8", newline="\n")
        return subprocess.run(
            ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1", "--manifest-path", str(self.tmp / "Cargo.toml")],
            capture_output=True,
            text=True,
        )

    def features(self, manifest: str) -> dict[str, list[str]]:
        """Die Features, wie Cargo sie liest, samt der impliziten Features optionaler Abhängigkeiten."""
        r = self.cargo(manifest)
        self.assertEqual(r.returncode, 0, r.stderr)
        return {k: sorted(v) for k, v in json.loads(r.stdout)["packages"][0]["features"].items()}

    def test_cargo_urteilt_ueber_jeden_feature_fall_wie_die_tabelle(self):
        for name, fragment, gueltig in FEATURE_FAELLE:
            with self.subTest(name):
                r = self.cargo(self.KOPF + fragment)
                self.assertEqual(r.returncode == 0, gueltig, r.stderr)

    def test_skript_und_cargo_urteilen_ueber_ein_gitter_von_feature_tabellen_gleich(self):
        # Ohne von Hand gesetztes Urteil: jede Kombination aus Art der Abhängigkeit, einem Feature `a` und einem
        # Feature, das wie die Abhängigkeit heißt. Was die Falltabelle übersieht, fällt hier auf.
        arten = {"optional": FREI, "in einer von zwei Tabellen optional": HALB, "nicht optional": FEST}
        verweise = ["serde", "dep:serde", "serde/rc", "serde?/rc"]
        fuer_a = [[]] + [[e] for e in verweise] + [["dep:serde", "serde"]]
        gleichnamig = [None, []] + [[e] for e in verweise]
        for art, kopf in arten.items():
            for a in fuer_a:
                for g in gleichnamig:
                    fragment = kopf + f"a = {json.dumps(a)}\n" + ("" if g is None else f"serde = {json.dumps(g)}\n")
                    with self.subTest(art=art, a=a, serde=g):
                        try:
                            erzeuge(PAKET + fragment)
                            skript = True
                        except SystemExit:
                            skript = False
                        r = self.cargo(self.KOPF + fragment)
                        self.assertEqual(skript, r.returncode == 0, r.stderr)

    def test_cargo_liest_aus_der_ausgabe_dieselben_features_wie_aus_der_quelle(self):
        for name, fragment, gueltig in FEATURE_FAELLE:
            if not gueltig:
                continue
            with self.subTest(name):
                self.assertEqual(self.features(erzeuge(PAKET + fragment)), self.features(self.KOPF + fragment))


if __name__ == "__main__":
    unittest.main()
