"""Tests fuer die Benchmark-Skripte: python -m pytest scripts/bench -q (kein Netz, keine TTS, keine Modelle)."""
import json
import random
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchlib  # noqa: E402
import make_corpus as mc  # noqa: E402
import scenes as sc  # noqa: E402
import sentence_bench as sb  # noqa: E402


def test_wer_known_result():
    assert benchlib.word_errors("das ist ein test", "das ist kein test") == (1, 4)
    assert benchlib.word_errors("Die Größe für Ärzte.", "die groesse fuer aerzte") == (0, 4)
    assert benchlib.word_errors("am dritten November", "am 3. November") == (1, 3)


def test_wer_is_sum_over_sum_not_mean():
    assert benchlib.aggregate_wer([(1, 4), (0, 6)]) == pytest.approx(0.1)
    assert benchlib.aggregate_wer([]) == 0.0


def test_aggregate_counts_failures_as_all_wrong_and_separates_load():
    rows = [
        {"ref": "a b c d", "hyp": "a b c x", "errors": 1, "ref_words": 4, "errors_lenient": 1,
         "audio_s": 4.0, "transcribe_ms": 1000, "load_ms": 900},
        {"ref": "e f", "error": "boom"},
    ]
    a = sb.aggregate(rows)
    assert (a["errors"], a["ref_words"], a["failed"]) == (3, 6, 1)
    assert a["rtf"] == pytest.approx(4.0)
    assert a["load_ms_median"] == 900


def test_selftest_passes():
    assert sb.selftest() == 0


def test_normalize_matches_rust_rules():
    assert benchlib.normalize_word("„Große“,") == "grosse"
    assert benchlib.normalize_word("FÜR") == "fuer"
    assert benchlib.words("  a   b\tü ") == ["a", "b", "ue"]


def test_tsv_parse():
    tsv = "12\ta.wav\tHallo, Welt.\thallo welt\tx\t100\tMALE\n13\tb.wav\tZwei\tzwei\tx\t1\tFEMALE\n"
    rows = mc.parse_tsv(tsv)
    assert rows["a.wav"] == {"sent_id": "12", "text": "Hallo, Welt.", "gender": "MALE"}
    assert len(rows) == 2


def _plan(scene):
    rng = random.Random(scene.seed)
    return sc.plan_utterances(scene, rng, 30, first=True)


def test_scene_plan_is_deterministic_and_covers_vocabulary():
    for scene in sc.SCENES:
        a, b = _plan(scene), _plan(scene)
        assert [u["text"] for u in a] == [u["text"] for u in b]
        assert {u["speaker"] for u in a} == {s.name for s in scene.speakers}
        text = " ".join(u["text"] for u in a)
        assert not any(c.isdigit() for c in text)  # TTS-Text == Referenz, keine Ziffern
        assert "{" not in text and "}" not in text
        assert any(term.split()[0] in text for term, _ in scene.nouns_en)


def test_schedule_overlaps_only_across_channels():
    rng = random.Random(5)
    chans = [rng.choice(["mic", "system"]) for _ in range(200)]
    durs = [rng.uniform(2, 9) for _ in chans]
    starts = sc.schedule(durs, chans, random.Random(6), 0.5)
    last = {"mic": -1.0, "system": -1.0}
    overlaps = 0
    for i, (s, d, c) in enumerate(zip(starts, durs, chans)):
        assert s >= last[c]  # nie zwei Aeusserungen gleichzeitig im selben Kanal
        last[c] = s + d
        if i and s < starts[i - 1] + durs[i - 1]:
            overlaps += 1
            assert c != chans[i - 1]
    assert overlaps > 10
    assert starts == sc.schedule(durs, chans, random.Random(6), 0.5)


def test_check_fails_on_empty_dir_and_passes_on_minimal_corpus(tmp_path, monkeypatch):
    monkeypatch.setenv("LVA_BENCH_DIR", str(tmp_path))
    assert mc.cmd_check() == 1

    root = tmp_path / "fleurs"
    (root / "wav").mkdir(parents=True)
    sents = []
    for i in range(mc.MIN_SENTENCES):
        f = f"wav/{i}.wav"
        sf.write(root / f, np.zeros(16000 * 2, np.float32), 16000, subtype="PCM_16")
        sents.append({"sent_id": str(i), "file": f, "text": "hallo welt"})
    (root / "manifest.json").write_text(json.dumps({"sentences": sents}), encoding="utf-8")
    assert mc.check_fleurs()[0]
    (root / "wav" / "7.wav").unlink()  # fehlende Datei -> nicht vollstaendig
    assert not mc.check_fleurs()[0]


def _write_fleurs(root: Path, n: int) -> None:
    (root / "wav").mkdir(parents=True)
    sents = []
    for i in range(n):
        f = f"wav/{i}.wav"
        sf.write(root / f, np.zeros(16000 * 2, np.float32), 16000, subtype="PCM_16")
        sents.append({"sent_id": str(i), "file": f, "text": "hallo welt"})
    (root / "manifest.json").write_text(json.dumps({"sentences": sents}), encoding="utf-8")


