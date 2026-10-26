"""Tests des TTS-Benchmarks: python -m pytest scripts/bench/tts -q

Kein Netz, keine GPU, keine App, kein ASR-Modell: Laeufe nutzen ``--source fake`` und ``--engine fake``.
Alle Ablagen liegen in tmp_path (LVA_BENCH_DIR), nie in produktiven Daten oder im Repo.
"""
import csv
import json
import subprocess
import sys
import time
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(HERE.parent))
import benchlib  # noqa: E402
import tts_bench  # noqa: E402
from ttsbench import config as cfgmod, corpus as corpus_mod, listening, report, rowschema, runner  # noqa: E402
from ttsbench import normalize as nz, sampler, sources, stats, textmetrics  # noqa: E402

N = nz.normalize


@pytest.fixture
def bench_env(tmp_path, monkeypatch):
    monkeypatch.setenv("LVA_BENCH_DIR", str(tmp_path / "bench"))
    return tmp_path


# --------------------------------------------------------------------------- Korpus


def test_corpus_schema_and_counts():
    items = corpus_mod.load("de-v1")
    assert len(items) >= 100
    ids = [i["id"] for i in items]
    assert len(set(ids)) == len(ids)
    cats = {}
    for i in items:
        assert set(corpus_mod.REQUIRED_FIELDS) <= set(i)
        assert isinstance(i["smoke"], bool)
        assert i["id"].startswith("de-v1-" + i["category"] + "-")
        cats[i["category"]] = cats.get(i["category"], 0) + 1
    assert set(cats) == set(corpus_mod.CATEGORIES)
    assert all(v >= 10 for v in cats.values()), cats


def test_corpus_smoke_subset_covers_every_category():
    items = corpus_mod.load("de-v1")
    smoke = corpus_mod.subset(items, "smoke")
    assert len(smoke) == 12
    assert {i["category"] for i in smoke} == set(corpus_mod.CATEGORIES)
    assert len(corpus_mod.subset(items, "full")) == len(items)
    with pytest.raises(ValueError):
        corpus_mod.subset(items, "unbekannt")


def test_corpus_validates_clean_including_hand_written_spoken_forms():
    errors, stats_ = corpus_mod.validate(corpus_mod.load("de-v1"))
    assert errors == []
    assert stats_["count"] >= 100


def test_corpus_validator_catches_problems():
    items = corpus_mod.load("de-v1")[:3]
    bad = [dict(items[0]), dict(items[0]), {"id": "x"}]
    errors, _ = corpus_mod.validate(bad, min_sentences=1, required_categories=None, smoke_size=None)
    joined = "\n".join(errors)
    assert "doppelte ID" in joined and "fehlt" in joined
    wrong = dict(items[1], expected_normalized="falsch")
    errors, _ = corpus_mod.validate([wrong], min_sentences=1, required_categories=None, smoke_size=None)
    assert any("expected_normalized" in e for e in errors)
    pii = dict(items[2], id="de-v1-alltag-999", text="Mail an hans@firma.de", expected_normalized=N("Mail an hans@firma.de"))
    errors, _ = corpus_mod.validate([pii], min_sentences=1, required_categories=None, smoke_size=None)
    assert any("Beispiel-Domain" in e for e in errors)


def test_corpus_sha_ignores_line_endings(tmp_path):
    a, b = tmp_path / "a.jsonl", tmp_path / "b.jsonl"
    a.write_bytes(b'{"id":"x"}\n{"id":"y"}\n')
    b.write_bytes(b'{"id":"x"}\r\n{"id":"y"}\r\n')
    assert corpus_mod.sha256_of(a) == corpus_mod.sha256_of(b)


def test_functional_english_corpus_is_valid_basic():
    errors, st = corpus_mod.validate(corpus_mod.load("en-func-v1"), min_sentences=1, required_categories=None, smoke_size=None)
    assert errors == [] and st["count"] >= 5


def test_validate_corpus_cli_returns_zero(capsys):
    assert tts_bench.main(["validate-corpus"]) == 0
    assert "gültig" in capsys.readouterr().out


# --------------------------------------------------------------------------- Normalisierung


def test_umlaut_folding_matches_benchlib():
    for w in ("Größe", "FÜR", "Ärzte", "Öl", "Straße", "„Große“,"):
        assert N(w) == benchlib.normalize_word(w)
    assert N("Die Größe für Ärzte!") == "die groesse fuer aerzte"
    assert N("Café, naïve") == "cafe naive"


@pytest.mark.parametrize("words,expected", [
    ("dreiundvierzig", "43"), ("zweihundertdreiundvierzig", "243"), ("zweihundert dreiundvierzig", "243"),
    ("drei und zwanzig", "23"), ("eintausendzweihundertfünfzig", "1250"), ("hundertdreißig", "130"),
    ("neunzehnhundertneunundachtzig", "1989"), ("zweitausendsechsundzwanzig", "2026"),
    ("siebenundzwanzigtausendeins", "27001"), ("null", "0"), ("zwölf", "12"), ("sechzehn", "16"),
])
def test_number_words_become_digits(words, expected):
    assert N(words) == expected


def test_digits_and_number_words_agree():
    pairs = [("1.250", "eintausendzweihundertfünfzig"), ("840", "achthundertvierzig"), ("30", "dreißig"),
             ("3.000", "dreitausend"), ("4711", "viertausendsiebenhundertelf"), ("0", "null")]
    for digits, words in pairs:
        assert N(f"Es sind {digits} Leute") == N(f"Es sind {words} Leute"), (digits, words)


def test_decimals_currency_percent_units():
    assert N("12,50 €") == N("zwölf Euro fünfzig") == "12 euro 50"
    assert N("3,5 %") == N("drei Komma fünf Prozent") == "3 komma 5 prozent"
    assert N("42,195 Kilometer") == N("zweiundvierzig Komma eins neun fünf Kilometer") == "42 komma 195 kilometer"
    assert N("3,15") == N("drei Komma eins fünf") == "3 komma 15"
    assert N("1.250,50") == "1250 komma 50"
    assert N("0,99 €") == "0 euro 99"
    assert N("20 €") == "20 euro"
    assert N("130 km/h") == "130 kilometer pro stunde"
    assert N("30 °C") == N("dreißig Grad Celsius") == "30 grad celsius"
    assert N("82 kg") == "82 kilogramm"


