#!/usr/bin/env python3
"""Vendort die Projekt- und Recherche-Bausteine aus @wai/ui (wai-sdk, packages/ui) nach src/vendor/wai-ui.

Aufruf: python3 scripts/vendor-wai-ui.py <wai-sdk-Checkout> [--commit origin/main]
        python3 scripts/vendor-wai-ui.py <wai-sdk-Checkout> --check

Kopiert unveraendert: projekt/{typen,Projektordner,RechercheAnsicht,RechercheFreigabe,index},
components/{Button,Dialog,Drawer,StatusChip}, cx, icons, dichte (nur Kontext). Das Stylesheet
(`projekt.css`) enthaelt genau die Regeln aus styles/components.css, deren Selektoren mit den
Klassen dieser Bausteine beginnen. Einzige Anpassung: Ref-Typ in Dialog.tsx (React 18). Die Dateien nicht von Hand aendern.
"""
import re, subprocess, sys, pathlib

APP = pathlib.Path(__file__).resolve().parent.parent
ZIEL = APP / "src" / "vendor" / "wai-ui"
DATEIEN = [
    "projekt/typen.ts", "projekt/Projektordner.tsx", "projekt/RechercheAnsicht.tsx",
    "projekt/RechercheFreigabe.tsx", "projekt/index.ts",
    "components/Button.tsx", "components/Dialog.tsx", "components/Drawer.tsx",
    "components/StatusChip.tsx", "cx.ts", "icons.tsx",
]
PRAEFIXE = [".wai-projekt", ".wai-recherche", ".wai-freigabe", ".wai-btn", ".wai-badge", ".wai-chip",
            ".wai-dialog", ".wai-modal", ".wai-drawer", ".wai-icon"]
KOPF = "/* Kopie aus @wai/ui (wai-sdk {sha}): Klassen der Projekt- und Recherche-Bausteine. Nicht von Hand aendern (scripts/vendor-wai-ui.py). */\n"


def git(repo, *a):
    return subprocess.check_output(["git", "-C", str(repo), *a], text=True)


def blocks(css):
    """Zerlegt in Top-Level-Bloecke (Regel oder @-Regel mit Kind-Bloecken) samt vorangehenden Kommentaren."""
    out, i, n = [], 0, len(css)
    while i < n:
        m = re.compile(r"\s*(?:/\*.*?\*/\s*)*", re.S).match(css, i)
        j = m.end()
        if j >= n:
            break
        k = css.index("{", j)
        depth, p = 1, k + 1
        while depth:
            c = css[p]
            depth += c == "{"
            depth -= c == "}"
            p += 1
        out.append((css[j:k].strip(), css[j:p]))
        i = p
    return out


def waehlen(css):
    keep = []
    for sel, text in blocks(css):
        if sel.startswith("@media") or sel.startswith("@supports"):
            inner = re.search(r"\{(.*)\}\s*$", text, re.S).group(1)
            sub = [t for s, t in blocks(inner) if relevant(s)]
            if sub:
                keep.append(sel + " {\n" + "\n".join(sub) + "\n}")
        elif relevant(sel):
            keep.append(text)
    return "\n".join(keep) + "\n"


def relevant(sel):
    if sel.startswith("@"):
        return False
    return all(any(part.strip().startswith(p) for p in PRAEFIXE) for part in sel.split(","))


def erzeugen(repo, ref):
    sha = git(repo, "rev-parse", ref).strip()
    out = {}
    for d in DATEIEN:
        out[d] = git(repo, "show", f"{ref}:packages/ui/src/{d}")
    # React 18 (diese App) statt 19 (@wai/ui): ein Ref, dem Code zuweist, braucht den Typ `| null`.
    zeile = "const surface = useRef<HTMLDivElement>(null);"
    assert zeile in out["components/Dialog.tsx"], "Dialog.tsx hat sich geaendert"
    out["components/Dialog.tsx"] = out["components/Dialog.tsx"].replace(
        zeile, "const surface = useRef<HTMLDivElement | null>(null);")
    d = git(repo, "show", f"{ref}:packages/ui/src/dichte/Dichte.tsx")
    kontext = d.split("export interface DichteProviderProps")[0]
    out["dichte/Dichte.tsx"] = kontext.replace(
        'import {\n  createContext,\n  type ReactNode,\n  useCallback,\n  useContext,\n  useSyncExternalStore,\n} from "react";',
        'import { createContext, useContext } from "react";')
    css = git(repo, "show", f"{ref}:packages/ui/src/styles/components.css")
    out["projekt.css"] = KOPF.format(sha=sha[:12]) + waehlen(css)
    out["VENDOR.toml"] = (
        f'# Erzeugt von scripts/vendor-wai-ui.py\nquelle = "https://github.com/MrP42/wai-sdk"\n'
        f'pfad = "packages/ui/src"\ncommit = "{sha}"\ndateien = {sorted(out)!r}\n'.replace("'", '"'))
    return out


def main():
    a = sys.argv[1:]
    if not a:
        sys.exit(__doc__)
    repo = pathlib.Path(a[0])
    ref = a[a.index("--commit") + 1] if "--commit" in a else "origin/main"
    out = erzeugen(repo, ref)
    if "--check" in a:
        bad = [k for k, v in out.items() if not (ZIEL / k).exists() or (ZIEL / k).read_text(encoding="utf-8") != v]
        print("abweichend:", bad) if bad else print("ok")
        sys.exit(1 if bad else 0)
    for k, v in out.items():
        p = ZIEL / k
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(v, encoding="utf-8", newline="\n")
    print("geschrieben:", len(out), "Dateien nach", ZIEL)


main()
