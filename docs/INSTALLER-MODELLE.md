# Installer mit Modellen (Windows)

Stand: 13.11.2026, lva#574. Auftrag Patrick: „der installer soll ohne download Nemotron 3.5 ASR und Parakeet V3 mitbringen (preinstalled, bzw. während der installation abwählbar), ebenso soll Phonon mit allen DE und EN stimmen preinstalled sein“. Anlass: Hinter Behörden-Proxys scheitern Downloads von Hugging Face.

## Was am Release liegt

| Datei | Inhalt | Größe (ca.) |
|---|---|---|
| `Local Voice AI_<V>_x64-setup.exe` | Schlanker Installer, Quelle des In-App-Updates (`latest.json`) | ~150 MB |
| `LocalVoiceAI_<V>_x64-setup-mit-modellen.exe` | Derselbe Installer plus Komponentenseite mit Modellen | ~1,5 GB |

Komponenten (alle vorausgewählt, einzeln abwählbar):

| Komponente | Katalogkennung | Unkomprimiert | Lizenz |
|---|---|---|---|
| Local Voice AI (Programm) | – | ~150 MB | MIT (App), siehe THIRD-PARTY-NOTICES |
| Spracherkennung Nemotron 3.5 ASR Streaming (Q8_0) | `nemotron-3.5-asr-streaming-0.6b-Q8_0` | 751 MB | OpenMDW-1.1 |
| Spracherkennung Parakeet V3 (ONNX int8) | `parakeet-tdt-0.6b-v3` | ~670 MB (Download 456 MB) | CC BY 4.0 |
| Vorlesen Phonon: Programm `ptts`, Deutsch 40m, Englisch 31m, 13 deutsche und 37 englische Stimmen | `phonon-runtime`, `phonon-40m-de`, `phonon-31m-en` | ~160 MB | MIT/Apache-2.0; EN Apache-2.0; **DE CC BY-NC 4.0** |

## Warum ein zweiter Installer statt Modelle im einzigen Installer

Die App aktualisiert sich über den Installer, auf den `latest.json` zeigt. Lägen die Modelle in diesem Installer, lüde jedes Update rund 1,5 GB statt 150 MB. Deshalb gibt es zwei Dateien aus derselben Vorlage (`src-tauri/nsis/installer.nsi`):

- Ohne Umgebungsvariable `LVA_VORINSTALLIERT` beim Bau: schlanker Installer, unverändert.
- Mit `LVA_VORINSTALLIERT=<Nutzlast>`: zusätzlich Komponentenseite und je Modell ein Abschnitt.

Die Modelle liegen nach der Installation im Datenordner der App, nicht im Programmordner. Updates über den schlanken Installer lassen sie unberührt, die Deinstallation auch (wie bei geladenen Modellen). Nur „App-Daten löschen“ bei der Deinstallation entfernt sie.

## Grenzen

- **NSIS:** Ein Installer darf höchstens 2 GB groß sein. Erwartet ~1,5 GB (Modellgewichte komprimieren kaum). Das Bauskript bricht ab, wenn die Nutzlast 1,75 GiB übersteigt, der Workflow, wenn die EXE 1,95 GiB erreicht.
- **GitHub:** höchstens 2 GiB je Release-Datei. Eine Aufteilung in Zusatzpakete ist bei diesem Umfang nicht nötig. Käme ein lokales Sprachmodell (Qwen 3.5 4B, 2,7 GB) dazu, ginge das nicht mehr in einen Installer; dafür gibt es das portable Offline-Paket (siehe [PORTABLE.md](PORTABLE.md)).
- **Mac:** noch nicht enthalten, eigener Folge-PR (DMG mit Modellen).

## Ablage und erster Start

| Teil | Installiert | Portabel (Option im Installer) |
|---|---|---|
| Spracherkennung | `%APPDATA%\de.wolffappliedai.localvoiceai\models` (`app_data_dir`) | `Data\models` neben der EXE |
| Phonon | `%LOCALAPPDATA%\de.wolffappliedai.localvoiceai\tts\phonon` (`app_local_data_dir`) | `Data\tts\phonon` |

