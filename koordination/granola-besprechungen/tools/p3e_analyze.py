"""P3e: per-speaker breakdown of a diarization hypothesis against the script reference.

Scoring is the same as der.rs (--eval-diarization): 10 ms grid, collar +-0.25 s around
every reference boundary not scored, overlap counted, optimal 1:1 speaker mapping
(maximum overlap). The overall numbers are printed so they can be checked against the
app's eval JSON (cross-check of this re-implementation).

Additionally:
  - confusion matrix reference voice x hypothesis speaker (scored seconds)
  - per reference voice: scored speech, correct, confused (to whom), missed
  - by turn length (reference turns): share of correctly attributed frames
  - "main voices only": frames where a voice outside the given main set speaks are
    not scored -> separates "more voices than slots" from "similar voices mixed up"
  - per time window: DER with the global mapping (does the error drift / come in blocks?)
  - optional: stored segments of an --import-meeting dump (speaker per segment) against
    the reference voice with the largest overlap (majority vote per segment)

Usage:
  python p3e_analyze.py --ref R.rttm --hyp H.rttm [--hyp-raw H.raw.rttm]
         [--import-json import.json] [--main v1,v2,...] [--collar 0.25] [--window-s 60]
         --out analysis.json
stdlib only.
"""
import argparse
import itertools
import json
from collections import defaultdict

STEP = 0.01


def frame(t):
    return int(round(t / STEP))  # Python round = banker's rounding, like der.rs


def parse_rttm(path):
    segs = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            p = line.split()
            if len(p) >= 8 and p[0] == "SPEAKER":
                st, du = float(p[3]), float(p[4])
                segs.append((st, st + du, p[7]))
    return segs


def activity(segs, n):
    labels = sorted({s[2] for s in segs})
    act = {l: bytearray(n) for l in labels}
    for a, b, l in segs:
        fa, fb = max(0, frame(a)), min(n, max(0, frame(b)))
        if fb > fa:
            act[l][fa:fb] = b"\x01" * (fb - fa)
    return labels, act


def scored_mask(ref, n, collar):
    m = bytearray(b"\x01" * n)
    c = max(0, frame(collar))
    for a, b, _ in ref:
        for t in (a, b):
            f = frame(t)
            lo, hi = max(0, f - c), min(n, max(0, f + c))
            if hi > lo:
                m[lo:hi] = bytes(hi - lo)
    return m


def best_mapping(overlap, rl, hl):
    """ref label -> hyp label maximising total overlap (brute force, hyp <= 8)."""
    best, best_map = -1, {}
    k = min(len(rl), len(hl))
    for refs in itertools.permutations(rl, k):
        if len(hl) >= len(rl):
            continue
        tot = sum(overlap[r][h] for r, h in zip(refs, hl))
        if tot > best:
            best, best_map = tot, dict(zip(refs, hl))
    if len(hl) >= len(rl):
        for hyps in itertools.permutations(hl, len(rl)):
            tot = sum(overlap[r][h] for r, h in zip(rl, hyps))
            if tot > best:
                best, best_map = tot, dict(zip(rl, hyps))
    return best_map


