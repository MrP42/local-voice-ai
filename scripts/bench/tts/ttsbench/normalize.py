"""Textnormalisierung fuer WER/CER im TTS-Benchmark (Version ``norm-v1``).

Regeln und Grenzen: ``corpus/normalization.md``. Kurz:

* Kleinschreibung, Umlaute gefaltet (ae/oe/ue/ss) wie ``benchlib.normalize_word``, uebrige
  Akzente entfernt, Satzzeichen weg, Bindestriche/Schraegstriche trennen Woerter.
* Zahlen: Ziffern UND deutsche Zahlwoerter werden auf EINE Zahl-Zeichenfolge gebracht
  ("dreiundvierzig" -> ``43``, ``12,50 EUR`` -> ``12 euro 50``, ``3,5`` -> ``3 komma 5``).
  Ordinalzahlen werden zu ``<n>te`` (``3.`` vor Monatsnamen, ``dritten`` -> ``3te``).
* Akronyme: buchstabierte Folgen (``D S G V O``, ``de es ge vau o``) werden zu EINEM Token
  (``dsgvo``); als Wort gesprochene Akronyme (``NATO``) sind ohnehin ein Token.
* E-Mail/URL werden in gesprochene Form gebracht (``@`` -> ``at``, ``.`` -> ``punkt``, ``/`` -> ``slash``).

Die Normalisierung ist idempotent: ``normalize(normalize(t)) == normalize(t)``.
"""
from __future__ import annotations

import re
import unicodedata

NORM_VERSION = "norm-v1"

# --------------------------------------------------------------------------- Falten


def fold(text: str) -> str:
    """Kleinschreibung + Umlautfaltung (wie benchlib) + Akzente entfernen."""
    t = unicodedata.normalize("NFC", text).lower()
    t = t.replace("ä", "ae").replace("ö", "oe").replace("ü", "ue").replace("ß", "ss")
    t = unicodedata.normalize("NFD", t)
    t = "".join(c for c in t if not unicodedata.combining(c))
    return unicodedata.normalize("NFC", t)


# --------------------------------------------------------------------------- Zahlwoerter

_ONES = {"null": 0, "ein": 1, "eins": 1, "zwei": 2, "zwo": 2, "drei": 3, "vier": 4, "fuenf": 5,
         "sechs": 6, "sieben": 7, "acht": 8, "neun": 9}
_TEENS = {"zehn": 10, "elf": 11, "zwoelf": 12, "dreizehn": 13, "vierzehn": 14, "fuenfzehn": 15,
          "sechzehn": 16, "siebzehn": 17, "achtzehn": 18, "neunzehn": 19}
_TENS = {"zwanzig": 20, "dreissig": 30, "vierzig": 40, "fuenfzig": 50, "sechzig": 60,
         "siebzig": 70, "achtzig": 80, "neunzig": 90}
_UND_ONES = "|".join(k for k in _ONES if k not in ("null", "eins"))
_RE_ONES_UND_TENS = re.compile(rf"^({_UND_ONES})und({'|'.join(_TENS)})$")


def _below100(s: str) -> int | None:
    if s in _TEENS:
        return _TEENS[s]
    if s in _TENS:
        return _TENS[s]
    m = _RE_ONES_UND_TENS.match(s)
    if m:
        return _ONES[m.group(1)] + _TENS[m.group(2)]
    if s in _ONES and s != "null":
        return _ONES[s]
    return None


def _below1000(s: str, allow_teen_hundert: bool) -> int | None:
    if "hundert" in s:
        left, right = s.split("hundert", 1)
        if left == "":
            h = 1
        elif left in _ONES and left not in ("null", "eins"):
            h = _ONES[left]
        elif allow_teen_hundert and left in _TEENS and 11 <= _TEENS[left] <= 19:
            h = _TEENS[left]  # neunzehnhundert... (Jahreszahlen)
        else:
            return None
        if right.startswith("und"):
            right = right[3:]
        if right == "":
            return h * 100
        r = _below100(right)
        if r is None:
            return None
        return h * 100 + r
    return _below100(s)


