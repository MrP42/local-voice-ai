"""Messkonfigurationen aus ``configs/*.toml`` (tomllib, Python >= 3.11)."""
from __future__ import annotations

import tomllib
from dataclasses import dataclass, field, asdict
from pathlib import Path

TTS_DIR = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = TTS_DIR / "configs" / "default.toml"

STATUSES = ("run", "not_tested", "unsupported", "license_restricted")
DEVICES = ("cpu", "cuda", "metal", "remote", "any")


class ConfigError(ValueError):
    pass


@dataclass
class BenchConfig:
    id: str
    provider: str
    voice: str | None = None
    language: str = "de"
    device: str = "any"            # erwartetes Geraet; wird gegen env.json des Laufs geprueft
    status: str = "run"            # run | not_tested | unsupported | license_restricted
    reason: str = ""               # Pflicht bei status != run
    corpus: str = "de-v1"
    subset: str = "smoke"          # smoke | full
    warm_runs: int = 30            # Warmlaeufe JE Satz
    cold_runs: int = 3             # Kaltstarts JE Satz
    quality: bool = True           # False: kein ASR/WER (z. B. Funktionsnachweis in anderer Sprache)
    memory_limit_mb: int = 16384   # Job-Objekt-Deckel fuer die App samt Kindprozessen
    max_processes: int = 24
    timeout_s: int = 7200
    note: str = ""

    @property
    def uses_gpu(self) -> bool:
        return self.device == "cuda"

    def to_dict(self) -> dict:
        return asdict(self)


def _build(raw: dict, defaults: dict, idx: int) -> BenchConfig:
    merged = {**defaults, **raw}
    known = BenchConfig.__dataclass_fields__
    unknown = set(merged) - set(known)
    if unknown:
        raise ConfigError(f"config[{idx}] ({merged.get('id', '?')}): unbekannte Felder {sorted(unknown)}")
    for req in ("id", "provider"):
        if not merged.get(req):
            raise ConfigError(f"config[{idx}]: Feld '{req}' fehlt")
    if merged.get("voice") == "":
        merged["voice"] = None
    cfg = BenchConfig(**merged)
    if cfg.status not in STATUSES:
        raise ConfigError(f"{cfg.id}: status '{cfg.status}' ungültig (erlaubt: {', '.join(STATUSES)})")
    if cfg.device not in DEVICES:
        raise ConfigError(f"{cfg.id}: device '{cfg.device}' ungültig (erlaubt: {', '.join(DEVICES)})")
    if cfg.status != "run" and not cfg.reason.strip():
        raise ConfigError(f"{cfg.id}: status '{cfg.status}' braucht einen Grund (reason)")
    if cfg.subset not in ("smoke", "full"):
        raise ConfigError(f"{cfg.id}: subset '{cfg.subset}' ungültig (smoke | full)")
    if cfg.warm_runs < 0 or cfg.cold_runs < 0 or cfg.warm_runs + cfg.cold_runs == 0 and cfg.status == "run":
        raise ConfigError(f"{cfg.id}: warm_runs/cold_runs ungültig")
    return cfg


def load(path: str | Path | None = None) -> list[BenchConfig]:
    p = Path(path) if path else DEFAULT_CONFIG
    with p.open("rb") as f:
        data = tomllib.load(f)
    defaults = data.get("defaults", {})
    entries = data.get("config", [])
    if not entries:
        raise ConfigError(f"{p}: keine [[config]]-Eintraege")
    cfgs = [_build(e, defaults, i) for i, e in enumerate(entries)]
    ids = [c.id for c in cfgs]
    dup = {i for i in ids if ids.count(i) > 1}
    if dup:
        raise ConfigError(f"doppelte Konfigurations-IDs: {sorted(dup)}")
    return cfgs


def select(cfgs: list[BenchConfig], wanted: list[str] | None) -> list[BenchConfig]:
    if not wanted:
        return cfgs
    by = {c.id: c for c in cfgs}
    missing = [w for w in wanted if w not in by]
    if missing:
        raise ConfigError(f"unbekannte Konfigurationen {missing}; vorhanden: {', '.join(by)}")
    return [by[w] for w in wanted]
