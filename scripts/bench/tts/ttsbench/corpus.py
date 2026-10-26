"""Korpus laden und pruefen (``corpus/<name>.jsonl``, versioniert, Saetze werden nie geaendert)."""
from __future__ import annotations

import hashlib
import json
import re
from collections import Counter
from pathlib import Path

from . import normalize as nz

TTS_DIR = Path(__file__).resolve().parents[1]
CORPUS_DIR = TTS_DIR / "corpus"

REQUIRED_FIELDS = ("id", "category", "text", "expected_normalized", "smoke")
CATEGORIES = ("alltag", "lang", "zahlen", "datum", "akronyme", "codeswitch", "fachbegriffe",
              "email_url", "fragen", "ausdruck")
_ID_RE = re.compile(r"^[a-z]{2}-v\d+-[a-z_]+-\d{3}$")
_EMAIL_RE = re.compile(r"[\w.+-]+@([\w-]+(?:\.[\w-]+)+)")
_SAFE_DOMAINS = ("example.org", "example.com", "example.net", "example.de")
_PHONE_RE = re.compile(r"(?<!\d)(?:\+?\d[\d /-]{8,}\d)(?!\d)")


def resolve_path(name_or_path: str) -> Path:
    p = Path(name_or_path)
    if p.suffix == ".jsonl" or p.exists():
        return p
    return CORPUS_DIR / f"{name_or_path}.jsonl"


def sha256_of(path: Path) -> str:
    """Hash ueber den Inhalt mit LF-Zeilenenden (CRLF-Checkouts liefern denselben Wert)."""
    data = path.read_bytes().replace(b"\r\n", b"\n")
    return hashlib.sha256(data).hexdigest()


def load(name_or_path: str) -> list[dict]:
    path = resolve_path(name_or_path)
    if not path.exists():
        raise FileNotFoundError(f"Korpus nicht gefunden: {path}")
    items = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            items.append(json.loads(line))
        except json.JSONDecodeError as e:
            raise ValueError(f"{path}:{n}: kein gueltiges JSON ({e})") from e
    return items


def subset(items: list[dict], name: str | None) -> list[dict]:
    if not name or name in ("full", "all"):
        return list(items)
    if name == "smoke":
        return [i for i in items if i.get("smoke")]
    raise ValueError(f"unbekannte Teilmenge '{name}' (erlaubt: smoke, full)")


def by_id(items: list[dict]) -> dict[str, dict]:
    return {i["id"]: i for i in items}


def info(name_or_path: str) -> dict:
    path = resolve_path(name_or_path)
    items = load(name_or_path)
    return {"name": path.stem, "path": str(path), "sha256": sha256_of(path), "count": len(items),
            "smoke_count": sum(1 for i in items if i.get("smoke"))}


def validate(items: list[dict], *, min_sentences: int = 100, min_per_category: int = 10,
             smoke_size: int | None = 12, required_categories: tuple[str, ...] | None = CATEGORIES,
             check_spoken: bool = True) -> tuple[list[str], dict]:
    """Gibt (Fehlerliste, Kennzahlen) zurueck; leere Fehlerliste = gueltig."""
    errors: list[str] = []
    if len(items) < min_sentences:
        errors.append(f"nur {len(items)} Saetze (< {min_sentences})")
    seen: set[str] = set()
    texts: Counter = Counter()
    cats: Counter = Counter()
    smoke_cats: Counter = Counter()
    for k, it in enumerate(items, 1):
        where = it.get("id", f"#{k}")
        for f in REQUIRED_FIELDS:
            if f not in it:
                errors.append(f"{where}: Feld '{f}' fehlt")
        if any(f not in it for f in REQUIRED_FIELDS):
            continue
        if not isinstance(it["smoke"], bool):
            errors.append(f"{where}: 'smoke' ist kein bool")
        for f in ("id", "category", "text", "expected_normalized"):
            if not isinstance(it[f], str) or not it[f].strip():
                errors.append(f"{where}: Feld '{f}' leer oder kein String")
        if not isinstance(it["id"], str) or not isinstance(it["text"], str):
            continue
        if it["id"] in seen:
            errors.append(f"{where}: doppelte ID")
        seen.add(it["id"])
        if not _ID_RE.match(it["id"]):
            errors.append(f"{where}: ID entspricht nicht <sprache>-v<n>-<kategorie>-<nnn>")
        elif f"-{it['category']}-" not in it["id"]:
            errors.append(f"{where}: ID passt nicht zur Kategorie '{it['category']}'")
        cats[it["category"]] += 1
        if it["smoke"] is True:
            smoke_cats[it["category"]] += 1
        texts[it["text"]] += 1
        norm = nz.normalize(it["text"])
        if norm != it["expected_normalized"]:
            errors.append(f"{where}: normalize(text) = '{norm}' != expected_normalized")
        if nz.normalize(it["expected_normalized"]) != it["expected_normalized"]:
            errors.append(f"{where}: expected_normalized ist nicht idempotent")
        if check_spoken and "spoken" in it and nz.normalize(it["spoken"]) != it["expected_normalized"]:
            errors.append(f"{where}: normalize(spoken) = '{nz.normalize(it['spoken'])}' != expected_normalized")
        for dom in _EMAIL_RE.findall(it["text"]):
            if dom.lower().rstrip(".") not in _SAFE_DOMAINS:
                errors.append(f"{where}: E-Mail-Domain '{dom}' ist keine Beispiel-Domain")
        if _PHONE_RE.search(it["text"]):
            errors.append(f"{where}: Text enthaelt eine telefonnummernartige Ziffernfolge")
    for t, c in texts.items():
        if c > 1:
            errors.append(f"doppelter Text: '{t[:40]}...'")
    if required_categories is not None:
        for c in required_categories:
            if cats[c] < min_per_category:
                errors.append(f"Kategorie '{c}': {cats[c]} Saetze (< {min_per_category})")
            if smoke_size is not None and smoke_cats[c] < 1:
                errors.append(f"Smoke-Teilmenge enthaelt keinen Satz der Kategorie '{c}'")
        unknown = set(cats) - set(required_categories)
        if unknown:
            errors.append(f"unbekannte Kategorien: {sorted(unknown)}")
    n_smoke = sum(smoke_cats.values())
    if smoke_size is not None and n_smoke != smoke_size:
        errors.append(f"Smoke-Teilmenge hat {n_smoke} Saetze (erwartet {smoke_size})")
    stats = {"count": len(items), "categories": dict(cats), "smoke": n_smoke,
             "words_total": sum(len(i["text"].split()) for i in items if isinstance(i.get("text"), str))}
    return errors, stats