def parse_cardinal(s: str) -> int | None:
    """Gefaltetes Zahlwort (zusammengeschrieben) -> Wert; ``None`` wenn kein gueltiges Zahlwort.

    Unterstuetzt 0 bis 999 999 sowie ``neunzehnhundert..`` (Jahreszahlen); Millionen/Milliarden
    sind eigene Woerter (siehe ``_pass_millions``).
    """
    if not s:
        return None
    if s == "null":
        return 0
    if "tausend" in s:
        left, right = s.split("tausend", 1)
        if left == "":
            t = 1
        else:
            t = _below1000(left, False)
            if t is None or t == 0:
                return None
        if right.startswith("und"):
            right = right[3:]
        if right == "":
            return t * 1000
        r = _below1000(right, False)
        if r is None:
            return None
        return t * 1000 + r
    return _below1000(s, True)


_ORD_IRREG = {"erste": 1, "dritte": 3, "siebte": 7, "achte": 8}


def _ordinal_base_value(base: str) -> int | None:
    if base in _ORD_IRREG:
        return _ORD_IRREG[base]
    if base.endswith("ste"):
        n = parse_cardinal(base[:-3])
        if n is not None and n >= 20:
            return n
    if base.endswith("te"):
        n = parse_cardinal(base[:-2])
        if n is not None and 2 <= n <= 19:
            return n
        if n is not None and n >= 100 and 1 <= n % 100 <= 19:
            return n
    return None


def ordinal_value(word: str) -> int | None:
    """``dritten`` / ``sechsundzwanzigste`` -> Wert, sonst ``None`` (Wort bereits gefaltet)."""
    if word.endswith(("te", "ste")):
        return _ordinal_base_value(word)
    if word.endswith(("ten", "ter", "tes", "tem")):
        return _ordinal_base_value(word[:-1])
    return None


# --------------------------------------------------------------------------- Vorverarbeitung (Zeichenfolge)

_MONTHS_LIST = ["januar", "februar", "maerz", "april", "mai", "juni", "juli", "august",
                "september", "oktober", "november", "dezember"]
_MONTHS = "|".join(_MONTHS_LIST)
_ORD_NOUNS = ("jahrhundert|jahrtausend|platz|stock|etage|klasse|geburtstag|advent|auflage|mal|"
              "bezirk|liga|weltkrieg")
_NUM_INT = r"\d{1,3}(?:\.\d{3})+|\d+"
_TLDS = "org|com|de|net|eu|io|info|at|ch|dev|app"

_EMAIL = re.compile(r"(?<![\w.+-])[a-z0-9][a-z0-9._%+-]*@[a-z0-9-]+(?:\.[a-z0-9-]+)+")
_URL = re.compile(
    rf"(?<![\w@.-])(?:https?://)?(?:[a-z0-9-]+\.)+(?:{_TLDS})(?![a-z0-9-])(?:/[^\s,;!?()\"'“”„]*)?")
_ABBR = [(re.compile(p), r) for p, r in [
    (r"\bz\.\s*b\.", " zum beispiel "), (r"\bd\.\s*h\.", " das heisst "),
    (r"\bu\.\s*a\.", " unter anderem "), (r"\bu\.\s*s\.\s*w\.", " und so weiter "),
    (r"\busw\.", " und so weiter "), (r"\bca\.", " circa "), (r"\bbzw\.", " beziehungsweise "),
    (r"\bggf\.", " gegebenenfalls "), (r"\bevtl\.", " eventuell "), (r"\binkl\.", " inklusive "),
    (r"\bvgl\.", " vergleiche "), (r"\bnr\.", " nummer "),
]]
_DATE = re.compile(r"(?<![\d.,])(\d{1,2})\.(\d{1,2})\.(\d{4})?(?!\d)")
_ORD_RANGE = re.compile(rf"(?<![\d.,])(\d{{1,2}})\.(?=\s+(?:bis|und)\s+\d{{1,2}}\.\s*(?:{_MONTHS})\b)")
_ORD_CTX = re.compile(rf"(?<![\d.,])(\d{{1,3}})\.(?=\s*(?:{_MONTHS}|{_ORD_NOUNS})\b)")
_TIME = re.compile(
    r"(?<![\d.,:])(\d{1,2}):(\d{2})(?!\d)(?:\s*uhr\b)?|(?<![\d.,:])(\d{1,2})\.(\d{2})\s*uhr\b")
