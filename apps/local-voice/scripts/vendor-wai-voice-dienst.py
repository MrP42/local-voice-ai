#!/usr/bin/env python3
"""wai-voice-dienst aus dem WAI-SDK in Local Voice AI übernehmen (Vendoring, ADR-0021 Paket S2).

Quelle:   https://github.com/MrP42/wai-sdk, Pfad crates/wai-voice-dienst
Commit:   c51d643e (origin/main nach Merge von PR #1137: späte ende/guthaben beendeter Ströme verwerfen,
          start mit beendeter Nummer nur für den Strom abweisen). Vorher e624d851 (PR #994: `willkommen` mit
          Gegenbeweis, `hallo.zufall` und Kopplung mit Schluesselaustausch/Vergleichscode), a624d690, 6ecea3ad und
          249a4db3 (origin/main, #985). Der volle Commit steht in VENDOR.toml.
Ziel:     apps/local-voice/src-tauri/vendor/wai-voice-dienst/

Muster: wai-digitaler-posteingang/scripts/vendor-wai-rules.py. Kopiert `src/` des festen Commits per
`git archive` und erzeugt eine eigenständige `Cargo.toml` ohne Workspace-Vererbung. Die workspace-Abhängigkeiten
werden auf konkrete Versionen aufgelöst, passend zu dem, was Local Voice AI schon im Lock hat (sha2 0.10 statt
0.11 des SDK-Workspace; thiserror 2, serde 1, serde_json 1). Es kommt KEINE neue externe Abhängigkeit dazu (ring steht schon im Lock).
Die vendorten Dateien werden nie von Hand geändert; Anpassungen gehören ins SDK.

Was aus der `Cargo.toml` der Crate wird (Funktion `manifest`, Tests in scripts/test_vendor_wai_voice_dienst.py):
übernommen werden `[package]` (nur die bekannten Felder), `[dependencies]`, `[target.<ziel>.dependencies]` und
`[features]`, jeweils nach Namen sortiert und mit `optional = true`, wo die Quelle es setzt. Bewusst ausgelassen
werden `[dev-dependencies]` (auch unter einem Ziel) und `[lints]`, siehe AUSGELASSEN; `VENDOR.toml` nennt, was
bei der Übernahme ausgelassen wurde. Jede andere Tabelle, jedes
unbekannte Feld in `[package]` und jede Abhängigkeit, die nicht in ERLAUBT steht oder nicht von crates.io kommt,
bricht mit einer Meldung ab; still fällt nichts weg. Ein Feature, das auf etwas zeigt, das die erzeugte Datei
nicht enthält, bricht ebenfalls ab. Bei einem Abbruch bleibt die vorhandene Kopie unverändert.

    python3 scripts/vendor-wai-voice-dienst.py [<wai-sdk-pfad>] [--commit <rev>]   übernehmen
    python3 scripts/vendor-wai-voice-dienst.py [<wai-sdk-pfad>] --check            Ordner gegen den gepinnten Commit prüfen

Der SDK-Pfad kommt aus dem Argument oder der Umgebungsvariablen WAI_SDK. Gelesen wird nur per `git`, der
Arbeitsbaum des SDK wird nicht angefasst. `--check` liest den Commit aus `VENDOR.toml` und endet mit Exit 1,
wenn der Ordner (samt erzeugter Cargo.toml) vom Inhalt dieses Commits abweicht.

    python3 scripts/test_vendor_wai_voice_dienst.py                                Tests des Skripts (offline)
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
STANDARD_REV = "c51d643e"
REPO = "https://github.com/MrP42/wai-sdk"
# Versionen, die Local Voice AI schon nutzt (src-tauri/Cargo.toml und Cargo.lock). Sie ersetzen die
# Workspace-Versionen des SDK, damit im Lock nichts Zweites entsteht.
LVA_VERSIONEN = {"sha2": "0.10"}
# Mehr als das darf die Crate nicht brauchen; eine neue Abhängigkeit im SDK soll hier auffallen.
# ring (0.17, Apache-2.0 AND ISC) steht schon im Cargo.lock von Local Voice AI (rustls); es kommt
# nichts Neues dazu. Genutzt fuer X25519 und HKDF der Kopplung.
ERLAUBT = {"serde", "serde_json", "sha2", "thiserror", "ring"}
# Tabellen der Quelle, die bewusst nicht in die vendorte Kopie kommen. Jede andere unbekannte Tabelle bricht ab.
AUSGELASSEN = {
    "dev-dependencies": "die Tests der Crate werden nicht übernommen ([lib] test = false)",
    "lints": "betrifft nur Prüfungen des Compilers, nicht den erzeugten Code; ein deny oder forbid der Quelle gilt in der Kopie also nicht",
}
# Was [package] der Quelle nennen darf. Anderes (build, links, readme …) hieße, dass Dateien oder Verhalten
# fehlen, die das Skript nicht mitbringt.
PAKET_FELDER = {"name", "version", "edition", "rust-version", "license", "publish", "description"}
# Angaben einer Abhängigkeit, die eine Version von crates.io beschreiben. `workspace` ist vorher aufgelöst.
ABHAENGIGKEIT_REIHENFOLGE = ("version", "package", "features", "default-features", "optional")
ABHAENGIGKEIT_FELDER = set(ABHAENGIGKEIT_REIHENFOLGE)


def git(sdk: Path, *argumente: str) -> bytes:
    r = subprocess.run(["git", "-C", str(sdk), *argumente], capture_output=True)
    if r.returncode != 0:
        raise SystemExit(f"git {' '.join(argumente)} fehlgeschlagen: {r.stderr.decode(errors='replace').strip()}")
    return r.stdout


def abbruch(text: str) -> SystemExit:
    return SystemExit(f"{QUELLE}/Cargo.toml: {text}")


def toml_schluessel(k: str) -> str:
    """Ein Schlüssel für Tabellenkopf oder Zuweisung: blank, wenn TOML das erlaubt, sonst in Anführungszeichen."""
    if k and all(z.isascii() and (z.isalnum() or z in "_-") for z in k):
        return k
    return toml_wert(k)


def toml_wert(w: object) -> str:
    if isinstance(w, bool):
        return "true" if w else "false"
    if isinstance(w, str):
        if any(ord(z) < 0x20 or ord(z) == 0x7F for z in w):
            raise SystemExit(f"TOML-Wert mit Steuerzeichen nicht unterstützt: {w!r}")
        return '"' + w.replace("\\", "\\\\").replace('"', '\\"') + '"'
    if isinstance(w, list):
        return "[" + ", ".join(toml_wert(x) for x in w) + "]"
    if isinstance(w, dict):
        return "{ " + ", ".join(f"{toml_schluessel(k)} = {toml_wert(v)}" for k, v in w.items()) + " }"
    raise SystemExit(f"TOML-Wert nicht unterstützt: {w!r}")


def abhaengigkeit(name: str, eigen: object, workspace: dict) -> object:
    """Löst `workspace = true` gegen [workspace.dependencies] auf; zusätzliche Features werden vereinigt.

    Geprüft wird der **Name des Pakets**, nicht der Schlüssel: `serde = { package = "anderes" }` scheitert an
    ERLAUBT. Übrig bleiben nur Angaben, die eine Version von crates.io beschreiben; `path`, `git` und andere
    Quellen brechen ab, weil die vendorte Kopie sie nicht mitbringt.
    """
    if isinstance(eigen, dict) and eigen.get("workspace") is True:
        if name not in workspace:
            raise abbruch(f"Abhängigkeit {name!r} erbt vom Workspace, steht dort aber nicht")
        basis = workspace[name]
        basis = {"version": basis} if isinstance(basis, str) else dict(basis)
        for k, v in eigen.items():
            if k == "workspace":
                continue
            if k == "features":
                basis["features"] = sorted(set(basis.get("features", [])) | set(v))
            else:
                basis[k] = v
    elif isinstance(eigen, str):
        basis = {"version": eigen}
    elif isinstance(eigen, dict):
        basis = dict(eigen)
    else:
        raise abbruch(f"Abhängigkeit {name!r} hat eine Form, die das Skript nicht kennt: {eigen!r}")
    paket = basis.get("package", name)
    if paket not in ERLAUBT:
        raise SystemExit(f"Neue Abhängigkeit {paket!r} in {QUELLE}: erst Lizenz prüfen und melden, dann ERLAUBT erweitern")
    fremd = sorted(set(basis) - ABHAENGIGKEIT_FELDER)
    if fremd:
        raise abbruch(
            f"Abhängigkeit {name!r} nutzt {', '.join(fremd)}; übernommen werden nur Versionen von crates.io "
            f"({', '.join(sorted(ABHAENGIGKEIT_FELDER))})"
        )
    if paket in LVA_VERSIONEN:
        basis["version"] = LVA_VERSIONEN[paket]
    if not isinstance(basis.get("version"), str):
        raise abbruch(f"Abhängigkeit {name!r} hat keine Version")
    if "features" in basis:
        f = basis["features"]
        if not isinstance(f, list) or not all(isinstance(x, str) for x in f):
            raise abbruch(f"Abhängigkeit {name!r}: features ist keine Liste von Texten")
        basis["features"] = sorted(set(f))
    # Feste Reihenfolge der Felder, damit die Ausgabe nicht an der Schreibweise der Quelle hängt.
    basis = {k: basis[k] for k in ABHAENGIGKEIT_REIHENFOLGE if k in basis}
    return basis["version"] if list(basis) == ["version"] else basis


def abhaengigkeiten(tabelle: object, workspace: dict, wo: str) -> dict[str, object]:
    """Eine Abhängigkeitstabelle der Quelle, aufgelöst und nach Namen sortiert."""
    if not isinstance(tabelle, dict):
        raise abbruch(f"[{wo}] ist keine Tabelle")
    return {name: abhaengigkeit(name, tabelle[name], workspace) for name in sorted(tabelle)}


def ist_optional(a: object) -> bool:
    return isinstance(a, dict) and a.get("optional") is True


def pruefe_features(features: dict, alle: dict[str, list[object]]) -> None:
    """Jeder Eintrag eines Features muss auf etwas zeigen, das die erzeugte Datei auch enthält.

    `alle` nennt je Abhängigkeit ihre Erklärungen aus `[dependencies]` und den Zieltabellen. Cargo meldet
    dieselben Fehler erst beim Bauen in Local Voice AI; hier fallen sie beim Übernehmen auf, mit dem Namen des
    Features. Die Regeln folgen Cargo (Gegenprobe im Test, der Cargo dieselben Fälle vorlegt):

    - Eine Abhängigkeit ist optional, sobald **eine** ihrer Erklärungen `optional = true` setzt.
    - `dep:x` und `x?/f` verlangen ein optionales `x`; `x/f` verlangt nur, dass es `x` gibt.
    - Ein bloßer Name ist ein Feature oder das implizite Feature einer optionalen Abhängigkeit. Dieses gibt es
      nur, solange **kein** Feature `dep:x` nennt.
    - Heißt ein Feature wie eine optionale Abhängigkeit, verdeckt es deren implizites Feature. Dann muss
      irgendein Feature die Abhängigkeit nennen: mit `dep:x`, `x/f` oder `x?/f`.
    - Ein Verweis hat höchstens einen Schrägstrich.

    Welche Zeichen ein Featurename haben darf, prüft das Skript nicht nach: Cargos Regel hängt an Unicode-Tabellen
    (XID_Start, XID_Continue), und die des laufenden Python können älter sein als die von Cargo. Abgelehnt werden
    nur `/` und ein `dep:` am Anfang, weil ein solcher Name als Verweis gelesen würde.

    Strenger als Cargo an einer Stelle: Testabhängigkeiten zählen nicht, weil die Kopie sie nicht enthält.
    """
    for feature in sorted(features):
        eintraege = features[feature]
        if not isinstance(eintraege, list) or not all(isinstance(e, str) for e in eintraege):
            raise abbruch(f"Feature {feature!r} ist keine Liste von Texten")
        if "/" in feature or feature.startswith("dep:"):
            raise abbruch(f"Feature {feature!r} hat einen Namen, den Cargo nicht annimmt")
    optional = {n for n, erkl in alle.items() if any(ist_optional(e) for e in erkl)}
    mit_dep = {e[4:] for eintraege in features.values() for e in eintraege if e.startswith("dep:")}
    # Was irgendein Feature als Abhängigkeit nennt: `dep:x`, `x/f` und `x?/f`.
    genannt = mit_dep | {
        e.partition("/")[0].removesuffix("?") for eintraege in features.values() for e in eintraege if "/" in e
    }
    for feature in sorted(features):
        for e in features[feature]:
            if e.startswith("dep:"):
                name = e[4:]
                if "/" in name:
                    raise abbruch(f"Feature {feature!r} nennt {e!r} mit `dep:` und `/` zugleich")
                if name not in optional:
                    raise abbruch(f"Feature {feature!r} schaltet {e!r} ein, aber {name!r} ist keine optionale Abhängigkeit")
            elif "/" in e:
                name, _, rest = e.partition("/")
                if "/" in rest:
                    raise abbruch(f"Feature {feature!r} nennt {e!r} mit mehr als einem Schrägstrich")
                schwach = name.endswith("?")
                name = name.removesuffix("?")
                if name not in alle or (schwach and name not in optional):
                    raise abbruch(f"Feature {feature!r} verweist mit {e!r} auf eine Abhängigkeit, die es so nicht gibt")
            elif e in features:
                continue
            elif e in optional and e in mit_dep:
                raise abbruch(
                    f"Feature {feature!r} verweist auf {e!r}; die optionale Abhängigkeit hat kein gleichnamiges "
                    f"Feature, weil ein Feature sie mit 'dep:{e}' einschaltet"
                )
            elif e not in optional:
                raise abbruch(f"Feature {feature!r} verweist auf {e!r}; das ist weder ein Feature noch eine optionale Abhängigkeit")
    for name in sorted((optional & set(features)) - genannt):
        raise abbruch(
            f"Die optionale Abhängigkeit {name!r} wird von keinem Feature genannt: Das Feature {name!r} "
            f"verdeckt ihr gleichnamiges, und keines nennt 'dep:{name}', '{name}/…' oder '{name}?/…'"
        )


def geerbt(p: dict, ws_paket: dict, feld: str) -> object | None:
    """Ein Feld aus [package]: eigener Wert oder, bei `feld.workspace = true`, der aus [workspace.package]."""
    if feld not in p:
        return None
    w = p[feld]
    if isinstance(w, dict):
        if w != {"workspace": True} or feld not in ws_paket:
            raise abbruch(f"[package] {feld} = {w!r} lässt sich nicht auflösen")
        return ws_paket[feld]
    return w


def manifest(paket: dict, ws: dict, commit: str) -> str:
    """Die `Cargo.toml` der vendorten Kopie aus der `Cargo.toml` der Crate und der des Workspace.

    Reine Funktion. Jede Tabelle der Quelle wird übernommen, bewusst ausgelassen (AUSGELASSEN) oder bricht mit
    einer Meldung ab; still fällt nichts weg. Reihenfolge der Ausgabe: `[package]`, `[lib]`, `[dependencies]`,
    Zieltabellen nach Ziel sortiert, `[features]`; Abhängigkeiten und Features nach Namen sortiert.
    """
    ws_paket = ws.get("package", {})
    ws_abh = ws.get("dependencies", {})
    for tabelle in paket:
        if tabelle in AUSGELASSEN or tabelle in ("package", "dependencies", "target", "features"):
            continue
        raise abbruch(
            f"Tabelle [{tabelle}] wird nicht unterstützt. Im Skript entscheiden, ob sie übernommen oder "
            "ausgelassen wird (AUSGELASSEN), statt sie still fallen zu lassen"
        )
    p = paket.get("package")
    if not isinstance(p, dict):
        raise abbruch("[package] fehlt")
    fremd = sorted(set(p) - PAKET_FELDER)
    if fremd:
        raise abbruch(
            f"[package] nennt {', '.join(fremd)}; das Skript kennt nur {', '.join(sorted(PAKET_FELDER))}. "
            "Im Skript entscheiden, was damit geschieht"
        )
    kopf = [
        f"# Erzeugt von scripts/vendor-wai-voice-dienst.py aus {REPO} ({QUELLE}, Commit {commit[:12]}). Nicht von Hand ändern.",
        "[package]",
    ]
    for feld in ("name", "version", "edition", "rust-version", "license", "publish", "description"):
        if feld == "publish":
            kopf.append("publish = false")
            continue
        wert = geerbt(p, ws_paket, feld)
        if wert is None:
            if feld in ("name", "version", "edition"):
                raise abbruch(f"[package] {feld} fehlt")
            continue
        kopf.append(f"{feld} = {toml_wert(wert)}")
    abschnitte = [kopf, ["[lib]", "test = false", "doctest = false"]]

    def tabelle_schreiben(kopfzeile: str, eintraege: dict[str, object]) -> None:
        abschnitte.append([kopfzeile] + [f"{toml_schluessel(n)} = {toml_wert(w)}" for n, w in eintraege.items()])

    alle: dict[str, list[object]] = {}

    def merken(eintraege: dict[str, object]) -> dict[str, object]:
        for n, w in eintraege.items():
            alle.setdefault(n, []).append(w)
        return eintraege

    tabelle_schreiben("[dependencies]", merken(abhaengigkeiten(paket.get("dependencies", {}), ws_abh, "dependencies")))
    ziele = paket.get("target", {})
    if not isinstance(ziele, dict):
        raise abbruch("[target] ist keine Tabelle")
    for ziel in sorted(ziele):
        inhalt = ziele[ziel]
        if not isinstance(inhalt, dict):
            raise abbruch(f"[target.{ziel}] ist keine Tabelle")
        for tabelle in inhalt:
            if tabelle != "dependencies" and tabelle not in AUSGELASSEN:
                raise abbruch(
                    f"Tabelle [target.{toml_schluessel(ziel)}.{tabelle}] wird nicht unterstützt. Im Skript "
                    "entscheiden, ob sie übernommen oder ausgelassen wird, statt sie still fallen zu lassen"
                )
        if inhalt.get("dependencies"):
            wo = f"target.{toml_schluessel(ziel)}.dependencies"
            tabelle_schreiben(f"[{wo}]", merken(abhaengigkeiten(inhalt["dependencies"], ws_abh, wo)))
    features = paket.get("features", {})
    if not isinstance(features, dict):
        raise abbruch("[features] ist keine Tabelle")
    pruefe_features(features, alle)
    if features:
        tabelle_schreiben("[features]", {f: sorted(features[f]) for f in sorted(features)})
    return "\n\n".join("\n".join(a) for a in abschnitte) + "\n"


def ausgelassene_tabellen(paket: dict) -> list[str]:
    """Die Tabellen der Quelle, die `manifest` bewusst nicht übernimmt, so wie sie in `VENDOR.toml` stehen."""
    aus = [f"[{t}]" for t in sorted(paket) if t in AUSGELASSEN]
    ziele = paket.get("target", {})
    for ziel in sorted(ziele) if isinstance(ziele, dict) else []:
        inhalt = ziele[ziel] if isinstance(ziele[ziel], dict) else {}
        aus += [f"[target.{toml_schluessel(ziel)}.{t}]" for t in sorted(inhalt) if t in AUSGELASSEN]
    return aus


def quell_manifest(sdk: Path, commit: str) -> tuple[dict, dict]:
    ws = tomllib.loads(git(sdk, "show", f"{commit}:Cargo.toml").decode("utf-8"))["workspace"]
    paket = tomllib.loads(git(sdk, "show", f"{commit}:{QUELLE}/Cargo.toml").decode("utf-8"))
    return paket, ws


def cargo_toml(sdk: Path, commit: str) -> bytes:
    paket, ws = quell_manifest(sdk, commit)
    return manifest(paket, ws, commit).encode("utf-8")


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
        f'ausgelassen = {toml_wert(["Tests im SDK-Arbeitsbaum"] + ausgelassene_tabellen(quell_manifest(sdk, commit)[0]))}\n',
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
