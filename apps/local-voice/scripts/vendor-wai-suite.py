#!/usr/bin/env python3
"""Projekte, Recherche (ADR-0026) und Assistent-Kern (ADR-0028) der Suite aus dem WAI-SDK in Local Voice AI übernehmen (Vendoring).

Quelle:   https://github.com/MrP42/wai-sdk, Pfade crates/<name> der Crates unten
Ziel:     apps/local-voice/src-tauri/vendor/<name>/

Übernommen werden `wai-projekte`, `wai-recherche` und `wai-assistent` samt ihren wai-*-Pfadabhängigkeiten
(`wai-security`, `wai-storage`, `wai-links`, `wai-providers`). Muster wie `vendor-wai-voice-dienst.py`:
Kopie von `src/` des festen Commits per `git archive` und eine eigenständige `Cargo.toml` ohne
Workspace-Vererbung. Workspace-Abhängigkeiten werden auf konkrete Versionen aufgelöst; wo Local Voice AI
schon eine Version im Lock hat, gilt diese (sha2 0.10). Die vendorten Dateien werden nie von Hand
geändert; Anpassungen gehören ins SDK.

Zwei bewusste Abweichungen der erzeugten `Cargo.toml` vom SDK:
- `reqwest`: Feature `rustls` wird zu `rustls-no-provider`. `rustls` zöge `aws-lc-rs` (aws-lc-sys, Lizenz
  OpenSSL) ins Lock; Local Voice AI nutzt `ring`, und der Updater (reqwest 0.13) läuft schon so.
- `[lib] test = false`: die Tests der Crates laufen im SDK. Ausnahme `MIT_TESTS` (Codex lva#414): deren
  Tests (Sicherheitsregeln des Notizbuch-Zugangs) laufen auch hier, `scripts/dev.ps1 test|check` ruft sie
  verpflichtend mit `cargo test -p <crate>` auf; die Dev-Abhängigkeiten kommen dazu (nur `ERLAUBT_DEV`).

    python3 scripts/vendor-wai-suite.py [<wai-sdk-pfad>] [--commit <rev>]   übernehmen (Standard: origin/main)
    python3 scripts/vendor-wai-suite.py [<wai-sdk-pfad>] --nur <crate> [--commit <rev>]   nur ein Crate übernehmen
    python3 scripts/vendor-wai-suite.py [<wai-sdk-pfad>] --check            Ordner gegen den gepinnten Commit prüfen
    python3 scripts/vendor-wai-suite.py --test                              Tests der Crates in MIT_TESTS ausführen

Der SDK-Pfad kommt aus dem Argument oder der Umgebungsvariablen WAI_SDK. Gelesen wird nur per `git`, der
Arbeitsbaum des SDK wird nicht angefasst. `--check` liest den Commit aus den `VENDOR.toml` und endet mit
Exit 1, wenn ein Ordner (samt erzeugter Cargo.toml) vom Inhalt dieses Commits abweicht.
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
VENDOR = WURZEL / "src-tauri" / "vendor"
CRATES = (
    "wai-links", "wai-security", "wai-storage", "wai-providers", "wai-projekte", "wai-recherche", "wai-assistent",
    "wai-notizbuch-zugang",
)
UEBERNOMMEN = ("src/", "regeln/")
# wai-assistent (ADR-0028 A2) bindet `regeln/standard.json` per include_str! ein und die Vertragsfixture aus
# dem SDK-Verzeichnis contracts/, das ausserhalb des Crates liegt. Beim Vendoren wird die Fixture daher als
# regeln/vertragsfixture.json mitkopiert und der Pfad in regeltabelle.rs umgeschrieben.
FIXTURE_QUELLE = "contracts/fixtures/assistent-einstufung/valid/regeltabelle-de-en.json"
FIXTURE_ALT = 'include_str!("../../../contracts/fixtures/assistent-einstufung/valid/regeltabelle-de-en.json")'
FIXTURE_NEU = 'include_str!("../regeln/vertragsfixture.json")'

# Crates, deren eigene Tests auch in Local Voice AI laufen (Sicherheitsregeln, Codex lva#414 M6).
MIT_TESTS = ("wai-notizbuch-zugang",)
# Dev-Abhängigkeiten dieser Tests; tempfile 3 (MIT OR Apache-2.0) nutzt Local Voice AI schon.
ERLAUBT_DEV = {"tempfile"}

VENDOR_DATEI = "VENDOR.toml"
STANDARD_REV = "origin/main"
REPO = "https://github.com/MrP42/wai-sdk"
# Versionen, die Local Voice AI schon nutzt; sie ersetzen die Workspace-Versionen des SDK.
LVA_VERSIONEN = {"sha2": "0.10"}
# Ausnahmen davon je Crate: wai-assistent braucht sha2 0.11 (digest 0.11), weil hmac 0.13 und hkdf 0.13 darauf
# aufsetzen (Kopplungs-MAC, Schlüsselpakete, E3.8). sha2 0.11 liegt bereits im Lockfile; der Crate gibt keine
# sha2-Typen nach außen, beide Versionen bestehen nebeneinander.
SDK_VERSION_BEHALTEN = {"wai-assistent": {"sha2"}}
# Mehr als das dürfen die Crates nicht brauchen; eine neue Abhängigkeit im SDK soll hier auffallen
# (erst Lizenz prüfen und melden, dann erweitern).
ERLAUBT = {
    "serde", "serde_json", "thiserror", "getrandom", "tokio", "sqlx", "sha2", "cap-std", "keyring",
    "chacha20poly1305", "zeroize", "regex", "async-trait", "tokio-util", "futures-util", "bytes",
    "reqwest", "url", "scraper", "ego-tree",
    # base64 0.22 (MIT OR Apache-2.0), im Lockfile und in Cargo.toml schon vorhanden; Sync-Kern von wai-assistent.
    "base64",
    # rusqlite 0.37 „bundled“ (MIT), in Local Voice schon vorhanden; nur lesender Notizbuch-Zugang (lva#396 N2).
    "rusqlite",
    # Sync-Kern gegen bösartigen Hub (ADR-0028 E3.8, wai-sdk#1092): ed25519-dalek 3 und x25519-dalek 3
    # (dalek, BSD-3-Clause), hkdf 0.13 und hmac 0.13 (RustCrypto, MIT OR Apache-2.0), subtle 2.6 (BSD-3-Clause).
    "ed25519-dalek", "x25519-dalek", "hkdf", "hmac", "subtle",
}


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


def abhaengigkeit(crate: str, name: str, eigen: object, workspace: dict, erlaubt: set[str] = ERLAUBT) -> object:
    """Löst `workspace = true` gegen [workspace.dependencies] auf; zusätzliche Features werden vereinigt."""
    if name.startswith("wai-"):
        if name not in CRATES:
            raise SystemExit(f"{crate} braucht {name!r}, das hier nicht vendort wird: CRATES erweitern")
        return {"path": f"../{name}", "version": "0.1.0"}
    if name not in erlaubt:
        raise SystemExit(f"Neue Abhängigkeit {name!r} in {crate}: erst Lizenz prüfen und melden, dann ERLAUBT erweitern")
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
    if name in LVA_VERSIONEN and name not in SDK_VERSION_BEHALTEN.get(crate, set()):
        basis["version"] = LVA_VERSIONEN[name]
    if name == "reqwest" and "features" in basis:
        basis["features"] = ["rustls-no-provider" if f == "rustls" else f for f in basis["features"]]
    return basis["version"] if list(basis) == ["version"] else basis


def cargo_toml(sdk: Path, commit: str, crate: str) -> bytes:
    quelle = f"crates/{crate}"
    ws = tomllib.loads(git(sdk, "show", f"{commit}:Cargo.toml").decode("utf-8"))["workspace"]
    paket = tomllib.loads(git(sdk, "show", f"{commit}:{quelle}/Cargo.toml").decode("utf-8"))
    p = paket["package"]
    zeilen = [
        f"# Erzeugt von scripts/vendor-wai-suite.py aus {REPO} ({quelle}, Commit {commit[:12]}). Nicht von Hand ändern.",
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
        f"test = {'true' if crate in MIT_TESTS else 'false'}",
        "doctest = false",
        "",
        "[dependencies]",
    ]
    for name, eigen in paket.get("dependencies", {}).items():
        zeilen.append(f"{name} = {toml_wert(abhaengigkeit(crate, name, eigen, ws['dependencies']))}")
    if crate in MIT_TESTS and paket.get("dev-dependencies"):
        zeilen += ["", "[dev-dependencies]"]
        for name, eigen in paket["dev-dependencies"].items():
            zeilen.append(
                f"{name} = {toml_wert(abhaengigkeit(crate, name, eigen, ws['dependencies'], ERLAUBT_DEV))}"
            )
    return ("\n".join(zeilen) + "\n").encode("utf-8")


def dateien_aus_commit(sdk: Path, commit: str, crate: str) -> dict[str, bytes]:
    quelle = f"crates/{crate}"
    archiv = git(sdk, "archive", "--format=tar", commit, quelle)
    dateien: dict[str, bytes] = {}
    with tarfile.open(fileobj=io.BytesIO(archiv)) as tar:
        for eintrag in tar:
            rel = eintrag.name[len(quelle) + 1 :]
            if not eintrag.isfile() or not rel.startswith(UEBERNOMMEN):
                continue
            daten = tar.extractfile(eintrag)
            assert daten is not None
            dateien[rel] = daten.read()
    if not any(r.startswith("src/") for r in dateien):
        raise SystemExit(f"{quelle}/src ist im Commit {commit} leer")
    if crate == "wai-assistent":
        quelltext = dateien["src/regeltabelle.rs"].decode("utf-8")
        if FIXTURE_ALT not in quelltext:
            raise SystemExit("wai-assistent: Fixture-Einbindung in regeltabelle.rs nicht gefunden, Skript anpassen")
        dateien["src/regeltabelle.rs"] = quelltext.replace(FIXTURE_ALT, FIXTURE_NEU).encode("utf-8")
        dateien["regeln/vertragsfixture.json"] = git(sdk, "show", f"{commit}:{FIXTURE_QUELLE}")
    dateien["Cargo.toml"] = cargo_toml(sdk, commit, crate)
    # rustfmt von Local Voice AI steht auf edition 2021; die Crates sind edition 2024 (let-chains).
    dateien["rustfmt.toml"] = b'edition = "2024"\n'
    return dateien


def dateien_im_ordner(ziel: Path) -> dict[str, bytes]:
    if not ziel.is_dir():
        return {}
    return {
        p.relative_to(ziel).as_posix(): p.read_bytes()
        for p in sorted(ziel.rglob("*"))
        if p.is_file() and p.relative_to(ziel).as_posix() != VENDOR_DATEI
    }


def sdk_pfad(argument: str | None) -> Path:
    roh = argument or os.environ.get("WAI_SDK")
    if not roh:
        raise SystemExit("wai-sdk-Pfad fehlt: als Argument angeben oder WAI_SDK setzen")
    pfad = Path(roh).expanduser().resolve()
    if not (pfad / ".git").exists():
        raise SystemExit(f"{pfad} ist kein git-Checkout")
    return pfad


def uebernehmen(sdk: Path, rev: str, nur: list[str] | None = None) -> int:
    commit = git(sdk, "rev-parse", "--verify", f"{rev}^{{commit}}").decode().strip()
    datum = git(sdk, "show", "-s", "--format=%cs", commit).decode().strip()
    for crate in nur or CRATES:
        ziel = VENDOR / crate
        dateien = dateien_aus_commit(sdk, commit, crate)
        if ziel.exists():
            shutil.rmtree(ziel)
        for rel, daten in dateien.items():
            pfad = ziel / rel
            pfad.parent.mkdir(parents=True, exist_ok=True)
            pfad.write_bytes(daten)
        (ziel / VENDOR_DATEI).write_text(
            f"# Vendorte Kopie von {crate} (WAI SDK). Nicht von Hand ändern: scripts/vendor-wai-suite.py\n"
            f'paket = "{crate}"\n'
            f'quelle = "{REPO}"\n'
            f'pfad = "crates/{crate}"\n'
            f'referenz = "{rev}"\n'
            f'commit = "{commit}"\n'
            f'commit_datum = "{datum}"\n'
            f'uebernommen_am = "{dt.date.today().isoformat()}"\n'
            'ausgelassen = ["Tests und Fixtures im SDK-Arbeitsbaum"]\n',
            encoding="utf-8",
            newline="\n",
        )
        print(f"{crate} {commit[:7]} ({datum}) übernommen: {len(dateien)} Dateien nach {ziel.relative_to(WURZEL).as_posix()}")
    return 0


def pruefen(sdk: Path) -> int:
    fehler = 0
    for crate in CRATES:
        ziel = VENDOR / crate
        vendor = ziel / VENDOR_DATEI
        if not vendor.is_file():
            print(f"{vendor.relative_to(WURZEL).as_posix()} fehlt", file=sys.stderr)
            fehler = 1
            continue
        commit = tomllib.loads(vendor.read_text(encoding="utf-8"))["commit"]
        soll = dateien_aus_commit(sdk, commit, crate)
        ist = dateien_im_ordner(ziel)
        abweichungen = sorted(
            [f"fehlt: {p}" for p in soll.keys() - ist.keys()]
            + [f"zusätzlich: {p}" for p in ist.keys() - soll.keys()]
            + [f"geändert: {p}" for p in soll.keys() & ist.keys() if soll[p] != ist[p]]
        )
        if abweichungen:
            print(f"{crate} weicht vom Commit {commit[:7]} ab:", file=sys.stderr)
            for a in abweichungen:
                print(f"  {a}", file=sys.stderr)
            fehler = 1
        else:
            print(f"{crate} entspricht Commit {commit[:7]} ({len(ist)} Dateien)")
    return fehler


def testen() -> int:
    """Führt die Tests der Crates in `MIT_TESTS` aus, ohne SDK-Checkout und ohne den Baum zu verändern.

    Die vendorten Crates sind keine Workspace-Mitglieder von Local Voice AI; `cargo test -p` geht für
    Crates mit Dev-Abhängigkeiten deshalb nicht. Der Crate wird in einen temporären Ordner kopiert, mit
    dem Cargo.lock von Local Voice AI (gleiche Versionen) und dem geteilten Target gebaut und getestet.
    """
    import tempfile

    lock = WURZEL / "src-tauri" / "Cargo.lock"
    target = os.environ.get("CARGO_TARGET_DIR") or str(WURZEL / "src-tauri" / "target")
    fehler = 0
    for crate in MIT_TESTS:
        with tempfile.TemporaryDirectory(prefix=f"lva-test-{crate}-") as tmp:
            kopie = Path(tmp) / crate
            shutil.copytree(VENDOR / crate, kopie)
            shutil.copy2(lock, kopie / "Cargo.lock")
            r = subprocess.run(
                ["cargo", "test", "--manifest-path", str(kopie / "Cargo.toml"), "--target-dir", target],
                env={**os.environ, "CARGO_TARGET_DIR": target},
            )
            if r.returncode != 0:
                print(f"{crate}: Tests rot (Exit {r.returncode})", file=sys.stderr)
                fehler = 1
            else:
                print(f"{crate}: Tests grün")
    return fehler


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("sdk", nargs="?", help="Pfad zum wai-sdk-Checkout (sonst WAI_SDK)")
    parser.add_argument("--commit", default=STANDARD_REV, help=f"Commit oder Zweig (Standard: {STANDARD_REV})")
    parser.add_argument("--nur", action="append", choices=CRATES, help="nur dieses Crate übernehmen (mehrfach möglich)")
    parser.add_argument("--check", action="store_true", help="nur prüfen, Exit 1 bei Abweichung")
    parser.add_argument("--test", action="store_true", help="Tests der Crates in MIT_TESTS ausführen")
    a = parser.parse_args()
    if a.test:
        return testen()
    sdk = sdk_pfad(a.sdk)
    return pruefen(sdk) if a.check else uebernehmen(sdk, a.commit, a.nur)


if __name__ == "__main__":
    sys.exit(main())
