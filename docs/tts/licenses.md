# TTS-Anbieter: Lizenzen

Stand 26.10.2026 · Goal `tts-provider` (Paket D1). Quellen: [provider-research.md](provider-research.md) (alle Quellen am
26.10.2026 abgerufen), Lizenzfelder im Code (`LicenseInfo`, `providers/contract.rs:118`), Katalog
`apps/local-voice/src-tauri/src/catalog/catalog.json`. Dies ist eine technische Zusammenstellung, **keine Rechtsberatung**.

**Regel: Nichts als freigegeben darstellen, was nicht verifiziert ist.** `verified = true` gibt es nur mit Quelle UND
Prüfdatum (`source_url`, `verified_on`); sonst `false` mit `note`, was noch zu prüfen ist (contract.rs:114-117, durch
`contract_suite` erzwungen, contract_tests.rs:166-181). Unbekannte Gewichte gelten vorsorglich als `non_commercial`
(phonon.rs:618-641, ollama_speech.rs:236-250). Nutzerangaben beim generischen Server sind immer unverifiziert
(openai_speech.rs:1025-1050). „Verifiziert“ heißt: steht so in der Primärquelle, nicht: rechtlich geprüft.

## 1. Übersicht

| Anbieter / Modell | Code-Lizenz | Gewichte / Stimmen | nichtkommerziell | verifiziert (Quelle, Datum) | Auslieferung erlaubt? |
|---|---|---|---|---|---|
| `fish` / s2-pro | Apache-2.0 (aus dem Plan, **nicht** gegengeprüft; builtin.rs:133-146) | ungeprüft: Model Card nötig | unbekannt (im Code `false`) | **nein** (kein Datum, builtin.rs:133-139) | **nicht entschieden** |
| `piper` (Programm) | GPL-3.0 (piper1-gpl, provider-research.md, C; 26.10.2026) | je Stimme | je Stimme | Code ja (Recherche), im Deskriptor `false` (builtin.rs:320-332) | offen: GPL bei separatem Prozess juristisch zu klären |
| Piper `de_DE-thorsten-high` | siehe Programm | Katalog: `CC0-1.0`, `non_commercial: true` (catalog.json:5223-5230) | ja (Katalogwert) | **Widerspruch** (Abschnitt 2) | nein, bis geklärt |
| Piper thorsten/kerstin (HF-Karten) | siehe Programm | Dataset CC0 laut Model Card | nicht festgestellt | ja (Recherche C, 26.10.2026), im Code nie `verified` (builtin.rs:273-287) | offen |
| Piper übrige deutsche Stimmen (eva_k, karlsson, mls, pavoque, ramona) | siehe Programm | nicht geprüft | unbekannt | nein | nein |
| `phonon` / phonon-31m-en | MIT OR Apache-2.0 (README, 26.10.2026) | Apache-2.0 (README-Tabelle, HF-Karte, 26.10.2026) | nein | ja, `https://huggingface.co/gradium/phonon-31m-en`, 2026-10-26 (phonon.rs:643-657) | ja nach Quellenlage (nur Englisch); Auslieferung der Gewichte nicht geprüft |
| `phonon` / phonon-40m-de, -multilingual | MIT OR Apache-2.0 | CC BY-NC 4.0, gated (HF-Karte, 26.10.2026) | **ja** | ja, `https://huggingface.co/gradium/phonon-40m-de`, 2026-10-26 (phonon.rs:658-673) | **nein** (nichtkommerziell) |
| `phonon` / anderes Modell | MIT OR Apache-2.0 | unbekannt | vorsorglich ja | nein (phonon.rs:618-641) | nein |
| `ollama-speech` / `phonon:40m-de`, `phonon:de`, `phonon:multilingual` | MIT OR Apache-2.0 (Phonon-Code) | CC BY-NC 4.0 (Ollama-Bibliothek, 26.10.2026) | **ja** | ja, `https://ollama.com/library/phonon`, 2026-10-26 (ollama_speech.rs:216-234) | **nein** |
| `ollama-speech` / anderes Modell (auch `phonon` ohne Tag) | wie oben | nicht gegengeprüft (Gradium-31M: Apache-2.0) | vorsorglich ja | nein (ollama_speech.rs:236-250) | nein |
| `openai-speech` | je Server | je Server und Modell | laut Nutzerangabe | **nie** (Nutzerangabe, openai_speech.rs:1042) | nur nach eigener Prüfung |
| ptts-openai-server | MIT/Apache (provider-research.md, D) | je geladenem Modell | je Modell | Code ja (Recherche), Gewichte je Modell | wie das Modell |
| Kokoro-FastAPI / Kokoro-82M | Apache-2.0 | Apache-2.0 (HF, 26.10.2026) | nein | ja (Recherche C), kein Adapter-Deskriptor | ja nach Quellenlage, **kein Deutsch** |
| `mock` | n. a. | n. a. | n. a. | n. a. | nur Tests |
| XTTS-v2, F5-TTS | kein Adapter | CPML (nichtkommerziell) bzw. CC-BY-NC | ja | ja (Recherche C) | nein |