_CURRENCY = re.compile(rf"(?<![\d.,])({_NUM_INT}),(\d{{2}})\s*(?:€|euro\b)")
_DECIMAL = re.compile(rf"(?<![\d.,])({_NUM_INT}),(\d+)(?!\d)")
_THOUSANDS = re.compile(r"(?<![\d.,])\d{1,3}(?:\.\d{3})+(?!\d)")
_VERSION = re.compile(r"(?<![\d.,:])(\d+)\.(\d{1,2})(?!\d)(?!\.\d)(?!\w)")
_UNITS = [(re.compile(p), r) for p, r in [
    (r"km\s*/\s*h", " kilometer pro stunde "), (r"°\s*c\b", " grad celsius "), (r"°", " grad "),
    (r"%", " prozent "), (r"€", " euro "), (r"\$", " dollar "), (r"&", " und "), (r"§", " paragraf "),
    (r"(?<=\d)\s*kg\b", " kilogramm "), (r"(?<=\d)\s*km\b", " kilometer "),
    (r"(?<=\d)\s*cm\b", " zentimeter "), (r"(?<=\d)\s*mm\b", " millimeter "),
]]
_APOSTROPHE = re.compile(r"(?<=\w)['’](?=\w)")
_BOUNDARY = re.compile(r"[.,;:!?\"“”„«»‹›‘’‚'()\[\]{}…|]")
_SPACE_CHARS = re.compile(r"[-‐‑–—/*#_~=<>+\\]")


def _spoken_address(s: str) -> str:
    s = s.replace("://", " ")
    for a, b in (("@", " at "), (".", " punkt "), ("/", " slash "), ("_", " unterstrich "),
                 ("+", " plus "), ("-", " "), (":", " ")):
        s = s.replace(a, b)
    return f" {s} "


def _sub_url(m: re.Match) -> str:
    s = m.group(0)
    tail = ""
    while s and s[-1] in ".:":
        tail = s[-1] + tail
        s = s[:-1]
    return _spoken_address(s) + tail


def _sub_date(m: re.Match) -> str:
    d, mo, y = int(m.group(1)), int(m.group(2)), m.group(3)
    if not (1 <= d <= 31 and 1 <= mo <= 12):
        return m.group(0)
    return f" {d}te {_MONTHS_LIST[mo - 1]}" + (f" {y}" if y else "") + " "


def _sub_time(m: re.Match) -> str:
    h, mi = (m.group(1), m.group(2)) if m.group(1) is not None else (m.group(3), m.group(4))
    h, mi = int(h), int(mi)
    if h > 24 or mi > 59:
        return m.group(0)
    return f" {h} uhr" + (f" {mi}" if mi else "") + " "


def _sub_currency(m: re.Match) -> str:
    euro = m.group(1).replace(".", "")
    cents = int(m.group(2))
    return f" {euro} euro" + (f" {cents}" if cents else "") + " "


def _sub_decimal(m: re.Match) -> str:
    return f" {m.group(1).replace('.', '')} komma {m.group(2)} "


def _prepare(text: str) -> str:
    t = fold(text)
    t = _EMAIL.sub(lambda m: _spoken_address(m.group(0)), t)
    t = _URL.sub(_sub_url, t)
    for rx, repl in _ABBR:
        t = rx.sub(repl, t)
    t = _DATE.sub(_sub_date, t)
    t = _ORD_RANGE.sub(lambda m: f" {int(m.group(1))}te ", t)
    t = _ORD_CTX.sub(lambda m: f" {int(m.group(1))}te ", t)
    t = _TIME.sub(_sub_time, t)
    t = _CURRENCY.sub(_sub_currency, t)
    t = _DECIMAL.sub(_sub_decimal, t)
    t = _THOUSANDS.sub(lambda m: f" {m.group(0).replace('.', '')} ", t)
    t = _VERSION.sub(lambda m: f" {m.group(1)} punkt {m.group(2)} ", t)
    for rx, repl in _UNITS:
        t = rx.sub(repl, t)
    t = _APOSTROPHE.sub("", t)
    t = _BOUNDARY.sub(" | ", t)
    t = _SPACE_CHARS.sub(" ", t)
    return t