def test_millions_combine_to_one_number():
    assert N("2.300.000") == N("zwei Millionen dreihunderttausend") == "2300000"
    assert N("eine Million") == N("1 Million") == "1000000"
    assert N("2,3 Millionen") == "2 komma 3 millionen"
    assert N("1 Million Euro") == "1000000 euro"


def test_digit_by_digit_readings_merge():
    assert N("ISO 27001") == N("ISO zwei sieben null null eins") == "iso 27001"
    assert N("PLZ 10115") == N("PLZ eins null eins eins fünf") == "plz 10115"
    # Satzzeichen trennen: Aufzaehlung nicht zu einer Zahl verschmelzen
    assert N("3, 4, 5") == "3 4 5"
    assert N("zwei, drei, vier") == "2 3 4"


def test_ordinals_dates_and_times():
    assert N("am 3. November") == N("am dritten November") == "am 3te november"
    assert N("am 1. Mai") == N("am ersten Mai") == "am 1te mai"
    assert N("der 26.10.2026") == N("der sechsundzwanzigste Oktober zweitausendsechsundzwanzig") == "der 26te oktober 2026"
    assert N("vom 5. bis 7. Juni") == N("vom fünften bis siebten Juni") == "vom 5te bis 7te juni"
    assert N("im neunzehnten Jahrhundert") == N("im 19. Jahrhundert") == "im 19te jahrhundert"
    assert N("14:30 Uhr") == N("vierzehn Uhr dreißig") == "14 uhr 30"
    assert N("8:00 Uhr") == N("acht Uhr") == "8 uhr"
    assert N("22:05 Uhr") == N("zweiundzwanzig Uhr fünf") == "22 uhr 5"
    assert N("14.30 Uhr") == "14 uhr 30"
    assert N("Es sind 3.") == "es sind 3"          # Punkt am Satzende ist keine Ordinalzahl
    assert N("Mitte der Woche") == "mitte der woche"  # kein falscher Ordinal-Treffer


def test_article_ein_only_folds_before_measure_words():
    assert N("ein Euro") == N("1 Euro") == "1 euro"
    assert N("eine Stunde") == N("1 Stunde") == "1 stunde"
    assert N("ein Haus") == "ein haus"


def test_acronyms_spelled_vs_word():
    assert N("DSGVO") == "dsgvo"
    assert N("D S G V O") == N("D-S-G-V-O") == N("de es ge vau o") == "dsgvo"
    assert N("K I") == "ki"
    assert N("NATO und UNO") == "nato und uno"
    assert N("E-Mail") == "e mail"                    # einzelner Buchstabe + Wort: nicht verschmolzen
    assert N("z. B.") == N("zum Beispiel") == "zum beispiel"
    assert N("d. h.") == N("das heißt") == "das heisst"
    assert N("QR-Code") == "qr code"


def test_email_and_url_spoken_form():
    assert N("info@example.org, bitte") == "info at example punkt org bitte"
    assert N("max.mustermann@example.com.") == "max punkt mustermann at example punkt com"
    assert N("www.example.org/hilfe.") == "www punkt example punkt org slash hilfe"
    assert N("docs.example.org/handbuch") == "docs punkt example punkt org slash handbuch"
    assert N("Der Satz endet.Neu beginnt") != ""  # kein URL-Treffer ohne bekannte Domain-Endung
    assert N("das Haus.") == "das haus"


def test_normalization_is_idempotent_on_corpus_and_samples():
    samples = [i["text"] for i in corpus_mod.load("de-v1")] + [
        "Am 3. November um 14:30 Uhr kostet es 12,50 € (ca. 3,5 %).", "D-S-G-V-O & KI, E-Mail: a@example.org",
        "eine Million Euro, zwei sieben null null eins", "„Zitat“ – mit Gedankenstrich … und Auslassung"]
    for t in samples:
        once = N(t)
        assert N(once) == once, t


def test_number_parser_rejects_non_numbers():
    for w in ("haus", "dreizehnhundertundx", "und", "einsundzwanzig", ""):
        assert nz.parse_cardinal(w) is None
    assert nz.ordinal_value("dritten") == 3 and nz.ordinal_value("sechsundzwanzigste") == 26
    assert nz.ordinal_value("achte") == 8 and nz.ordinal_value("nächste") is None


# --------------------------------------------------------------------------- WER / CER


def test_wer_known_results_and_benchlib_agreement():
    assert textmetrics.word_errors("das ist ein test", "das ist kein test") == (1, 4)
    for ref, hyp in [("das ist ein test", "das ist kein test"), ("Die Größe für Ärzte.", "die groesse fuer aerzte"),
                     ("alpha beta gamma delta epsilon", "alpha gamma delta xi epsilon zeta"), ("hallo welt", "")]:
        assert textmetrics.word_errors(ref, hyp) == benchlib.word_errors(ref, hyp)


def test_wer_is_number_form_independent():
    assert textmetrics.word_errors("am dritten November um vierzehn Uhr dreißig", "Am 3. November um 14:30 Uhr") == (0, 7)
    assert benchlib.word_errors("am dritten November", "am 3. November")[0] == 1  # benchlib allein waere hier zu streng


def test_cer_known_results():
    assert textmetrics.char_errors("abc", "abd") == (1, 3)
    assert textmetrics.char_errors("Größe", "Groesse") == (0, 7)
    assert textmetrics.char_errors("", "x")[1] == 0


def test_corpus_wer_is_sum_over_sum():
    a = textmetrics.score("alpha beta gamma delta", "alpha beta gamma epsilon")
    b = textmetrics.score("rot gelb gruen blau lila braun", "rot gelb gruen blau lila braun")
    agg = textmetrics.aggregate([a, b])
    assert agg["word_errors"] == 1 and agg["ref_words"] == 10 and agg["wer"] == pytest.approx(0.1)
    assert textmetrics.aggregate([])["wer"] is None


# --------------------------------------------------------------------------- Statistik


