"""Pruefung der ``rows.jsonl``-Zeilen gegen ``schema/row.schema.json`` (ohne jsonschema-Abhaengigkeit).

Unterstuetzt die im Schema benutzten Schluesselwoerter: type (auch als Liste), required, properties,
enum, const, minimum, minLength, items. Dazu die semantischen Regeln aus der Schema-Beschreibung.
"""
from __future__ import annotations

import json
from pathlib import Path

SCHEMA_DIR = Path(__file__).resolve().parents[1] / "schema"
ROW_SCHEMA_PATH = SCHEMA_DIR / "row.schema.json"
ROW_SCHEMA_ID = "tts-bench-row/1"
ENV_SCHEMA_ID = "tts-bench-env/1"

_TYPES = {
    "string": lambda v: isinstance(v, str),
    "number": lambda v: isinstance(v, (int, float)) and not isinstance(v, bool),
    "integer": lambda v: isinstance(v, int) and not isinstance(v, bool),
    "boolean": lambda v: isinstance(v, bool),
    "null": lambda v: v is None,
    "array": lambda v: isinstance(v, list),
    "object": lambda v: isinstance(v, dict),
}


def load_schema(path: Path = ROW_SCHEMA_PATH) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def _check(value, schema: dict, where: str, errors: list[str]) -> None:
    t = schema.get("type")
    if t is not None:
        names = t if isinstance(t, list) else [t]
        if not any(_TYPES[n](value) for n in names):
            errors.append(f"{where}: Typ {type(value).__name__} nicht in {names}")
            return
    if "const" in schema and value != schema["const"]:
        errors.append(f"{where}: erwartet {schema['const']!r}, ist {value!r}")
    if "enum" in schema and value not in schema["enum"]:
        errors.append(f"{where}: {value!r} nicht in {schema['enum']}")
    if isinstance(value, (int, float)) and not isinstance(value, bool) and "minimum" in schema \
            and value < schema["minimum"]:
        errors.append(f"{where}: {value} < {schema['minimum']}")
    if isinstance(value, str) and "minLength" in schema and len(value) < schema["minLength"]:
        errors.append(f"{where}: zu kurz")
    if isinstance(value, list) and "items" in schema:
        for k, v in enumerate(value):
            _check(v, schema["items"], f"{where}[{k}]", errors)


def validate_row(row: dict, schema: dict | None = None) -> list[str]:
    """Fehlerliste; leer = gueltig."""
    schema = schema or load_schema()
    errors: list[str] = []
    if not isinstance(row, dict):
        return ["Zeile ist kein Objekt"]
    for req in schema.get("required", []):
        if req not in row:
            errors.append(f"Pflichtfeld '{req}' fehlt")
    for key, sub in schema.get("properties", {}).items():
        if key in row:
            _check(row[key], sub, key, errors)
    if errors:
        return errors
    if row["ok"]:
        for f in ("ttfa_ms", "total_ms", "audio_ms", "rtf", "sample_rate", "channels", "encoding", "chunk_count"):
            if row.get(f) is None:
                errors.append(f"ok=true, aber '{f}' fehlt oder ist null")
    else:
        for f in ("error_kind", "error"):
            if not row.get(f):
                errors.append(f"ok=false, aber '{f}' fehlt")
    if row["run_kind"] == "warm" and row.get("process_start_ms") is not None:
        errors.append("process_start_ms gibt es nur bei run_kind=cold")
    gaps = row.get("chunk_gaps_ms")
    if gaps is not None and row.get("chunk_count") is not None and row["ok"] \
            and len(gaps) != max(0, row["chunk_count"] - 1):
        errors.append("chunk_gaps_ms hat nicht chunk_count-1 Eintraege")
    return errors


def read_rows(path: Path) -> tuple[list[dict], list[str]]:
    """Liest rows.jsonl; gibt (gueltige Zeilen, Problemliste) zurueck. Ungueltige Zeilen werden nicht verwendet."""
    schema = load_schema()
    rows: list[dict] = []
    problems: list[str] = []
    if not path.exists():
        return rows, [f"{path} fehlt"]
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError as e:
            problems.append(f"Zeile {n}: kein JSON ({e})")
            continue
        errs = validate_row(row, schema)
        if errs:
            problems.append(f"Zeile {n}: " + "; ".join(errs))
            continue
        rows.append(row)
    return rows, problems