# --------------------------------------------------------------------------- Token-Durchlaeufe

_ARTICLE_ALIAS = {"eine": "ein"}
_MAX_RUN = 6


def _pass_ordinals(toks: list[str]) -> list[str]:
    out = []
    for t in toks:
        v = ordinal_value(t) if not t.isdigit() else None
        out.append(f"{v}te" if v is not None else t)
    return out


def _numeric_piece(t: str) -> bool:
    return parse_cardinal(_ARTICLE_ALIAS.get(t, t)) is not None


def _pass_cardinals(toks: list[str]) -> list[str]:
    """Laengster zusammenhaengender Lauf von Zahlwoertern (auch getrennt geschrieben) -> Ziffern."""
    out: list[str] = []
    i = 0
    n = len(toks)
    while i < n:
        done = False
        for j in range(min(n, i + _MAX_RUN), i, -1):
            run = toks[i:j]
            if j - i > 1 and (run[-1] == "und" or run[0] == "und"):
                continue
            if not all(_numeric_piece(t) or (t == "und" and 0 < k < len(run) - 1) for k, t in enumerate(run)):
                continue
            if j - i == 1 and run[0] in _ARTICLE_ALIAS.keys() | {"ein"}:
                break  # einzelnes "ein"/"eine" ist meist Artikel: siehe _pass_ein_measure
            joined = "".join(_ARTICLE_ALIAS.get(t, t) for t in run)
            v = parse_cardinal(joined)
            if v is None:
                continue
            out.append(str(v))
            i = j
            done = True
            break
        if not done:
            out.append(toks[i])
            i += 1
    return out


_MEASURE = {
    "euro", "cent", "dollar", "uhr", "prozent", "million", "millionen", "milliarde", "milliarden",
    "kilometer", "kilogramm", "meter", "zentimeter", "millimeter", "stunde", "stunden", "minute",
    "minuten", "sekunde", "sekunden", "tag", "tage", "woche", "wochen", "monat", "monate", "jahr",
    "jahre", "grad", "mal", "gigabyte", "megabyte", "terabyte",
}


def _pass_ein_measure(toks: list[str]) -> list[str]:
    """``ein``/``eine`` vor einem Mass-/Mengenwort ist die Zahl 1 (ASR schreibt ``1 Euro``)."""
    return ["1" if t in ("ein", "eine") and k + 1 < len(toks) and toks[k + 1] in _MEASURE else t
            for k, t in enumerate(toks)]


def _single_digit(t: str) -> bool:
    return len(t) == 1 and t.isdigit()


def _pass_digit_runs(toks: list[str]) -> list[str]:
    """``komma`` + Einzelziffern -> eine Ziffernfolge; sonst >= 3 Einzelziffern -> eine Ziffernfolge."""
    out: list[str] = []
    i, n = 0, len(toks)
    while i < n:
        if toks[i] == "komma":
            out.append("komma")
            j = i + 1
            while j < n and _single_digit(toks[j]):
                j += 1
            if j > i + 1:
                out.append("".join(toks[i + 1:j]))
            i = j if j > i + 1 else i + 1
            continue
        if _single_digit(toks[i]):
            j = i
            while j < n and _single_digit(toks[j]):
                j += 1
            run = toks[i:j]
            if len(run) >= 3:
                out.append("".join(run))
            else:
                out.extend(run)
            i = j
            continue
        out.append(toks[i])
        i += 1
    return out


_MILLION = {"million": 10**6, "millionen": 10**6, "milliarde": 10**9, "milliarden": 10**9}