def test_percentiles_match_numpy_linear():
    vals = [5.0, 1.0, 9.0, 3.0, 7.0, 11.0, 2.0, 20.0, 4.0, 6.0]
    s = stats.summarize(vals)
    for key, p in (("median", 50), ("p90", 90), ("p95", 95)):
        assert s[key] == pytest.approx(float(np.percentile(vals, p)))
    assert s["mean"] == pytest.approx(np.mean(vals))
    assert s["stdev"] == pytest.approx(np.std(vals, ddof=1))
    assert (s["min"], s["max"], s["n"]) == (1.0, 20.0, 10)


def test_summarize_edge_cases():
    assert stats.summarize([]) is None
    assert stats.summarize([None, float("nan")]) is None
    one = stats.summarize([4.0])
    assert one["median"] == one["p95"] == 4.0 and one["stdev"] == 0.0
    assert stats.percentile([1.0, 2.0], 50) == 1.5


# --------------------------------------------------------------------------- Konfiguration


def test_default_config_has_required_rows_and_reasons():
    cfgs = {c.id: c for c in cfgmod.load()}
    assert cfgs["piper-thorsten-high-cpu"].provider == "piper" and cfgs["piper-thorsten-high-cpu"].device == "cpu"
    assert cfgs["piper-thorsten-high-cpu"].voice == "de_DE-thorsten-high"
    assert cfgs["fish-s2-pro-cuda"].device == "cuda" and cfgs["fish-s2-pro-cuda"].status == "run"
    assert cfgs["phonon-31m-en-cpu"].status == "run" and cfgs["phonon-31m-en-cpu"].quality is False
    assert cfgs["phonon-40m-de"].status == "license_restricted" and "CC BY-NC" in cfgs["phonon-40m-de"].reason
    assert cfgs["ollama-speech-windows"].status == "unsupported" and "0.40.11" in cfgs["ollama-speech-windows"].reason
    assert cfgs["kokoro"].status == "unsupported" and "Deutsch" in cfgs["kokoro"].reason
    assert cfgs["xtts-v2"].status == "not_tested" and cfgs["f5-tts"].status == "not_tested"
    assert all(c.reason for c in cfgs.values() if c.status != "run")
    assert all(c.warm_runs == 30 and c.cold_runs == 3 for c in cfgs.values() if c.id in ("piper-thorsten-high-cpu", "fish-s2-pro-cuda"))


def test_fish_job_cap_leaves_room_for_the_model_load():
    # Messlauf 26.10.2026: mit 16 GB Job-Deckel starb der Fish-Server beim Laden mit 0xc0000005
    # (er braucht kurzzeitig ~48 GB Commit, vgl. process_guard.rs). Der Deckel muss darueber liegen.
    cfgs = {c.id: c for c in cfgmod.load()}
    assert cfgs["fish-s2-pro-cuda"].memory_limit_mb >= 49152
    assert cfgs["piper-thorsten-high-cpu"].memory_limit_mb <= 16384


def test_config_validation(tmp_path):
    p = tmp_path / "c.toml"
    p.write_text('[[config]]\nid="a"\nprovider="x"\nstatus="kaputt"\n', encoding="utf-8")
    with pytest.raises(cfgmod.ConfigError, match="status"):
        cfgmod.load(p)
    p.write_text('[[config]]\nid="a"\nprovider="x"\nstatus="unsupported"\n', encoding="utf-8")
    with pytest.raises(cfgmod.ConfigError, match="Grund"):
        cfgmod.load(p)
    p.write_text('[[config]]\nid="a"\nprovider="x"\n[[config]]\nid="a"\nprovider="y"\n', encoding="utf-8")
    with pytest.raises(cfgmod.ConfigError, match="doppelte"):
        cfgmod.load(p)
    p.write_text('[[config]]\nid="a"\nprovider="x"\nbogus=1\n', encoding="utf-8")
    with pytest.raises(cfgmod.ConfigError, match="unbekannte"):
        cfgmod.load(p)
    with pytest.raises(cfgmod.ConfigError):
        cfgmod.select(cfgmod.load(cfgmod.DEFAULT_CONFIG), ["gibtsnicht"])


# --------------------------------------------------------------------------- Zeilen-Schema


def _row(**kw):
    r = {"schema": "tts-bench-row/1", "provider": "piper", "voice": "v", "language": "de", "utterance_id": "u1",
         "run_kind": "warm", "run_idx": 0, "ok": True, "error_kind": None, "error": None, "process_start_ms": None,
         "ttfa_ms": 80.0, "total_ms": 80.0, "audio_ms": 1500.0, "rtf": 0.0533, "sample_rate": 22050, "channels": 1,
         "encoding": "pcm16le", "chunk_count": 1, "chunk_gaps_ms": [], "max_gap_ms": 0.0, "wav_path": "wavs/u1.wav",
         "started_at": "2026-10-26T12:00:00.000Z"}
    r.update(kw)
    return r


def test_row_schema_file_and_valid_row():
    schema = rowschema.load_schema()
    assert schema["$id"] == "tts-bench-row/1"
    assert {"schema", "provider", "utterance_id", "run_kind", "ok"} <= set(schema["required"])
    assert rowschema.validate_row(_row()) == []
    assert rowschema.validate_row(_row(ok=False, error_kind="timeout", error="t", ttfa_ms=None, total_ms=None,
                                       audio_ms=None, rtf=None, sample_rate=None, channels=None, encoding=None,
                                       chunk_count=None, chunk_gaps_ms=None, max_gap_ms=None, wav_path=None)) == []


@pytest.mark.parametrize("mutation,needle", [
    ({"schema": "tts-bench-row/2"}, "erwartet"), ({"run_kind": "lauwarm"}, "nicht in"),
    ({"ttfa_ms": -1}, "< 0"), ({"ttfa_ms": None}, "ttfa_ms"), ({"ok": False}, "error_kind"),
    ({"process_start_ms": 5.0}, "cold"), ({"chunk_count": 3, "chunk_gaps_ms": [1.0]}, "chunk_count-1"),
    ({"run_idx": "0"}, "Typ"),
])
def test_row_schema_rejects(mutation, needle):
    errs = rowschema.validate_row(_row(**mutation))
    assert errs and any(needle in e for e in errs), errs


