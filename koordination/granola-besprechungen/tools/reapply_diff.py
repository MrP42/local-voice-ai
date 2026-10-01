"""Aenderungen eines Zweigs an EINER Datei auf einen neueren Stand neu anwenden.

Fuer additive Sammeldateien (bindings.ts), bei denen Gits Konfliktgrenzen mitten in
Funktionen liegen. Nimmt den Diff base->zweig und setzt jeden Hunk anhand von drei
Kontextzeilen in die Zieldatei ein; nicht verankerbare Hunks werden gemeldet.

Aufruf (im Worktree, nach `git rebase` mit Konflikt in <datei>):
  python reapply_diff.py <datei> <base-commit> <zweig-commit> <ziel-ref>
Schreibt <datei> neu (= Ziel-Stand + Zweig-Aenderungen). Danach tsc/Tests laufen lassen.
"""

import difflib
import subprocess
import sys


def show(ref: str, path: str) -> list[str]:
    raw = subprocess.run(["git", "show", f"{ref}:{path}"], capture_output=True, check=True).stdout
    return raw.decode("utf-8").split("\n")


def main() -> int:
    path, base_ref, branch_ref, target_ref = sys.argv[1:5]
    base, branch, out = show(base_ref, path), show(branch_ref, path), show(target_ref, path)
    ops = difflib.SequenceMatcher(None, base, branch, autojunk=False).get_opcodes()
    missing = 0
    for tag, i1, i2, j1, j2 in reversed(ops):
        if tag == "equal":
            continue
        old, new, ctx = base[i1:i2], branch[j1:j2], base[max(0, i1 - 3):i1]
        pos = next((k + len(ctx) for k in range(len(out))
                    if out[k:k + len(ctx)] == ctx and out[k + len(ctx):k + len(ctx) + len(old)] == old), None)
        if pos is None:
            missing += 1
            print(f"KEIN ANKER ({tag}, Basiszeile {i1}):")
            print("\n".join("  + " + line for line in new[:12]))
            continue
        out[pos:pos + len(old)] = new
    with open(path, "w", encoding="utf-8", newline="") as fh:
        fh.write("\n".join(out))
    print(f"fertig, {missing} Hunk(s) von Hand nachziehen")
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
