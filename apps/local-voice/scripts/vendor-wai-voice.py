#!/usr/bin/env python3
"""@wai/voice aus dem WAI-SDK in Local Voice AI übernehmen (Vendoring, Muster wie wai-rules in der Poststelle).

Kopiert die Nicht-Test-Quellen (`packages/voice/src/*.ts`, ohne `*.test.ts`) eines festen Commits aus einem lokalen
wai-sdk-Checkout per `git archive` nach `src/vendor/wai-voice/`. Jede Datei bekommt einen Kopfkommentar mit Quelle und
Commit; dazu entsteht `VENDOR.toml`. Die vendorten Dateien werden nie von Hand geändert; Anpassungen gehören ins SDK.
Tests bleiben im SDK.

    python3 scripts/vendor-wai-voice.py [<wai-sdk-pfad>] [--commit <rev>]   übernehmen (Standard: origin/main)
    python3 scripts/vendor-wai-voice.py [<wai-sdk-pfad>] --check            Ordner gegen den gepinnten Commit prüfen

Der SDK-Pfad kommt aus dem Argument oder der Umgebungsvariablen WAI_SDK. `--check` liest den Commit aus
`VENDOR.toml` und endet mit Exit 1, wenn der Ordner vom Inhalt dieses Commits abweicht.
"""

from __future__ import annotations

import argparse
import datetime as dt
import io
import os
import shutil
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path

APP = Path(__file__).resolve().parent.parent
ZIEL = APP / "src" / "vendor" / "wai-voice"
QUELLE = "packages/voice/src"
VENDOR_DATEI = "VENDOR.toml"
STANDARD_REV = "origin/main"
REPO = "https://github.com/MrP42/wai-sdk"


def git(sdk: Path, *argumente: str) -> bytes:
    r = subprocess.run(["git", "-C", str(sdk), *argumente], capture_output=True)
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(argumente)} fehlgeschlagen: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def kopfkommentar(commit: str) -> bytes:
    return (
        f"// Vendort aus {REPO} ({QUELLE}, Commit {commit[:12]}) durch scripts/vendor-wai-voice.py.\n"
        "// Nicht von Hand ändern: Anpassungen gehören ins WAI-SDK, danach neu übernehmen.\n"
    ).encode("utf-8")


def dateien_aus_commit(sdk: Path, commit: str) -> dict[str, bytes]:
    archiv = git(sdk, "archive", "--format=tar", commit, QUELLE)
    dateien: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archiv)) as tar:
        for eintrag in tar:
            rel = eintrag.name[len(QUELLE) + 1 :]
            if not eintrag.isfile() or "/" in rel or not rel.endswith(".ts") or rel.endswith(".test.ts"):
                continue
            daten = tar.extractfile(eintrag)
            assert daten is not None
            # Zeilenenden normalisieren: der Windows-Checkout darf die Kopie nicht verändern.
            inhalt = daten.read().replace(b"\r\n", b"\n")
            dateien[rel] = kopfkommentar(commit) + inhalt
    if not dateien:
        raise SystemExit(f"{QUELLE} ist im Commit {commit} leer")
    return dateien


def dateien_im_ordner() -> dict[str, bytes]:
    if not ZIEL.is_dir():
        return {}
    return {
        p.relative_to(ZIEL).as_posix(): p.read_bytes()
        for p in sorted(ZIEL.rglob("*"))
        if p.is_file() and p.relative_to(ZIEL).as_posix() != VENDOR_DATEI
    }


def sdk_pfad(argument: str | None) -> Path:
    roh = argument or os.environ.get("WAI_SDK")
    if not roh:
        raise SystemExit("wai-sdk-Pfad fehlt: als Argument angeben oder WAI_SDK setzen")
    pfad = Path(roh).expanduser().resolve()
    if not (pfad / ".git").exists():
        raise SystemExit(f"{pfad} ist kein git-Checkout")
    return pfad


def uebernehmen(sdk: Path, rev: str) -> int:
    commit = git(sdk, "rev-parse", "--verify", f"{rev}^{{commit}}").decode().strip()
    dateien = dateien_aus_commit(sdk, commit)
    if ZIEL.exists():
        shutil.rmtree(ZIEL)
    for rel, daten in dateien.items():
        pfad = ZIEL / rel
        pfad.parent.mkdir(parents=True, exist_ok=True)
        pfad.write_bytes(daten)
    datum = git(sdk, "show", "-s", "--format=%cs", commit).decode().strip()
    (ZIEL / VENDOR_DATEI).write_text(
        "# Vendorte Kopie von @wai/voice (WAI-SDK). Nicht von Hand ändern: scripts/vendor-wai-voice.py\n"
        'paket = "@wai/voice"\n'
        f'quelle = "{REPO}"\n'
        f'pfad = "{QUELLE}"\n'
        f'referenz = "{rev}"\n'
        f'commit = "{commit}"\n'
        f'commit_datum = "{datum}"\n'
        f'uebernommen_am = "{dt.date.today().isoformat()}"\n'
        'ausgelassen = ["*.test.ts"]\n',
        encoding="utf-8",
        newline="\n",
    )
    print(f"@wai/voice {commit[:7]} ({datum}) übernommen: {len(dateien)} Dateien nach {ZIEL.relative_to(APP).as_posix()}")
    return 0


def pruefen(sdk: Path) -> int:
    vendor = ZIEL / VENDOR_DATEI
    if not vendor.is_file():
        print(f"{vendor.relative_to(APP).as_posix()} fehlt", file=sys.stderr)
        return 1
    commit = tomllib.loads(vendor.read_text(encoding="utf-8"))["commit"]
    soll = dateien_aus_commit(sdk, commit)
    ist = dateien_im_ordner()
    abweichungen = sorted(
        [f"fehlt: {p}" for p in soll.keys() - ist.keys()]
        + [f"zusätzlich: {p}" for p in ist.keys() - soll.keys()]
        + [f"geändert: {p}" for p in soll.keys() & ist.keys() if soll[p] != ist[p]]
    )
    if abweichungen:
        print(f"@wai/voice weicht vom Commit {commit[:7]} ab:", file=sys.stderr)
        for a in abweichungen:
            print(f"  {a}", file=sys.stderr)
        return 1
    print(f"@wai/voice entspricht Commit {commit[:7]} ({len(ist)} Dateien)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sdk", nargs="?", help="Pfad zum wai-sdk-Checkout (sonst WAI_SDK)")
    parser.add_argument("--commit", default=STANDARD_REV, help=f"Commit oder Zweig (Standard: {STANDARD_REV})")
    parser.add_argument("--check", action="store_true", help="nur prüfen, Exit 1 bei Abweichung")
    a = parser.parse_args()
    sdk = sdk_pfad(a.sdk)
    return pruefen(sdk) if a.check else uebernehmen(sdk, a.commit)


if __name__ == "__main__":
    sys.exit(main())