def test_read_rows_skips_invalid_lines(tmp_path):
    p = tmp_path / "rows.jsonl"
    p.write_text(json.dumps(_row()) + "\nkein json\n" + json.dumps(_row(run_kind="x")) + "\n", encoding="utf-8")
    rows, problems = rowschema.read_rows(p)
    assert len(rows) == 1 and len(problems) == 2


def test_app_command_follows_the_rust_contract(tmp_path):
    cfg = cfgmod.select(cfgmod.load(), ["piper-thorsten-high-cpu"])[0]
    cmd = sources.app_command(Path("app.exe"), cfg, Path("c.jsonl"), "smoke", 30, 3, tmp_path)
    assert cmd == ["app.exe", "--tts-bench", "--tts-provider", "piper", "--tts-voice", "de_DE-thorsten-high",
                   "--tts-lang", "de", "--bench-corpus", "c.jsonl", "--bench-subset", "smoke", "--bench-warm", "30",
                   "--bench-cold", "3", "--bench-out", str(tmp_path), "--json"]
    fish = cfgmod.select(cfgmod.load(), ["fish-s2-pro-cuda"])[0]
    cmd = sources.app_command(Path("app.exe"), fish, Path("c.jsonl"), "full", 1, 0, tmp_path)
    assert "--tts-voice" not in cmd and "--bench-subset" not in cmd and cmd[cmd.index("--bench-cold") + 1] == "0"


# --------------------------------------------------------------------------- Ressourcen-Sampling


def _sleeper(seconds: float):
    return subprocess.Popen([sys.executable, "-c", f"import time; x = bytearray(40*1024*1024); time.sleep({seconds})"])


@pytest.mark.parametrize("fallback", [False, True])
def test_sampler_measures_child_ram(fallback):
    if fallback and sys.platform != "win32":
        pytest.skip("Windows-API-Fallback nur unter Windows")
    if not fallback:
        pytest.importorskip("psutil")
    s = sampler.ResourceSampler(interval_ms=100, gpu=False, force_fallback=fallback)
    s.start()
    p = _sleeper(1.5)
    try:
        s.attach(p.pid)
        time.sleep(1.0)
    finally:
        res = s.stop()
        p.kill()
        p.wait()
    assert res["ram_peak_mb"] and res["ram_peak_mb"] > 20
    assert res["probe"] == ("winapi" if fallback else "psutil")
    assert res["ram_reason"] is None
    assert res["gpu"]["available"] is False and res["gpu"]["vram_peak_mb"] is None and res["gpu"]["reason"]


def test_sampler_without_process_reports_reason_instead_of_numbers():
    s = sampler.ResourceSampler(gpu=False)
    s.start()
    res = s.stop()
    assert res["ram_peak_mb"] is None and res["ram_reason"]
    assert res["cpu_avg_pct_of_machine"] is None and res["cpu_reason"]


def test_gpu_summary_baseline_and_delta():
    s = sampler.ResourceSampler(gpu=True)
    s._attach_t = 10.0
    s._gpu_samples = [(8.0, 1.0, 1000.0), (9.0, 2.0, 1010.0), (11.0, 50.0, 5000.0), (12.0, 70.0, 4000.0)]
    g = s._gpu_summary()
    assert g["vram_baseline_mb"] == 1010.0 and g["vram_peak_mb"] == 5000.0 and g["vram_delta_peak_mb"] == 3990.0
    assert g["util_avg_pct"] == 60.0 and g["util_peak_pct"] == 70.0


# --------------------------------------------------------------------------- Fake-Lauf Ende-zu-Ende


E2E_TOML = """
[defaults]
warm_runs = 4
cold_runs = 1
[[config]]
id = "piper-x"
provider = "piper"
voice = "v"
device = "cpu"
[[config]]
id = "stream-x"
provider = "stream"
device = "cuda"
[[config]]
id = "phonon-func"
provider = "phonon"
language = "en"
corpus = "en-func-v1"
subset = "full"
quality = false
[[config]]
id = "xtts-x"
provider = "xtts-v2"
status = "not_tested"
reason = "nicht installiert"
[[config]]
id = "ollama-x"
provider = "ollama-speech"
status = "unsupported"
reason = "unter Windows nicht unterstützt"
[[config]]
id = "phonon-de"
provider = "phonon"
status = "license_restricted"
reason = "gated, CC BY-NC 4.0"
"""


def _fake_run(tmp_path, run_id="e2e", extra=()):
    cfg = tmp_path / "e2e.toml"
    cfg.write_text(E2E_TOML, encoding="utf-8")
    assert tts_bench.main(["run", "--source", "fake", "--config", str(cfg), "--run-id", run_id,
                           "--fake-error-rate", "0.05", "--seed", "7", *extra]) == 0
    return runner.tts_root() / run_id


