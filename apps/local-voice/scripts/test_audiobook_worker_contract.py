"""Offline source-contract checks; Rust behaviour tests are run by the build thread."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1] / "src-tauri"


def contract(files):
    main = files["src/main.rs"]
    assert main.index('Some("--lva-audiobook-worker")') < main.index("CliArgs::parse()")
    assert '#[cfg(feature = "audiobook-worker")]' in main
    assert '#[cfg(not(feature = "audiobook-worker"))]' in main
    features = files["Cargo.toml"].split("[features]", 1)[1].split("[target.", 1)[0]
    assert "audiobook-worker = []" in features
    assert "default =" not in features
    protocol = files["src/audiobook_worker/protocol.rs"]
    assert protocol.index("len > MAX_FRAME_BYTES") < protocol.index("vec![0; len]")
    assert "version != VERSION" in protocol
    process = files["src/audiobook_worker/process.rs"]
    assert ".stderr(Stdio::null())" in process
    assert "creation_flags(priority_flags(tempo)" in process
    assert "JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE" in process
    assert "libc::nice(10)" in process
    worker = files["src/audiobook_worker/mod.rs"]
    for forbidden in ("println!", "eprintln!", "debug!", "info!", "warn!", "error!"):
        assert forbidden not in worker + protocol + process
    transcription = files["src/managers/transcription.rs"]
    assert transcription.count("fn build_local_engine(") == 1
    assert transcription.count("fn infer_local_segments(") == 1
    assert "transcription::build_local_engine(" in worker
    assert "transcription::infer_local_segments(" in worker
    assert "let loaded_engine = build_local_engine(" in transcription
    assert "infer_local_segments(" in transcription.split("pub fn transcribe_segments_detecting", 1)[1]


class Contract(unittest.TestCase):
    def test_contract_and_counterprobes(self):
        names = ["src/main.rs", "Cargo.toml", "src/managers/transcription.rs"]
        names += ["src/audiobook_worker/" + n + ".rs" for n in ("mod", "protocol", "process")]
        files = {n: (ROOT / n).read_text(encoding="utf-8") for n in names}
        contract(files)
        mutations = [
            ("src/main.rs", 'Some("--lva-audiobook-worker")', 'Some("--removed")'),
            ("src/audiobook_worker/protocol.rs", "len > MAX_FRAME_BYTES", "false"),
            ("src/audiobook_worker/protocol.rs", "version != VERSION", "false"),
            ("src/audiobook_worker/process.rs", ".stderr(Stdio::null())", ".stderr(Stdio::inherit())"),
            ("src/audiobook_worker/process.rs", "JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE", "0"),
            ("src/audiobook_worker/mod.rs", "transcription::infer_local_segments(", "duplicate_inference("),
        ]
        for name, old, new in mutations:
            with self.subTest(mutation=old):
                self.assertIn(old, files[name])
                changed = dict(files)
                changed[name] = changed[name].replace(old, new)
                with self.assertRaises((AssertionError, ValueError)):
                    contract(changed)
        names += ["src/lib.rs", "src/audiobook_worker/tests.rs"]
        for name in names:
            self.assertFalse(b"\r" in (ROOT / name).read_bytes(), name + " must use LF")


if __name__ == "__main__":
    unittest.main()