def _pass_millions(toks: list[str]) -> list[str]:
    """``2 millionen 300000`` -> ``2300000`` (Ziffernform ``2.300.000`` ist bereits eine Zahl)."""
    out: list[str] = []
    i, n = 0, len(toks)
    while i < n:
        t = toks[i]
        prev_komma = bool(out) and out[-1] == "komma"
        if t.isdigit() and not prev_komma and i + 1 < n and toks[i + 1] in _MILLION:
            total = int(t) * _MILLION[toks[i + 1]]
            factor = _MILLION[toks[i + 1]]
            j = i + 2
            while j < n and toks[j].isdigit() and int(toks[j]) < factor:
                if j + 1 < n and toks[j + 1] in _MILLION and _MILLION[toks[j + 1]] < factor:
                    total += int(toks[j]) * _MILLION[toks[j + 1]]
                    factor = _MILLION[toks[j + 1]]
                    j += 2
                    continue
                total += int(toks[j])
                j += 1
                break
            out.append(str(total))
            i = j
            continue
        out.append(t)
        i += 1
    return out


_LETTER_NAMES = {
    "a": "a", "be": "b", "ce": "c", "de": "d", "e": "e", "ef": "f", "ge": "g", "ha": "h", "i": "i",
    "jot": "j", "ka": "k", "el": "l", "em": "m", "en": "n", "o": "o", "pe": "p", "ku": "q",
    "er": "r", "es": "s", "te": "t", "u": "u", "vau": "v", "we": "w", "ix": "x", "ypsilon": "y",
    "zet": "z", "zett": "z",
}
_STRICT_NAMES = {"be", "ce", "ef", "ge", "ha", "jot", "ka", "ku", "pe", "vau", "we", "ix",
                 "ypsilon", "zet", "zett", "el", "em"}


def _pass_letter_names(toks: list[str]) -> list[str]:
    """>= 3 aufeinanderfolgende Buchstabennamen (mind. 2 eindeutige) -> ein Akronym-Token."""
    out: list[str] = []
    i, n = 0, len(toks)
    while i < n:
        j = i
        while j < n and toks[j] in _LETTER_NAMES:
            j += 1
        run = toks[i:j]
        if len(run) >= 3 and sum(t in _STRICT_NAMES for t in run) >= 2:
            out.append("".join(_LETTER_NAMES[t] for t in run))
            i = j
        else:
            out.append(toks[i])
            i += 1
    return out


def _single_letter(t: str) -> bool:
    return len(t) == 1 and t.isalpha() and t.isascii()


def _pass_single_letters(toks: list[str]) -> list[str]:
    """>= 2 einzelne Buchstaben in Folge (``K I``, ``D-S-G-V-O``) -> ein Token."""
    out: list[str] = []
    i, n = 0, len(toks)
    while i < n:
        if _single_letter(toks[i]):
            j = i
            while j < n and _single_letter(toks[j]):
                j += 1
            run = toks[i:j]
            out.append("".join(run) if len(run) >= 2 else run[0])
            i = j
        else:
            out.append(toks[i])
            i += 1
    return out


# --------------------------------------------------------------------------- Oeffentliche API


def tokens(text: str) -> list[str]:
    """Normalisierte Tokenliste fuer WER (Satzzeichen trennen Zahl-/Buchstabenlaeufe)."""
    segments: list[list[str]] = [[]]
    for raw in _prepare(text).split():
        if raw == "|":
            segments.append([])
            continue
        tok = re.sub(r"[^\w]|_", "", raw)
        if tok:
            segments[-1].append(tok)
    result: list[str] = []
    for seg in segments:
        seg = _pass_ordinals(seg)
        seg = _pass_cardinals(seg)
        seg = _pass_digit_runs(seg)
        seg = _pass_ein_measure(seg)
        seg = _pass_millions(seg)
        seg = _pass_letter_names(seg)
        seg = _pass_single_letters(seg)
        result.extend(seg)
    return result


def normalize(text: str) -> str:
    return " ".join(tokens(text))