def test_fake_run_end_to_end_run_asr_report(bench_env, capsys):
    run_dir = _fake_run(bench_env)
    manifest = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))
    states = {c["id"]: c["state"] for c in manifest["configs"]}
    assert states == {"piper-x": "measured", "stream-x": "measured", "phonon-func": "measured",
                      "xtts-x": "not_tested", "ollama-x": "unsupported", "phonon-de": "license_restricted"}
    assert (run_dir / "host.json").exists() and (run_dir / "corpus" / "de-v1.jsonl").exists()
    host = manifest["host"]
    assert {"os", "cpu", "ram_total_mb", "gpu", "python", "git", "timestamp"} <= set(host)
    assert manifest["harness_version"].startswith("tts-bench/") and manifest["corpora"]["de-v1"]["count"] >= 100
    # Zeilen halten das Schema ein und enthalten Kalt- UND Warmlaeufe
    rows, problems = rowschema.read_rows(run_dir / "piper-x" / "rows.jsonl")
    assert problems == []
    assert len(rows) == 12 * (4 + 1)
    assert {r["run_kind"] for r in rows} == {"cold", "warm"}
    assert any(not r["ok"] for r in rows) and all(r["error_kind"] for r in rows if not r["ok"])
    assert all(r["process_start_ms"] is not None for r in rows if r["run_kind"] == "cold")
    wav = run_dir / "piper-x" / "wavs" / "de-v1-alltag-001_warm0.wav"
    assert wav.exists() and sf.info(str(wav)).samplerate == 22050

    assert tts_bench.main(["asr", "--run", "e2e", "--engine", "fake"]) == 0
    asr = json.loads((run_dir / "asr.json").read_text(encoding="utf-8"))
    assert asr["asr"]["model"] == "fake-asr" and "Proxy" in asr["note"] and "phonon-func" in asr["skipped"]
    assert set(asr["configs"]) == {"piper-x", "stream-x"}
    assert set(asr["configs"]["piper-x"]["by_category"]) == set(corpus_mod.CATEGORIES) - set()

    out = bench_env / "report"
    assert tts_bench.main(["report", "--runs", "e2e", "--out", str(out)]) == 0
    res = json.loads((out / "results.json").read_text(encoding="utf-8"))
    by = {c["id"]: c for c in res["configs"]}

    # Kennzahlen == direkte Berechnung aus den Zeilen
    warm = [r for r in rows if r["run_kind"] == "warm" and r["ok"]]
    m = by["piper-x"]["metrics"]
    assert m["warm"]["n"] == len(warm)
    for key, col in (("ttfa_ms", "ttfa_ms"), ("total_ms", "total_ms")):
        vals = [r[col] for r in warm]
        for k, p in (("median", 50), ("p90", 90), ("p95", 95)):
            assert m["warm"][key][k] == pytest.approx(float(np.percentile(vals, p)))
    rtfs = [r["total_ms"] / r["audio_ms"] for r in warm]
    assert m["warm"]["rtf"]["median"] == pytest.approx(float(np.median(rtfs)))
    assert m["rows_failed"] == sum(1 for r in rows if not r["ok"]) and m["errors_by_kind"]["backend"] == m["rows_failed"]
    assert m["cold"]["process_start_ms"]["n"] == 12

    # Zielwerte als Ziele markiert, ja/nein
    t = by["piper-x"]["targets"]
    assert t["ttfa_ms"]["target"] == 200 and t["rtf"]["target"] == 0.2 and t["wer"]["target"] == 0.05
    assert t["rtf"]["met"] is True and isinstance(t["wer"]["met"], bool)
    assert by["stream-x"]["targets"]["ttfa_ms"]["met"] is True          # fake-Streaming: TTFA ~140 ms
    assert by["stream-x"]["metrics"]["chunked_rows"] > 0
    assert by["phonon-func"]["targets"]["wer"]["met"] is None and by["phonon-func"]["quality"] is None
    assert "Ziele" in res["targets"]["note"]

    # Statuszeilen: mit Grund, OHNE Zahlen
    for cid, status in (("xtts-x", "not_tested"), ("ollama-x", "unsupported"), ("phonon-de", "license_restricted")):
        c = by[cid]
        assert c["status"] == status and c["reason"]
        assert c["metrics"] is None and c["quality"] is None and c["resources"] is None and c["targets"] is None
    with (out / "results.csv").open(encoding="utf-8", newline="") as f:
        csv_rows = {r["id"]: r for r in csv.DictReader(f)}
    for cid in ("xtts-x", "ollama-x", "phonon-de"):
        numeric = [v for k, v in csv_rows[cid].items() if k not in ("id", "provider", "voice", "language", "device_expected", "device_actual", "status", "reason", "flags")]
        assert all(v == "" for v in numeric), (cid, numeric)
    assert csv_rows["piper-x"]["warm_ttfa_ms_median"] != "" and csv_rows["piper-x"]["target_rtf_met"] == "ja"
    md = (out / "report.md").read_text(encoding="utf-8")
    assert "Ziele, keine Zusagen" in md and "Verständlichkeits-Proxy" in md and "Nicht gemessene Konfigurationen" in md
    assert "nicht installiert" in md and "CC BY-NC 4.0" in md
    section = md.split("## Nicht gemessene Konfigurationen")[1].split("## Reproduktion")[0]
    assert not any(ch.isdigit() for ch in section.replace("4.0", "").replace("xtts-v2", ""))

    # Veroeffentlichen: nur Zusammenfassung, keine WAVs/Rohzeilen
    pub = bench_env / "docs" / "benchmarks" / "2026-10-26"
    assert tts_bench.main(["report", "--runs", "e2e", "--out", str(out), "--publish", "--publish-dir", str(pub)]) == 0
    assert sorted(p.name for p in pub.iterdir()) == ["report.md", "results.csv", "results.json"]


def test_report_without_asr_marks_quality_as_not_measured(bench_env):
    run_dir = _fake_run(bench_env, "noasr")
    res = report.build_results([run_dir])
    c = next(c for c in res["configs"] if c["id"] == "piper-x")
    assert c["quality"] is None and "ASR nicht gelaufen" in c["quality_reason"]
    assert c["targets"]["wer"]["met"] is None
    assert "nicht ermittelt" in report.render_markdown(res)


def test_report_merges_latency_and_quality_runs(bench_env):
    cfg = bench_env / "m.toml"
    cfg.write_text(E2E_TOML, encoding="utf-8")
    base = ["run", "--source", "fake", "--config", str(cfg), "--configs", "piper-x"]
    assert tts_bench.main(base + ["--run-id", "lat"]) == 0
    assert tts_bench.main(base + ["--run-id", "qual", "--subset", "full", "--warm", "1", "--cold", "0"]) == 0
    assert tts_bench.main(["asr", "--run", "qual", "--engine", "fake"]) == 0
    res = report.build_results([runner.tts_root() / "lat", runner.tts_root() / "qual"])
    c = res["configs"][0]
    assert c["quality"]["n"] >= 100 and c["metrics"]["warm"]["n"] > 12 * 4 - 1
    assert res["runs"][0]["run_id"] == "lat" and len(c["runs"]) == 2


def test_run_dry_run_starts_nothing(bench_env, capsys):
    assert tts_bench.main(["run", "--dry-run", "--source", "fake"]) == 0
    out = capsys.readouterr().out
    assert "piper-thorsten-high-cpu" in out and "license_restricted" in out
    assert not (runner.tts_root()).exists() or not any(runner.tts_root().glob("*/run.json"))


