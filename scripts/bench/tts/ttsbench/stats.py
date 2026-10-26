"""Kennzahlen: Perzentile (lineare Interpolation, wie numpy ``method=linear``), Streuung."""
from __future__ import annotations

import math
from typing import Iterable


def percentile(sorted_values: list[float], p: float) -> float:
    """p in [0, 100]; lineare Interpolation zwischen den Rangplaetzen (numpy-Standard)."""
    if not sorted_values:
        raise ValueError("leere Liste")
    if len(sorted_values) == 1:
        return float(sorted_values[0])
    pos = (len(sorted_values) - 1) * p / 100.0
    lo = math.floor(pos)
    hi = math.ceil(pos)
    frac = pos - lo
    return float(sorted_values[lo] + (sorted_values[hi] - sorted_values[lo]) * frac)


def summarize(values: Iterable[float | None]) -> dict | None:
    """Median, P90, P95, Mittel, Standardabweichung (Stichprobe, n-1), Min, Max, n; ``None`` bei n=0."""
    vals = sorted(float(v) for v in values if v is not None and not (isinstance(v, float) and math.isnan(v)))
    n = len(vals)
    if n == 0:
        return None
    mean = sum(vals) / n
    stdev = math.sqrt(sum((v - mean) ** 2 for v in vals) / (n - 1)) if n > 1 else 0.0
    return {
        "n": n,
        "median": percentile(vals, 50),
        "p90": percentile(vals, 90),
        "p95": percentile(vals, 95),
        "mean": mean,
        "stdev": stdev,
        "min": vals[0],
        "max": vals[-1],
    }
