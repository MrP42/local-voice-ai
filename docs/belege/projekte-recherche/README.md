# Belege zu lva#203 (ADR-0026 PR-7): Projektseite, Recherche, Touchziele, Kontrast, Vorlesen-Sperre

Maschinell aus dem Playwright-Lauf erzeugt, nichts von Hand eingetragen.

- Kopf der Läufe: Code `8f17ad26`; der Test `tests/projekte-recherche.spec.ts` schreibt die Messwerte seit Commit `b05ad3eb` (nur Testausgabe, keine Layoutänderung).
- Datum: 2026-11-02
- Ordner `apps/local-voice`, Browser Chromium (Playwright), Zeitzone Europe/Berlin, Sprache de-DE.

## Befehle

```
# Messwerte, axe, Listenausgabe (hell)
LVA_BELEGE=../../docs/belege/projekte-recherche npx playwright test tests/projekte-recherche.spec.ts --reporter=list
# TTS-Sperre
npx playwright test tests/projekte-recherche.spec.ts -g "Vorlesen" --reporter=list
# Dunkelbilder
LVA_THEME=dark LVA_SCREENSHOTS=1 npx playwright test tests/projekte-recherche.spec.ts --reporter=list
```

Ergebnis des Laufs: 22 von 22 Tests bestanden (`playwright-list.log`).

## Dateien

| Datei | Inhalt |
|---|---|
| `messwerte.md`, `messwerte-fluss.json`, `messwerte-touch.json` | 390 px: Scrollcontainer um das Ergebnis, Fensterhöhe, letzte Quelle erreichbar; boundingBox aller geprüften Touchziele und Checkbox-Zeilen |
| `axe-390-hell.json`, `axe-390-dunkel.json` | axe-core 4.13.0: Projektseite (alle Regeln) und Ordnerliste (`color-contrast`), Verstöße, Anzahl bestandener Regeln |
| `tts-sperre.log` | E2E-Ausgabe: PERSONAL mit Remote-TTS gesperrt mit Hinweis (1440 und 390 px), mit lokaler TTS frei; list-Reporter mit Testnamen |
| `playwright-list.log` | list-Reporter des Gesamtlaufs |
| `../../vorschau/projekte-recherche/*-dunkel-*.png` | Dunkelbilder (1440 und 390 px) |
