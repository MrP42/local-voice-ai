#!/usr/bin/env python3
"""wai-voice-dienst aus dem WAI-SDK in Local Voice AI übernehmen (Vendoring, ADR-0021 Paket S2).

Quelle:   https://github.com/MrP42/wai-sdk, Pfad crates/wai-voice-dienst
Commit:   e624d851 (origin/main nach Merge von PR #994: `willkommen` mit
          Gegenbeweis, `hallo.zufall` und Kopplung mit Schluesselaustausch/Vergleichscode). Vorher a624d690, 6ecea3ad und
          249a4db3 (origin/main, #985). Der volle Commit steht in VENDOR.toml.
Ziel:     apps/local-voice/src-tauri/vendor/wai-voice-dienst/

Muster: wai-digitaler-posteingang/scripts/vendor-wai-rules.py. Kopiert `src/` des festen Commits per
`git archive` und erzeugt eine eigenständige `Cargo.toml` ohne Workspace-Vererbung. Die workspace-Abhängigkeiten
werden auf konkrete Versionen aufgelöst, passend zu dem, was Local Voice AI schon im Lock hat (sha2 0.10 statt
0.11 des SDK-Workspace; thiserror 2, serde 1, serde_json 1). Es kommt KEINE neue externe Abhängigkeit dazu (ring steht schon im Lock).
Die vendorten Dateien werden nie von Hand geändert; Anpassungen gehören ins SDK.

    python3 scripts/vendor-wai-voice-dienst.py [<wai-sdk-pfad>] [--commit <rev>]   übernehmen
    python3 scripts/vendor-wai-voice-dienst.py [<wai-sdk-pfad>] --check            Ordner gegen den gepinnten Commit prüfen

Der SDK-Pfad kommt aus dem Argument oder der Umgebungsvariablen WAI_SDK. Gelesen wird nur per `git`, der
Arbeitsbaum des SDK wird nicht angefasst. `--check` liest den Commit aus `VENDOR.toml` und endet mit Exit 1,
wenn der Ordner (samt erzeugter Cargo.toml) vom Inhalt dieses Commits abweicht.
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

WURZEL = Path(__file__).resolve().parent.parent
ZIEL = WURZEL / "src-tauri" / "vendor" / "wai-voice-dienst"
QUELLE = "crates/wai-voice-dienst"
UEBERNOMMEN = ("src/",)
VENDOR_DATEI = "VENDOR.toml"
STANDARD_REV = "e624d851"
REPO = "https://github.com/MrP42/wai-sdk"
# Versionen, die Local Voice AI schon nutzt (src-tauri/Cargo.toml und Cargo.lock). Sie ersetzen die
# Workspace-Versionen des SDK, damit im Lock nichts Zweites entsteht.
LVA_VERSIONEN = {"sha2": "0.10"}
# Mehr als das darf die Crate nicht brauchen; eine neue Abhängigkeit im SDK soll hier auffallen.
# ring (0.17, Apache-2.0 AND ISC) steht schon im Cargo.lock von Local Voice AI (rustls); es kommt
# nichts Neues dazu. Genutzt fuer X25519 und HKDF der Kopplung.
ERLAUBT = {"serde", "serde_json", "sha2", "thiserror", "ring"}


def git(sdk: Path, *argumente: str) -> bytes:
    r = subprocess.run(["git", "-C", str(sdk), *argumente], capture_output=True)
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(argumente)} fehlgeschlagen: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def toml_wert(w: object) -> str:
    if isinstance(w, bool):
        return "true" if w else "false"
    if isinstance(w, str):
        return '"' + w.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(w, list):
        return "[" + ", ".join(toml_wert(x) for x in w) + "]"
    if isinstance(w, dict):
        return "{ " + ", ".join(f"{k} = {toml_wert(v)}" for k, v in w.items()) + " }"
    raise SystemExit(f"TOML-Wert nicht unterstützt: {w!r}")


def abhaengigkeit(name: str, eigen: object, workspace: dict) -> object:
    """Löst `workspace = true` gegen [workspace.dependencies] auf; zusätzliche Features werden vereinigt."""
    if name not in ERLAUBT:
        raise SystemExit(f"Neue Abhängigkeit {name!r} in {QUELLE}: erst Lizenz prüfen und melden, dann ERLAUBT erweitern")
    if isinstance(eigen, dict) and eigen.get("workspace"):
        basis = workspace[name]
        basis = {"version": basis} if isinstance(basis, str) else dict(basis)
        for k, v in eigen.items():
            if k == "workspace":
                continue
            if k == "features":
                basis["features"] = sorted(set(basis.get("features", [])) | set(v))
            else:
                basis[k] = v
        basis.pop("path", None)
    elif isinstance(eigen, str):
        basis = {"version": eigen}
    else:
        basis = dict(eigen)  # type: ignore[arg-type]
    if name in LVA_VERSIONEN:
        basis["version"] = LVA_VERSIONEN[name]
    return basis["version"] if list(basis) == ["version"] else basis


def cargo_toml(sdk: Path, commit: str) -> bytes:
    ws = tomllib.loads(git(sdk, "show", f"{commit}:Cargo.toml").decode("utf-8"))["workspace"]
    paket = tomllib.loads(git(sdk, "show", f"{commit}:{QUELLE}/Cargo.toml").decode("utf-8"))
    p = paket["package"]
    zeilen = [
        f"# Erzeugt von scripts/vendor-wai-voice-dienst.py aus {REPO} ({QUELLE}, Commit {commit[:12]}). Nicht von Hand ändern.",
        "[package]",
        f'name = {toml_wert(p["name"])}',
        f'version = {toml_wert(p["version"])}',
        f'edition = {toml_wert(ws["package"]["edition"])}',
        f'rust-version = {toml_wert(ws["package"]["rust-version"])}',
        f'license = {toml_wert(ws["package"]["license"])}',
        "publish = false",
        f'description = {toml_wert(p["description"])}',
        "",
        "[lib]",
        "test = false",
        "doctest = false",
        "",
        "[dependencies]",
    ]
    for name, eigen in paket.get("dependencies", {}).items():
        zeilen.append(f"{name} = {toml_wert(abhaengigkeit(name, eigen, ws['dependencies']))}")
    return ("\n".join(zeilen) + "\n").encode("utf-8")


def dateien_aus_commit(sdk: Path, commit: str) -> dict[str, bytes]:
    archiv = git(sdk, "archive", "--format=tar", commit, QUELLE)
    dateien: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archiv)) as tar:
        for eintrag in tar:
            rel = eintrag.name[len(QUELLE) + 1 :]
            if not eintrag.isfile() or not rel.startswith(UEBERNOMMEN):
                continue
            daten = tar.extractfile(eintrag)
            assert daten is not None
            dateien[rel] = daten.read()
    if not any(r.startswith("src/") for r in dateien):
        raise SystemExit(f"{QUELLE}/src ist im Commit {commit} leer")
    dateien["Cargo.toml"] = cargo_toml(sdk, commit)
    # rustfmt von Local Voice AI steht auf edition 2021; die Crate ist edition 2024 (let-chains).
    dateien["rustfmt.toml"] = b'edition = "2024"\n'
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
        "# Vendorte Kopie von wai-voice-dienst (WAI SDK). Nicht von Hand ändern: scripts/vendor-wai-voice-dienst.py\n"
        'paket = "wai-voice-dienst"\n'
        f'quelle = "{REPO}"\n'
        f'pfad = "{QUELLE}"\n'
        f'referenz = "{rev}"\n'
        f'commit = "{commit}"\n'
        f'commit_datum = "{datum}"\n'
        f'uebernommen_am = "{dt.date.today().isoformat()}"\n'
        'ausgelassen = ["Tests im SDK-Arbeitsbaum"]\n',
        encoding="utf-8",
        newline="\n",
    )
    print(f"wai-voice-dienst {commit[:7]} ({datum}) übernommen: {len(dateien)} Dateien nach {ZIEL.relative_to(WURZEL).as_posix()}")
    return 0


def pruefen(sdk: Path) -> int:
    vendor = ZIEL / VENDOR_DATEI
    if not vendor.is_file():
        print(f"{vendor.relative_to(WURZEL).as_posix()} fehlt", file=sys.stderr)
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
        print(f"wai-voice-dienst weicht vom Commit {commit[:7]} ab:", file=sys.stderr)
        for a in abweichungen:
            print(f"  {a}", file=sys.stderr)
        return 1
    print(f"wai-voice-dienst entspricht Commit {commit[:7]} ({len(ist)} Dateien)")
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
