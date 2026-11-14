"""Tests für die Git-Zeilenenden der vendorten wai-Crates.

Kein Netz und kein wai-sdk-Checkout nötig. Geprüft werden die wirksamen Attribute und der Index;
CRLF oder LF im Arbeitsbaum ändern das Ergebnis nicht. Git und mindestens eine vendorte Datei
sind erforderlich; fehlende Voraussetzungen sind Fehler und werden nicht übersprungen.

Aufruf:  python scripts/test_vendor_zeilenenden.py
"""

import subprocess
import unittest
from pathlib import Path

HIER = Path(__file__).resolve().parent
VENDOR_MUSTER = "apps/local-voice/src-tauri/vendor/wai-*/**"


def git(ort: Path, *argumente: str, eingabe: str | None = None) -> str:
    try:
        ergebnis = subprocess.run(
            ["git", "-c", "core.quotepath=off", *argumente],
            cwd=ort,
            input=eingabe,
            capture_output=True,
            encoding="utf-8",
            check=True,
        )
    except FileNotFoundError as fehler:
        raise AssertionError("Git fehlt: git muss im PATH verfügbar sein.") from fehler
    except subprocess.CalledProcessError as fehler:
        raise AssertionError(
            f"Git-Aufruf fehlgeschlagen ({' '.join(argumente)}):\n{fehler.stderr}"
        ) from fehler
    return ergebnis.stdout


class VendorZeilenendenTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.wurzel = Path(git(HIER, "rev-parse", "--show-toplevel").rstrip("\r\n"))
        cls.dateien = git(
            cls.wurzel, "ls-files", "-z", "--", f":(glob){VENDOR_MUSTER}"
        ).split("\0")[:-1]
        if not cls.dateien:
            raise AssertionError(f"Keine versionierten Dateien unter {VENDOR_MUSTER} gefunden.")

    def test_alle_vendor_dateien_haben_lf_attribut_und_lf_im_index(self):
        attribute = git(
            self.wurzel,
            "check-attr", "-z", "--stdin", "eol",
            eingabe="\0".join(self.dateien) + "\0",
        ).split("\0")[:-1]
        self.assertEqual(len(attribute), 3 * len(self.dateien), "Unvollständige check-attr-Ausgabe")
        eol_attribute = {
            attribute[i]: (attribute[i + 1], attribute[i + 2])
            for i in range(0, len(attribute), 3)
        }
        index_eol = {}
        for zeile in git(
            self.wurzel, "ls-files", "--eol", "-z", "--", f":(glob){VENDOR_MUSTER}"
        ).split("\0")[:-1]:
            spalten, datei = zeile.split("\t", 1)
            # Nur die Index-Spalte auswerten; w/crlf im Arbeitsbaum ist erlaubt.
            index_eol[datei] = spalten.split()[0]

        fehler = []
        for datei in self.dateien:
            gruende = []
            attribut = eol_attribute.get(datei)
            if attribut != ("eol", "lf"):
                gruende.append(f"erwartet eol: lf, erhalten {attribut!r}")
            index = index_eol.get(datei)
            if index not in {"i/lf", "i/none"}:
                gruende.append(f"erwartet i/lf oder i/none, erhalten {index!r}")
            elif index == "i/none":
                # i/none kann auch Text ohne Zeilenumbruch bedeuten: nur leere Blobs zulassen.
                if git(self.wurzel, "cat-file", "-s", f":{datei}").strip() != "0":
                    gruende.append("i/none, aber die Datei im Index ist nicht leer")
            if gruende:
                fehler.append(f"{datei}: {'; '.join(gruende)}")
        if fehler:
            self.fail(
                f"{len(fehler)} Vendor-Dateien mit falschen Zeilenenden/Attributen "
                "(maximal 20 angezeigt):\n" + "\n".join(fehler[:20])
            )

    def test_gitattributes_enthaelt_vendor_regel(self):
        zeilen = (self.wurzel / ".gitattributes").read_text(encoding="utf-8").splitlines()
        self.assertTrue(
            any(
                teile and teile[0] == VENDOR_MUSTER and {"text", "eol=lf"} <= set(teile[1:])
                for teile in (zeile.split() for zeile in zeilen)
            ),
            f".gitattributes: Regel '{VENDOR_MUSTER} text eol=lf' fehlt.",
        )


if __name__ == "__main__":
    unittest.main()