def score(ref, hyp, collar, extra_mask=None, mapping=None, frames=None):
    end = max([s[1] for s in ref] + [s[1] for s in hyp] + [0.0])
    n = int(-(-end // STEP)) + 1
    rl, ra = activity(ref, n)
    hl, ha = activity(hyp, n)
    sm = scored_mask(ref, n, collar)
    if extra_mask is not None:
        sm = bytearray(x & y for x, y in zip(sm, extra_mask[:n].ljust(n, b"\x01")))
    lo, hi = frames if frames else (0, n)
    total = miss = fa = min_sum = 0
    overlap = {r: defaultdict(int) for r in rl}
    per_ref_frames = defaultdict(int)
    for f in range(lo, min(hi, n)):
        if not sm[f]:
            continue
        rs = [r for r in rl if ra[r][f]]
        hs = [h for h in hl if ha[h][f]]
        nr, nh = len(rs), len(hs)
        total += nr
        miss += max(0, nr - nh)
        fa += max(0, nh - nr)
        min_sum += min(nr, nh)
        for r in rs:
            per_ref_frames[r] += 1
            for h in hs:
                overlap[r][h] += 1
    if mapping is None:
        mapping = best_mapping(overlap, rl, hl)
    correct = sum(overlap[r][h] for r, h in mapping.items())
    conf = max(0, min_sum - correct)
    pct = lambda x: round(100.0 * x / total, 2) if total else 0.0
    return {
        "total_s": round(total * STEP, 2), "der": pct(miss + fa + conf), "miss": pct(miss),
        "fa": pct(fa), "conf": pct(conf), "ref_speakers": len(rl), "hyp_speakers": len(hl),
        "mapping": mapping,
        "overlap_s": {r: {h: round(v * STEP, 2) for h, v in sorted(overlap[r].items())} for r in rl},
        "per_ref_s": {r: round(per_ref_frames[r] * STEP, 2) for r in rl},
        "_n": n, "_ra": ra, "_ha": ha, "_sm": sm, "_rl": rl, "_hl": hl,
    }


def public(res):
    return {k: v for k, v in res.items() if not k.startswith("_")}


def per_voice_table(res):
    """Per reference voice, frame-wise: correct (mapped hyp speaker active), confused
    (mapped one not active, another one is), missed (no hyp speaker active). Overlap
    seconds with every hyp speaker are listed as well (hyp turns may overlap)."""
    rows = []
    mp, ra, ha, sm, n = res["mapping"], res["_ra"], res["_ha"], res["_sm"], res["_n"]
    for r in sorted(res["_rl"], key=lambda r: -res["per_ref_s"][r]):
        h = mp.get(r)
        ok = conf = miss = 0
        for f in range(n):
            if not (sm[f] and ra[r][f]):
                continue
            if h is not None and ha[h][f]:
                ok += 1
            elif any(ha[x][f] for x in res["_hl"]):
                conf += 1
            else:
                miss += 1
        tot = ok + conf + miss
        ov = res["overlap_s"][r]
        rows.append({
            "voice": r, "scored_s": round(tot * STEP, 2), "mapped_to": h,
            "correct_s": round(ok * STEP, 2), "confused_s": round(conf * STEP, 2),
            "missed_s": round(miss * STEP, 2),
            "correct_pct": round(100.0 * ok / tot, 1) if tot else 0.0,
            "overlap_with": {k: v for k, v in sorted(ov.items(), key=lambda kv: -kv[1]) if v > 0},
        })
    return rows


def by_turn_length(ref, res, buckets=(1.0, 2.0, 5.0)):
    """Share of scored frames attributed to the mapped speaker, by reference turn length."""
    mp, ha, sm, n = res["mapping"], res["_ha"], res["_sm"], res["_n"]
    edges = [0.0] + list(buckets) + [1e9]
    out = []
    for lo, hi in zip(edges, edges[1:]):
        turns = [s for s in ref if lo <= s[1] - s[0] < hi]
        tot = ok = 0
        for a, b, r in turns:
            h = mp.get(r)
            for f in range(max(0, frame(a)), min(n, frame(b))):
                if sm[f]:
                    tot += 1
                    if h is not None and ha[h][f]:
                        ok += 1
        label = "<{:g} s".format(hi) if lo == 0 else (">={:g} s".format(lo) if hi > 1e8 else "{:g}-{:g} s".format(lo, hi))
        out.append({"turn_len": label, "turns": len(turns), "scored_s": round(tot * STEP, 2),
                    "correct_pct": round(100.0 * ok / tot, 1) if tot else None})
    return out


def windows(ref, hyp, collar, mapping, window_s):
    end = max(s[1] for s in ref)
    out = []
    t = 0.0
    while t < end:
        fr = (frame(t), frame(min(end, t + window_s)))
        r = score(ref, hyp, collar, mapping=mapping, frames=fr)
        dom = max(r["per_ref_s"].items(), key=lambda kv: kv[1])[0] if r["per_ref_s"] else None
        out.append({"from_s": round(t), "to_s": round(min(end, t + window_s)), "der": r["der"],
                    "conf": r["conf"], "scored_s": r["total_s"],
                    "voices": {k: v for k, v in r["per_ref_s"].items() if v > 0}, "dominant": dom})
        t += window_s
    return out


def import_segments(ref, path):
    with open(path, encoding="utf-8-sig") as f:
        d = json.load(f)
    segs = d.get("segments", [])
    rows = []
    for s in segs:
        a, b = s["start_ms"] / 1000.0, s["end_ms"] / 1000.0
        ov = defaultdict(float)
        for ra, rb, r in ref:
            x = min(b, rb) - max(a, ra)
            if x > 0:
                ov[r] += x
        voice = max(ov.items(), key=lambda kv: kv[1])[0] if ov else None
        purity = (ov[voice] / sum(ov.values())) if ov else 0.0
        rows.append({"a": a, "b": b, "spk": s.get("speaker_index"), "ch": s.get("channel"),
                     "voice": voice, "purity": purity, "text": s.get("text", "")})
    spk_labels = sorted({str(r["spk"]) for r in rows})
    voices = sorted({r["voice"] for r in rows if r["voice"]})
    cnt = {v: defaultdict(float) for v in voices}
    for r in rows:
        if r["voice"]:
            cnt[r["voice"]][str(r["spk"])] += r["b"] - r["a"]
    # best 1:1 mapping voice -> speaker label by duration
    ov = {v: dict(cnt[v]) for v in voices}
    mp = best_mapping({v: defaultdict(float, ov[v]) for v in voices}, voices, spk_labels)
    ok = sum(r["b"] - r["a"] for r in rows if r["voice"] and mp.get(r["voice"]) == str(r["spk"]))
    tot = sum(r["b"] - r["a"] for r in rows if r["voice"])
    ok_n = sum(1 for r in rows if r["voice"] and mp.get(r["voice"]) == str(r["spk"]))
    mixed = sum(1 for r in rows if r["purity"] < 0.8)
    return {
        "meeting_status": d.get("status"), "segment_count": len(rows),
        "speaker_labels": spk_labels, "distinct_speakers": len([l for l in spk_labels if l != "None"]),
        "unassigned_segments": sum(1 for r in rows if r["spk"] is None),
        "mapping_voice_to_speaker": mp,
        "seconds_voice_x_speaker": {v: {k: round(x, 1) for k, x in sorted(cnt[v].items())} for v in voices},
        "segment_accuracy_time_pct": round(100.0 * ok / tot, 1) if tot else None,
        "segment_accuracy_count_pct": round(100.0 * ok_n / len(rows), 1) if rows else None,
        "segments_with_mixed_voices": mixed,
        "import_ms": d.get("import_ms"),
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ref", required=True)
    ap.add_argument("--hyp", required=True)
    ap.add_argument("--hyp-raw")
    ap.add_argument("--import-json")
    ap.add_argument("--main", default="")
    ap.add_argument("--collar", type=float, default=0.25)
    ap.add_argument("--window-s", type=float, default=60.0)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    ref, hyp = parse_rttm(a.ref), parse_rttm(a.hyp)
    res = score(ref, hyp, a.collar)
    out = {"ref": a.ref, "hyp": a.hyp, "collar_s": a.collar, "overall": public(res),
           "per_voice": per_voice_table(res), "by_turn_length": by_turn_length(ref, res)}
    if a.hyp_raw:
        out["overall_raw"] = public(score(ref, parse_rttm(a.hyp_raw), a.collar))
    if a.main:
        main_set = set(a.main.split(","))
        n = res["_n"]
        mask = bytearray(b"\x01" * n)
        for s0, s1, r in ref:
            if r not in main_set:
                lo, hi = max(0, frame(s0)), min(n, frame(s1))
                mask[lo:hi] = bytes(hi - lo)
        ref_main = [s for s in ref if s[2] in main_set]
        rm = score(ref_main, hyp, a.collar, extra_mask=mask)
        out["main_voices_only"] = {"voices": sorted(main_set), **public(rm), "per_voice": per_voice_table(rm)}
    out["windows"] = windows(ref, hyp, a.collar, res["mapping"], a.window_s)
    if a.import_json:
        out["import"] = import_segments(ref, a.import_json)
    with open(a.out, "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
    o = out["overall"]
    print("overall: DER {der} miss {miss} fa {fa} conf {conf} scored {total_s} s, ref {ref_speakers} hyp {hyp_speakers}".format(**o))
    print("mapping:", o["mapping"])
    for r in out["per_voice"]:
        print("  {voice:34s} {scored_s:7.1f}s -> {mapped_to}  ok {correct_pct:5.1f}%  conf {confused_s:6.1f}s  miss {missed_s:5.1f}s  {overlap_with}".format(**r))
    for b in out["by_turn_length"]:
        print("  turn", b)
    if "main_voices_only" in out:
        m = out["main_voices_only"]
        print("main only: DER {der} miss {miss} fa {fa} conf {conf} scored {total_s} s mapping {mapping}".format(**m))
    if "import" in out:
        print("import:", {k: v for k, v in out["import"].items() if k != "seconds_voice_x_speaker"})


if __name__ == "__main__":
    main()