def _write_scene(root: Path, scene) -> Path:
    """Kleinstmoegliche gueltige Szene: 180 s Stille je Spur, 12 Aeusserungen, eine Ueberlappung."""
    d = root / "synth" / scene.key
    d.mkdir(parents=True)
    dur_ms = 180_000
    for f in mc.SCENE_FILES[:4]:
        sf.write(d / f, np.zeros(16 * dur_ms, np.int16), 16000, subtype="PCM_16")
    utts = []
    for i in range(12):
        spk = scene.speakers[i % len(scene.speakers)]
        start = i * 10_000
        end = start + (12_000 if i == 0 else 8_000)  # 0 (mic) ueberlappt 1 (system)
        utts.append({"id": i, "speaker": spk.name, "channel": spk.channel, "text": "wort",
                     "start_ms": start, "end_ms": end})
    (d / "reference.json").write_text(json.dumps({"duration_ms": dur_ms, "utterances": utts}), encoding="utf-8")
    return d


def test_check_scene_passes_on_minimal_scene_and_catches_defects(tmp_path, monkeypatch):
    monkeypatch.setenv("LVA_BENCH_DIR", str(tmp_path))
    scene = sc.SCENES[0]
    d = _write_scene(tmp_path, scene)
    assert mc.check_scene(scene) == (True, "3.0 min, 12 Aeusserungen")

    ref = json.loads((d / "reference.json").read_text(encoding="utf-8"))
    no_overlap = dict(ref, utterances=[dict(u, end_ms=u["start_ms"] + 8_000) for u in ref["utterances"]])
    (d / "reference.json").write_text(json.dumps(no_overlap), encoding="utf-8")
    assert not mc.check_scene(scene)[0]

    (d / "reference.json").write_text(json.dumps(ref), encoding="utf-8")
    sf.write(d / "system.wav", np.zeros(16 * 1000, np.int16), 16000, subtype="PCM_16")  # Laenge weicht ab
    assert not mc.check_scene(scene)[0]

    (d / "mic.wav").unlink()
    ok, msg = mc.check_scene(scene)
    assert not ok and "mic.wav" in msg


def test_check_exit_code_on_full_mini_corpus(tmp_path, monkeypatch, capsys):
    monkeypatch.setenv("LVA_BENCH_DIR", str(tmp_path))
    monkeypatch.setattr(mc.sc, "SCENES", sc.SCENES[:1])  # eine Szene reicht fuer den Exit-Code-Pfad
    assert mc.main(["--check"]) == 1
    _write_fleurs(tmp_path / "fleurs", mc.MIN_SENTENCES)
    assert mc.main(["--check"]) == 1  # Szene fehlt noch
    d = _write_scene(tmp_path, sc.SCENES[0])
    assert mc.main(["--check"]) == 0
    assert "Korpus vollstaendig" in capsys.readouterr().out
    (d / "reference.json").write_text("{kaputt", encoding="utf-8")
    assert mc.main(["--check"]) == 1


def test_check_cli_exit_1_on_empty_dir(tmp_path):
    import os
    import subprocess
    env = dict(os.environ, LVA_BENCH_DIR=str(tmp_path))
    script = Path(__file__).resolve().parent / "make_corpus.py"
    p = subprocess.run([sys.executable, str(script), "--check"], env=env, capture_output=True, text=True, timeout=120)
    assert p.returncode == 1 and "UNVOLLSTAENDIG" in p.stdout


def test_check_fleurs_rejects_too_few_and_duplicate_ids(tmp_path, monkeypatch):
    monkeypatch.setenv("LVA_BENCH_DIR", str(tmp_path))
    root = tmp_path / "fleurs"
    _write_fleurs(root, mc.MIN_SENTENCES - 1)
    assert not mc.check_fleurs()[0]
    mf = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    mf["sentences"].append(dict(mf["sentences"][0]))  # 220 Eintraege, aber Duplikat
    (root / "manifest.json").write_text(json.dumps(mf), encoding="utf-8")
    assert not mc.check_fleurs()[0]


def test_table_and_doc_update(tmp_path):
    res = tmp_path / "results"
    res.mkdir()
    rows = [{"ref": "a b", "hyp": "a c", "errors": 1, "ref_words": 2, "errors_lenient": 1,
             "audio_s": 2.0, "transcribe_ms": 100, "load_ms": 500}]
    doc = {"alias": "parakeet-onnx", "spec": {"label": "X", "engine": "E", "device": "CPU"},
           "aggregate": sb.aggregate(rows), "rows": rows}
    (res / "parakeet-onnx.json").write_text(json.dumps(doc), encoding="utf-8")
    table = sb.build_table(res)
    assert "50,00 %" in table and "20,0×" in table
    md = tmp_path / "bench.md"
    md.write_text(f"kopf\n{sb.DOC_BEGIN}\nalt\n{sb.DOC_END}\nfuss\n", encoding="utf-8")
    assert sb.update_doc(md, table)
    out = md.read_text(encoding="utf-8")
    assert "alt" not in out and "kopf" in out and "fuss" in out and "50,00 %" in out


def test_ram_gate_aborts_when_not_enough_memory():
    with pytest.raises(SystemExit):
        benchlib.ram_gate(10 ** 9)
