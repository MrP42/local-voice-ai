# Local Voice AI 0.18.0

Erstes Release seit 0.16.0. Enthält die Arbeit aus PR #24 (Sprachmodelle) und PR #25
(Vorlesen, Kontexthilfe, Multi-Device-Sync).

## Vorlesen

- Vorlesen ist eine Seite: Text links in voller Höhe, Bedienung rechts, Verlauf links,
  Dateien und Hilfe rechts. Unter 1100 px stapelt sich das Layout.
- Stimmen-Dropdown mit Piper-Stimmen (Sprache und Qualität im Namen), Stimme je Reiter
  (Original, Übersetzung, Zusammenfassung) bleibt gespeichert, „Stimmen verwalten …"
  springt in Einstellungen → Vorlesen.
- Piper erkennt die Sprache je Satz und wählt automatisch die passende Stimme
  (abschaltbar). Fünf englische Piper-Stimmen im Katalog (Lessac, Ryan, Amy, Alan, Alba).
- Piper liest keine Sprechermarker `<…>` und keine Stil-Tags `[…]` mehr vor; unbekannte
  Marker werden bei allen Engines gestrichen.
- Neu: „Text aufbereiten" entfernt Seitenzahlen, Kopfzeilen und Trennreste, ohne Inhalt
  zu verlieren. Zusammenfassung antwortet in der erkannten Sprache des Textes.
- Tempo als Wert in der Transportzeile mit Klappliste; Auto-Tagging-Anbieter und -Gerät
  in den Einstellungen.

## Sprachmodelle

- Katalog mit lokalen Modellen (L1–L4), Verbrauchs-Ledger, Speicherprognose je
  Architektur. Neu im Katalog: Qwen 3.5 (4B, 9B) und Gemma 4 (E4B, 12B).

## Multi-Device-Sync (Stufe 1)

- Konto & Geräte in den Einstellungen: Anmeldung mit Portal-Benutzer, Ende-zu-Ende
  verschlüsselt (Argon2id, XChaCha20-Poly1305), Konfliktkopien statt Datenverlust,
  gelöschte Seiten landen unter `projects/_geloescht/`.
- Erfordert den Portal-Hub mit Migration 0024 (Deploy separat).

## Oberfläche

- Kontexthilfe auf allen fünf Seiten (Reiter „Hilfe" rechts).
- Einstellungen an ihrem Ort: Stimmenverwaltung unter Einstellungen → Vorlesen,
  Stimmwechsler entfernt, Modelle-Seite auf Modelle beschränkt.

## Bekannt

- Updater-Signatur fehlt weiterhin (kein privater Schlüssel im CI); Installer per Hand.
- PR #21 (Strg hängt während Diktat) ist nicht enthalten.