def test_run_refuses_existing_run_and_resume_skips_done(bench_env, capsys):
    _fake_run(bench_env, "dup")
    cfg = bench_env / "e2e.toml"
    with pytest.raises(SystemExit, match="existiert bereits"):
        tts_bench.main(["run", "--source", "fake", "--config", str(cfg), "--run-id", "dup"])
    capsys.readouterr()
    assert tts_bench.main(["run", "--source", "fake", "--config", str(cfg), "--run-id", "dup", "--resume"]) == 0
    assert "übersprungen" in capsys.readouterr().out


def test_missing_app_binary_is_a_clear_error(bench_env):
    cfg = bench_env / "e2e.toml"
    cfg.write_text(E2E_TOML, encoding="utf-8")
    with pytest.raises(SystemExit, match="App-Binary nicht gefunden"):
        tts_bench.main(["run", "--config", str(cfg), "--exe", str(bench_env / "gibt-es-nicht.exe"), "--run-id", "x"])


def test_asr_refuses_while_a_measurement_runs(bench_env):
    run_dir = _fake_run(bench_env, "lock")
    other = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"])
    try:
        runner.lock_path().write_text(json.dumps({"pid": other.pid, "run_id": "andere", "started": "x"}), encoding="utf-8")
        with pytest.raises(SystemExit, match="läuft bereits"):
            tts_bench.main(["asr", "--run", "lock", "--engine", "fake"])
        with pytest.raises(SystemExit, match="läuft bereits"):
            tts_bench.main(["run", "--source", "fake", "--run-id", "zweiter"])
    finally:
        other.kill()
        other.wait()
    # veraltete Sperre (toter Prozess) blockiert nicht
    assert tts_bench.main(["asr", "--run", "lock", "--engine", "fake"]) == 0
    assert not runner.lock_path().exists()


# --------------------------------------------------------------------------- ASR-Engine app-cli (Attrappe)


def _fake_asr_exe(tmp_path) -> Path:
    exe = tmp_path / "fake-asr-app.cmd"
    exe.write_text(f'@echo off\r\n"{sys.executable}" "{HERE / "fake_asr_cli.py"}" %*\r\n', encoding="utf-8")
    return exe


def test_to_16k_mono_resamples_other_rates_and_keeps_duration(tmp_path):
    from ttsbench import asr
    t = np.arange(24000) / 24000.0
    stereo = np.stack([0.3 * np.sin(2 * np.pi * 440 * t)] * 2, axis=1)
    src, dst = tmp_path / "a24k.wav", tmp_path / "out.wav"
    sf.write(str(src), stereo, 24000, subtype="PCM_16")
    assert asr.to_16k_mono_pcm16(src, dst) is True
    i = sf.info(str(dst))
    assert (i.samplerate, i.channels, i.subtype, i.frames) == (16000, 1, "PCM_16", 16000)
    ok = tmp_path / "ok.wav"
    sf.write(str(ok), np.zeros(1600, dtype="float32"), 16000, subtype="PCM_16")
    assert asr.to_16k_mono_pcm16(ok, tmp_path / "none.wav") is False and not (tmp_path / "none.wav").exists()
    odd = tmp_path / "odd.wav"  # 22,05 kHz (Piper): ungerades Verhaeltnis 160/441
    sf.write(str(odd), np.zeros(22050, dtype="float32"), 22050, subtype="PCM_16")
    asr.to_16k_mono_pcm16(odd, dst)
    assert sf.info(str(dst)).frames == 16000


@pytest.mark.skipif(sys.platform != "win32", reason="Attrappe ist ein .cmd (Windows)")
def test_asr_app_cli_end_to_end_resamples_and_records_engine(bench_env, monkeypatch):
    run_dir = _fake_run(bench_env, "appasr")
    log = bench_env / "asr_calls.jsonl"
    monkeypatch.setenv("FAKE_ASR_LOG", str(log))
    monkeypatch.setenv("FAKE_ASR_TEXT", "das ist ein test")
    exe = _fake_asr_exe(bench_env)
    assert tts_bench.main(["asr", "--run", "appasr", "--engine", "app-cli", "--exe", str(exe),
                           "--min-free-ram-mb", "1"]) == 0
    asr_json = json.loads((run_dir / "asr.json").read_text(encoding="utf-8"))
    a = asr_json["asr"]
    assert a["engine"] == "app-cli" and a["model"].endswith("whisper-large-v3-turbo-Q8_0.gguf")
    assert a["exe"] == str(exe) and a["exe_size_bytes"] == exe.stat().st_size and a["exe_mtime"].endswith("Z")
    assert a["bound_backend"] == "CPU (Attrappe)" and a["device"] == "CPU (Attrappe)"
    calls = [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]
    n = sum(len(c["items"]) for c in asr_json["configs"].values())
    assert len(calls) == n == a["files"] > 0 and a["resampled_files"] == n  # Synthese hat 22,05/24 kHz
    assert {c["samplerate"] for c in calls} == {16000} and {c["model"] for c in calls} == {a["model"]}
    item = asr_json["configs"]["piper-x"]["items"][0]
    assert item["hyp"] == "das ist ein test"
    src = run_dir / "piper-x" / "wavs" / item["wav"]
    assert abs(calls[0]["audio_secs"] - sf.info(str(src)).duration) < 0.01  # Resampling erhält die Dauer


@pytest.mark.skipif(sys.platform != "win32", reason="Attrappe ist ein .cmd (Windows)")
def test_report_after_app_cli_asr_has_no_version_field(bench_env, monkeypatch):
    # Messlauf 26.10.2026: app-cli liefert keine ASR-"version" (nur app_version/exe); der Bericht brach mit KeyError ab.
    _fake_run(bench_env, "appasr2")
    monkeypatch.setenv("FAKE_ASR_TEXT", "das ist ein test")
    exe = _fake_asr_exe(bench_env)
    assert tts_bench.main(["asr", "--run", "appasr2", "--engine", "app-cli", "--exe", str(exe), "--min-free-ram-mb", "1"]) == 0
    out = bench_env / "rep"
    assert tts_bench.main(["report", "--runs", "appasr2", "--out", str(out)]) == 0
    md = (out / "report.md").read_text(encoding="utf-8")
    assert "app-cli" in md and "whisper-large-v3-turbo-Q8_0.gguf" in md and "vNone" not in md


