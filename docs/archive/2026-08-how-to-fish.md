So verwendest du es
Sofort anhören (deutsche Testdateien liegen bereit):


explorer C:\AI\fish-speech\output   # fish_speech_test.wav = der 11-s-Haupttest
Server starten / prüfen (falls nicht schon aktiv — aktuell läuft er):


C:\AI\fish-speech\start-fish-speech.ps1    # startet auf 127.0.0.1:8080
C:\AI\fish-speech\test-fish-speech.ps1     # Healthcheck, Exit 0 = PASS
TTS aus beliebiger Anwendung (PowerShell, Python, AI OS — gleiche API):


Invoke-WebRequest -Uri http://127.0.0.1:8080/v1/tts -Method Post `
  -ContentType "application/json" `
  -Body '{"text": "Hallo Patrick", "format": "wav"}' -OutFile hallo.wav
Deine Stimme klonen (zero-shot, kein Training): Lege reference.wav (10–30 s saubere Sprache) + reference.txt (exaktes Transkript) in C:\AI\fish-speech\voices\patrick\, dann einmal register-voice.ps1 -Voice patrick ausführen — danach einfach "reference_id": "patrick" im Request mitgeben. Status: NOT TESTED – reference audio required.

Eckdaten
Repo main @ e5e2926 (v2.0.0), isoliertes uv-venv mit Python 3.12.11 + PyTorch 2.8.0+cu126 (bewusst nicht cu129 — für sm_89 die reifste Variante), Modell zentral in C:\AI\models\fish-audio\s2-pro (10,5 GB, per Junction eingebunden). Deine globale Python-/CUDA-Landschaft ist unangetastet.

Drei Dinge musst du wissen
VRAM: Der Server belegt ~17,3 GB. Läuft parallel ComfyUI mit geladenen Modellen, lagert Windows in den RAM aus und alles wird 10–20× langsamer (gemessen: derselbe Satz 25 s vs. >570 s). Vor Nutzung ComfyUI-Modelle entladen oder die App schließen. Mein Versuch, ComfyUI per API zu entlasten, wurde vom Permission-System blockiert — das bleibt deine Entscheidung.
Latenz: Ohne torch.compile (Windows: Triton fehlt) liegt der Real-Time-Factor bei ~6 — für Batch-TTS okay, für einen Voice Assistant zu langsam. Echtes Streaming kommt aktuell erst am Ende an (TTFA ≈ Gesamtzeit). Die zwei dokumentierten Lösungspfade: triton-windows + --compile, oder WSL2 + SGLang (Upstream-Produktionspfad). Sag Bescheid, wenn ich einen davon angehen soll.
Lizenz: Fish Audio Research License — privat/Forschung/Evaluation frei, aber jede kommerzielle Nutzung inklusive interner Unternehmensnutzung braucht eine separate Lizenz (business@fish.audio). Für einen Einsatz bei Wolff Applied AI wäre das vorher zu klären.