Hinweise: Der Piper-Programmcode läuft als **separater Prozess**; ob das die GPL-Pflichten für die App berührt, ist laut
Recherche und Deskriptor-Note juristisch offen (builtin.rs:320-332, provider-research.md, Offene Punkte).
Phonon-Stimmen haben eigene Lizenzen laut `docs/cli.md` (phonon.rs:626-630); die App führt dafür kein Feld je Stimme für
Phonon (`VoiceDescriptor.license` ist optional, contract.rs:206).

## 2. Widersprüche und offene Punkte

* **B3 Piper `thorsten-high`**: provider-research.md nennt den thorsten-Datensatz CC0 (Model Cards von medium/high, Recherche
  C). Der App-Katalog führt den Eintrag mit `"license": "CC0-1.0"` und zugleich `"non_commercial": true`; die Notiz begründet
  das mit der Ableitung (Feinabstimmung von der englischen Lessac-Stimme, deren Trainingsdaten Blizzard 2013 nur zur
  Forschung freigegeben sind) und nennt die Lizenzlage des Modells „ungeklärt“ (catalog.json:5223-5230). Konsequenz: die
  Hülle bleibt konservativ, `verified = false`, `non_commercial` aus dem Katalog (builtin.rs:268-287, BEFUNDE.md B3).
  **Beide Aussagen können zugleich stimmen** (Datensatz CC0, Modell abgeleitet); geklärt ist die Auslieferbarkeit damit
  nicht. Entscheidung beim Owner. Status: offen.
* **Fish-Gewichte ungeprüft**: Code-Lizenz Apache-2.0 stammt aus dem Plan, nicht aus einer gegengeprüften Quelle; Lizenz der
  s2-pro-Gewichte steht „laut Model Card prüfen“ aus (builtin.rs:133-146, implementation-plan.md §4). Im Plan-Katalog hat
  Fish keinen Lizenzeintrag (implementation-plan.md §1).
* **Phonon 40m (Deutsch)**: die einzigen deutschen Phonon-Gewichte sind nichtkommerziell und gated; eine kommerzielle Lizenz
  bei Gradium wurde nicht gefunden (provider-research.md, Offene Punkte). Owner-Entscheidung (Goal O1, implementation-plan.md §7).
* **Übrige deutsche Piper-Stimmen**: Lizenzen einzeln zu prüfen.
* **GPL-3.0 von piper1-gpl**: Wirkung der Prozess-Anbindung auf die App-Lizenz juristisch offen.
* **Ollama-Fassung von `phonon` (31M)**: Lizenz der Ollama-Fassung nicht gegengeprüft, daher vorsorglich nichtkommerziell.
* **Kokoro-Stimmen** deutsch: nicht verifiziert (Recherche C).
* **Windows-Lauf** von Ollama/ptts-server betrifft die Funktion, nicht die Lizenz; siehe
  [installation-troubleshooting.md](installation-troubleshooting.md).