@pytest.mark.skipif(sys.platform != "win32", reason="Attrappe ist ein .cmd (Windows)")
def test_asr_app_cli_custom_model_and_failure_is_a_clear_error(bench_env, monkeypatch):
    _fake_run(bench_env, "appfail")
    exe = _fake_asr_exe(bench_env)
    monkeypatch.setenv("FAKE_ASR_MODE", "fail")
    with pytest.raises(SystemExit, match="App-CLI fehlgeschlagen"):
        tts_bench.main(["asr", "--run", "appfail", "--engine", "app-cli", "--exe", str(exe), "--min-free-ram-mb", "1",
                        "--model", "parakeet-tdt-0.6b-v3"])
    assert not runner.lock_path().exists()  # Sperre wird auch bei Fehler frei
    with pytest.raises(SystemExit, match="App-Binary nicht gefunden"):
        tts_bench.main(["asr", "--run", "appfail", "--engine", "app-cli", "--exe", str(bench_env / "nix.exe")])


def test_asr_auto_prefers_app_cli_only_when_binary_exists(bench_env, monkeypatch):
    import argparse
    from ttsbench import asr

    class _Stub:
        name = "faster-whisper"

        def __init__(self, *a, **k):
            pass
    monkeypatch.setattr(asr, "FasterWhisperEngine", _Stub)
    ns = lambda exe: argparse.Namespace(engine="auto", exe=exe, model="auto", device="auto", compute_type="auto",  # noqa: E731
                                        allow_download=False, min_free_ram_mb=1)
    assert isinstance(asr.make_engine(ns(str(bench_env / "gibt-es-nicht.exe"))), _Stub)
    exe = bench_env / "da.exe"
    exe.write_bytes(b"MZ")
    eng = asr.make_engine(ns(str(exe)))
    assert isinstance(eng, asr.AppCliEngine) and eng.exe == exe and eng.model_name == asr.APP_CLI_MODEL


# --------------------------------------------------------------------------- App-Quelle (Attrappe des Binarys)


def _fake_exe(tmp_path) -> Path:
    exe = tmp_path / "fake-app.cmd"
    exe.write_text(f'@echo off\r\n"{sys.executable}" "{HERE / "fake_app.py"}" %*\r\n', encoding="utf-8")
    return exe


APP_TOML = """
[defaults]
warm_runs = 2
cold_runs = 1
[[config]]
id = "app-ok"
provider = "piper"
voice = "v"
device = "cpu"
timeout_s = 2
"""


@pytest.mark.skipif(sys.platform != "win32", reason="Attrappe ist ein .cmd (Windows)")
def test_app_source_runs_binary_with_job_object_sampler_and_reads_rows(bench_env, monkeypatch):
    cfg = bench_env / "app.toml"
    cfg.write_text(APP_TOML.replace("timeout_s = 2", "timeout_s = 120"), encoding="utf-8")
    monkeypatch.setenv("FAKE_APP_MODE", "ok")
    assert tts_bench.main(["run", "--config", str(cfg), "--exe", str(_fake_exe(bench_env)), "--run-id", "app1",
                           "--min-free-ram-mb", "1", "--sample-ms", "50"]) == 0
    run_dir = runner.tts_root() / "app1"
    e = json.loads((run_dir / "run.json").read_text(encoding="utf-8"))["configs"][0]
    assert e["state"] == "measured" and e["rows_found"] == e["rows_expected"] == 12 * 3
    assert e["command"][1:4] == ["--tts-bench", "--tts-provider", "piper"] and e["returncode"] == 0
    assert e["resources"]["ram_peak_mb"] and e["resources"]["ram_peak_mb"] > 1
    assert "gpu" in e["resources"] and e["env"]["provider"] == "piper"
    assert (run_dir / "app-ok" / "app_stdout.log").exists()


@pytest.mark.skipif(sys.platform != "win32", reason="Attrappe ist ein .cmd (Windows)")
def test_app_source_failure_and_timeout_give_reason_not_numbers(bench_env, monkeypatch):
    cfg = bench_env / "app.toml"
    cfg.write_text(APP_TOML, encoding="utf-8")
    exe = _fake_exe(bench_env)
    monkeypatch.setenv("FAKE_APP_MODE", "fail")
    assert tts_bench.main(["run", "--config", str(cfg), "--exe", str(exe), "--run-id", "app-fail", "--min-free-ram-mb", "1"]) == 0
    e = json.loads((runner.tts_root() / "app-fail" / "run.json").read_text(encoding="utf-8"))["configs"][0]
    assert e["state"] == "failed" and "nicht installiert" in e["reason"] and e["rows_found"] == 0
    res = report.build_results([runner.tts_root() / "app-fail"])
    assert res["configs"][0]["status"] == "failed" and res["configs"][0]["metrics"] is None
    assert tts_bench.main(["run", "--config", str(cfg), "--exe", str(exe), "--run-id", "app-fail2",
                           "--min-free-ram-mb", "1", "--strict"]) == 1

    monkeypatch.setenv("FAKE_APP_MODE", "hang")
    t0 = time.time()
    assert tts_bench.main(["run", "--config", str(cfg), "--exe", str(exe), "--run-id", "app-hang", "--min-free-ram-mb", "1"]) == 0
    assert time.time() - t0 < 30
    e = json.loads((runner.tts_root() / "app-hang" / "run.json").read_text(encoding="utf-8"))["configs"][0]
    assert e["state"] == "failed" and "Zeitüberschreitung" in e["reason"]


def test_report_banner_marks_synthetic_data(bench_env):
    res = report.build_results([_fake_run(bench_env, "banner")])
    assert "SYNTHETISCHE DATEN" in report.render_markdown(res)


# --------------------------------------------------------------------------- Hoertest


def _kit(bench_env, seed=11, run_id="hk", **kw):
    run_dir = _fake_run(bench_env, run_id)
    out = bench_env / f"kit-{seed}"
    key = bench_env / f"key-{seed}"
    res = listening.make_kit([run_dir], out, key, seed, **kw)
    return run_dir, out, key, res