Vorhandene Dateien überschreibt der Installer nicht. Er schreibt `erststart.ini` mit dem ersten gewählten Spracherkennungsmodell (Nemotron vor Parakeet), sofern die Datei noch fehlt. Beim ersten Start stellt die App dieses Modell ohne Download ein (`firstRunSetup.ts`), auch wenn daneben Parakeet installiert ist. Die Seite „Diktat“ des schlanken Installers entfällt in diesem Installer.

Bei einer Neueinrichtung ist Phonon installiert, aber nicht eingeschaltet. Bei einer Aktualisierung bleiben vorhandene Einstellungen erhalten, ein schon eingeschaltetes Phonon bleibt also an. Einschalten unter Einstellungen › Ausgabe › Vorlesen, dabei bestätigt die Person den Lizenzhinweis (CC BY-NC beim deutschen Modell). Das geht offline.

## Lizenzen

Befund zu Phonon DE siehe [PORTABLE.md](PORTABLE.md#lizenzbefund-phonon-prüfung-04112026). Kurz: CC BY-NC 4.0 erlaubt Vervielfältigung und Weitergabe nur zu nicht-kommerziellen Zwecken (Begriff „nicht kommerziell“ in Abschnitt 1, Lizenzgewährung in Abschnitt 2(a)(1)) und unter Einhaltung der Lizenzbedingungen (Namensnennung über `LIZENZEN-MODELLE.txt` neben der EXE und `voices\SOURCE.json`, Lizenz unverändert, keine zusätzlichen Beschränkungen). Dass der Installer kostenlos ist, genügt dafür allein nicht: Maßgeblich ist, ob die Weitergabe oder Nutzung vorrangig auf einen geschäftlichen Vorteil oder eine Vergütung zielt. Eine kostenlose, aber geschäftlich motivierte Weitergabe, etwa als Beigabe zu einer bezahlten Beratung oder zur Kundengewinnung, ist nicht gedeckt. Für solche Fälle das deutsche Modell nach der Installation entfernen oder die Komponente „Vorlesen Phonon“ abwählen (dann fehlt auch Englisch, es lässt sich später laden). Ob ein konkreter Einsatz, etwa in der Verwaltung, nicht-kommerziell ist, ist keine Frage dieses Dokuments; siehe den Hinweis in PORTABLE.md. Parakeet V3 (CC BY 4.0) verlangt Namensnennung, ebenfalls in `LIZENZEN-MODELLE.txt`. Fehlt beim Bau `HF_TOKEN`, bricht der Release-Schritt ab (`-RequireHfToken`); lokal lässt das Skript das deutsche Modell mit Warnung aus.

## Bauen

```powershell
# aus apps/local-voice, nach `pnpm tauri build --features gpu-vulkan`
$env:HF_TOKEN = '…'
pwsh scripts/build-installer-modelle.ps1 -RequireHfToken -SmokeTest -OutDir D:/lv-build/vorinstalliert
$env:LVA_VORINSTALLIERT = 'D:/lv-build/vorinstalliert'
'{"bundle":{"createUpdaterArtifacts":false}}' | Set-Content $env:TEMP/ohne-updater.json
pnpm tauri build --features gpu-vulkan --bundles nsis --config $env:TEMP/ohne-updater.json
```

Tests: `node --test scripts/installer-vorinstalliert.test.mjs` (Wächter in der Vorlage, braucht makensis, sonst übersprungen; Lizenzaussage) und `pwsh scripts/test-build-installer-modelle.ps1` (`-OutDir` löscht nie fremde Dateien: ein vorhandener Ordner wird nur ersetzt, wenn er leer ist oder die Marke `.lva-vorinstalliert-nutzlast` trägt). Ist `LVA_VORINSTALLIERT` gesetzt, die Nutzlast aber nicht vorhanden, bricht makensis ab.

Das Skript lädt jede Komponente einzeln über `--provision` in einen portablen Probeordner, derselbe Ladeweg wie die Modellseite. Der Rauchtest legt alles zusammen und prüft ohne HF-Cache mit `--list-models`, dass Nemotron und Parakeet als installiert gelten. Im Release-Workflow (`.github/workflows/release-windows.yml`, Schritt „Build installer with models“) läuft das nach den portablen ZIPs, auch beim Nachpacken (`repack_tag`).
