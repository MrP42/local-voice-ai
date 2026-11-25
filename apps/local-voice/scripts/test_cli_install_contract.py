"""Offline source contracts for #831; Rust behaviour tests run in the Windows gate."""
import pathlib
import json
import re
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def read(name):
    path = ROOT / name
    return path.read_text(encoding="utf-8") if path.exists() else ""


def wired(source):
    return all(f"commands::llm::{name}," in source for name in (
        "llm_cli_download", "llm_cli_cancel_download", "llm_cli_set_path",
        "llm_cli_set_download_policy",
    ))


class Contract(unittest.TestCase):
    def test_all_locales_have_part_a_and_b_keys_and_counterprobe(self):
        reference = json.loads(read("src/i18n/locales/en/translation.json"))

        def leaves(data, prefix=()):
            for key, value in data.items():
                if isinstance(value, dict):
                    yield from leaves(value, prefix + (key,))
                else:
                    yield prefix + (key,)

        def missing(data):
            actual = set(leaves(data))
            return [key for key in required if key not in actual]

        required = [key for key in leaves(reference) if
                    key[:4] == ("settings", "llm", "cliStatus", "install") or
                    key[:2] == ("voiceCall", "fehler")]
        self.assertGreater(len(required), 40)
        for path in (ROOT / "src/i18n/locales").glob("*/translation.json"):
            self.assertNotIn(b"\r", path.read_bytes(), path.parent.name)
            data = json.loads(path.read_text(encoding="utf-8"))
            self.assertEqual(missing(data), [], path.parent.name)
            del data["settings"]["llm"]["cliStatus"]["install"]["download"]
            self.assertIn(("settings", "llm", "cliStatus", "install", "download"), missing(data))

    def test_commands_registered_and_counterprobe(self):
        source = read("src-tauri/src/lib.rs")
        self.assertTrue(wired(source))
        self.assertFalse(wired(source.replace("commands::llm::llm_cli_download,", "")))

    def test_ui_has_explicit_download_and_alternatives(self):
        source = read("src/components/settings/post-processing/LlmConnectionsSettings.tsx")
        for name in ("llmCliDownload", "llmCliCancelDownload", "llmCliSetPath", "llmCliSetup"):
            self.assertTrue(name in source, name)
            self.assertFalse(name in source.replace(name, ""), name)

    def test_policy_survives_stale_settings_writes(self):
        source = read("src-tauri/src/settings.rs")
        start = source.index("fn richtlinie_festhalten(")
        end = source.index("\n}", start)
        self.assertTrue("cli_download_disabled" in source[start:end])

    def test_de_en_placeholder_keys_match_without_duplicates(self):
        def unique(pairs):
            result = {}
            for key, value in pairs:
                if key in result:
                    raise ValueError(f"duplicate key: {key}")
                result[key] = value
            return result

        def leaves(data, prefix=""):
            return {f"{prefix}{key}": value for key, value in data.items()}

        translations = []
        for lang in ("de", "en"):
            data = json.loads(read(f"src/i18n/locales/{lang}/translation.json"), object_pairs_hook=unique)
            install = data["settings"]["llm"]["cliStatus"]["install"]
            flat = {}
            for key, value in install.items():
                flat.update(leaves(value, f"{key}.") if isinstance(value, dict) else {key: value})
            translations.append(flat)
        self.assertEqual(translations[0].keys(), translations[1].keys())
        self.assertIn("{{ziel}}", translations[0]["errors.netz_gesperrt"])
        with self.assertRaises(ValueError):
            json.loads('{"x": 1, "x": 2}', object_pairs_hook=unique)

    def test_documented_hosts_exactly_match_generated_table(self):
        source = read("src-tauri/src/netz/ziele.rs")
        rows = re.findall(r'z\(\s*"([^"]+)",\s*"([^"]+)",\s*"([^"]+)",?\s*\)', source)
        table = "| Funktion | Adresse | Port | Zweck |\n|---|---|---|---|\n"
        table += "\n".join(f"| {f} | {h} | 443 | {p} |" for f, h, p in rows)
        docs = (ROOT.parents[1] / "docs/FIRMENNETZ.md").read_text(encoding="utf-8")
        actual = docs.split("<!-- ZIELE:BEGIN -->")[1].split("<!-- ZIELE:END -->")[0].strip()
        self.assertEqual(actual, table)
        self.assertNotEqual(actual.replace("downloads.claude.ai", "invalid"), table)

    def test_download_has_no_execution_or_content_logging(self):
        source = read("src-tauri/src/managers/llm/cli_install.rs")
        for forbidden in ("Command::", "log::", ".extract("):
            self.assertFalse(forbidden in source, forbidden)
            self.assertTrue(forbidden in (source + forbidden))

    def test_failed_final_check_has_no_path_cleanup_and_counterprobe(self):
        source = read("src-tauri/src/managers/llm/cli_install.rs")
        check = source.split("fn endpruefung(", 1)[1].split("\nfn kontrollieren(", 1)[0]

        def deletes_by_path(body):
            return re.search(r"\b(?:remove_file|remove_dir|remove_dir_all|rename)\s*\(", body) is not None

        self.assertFalse(deletes_by_path(check))
        self.assertTrue(deletes_by_path(check + "std::fs::remove_file(target);"))

    def test_dialog_capability_and_progress_type_export(self):
        capability = json.loads(read("src-tauri/capabilities/default.json"))
        self.assertIn("dialog:default", capability["permissions"])
        source = read("src-tauri/src/lib.rs")
        registration = ".typ::<managers::llm::cli_install::InstallFortschritt>()"
        self.assertTrue(registration in source)
        self.assertFalse(registration in source.replace(registration, ""))

    def test_new_files_and_modified_files_use_lf(self):
        files = (
            "src-tauri/src/commands/llm.rs", "src-tauri/src/lib.rs",
            "src-tauri/src/managers/llm/cli.rs", "src-tauri/src/managers/llm/cli_netz.rs",
            "src-tauri/src/managers/llm/mod.rs", "src-tauri/src/managers/llm/cli_install.rs",
            "src-tauri/src/managers/llm/cli_install_tests.rs", "src-tauri/src/settings.rs",
            "src-tauri/src/netz/ziele.rs", "src-tauri/src/netz/firmentauglich_tests.rs",
            "src/components/settings/post-processing/LlmConnectionsSettings.tsx",
            "src/bindings.ts", "src/i18n/locales/de/translation.json",
            "src/i18n/locales/en/translation.json", "scripts/test_cli_install_contract.py",
        )
        for name in files:
            self.assertFalse(b"\r" in (ROOT / name).read_bytes(), name)
        self.assertFalse(b"\r" in (ROOT.parents[1] / "docs/FIRMENNETZ.md").read_bytes())

    def test_existing_script_bans_remain_enforced(self):
        forbidden = (
            "-encodedcommand", "-executionpolicy", "executionpolicy bypass",
            "| iex", "|iex", "invoke-expression", "downloadstring(",
            "irm http", "iwr http", "invoke-webrequest", "curl -fssl",
            "| bash", "| sh -", "frombase64string",
            'new("npm"', 'new("npm.cmd"',
        )
        for pattern in forbidden:
            self.assertTrue(pattern in pattern.upper().lower())
        for path in (ROOT / "src-tauri/src").rglob("*.rs"):
            if path.name in ("firmentauglich_tests.rs", "ziele.rs"):
                continue
            source = path.read_text(encoding="utf-8").lower()
            for pattern in forbidden:
                self.assertFalse(pattern in source, f"{path.name}: {pattern}")

    def test_selected_exe_login_uses_direct_process(self):
        source = read("src-tauri/src/managers/llm/cli.rs")
        call = "native_login_command(&program, &argv)"
        self.assertTrue(call in source)
        self.assertFalse(call in source.replace(call, ""))


if __name__ == "__main__":
    unittest.main()
