# Befunde des Planers — Vorlesen-Oberflaeche: einheitlich, kompakt, anpassbar

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B1 — P4: Backslash im Audio-Dateinamen nicht gefiltert, Schwanz der Dateileiste unbegrenzt
- Beobachtung: `safeName` in `exportName.ts` filterte `[\/:*?"<>|]` – `\/` ist nur ein maskierter Schrägstrich, der Backslash (Pfadtrenner unter Windows) blieb im Namen. `splitFileNameTail` nahm den Zeitstempel auch mitten im Namen als festen, nicht schrumpfenden Schwanz.
- Beleg: Diff-Review; der Test „audio names never carry characters Windows refuses“ enthielt keinen Backslash.
- Konsequenz: Planer-Einzeiler bei der Validierung (Regex `[\\/:*?"<>|]`, Schwanz nur bei ≤ 24 Zeichen), Tests um Backslash und Stempel-mitten-im-Namen ergänzt. Lehre für den Lehren-Block: Regex mit Backslash-Klassen immer mit einem Backslash-Testfall belegen.
- Status: erledigt (P4, c0da046)

## B2 — P3/P5/P7: Test-Specs schreiben bei jedem Lauf Screenshots ins Repo
- Beobachtung: `readaloud-palette.spec.ts` (P3/P7) und `readaloud-layout.spec.ts` (P5) schreiben ihre PNGs unter `koordination/…/screens/` bei JEDEM Testlauf neu; dazu ohne `animations: "disabled"` – das P7-Bild zeigte deshalb mitten in der Farbüberblendung „Favoriten“ statt „Alle“ als aktiv.
- Beleg: `git status` nach der Gesamtsuite im Integrationszweig: 6 geänderte PNGs; P7-Bild mit `animations: "disabled"` neu aufgenommen → „Alle“ korrekt aktiv.
- Konsequenz: Screenshot-Tests nur mit `LV_SHOTS=1` (wie P2) und immer `animations: "disabled"`; Nacharbeit P8.
- Status: erledigt (P8, c8176e0 — alle Aufnahmen nur mit SCREENS_DIR, Suite ohne SCREENS_DIR lässt koordination/ unverändert)

## B3 — Integration P2+P6: neue Toolbar-Tests suchten die Editor-Reiter als `button`
- Beobachtung: P6 machte die Editor-Reiter zu `role="tab"`; zwei in P2 parallel entstandene Tests (`readaloud-toolbar.spec.ts`) klickten sie noch als `button` → Timeout.
- Beleg: Gesamtsuite nach Merge P6: 2 failed (Z. 152, 326); nach Umstellung auf `getByRole("tab")` 116 passed, 7 skipped.
- Konsequenz: Planer-Einzeiler im Integrationszweig; Lehre: parallele Pakete, die Rollen ändern, im Briefing gegenseitig nennen.
- Status: erledigt (Integration, siehe Commit „test: Editor-Reiter als tab“)

## B4 — P2+P5: ☰-Menü wird von der gestapelten Bedienspalte abgeschnitten
- Beobachtung: Menü/Popover (P2, `ActionMenu.tsx`) sind absolut im Dokumentfluss positioniert; die gestapelte Bedienspalte (P5) scrollt in sich (max-height 50 %). Nach dem Verdichten durch P2 ist sie nur ~184 px hoch → vom Menü ist nur „Skript-Werkstatt…“ sichtbar, die Spalte scrollt beim Öffnen.
- Beleg: Nachher-Bild `screens/nachher/menue-offen-1920.png` (P8), Messung controls y 79–263, Einträge 263–364. Tests prüften nur `toBeVisible()`.
- Konsequenz: Nacharbeit P9 (Portal + fixed, Test per `elementFromPoint`). Lehre: Sichtbarkeit in overflow-Containern nie per `toBeVisible` belegen; Nachher-Bilder VOR der Abnahme ansehen.
- Status: offen