def test_listening_kit_is_anonymous_and_key_is_separate(bench_env):
    run_dir, out, key_dir, res = _kit(bench_env)
    key = json.loads((key_dir / "key.json").read_text(encoding="utf-8"))
    assert key["seed"] == 11 and key["loudness_matched"] is True
    n_cfg = 3  # piper-x, stream-x, phonon-func
    assert res["n"] == len(key["samples"]) == 12 * 2 + 6 and n_cfg == 3
    wavs = sorted(p.name for p in (out / "proben").iterdir())
    assert wavs == sorted(f"{pid}.wav" for pid in key["samples"])
    # Schluessel nicht im Proben-Ordner, Proben-Ordner verraet keine Konfiguration
    assert not list(out.rglob("key.json")) and not (out / "key.json").exists()
    blob = " ".join(p.read_text(encoding="utf-8-sig") for p in out.glob("*") if p.is_file()) + " " + " ".join(wavs)
    for secret in ("piper", "stream-x", "phonon", "fish", "seed"):
        assert secret not in blob.replace("Seed dieses Pakets: siehe Schlüssel", ""), secret
    with (out / "bewertungsbogen.csv").open(encoding="utf-8-sig", newline="") as f:
        rows = list(csv.DictReader(f, delimiter=";"))
    assert tuple(rows[0].keys()) == listening.SHEET_COLUMNS and len(rows) == res["n"]
    assert all(r[c] == "" for r in rows for c in listening.CRITERIA + ("freitext",))
    assert [r["probe_id"] for r in rows] == list(key["samples"]) or set(r["probe_id"] for r in rows) == set(key["samples"])
    # Pegel angeglichen
    pid = rows[0]["probe_id"]
    data, _ = sf.read(str(out / "proben" / f"{pid}.wav"))
    assert 20 * np.log10(np.sqrt(np.mean(data ** 2))) == pytest.approx(listening.TARGET_RMS_DBFS, abs=1.0)
    with pytest.raises(SystemExit, match="ineinander"):
        listening.make_kit([run_dir], out, out / "sub", 1)


def test_listening_kit_is_deterministic_per_seed(bench_env):
    run_dir = _fake_run(bench_env, "det")
    def mapping(seed, name):
        listening.make_kit([run_dir], bench_env / f"o{name}", bench_env / f"k{name}", seed, config_ids=["piper-x"])
        k = json.loads((bench_env / f"k{name}" / "key.json").read_text(encoding="utf-8"))
        sheet = (bench_env / f"o{name}" / "bewertungsbogen.csv").read_text(encoding="utf-8-sig")
        return {p: (v["config_id"], v["utterance_id"]) for p, v in k["samples"].items()}, sheet
    m1, s1 = mapping(5, "a")
    m2, s2 = mapping(5, "b")
    m3, s3 = mapping(6, "c")
    assert m1 == m2 and s1 == s2
    assert m1 != m3 and s1 != s3
    assert list(m1.values()) != sorted(m1.values())  # gemischt, nicht sortiert


def test_listening_eval_merges_sheets_with_key_and_invents_nothing(bench_env):
    run_dir, out, key_dir, res = _kit(bench_env, config_ids=["piper-x", "stream-x"])
    key = json.loads((key_dir / "key.json").read_text(encoding="utf-8"))
    sheet = out / "bewertungsbogen.csv"

    # leerer Bogen: keine Bewertungen, nichts erfunden
    empty = listening.evaluate(key_dir / "key.json", [sheet])
    assert empty["configs"] == {} and "keine Bewertungen" in empty["note"]

    with sheet.open(encoding="utf-8-sig", newline="") as f:
        rows = list(csv.DictReader(f, delimiter=";"))
    expect: dict[str, list[int]] = {"piper-x": [], "stream-x": []}
    for i, r in enumerate(rows):
        cid = key["samples"][r["probe_id"]]["config_id"]
        score = 4 if cid == "piper-x" else 2
        r.update({"natuerlichkeit": str(score), "aussprache": "3", "prosodie": str(score), "artefakte": "5", "hoerer": "A"})
        expect[cid].append(score)
    rows[0].update({"natuerlichkeit": "7", "freitext": "Wert ausserhalb"})   # ungueltig -> ignoriert
    expect[key["samples"][rows[0]["probe_id"]]["config_id"]].pop(0)
    rows[1]["probe_id"] = "p000000"                                         # unbekannte ID
    filled = bench_env / "bogen_A.csv"
    with filled.open("w", encoding="utf-8-sig", newline="") as f:
        w = csv.DictWriter(f, fieldnames=listening.SHEET_COLUMNS, delimiter=";")
        w.writeheader()
        w.writerows(rows)
    ev = listening.evaluate(key_dir / "key.json", [filled])
    assert len(ev["invalid_entries"]) == 1 and ev["unknown_probe_ids"]
    assert ev["raters"] == ["A"]
    p = ev["configs"]["piper-x"]["criteria"]["natuerlichkeit"]
    assert p["mean"] == pytest.approx(4.0) and ev["configs"]["stream-x"]["criteria"]["natuerlichkeit"]["mean"] == pytest.approx(2.0)
    assert ev["configs"]["piper-x"]["criteria"]["artefakte"]["mean"] == 5.0
    assert tts_bench.main(["listening-eval", "--key", str(key_dir / "key.json"), "--sheets", str(filled),
                           "--out", str(bench_env / "auswertung")]) == 0
    md = (bench_env / "auswertung" / "listening_results.md").read_text(encoding="utf-8")
    assert "piper-x" in md and "stream-x" in md


def test_listening_kit_cli_defaults_put_key_beside_not_inside(bench_env):
    _fake_run(bench_env, "cli")
    assert tts_bench.main(["listening-kit", "--run", "cli", "--configs", "piper-x", "--seed", "3"]) == 0
    base = runner.tts_root() / "cli"
    assert (base / "listening-kit" / "kit-3" / "bewertungsbogen.csv").exists()
    assert (base / "listening-key" / "kit-3" / "key.json").exists()
    assert not (base / "listening-kit" / "kit-3" / "key.json").exists()
