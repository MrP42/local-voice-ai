"""WER/CER auf normalisiertem Text (Normalisierung: ``normalize.py``).

WER = Levenshtein auf Wortebene (Substitution + Einfuegung + Loeschung) / Referenzwoerter.
CER = Levenshtein auf Zeichenebene des normalisierten Textes (Leerzeichen zaehlen als Zeichen).
Korpus-Werte sind Summe der Fehler / Summe der Referenzeinheiten (kein Mittel der Satzwerte),
wie ``benchlib.aggregate_wer``.
"""
from __future__ import annotations

from typing import Sequence

from . import normalize as nz


def edit_distance(ref: Sequence, hyp: Sequence) -> int:
    d = list(range(len(hyp) + 1))
    for i in range(1, len(ref) + 1):
        prev, d[0] = d[0], i
        for j in range(1, len(hyp) + 1):
            cur = min(d[j] + 1, d[j - 1] + 1, prev + (ref[i - 1] != hyp[j - 1]))
            prev, d[j] = d[j], cur
    return d[len(hyp)]


def word_errors(ref: str, hyp: str) -> tuple[int, int]:
    """(Fehler, Referenzwoerter) nach ``normalize.tokens``."""
    r, h = nz.tokens(ref), nz.tokens(hyp)
    return edit_distance(r, h), len(r)


def char_errors(ref: str, hyp: str) -> tuple[int, int]:
    """(Fehler, Referenzzeichen) auf dem normalisierten Text."""
    r, h = nz.normalize(ref), nz.normalize(hyp)
    return edit_distance(r, h), len(r)


def rate(errors: int, total: int) -> float | None:
    return errors / total if total else None


def score(ref: str, hyp: str) -> dict:
    we, wn = word_errors(ref, hyp)
    ce, cn = char_errors(ref, hyp)
    return {"word_errors": we, "ref_words": wn, "char_errors": ce, "ref_chars": cn,
            "wer": rate(we, wn), "cer": rate(ce, cn)}


def aggregate(items: list[dict]) -> dict:
    """Summiert ``score``-Dicts: Korpus-WER/CER = Summe Fehler / Summe Referenz."""
    we = sum(i["word_errors"] for i in items)
    wn = sum(i["ref_words"] for i in items)
    ce = sum(i["char_errors"] for i in items)
    cn = sum(i["ref_chars"] for i in items)
    return {"n": len(items), "word_errors": we, "ref_words": wn, "char_errors": ce, "ref_chars": cn,
            "wer": rate(we, wn), "cer": rate(ce, cn)}
