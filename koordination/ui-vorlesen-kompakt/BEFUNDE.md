# Befunde des Planers — Vorlesen-Oberflaeche: einheitlich, kompakt, anpassbar

(Je Befund: Überschrift `## B<n> — <Paket>: <Titel>`, Zeilen Beobachtung / Beleg / Konsequenz und
eine Zeile `- Status: offen` bzw. `- Status: erledigt (<Paket/Commit>)`. Offene Befunde verhindern COMPLETE.)

## B1 — P4: Backslash im Audio-Dateinamen nicht gefiltert, Schwanz der Dateileiste unbegrenzt
- Beobachtung: `safeName` in `exportName.ts` filterte `[\/:*?"<>|]` – `\/` ist nur ein maskierter Schrägstrich, der Backslash (Pfadtrenner unter Windows) blieb im Namen. `splitFileNameTail` nahm den Zeitstempel auch mitten im Namen als festen, nicht schrumpfenden Schwanz.
- Beleg: Diff-Review; der Test „audio names never carry characters Windows refuses“ enthielt keinen Backslash.
- Konsequenz: Planer-Einzeiler bei der Validierung (Regex `[\\/:*?"<>|]`, Schwanz nur bei ≤ 24 Zeichen), Tests um Backslash und Stempel-mitten-im-Namen ergänzt. Lehre für den Lehren-Block: Regex mit Backslash-Klassen immer mit einem Backslash-Testfall belegen.
- Status: erledigt (P4, c0da046)