## 3. Durchsetzung in der App

Umgesetzt (P3a, `apps/local-voice/src-tauri/src/managers/tts/providers/policy.rs`, hier **P/**) und getestet:

* **Eine Prüfstelle**: `check_use` (P/policy.rs:513) entscheidet vor jeder Nutzung (Vorlesen, Probestimme, Stimmenliste,
  Auswahl, CLI, Brückenwerkzeuge), nicht im Adapter und nicht erst im Tauri-Befehl. Reihenfolge:
  1. `opt_in`: Kennung muss in `enabled` stehen, sonst `LicenseBlocked`.
  2. `opt_in && (non_commercial || !verified)` (`license_needs_ack`, P/policy.rs:484): ein `license_ack` mit aktuellem
     Fingerabdruck, sonst `LicenseBlocked`.
  3. Adresse außerhalb von Loopback: Kennung in `allow_remote`, sonst `PrivacyBlocked` (unabhängig von `opt_in`).
* **Fingerabdruck** (`license_fingerprint`, P/policy.rs:461): SHA-256 über längenpräfixierte Teile von Kennung,
  Gewichts-Lizenz (`weights`), `nc`/`c` und Modell, erste 32 Hexzeichen. Ändert sich Modell oder Lizenzangabe, gilt die
  Zustimmung nicht mehr und wird erneut verlangt. Der Algorithmus ist durch einen Referenzwert-Test festgeschrieben.
  Bestätigt wird nur zum **aktuellen** Fingerabdruck (`acknowledge_in`, `commands/tts_providers.rs:145`).
* **Nur über die UI setzbar**: `tts_provider_set_enabled`, `tts_provider_acknowledge_license`,
  `tts_provider_set_allow_remote` (`commands/tts_providers.rs:305, 318, 342`). Weder Agentenbrücke noch CLI noch
  Umgebungsvariable noch `tts_provider_set_config` können sie setzen (P/policy.rs:25-29, 39-44). Die CLI meldet einen
  gesperrten Anbieter mit Exit 3 und nennt die fehlende Freigabe ([cli-and-automation.md](cli-and-automation.md)).
* Die Sperre hängt nicht allein an `!verified`: Fish und Piper (`opt_in = false`) sind nie gesperrt (P/policy.rs:479-483;
  builtin.rs:22-27). Neue Anbieter sind alle `opt_in = true`.
* Wirkung auf die Tabelle oben: `phonon` 40m-de und `ollama-speech` (`phonon:de`) sind nichtkommerziell, brauchen also
  Opt-in **und** Lizenz-Bestätigung; `phonon` 31m-en (verifiziert, Apache-2.0) braucht nur das Opt-in; `openai-speech` ist
  nie `verified` und braucht daher immer die Bestätigung.

**Grenzen der Umsetzung** (siehe auch [CHANGELOG.md](CHANGELOG.md)):

* Es gibt **keinen Widerruf-Befehl** für eine Lizenz-Zustimmung. Ein Opt-out nimmt nur die Freischaltung zurück; die
  gespeicherte Zustimmung bleibt, gilt aber nur bei unverändertem Fingerabdruck.
* Die Zustimmung bestätigt, dass der Nutzer die Angabe gesehen hat, nicht dass die Lizenz rechtlich geprüft ist.

## 4. Pflege

Neue Zeile in Abschnitt 1 mit Quelle (URL), Abrufdatum und Wortlaut der Lizenzaussage; erst dann `verified = true` mit
`verified_on` im Deskriptor setzen. Ändert sich eine Quelle, Datum aktualisieren oder `verified` zurücknehmen.
