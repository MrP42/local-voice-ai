#!/usr/bin/env python3
"""QG7 (Goal Granola-Besprechungen): i18n-Pruefung fuer de + en.

Prueft die Uebersetzungsschluessel, die das Goal gegenueber main hinzugefuegt hat
(Diff gegen den Merge-Base mit origin/main), sowie den gesamten Teilbaum
`meetings.*` und `settings.meetings.*`:

  1. gleiche Schluesselmenge in de und en (Plural-Suffixe _one/_other zaehlen als
     eigene Schluessel, beide Sprachen brauchen dieselben Formen),
  2. gleiche {{Platzhalter}} je Schluessel in de und en,
  3. keine leeren Werte, kein U+FFFD (kaputte Kodierung),
  4. keine Umlaut-Ersatzschreibung im deutschen Text (ae/oe/ue statt ä/ö/ü,
     ss statt ß in bekannten Woertern).

Nur Standardbibliothek. Exit 0 = alles in Ordnung, 1 = Befund (Liste auf stdout).

Aufruf (aus dem Repo-Root oder dem Worktree):
    python koordination/granola-besprechungen/tools/check_i18n_meetings.py
    python ... --base <git-rev>     # anderer Vergleichsstand
    python ... --all                # alle Schluessel statt nur Diff + meetings-Teilbaum
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REL = "apps/local-voice/src/i18n/locales/{lang}/translation.json"
SUBTREES = ("meetings.", "settings.meetings.")

# ae/oe/ue sind nur dann eine Ersatzschreibung, wenn das Wort nicht hierzu passt.
LEGIT_UE = re.compile(
    r"(?i)(?:[aeq]ue|uell|^zuerst$|bluetooth|statue|individuum|kontinuum|duell|residuum|"
    r"zuentwickel|^(?:queue|value|venue|revue|issue|cue|blue|due|true|argue|continue|glue)$)"
)
LEGIT_AE = re.compile(r"(?i)(?:israel|michael|raphael|rafael|gael|aer[oi]|mae$|maestro)")
LEGIT_OE = re.compile(r"(?i)(?:tahoe|poe|goethe|joel|zoe|noel|boeing|does?$|joe|shoe|oe[a-z]*ratio)")
# ss statt ß nur in Woertern melden, die im Deutschen sicher mit ß geschrieben werden.
SS_FOR_SZ = re.compile(
    r"(?i)^(?:gross\w*|heiss\w*|\w*strasse\w*|ausser\w*|\w*schliess\w*|\w*fliess\w*|"
    r"massnahme\w*|fuss\w*|gruss\w*|spass\w*|beiss\w*|reiss\w*|\w*maessig\w*)$"
)
PLACEHOLDER = re.compile(r"\{\{\s*([^}\s]+)\s*\}\}")
STRIP = re.compile(r"\{\{[^}]*\}\}|`[^`]*`|https?://\S+|<[^>]+>")
WORD = re.compile(r"[A-Za-zÄÖÜäöüß]+")


def flat(node, prefix=""):
    out = {}
    for key, value in node.items():
        full = prefix + key
        if isinstance(value, dict):
            out.update(flat(value, full + "."))
        else:
            out[full] = value
    return out


def git(*args, cwd):
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, check=False
    )


def repo_root(start: Path) -> Path:
    r = git("rev-parse", "--show-toplevel", cwd=start)
    if r.returncode != 0:
        sys.exit("Kein Git-Repo gefunden.")
    return Path(r.stdout.decode().strip())


def base_rev(root: Path, override: str | None) -> str | None:
    if override:
        return override
    for ref in ("origin/main", "main"):
        r = git("merge-base", "HEAD", ref, cwd=root)
        if r.returncode == 0:
            return r.stdout.decode().strip()
    return None


def load_file(root: Path, lang: str) -> dict:
    path = root / REL.format(lang=lang)
    return flat(json.loads(path.read_text(encoding="utf-8")))


def load_base(root: Path, rev: str, lang: str) -> dict:
    r = git("show", f"{rev}:{REL.format(lang=lang)}", cwd=root)
    if r.returncode != 0:
        return {}
    return flat(json.loads(r.stdout.decode("utf-8")))


def suspicious_words(text: str) -> list[str]:
    hits = []
    for word in WORD.findall(STRIP.sub(" ", text)):
        if word.isupper() and len(word) <= 3:  # Kuerzel wie AEC, VAD, SRT
            continue
        low = word.lower()
        if "ue" in low and not LEGIT_UE.search(low):
            hits.append(word)
        elif "ae" in low and not LEGIT_AE.search(low):
            hits.append(word)
        elif "oe" in low and not LEGIT_OE.search(low):
            hits.append(word)
        elif "ss" in low and SS_FOR_SZ.match(low):
            hits.append(word)
    return hits


def main() -> int:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--base", help="Git-Rev als Vergleichsstand (Standard: Merge-Base mit origin/main)")
    ap.add_argument("--all", action="store_true", help="alle Schluessel pruefen")
    args = ap.parse_args()

    root = repo_root(Path(__file__).resolve().parent)
    de, en = load_file(root, "de"), load_file(root, "en")

    if args.all:
        scope_de, scope_en, label = set(de), set(en), "alle Schluessel"
    else:
        rev = base_rev(root, args.base)
        if rev is None:
            sys.exit("Kein Merge-Base mit main gefunden; --base <rev> oder --all angeben.")
        new_de = set(de) - set(load_base(root, rev, "de"))
        new_en = set(en) - set(load_base(root, rev, "en"))
        tree_de = {k for k in de if k.startswith(SUBTREES)}
        tree_en = {k for k in en if k.startswith(SUBTREES)}
        scope_de, scope_en = new_de | tree_de, new_en | tree_en
        label = f"{len(new_de)} neue de- / {len(new_en)} neue en-Schluessel seit {rev[:8]} + Teilbaum meetings"

    problems: list[str] = []

    for key in sorted(scope_de - scope_en):
        problems.append(f"nur in de, fehlt in en: {key}")
    for key in sorted(scope_en - scope_de):
        problems.append(f"nur in en, fehlt in de: {key}")

    for key in sorted(scope_de & scope_en):
        d, e = de[key], en[key]
        if not isinstance(d, str) or not isinstance(e, str):
            problems.append(f"kein String-Wert: {key}")
            continue
        if bool(d.strip()) != bool(e.strip()):  # bewusst in beiden leere Werte sind ok
            problems.append(f"nur eine Sprache leer: {key}")
        if set(PLACEHOLDER.findall(d)) != set(PLACEHOLDER.findall(e)):
            problems.append(
                f"Platzhalter weichen ab: {key} de={sorted(set(PLACEHOLDER.findall(d)))} "
                f"en={sorted(set(PLACEHOLDER.findall(e)))}"
            )
        if "�" in d or "�" in e:
            problems.append(f"kaputte Kodierung (U+FFFD): {key}")

    for key in sorted(scope_de):
        value = de.get(key)
        if isinstance(value, str):
            words = suspicious_words(value)
            if words:
                problems.append(f"Umlaut-Ersatzschreibung in de: {key}: {sorted(set(words))}")

    print(f"i18n-Pruefung Besprechungen ({label}): {len(scope_de | scope_en)} Schluessel")
    if problems:
        print(f"{len(problems)} Befund(e):")
        for line in problems:
            print("  - " + line)
        return 1
    print("OK: gleiche Schluesselmenge und Platzhalter in de/en, keine Umlaut-Ersatzschreibung.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
