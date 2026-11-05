Windows-Build von Local Voice AI. Neuerungen: `docs/releases/<Version>.md` im Repository.

| Datei | Wofür |
|---|---|
| `Local.Voice.AI_<Version>_x64-setup.exe` | Installer. Die installierte App aktualisiert sich hierauf selbst. |
| `LocalVoiceAI_<Version>_x64-portable.zip` | **Portable Version ohne Installation (ca. 0,8 GB).** Entpacken, `local-voice-ai.exe` starten. Spracherkennung Nemotron 3.5 ASR ist enthalten und eingestellt, Diktat und Transkription laufen ohne Download. Weitere Modelle (KI, Vorlesen) lädt die App bei Bedarf. |
| `LocalVoiceAI_<Version>_x64-portable-offline.teil1vonN.zip` … | **Portable Offline-Version (ca. 4 GB)**, läuft komplett ohne Internet. ALLE Teile herunterladen und in denselben Ordner entpacken. Enthält zusätzlich Phonon (Programm, Englisch mit allen Stimmen, Deutsch soweit beim Bau verfügbar), die lokale KI-Laufzeit (Vulkan/CPU), Qwen 3.5 4B für Zusammenfassung und Protokoll sowie den Stimmabdruck CAM++. Das große Sprachmodell setzt die App beim ersten Start aus den Teilen zusammen (einmalig, ca. 1 min). |

Was genau vorinstalliert ist, steht in `INHALT.txt` im ZIP. Anleitung und Datenablage: `LIESMICH-PORTABLE.txt`. Lizenzen: `THIRD-PARTY-NOTICES.md` und `LIZENZEN-MODELLE.txt`. Das deutsche Phonon-Modell steht unter CC BY-NC 4.0 und ist nur für nicht-kommerzielle Nutzung freigegeben.
