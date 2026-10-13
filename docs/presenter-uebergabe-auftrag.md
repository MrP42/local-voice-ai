# Auftrag für den WAI Presenter: Besprechungen aus Local Voice AI übernehmen

Stand 13.10.2026. Gegenstück zu Local Voice AI (LVA), Zweig `feat/dokumenttypen-export`
(Entwurf `docs/superpowers/specs/2026-10-13-dokumenttypen-export-presenter-design.md`, Abschnitt 5).
LVA schreibt nie in `wai.vortrag/1`; der Presenter liest ein eigenes Paketformat und legt daraus
selbst ein Projekt an. Dieser Auftrag beschreibt, was im Repo `wai-presenter` zu bauen ist.

## 1. Austauschordner

Den Ordner wählt der Nutzer in beiden Apps selbst (LVA: Integrationen > WAI Presenter; Presenter:
Bibliothek > „Aus Local Voice importieren“ > Ordner wählen, rechnerlokal speichern wie
`bibliothek.json`). Es gibt keinen Standardordner und keinen Vorschlag in OneDrive.

```
<Austausch>/
  katalog.json                       LVA schreibt: wai.besprechung-katalog/1
  pakete/<datum>-<titel>-<id8>[-vN]/ LVA schreibt: ein Paket je Sendung
    besprechung.json                 wai.besprechung/1 (Manifest, zuletzt geschrieben)
    transkript.md  transkript.json
    dokumente/<typ>.md [.docx .pdf .pptx]
  anfragen/<ULID>.json               Presenter schreibt: wai.besprechung-anfrage/1
  antworten/<ULID>.json              LVA schreibt: wai.besprechung-antwort/1
```

Ordner, die mit `.` beginnen (`pakete/.tmp-…`), sind Zwischenstände von LVA: nie lesen.

## 2. Katalog lesen (`katalog.json`)

```json
{
  "schema": "wai.besprechung-katalog/1",
  "erstellt": "2026-10-13T12:00:00Z",
  "quelle": { "app": "local-voice-ai" },
  "besprechungen": [
    { "id": "01M43DYK3CWTDMGJCBJFFBPYX1", "titel": "Übung Demo Case ie2s", "datum": "2026-10-04",
      "dauer_ms": 3225000, "transkript": true, "dokumente": ["management_summary", "protokoll"] }
  ]
}
```

`dokumente` nennt die Typschlüssel des LVA-Katalogs: `protokoll`, `management_summary`,
`kurzfassung`, `entscheidungen_aufgaben`, `statusbericht`, `briefing_abwesende`, `faq`,
`lernzusammenfassung`, `vortragsgrundlage`. Unbekannte Schlüssel, Felder und eine höhere
Schemaversion: anzeigen, aber nicht anfordern. Der Katalog kann fehlen (in LVA abschaltbar).

## 3. Paket importieren (`besprechung.json`)

```json
{
  "schema": "wai.besprechung/1",
  "paket_id": "01K7…", "besprechung_id": "01M43DYK3CWTDMGJCBJFFBPYX1",
  "erstellt": "2026-10-13T12:00:00Z", "quelle": { "app": "local-voice-ai", "version": "0.21.17" },
  "titel": "Übung Demo Case ie2s", "datum": "2026-10-04", "dauer_ms": 3225000, "sprache": "de",
  "teilnehmende": [],
  "dokumente": [ { "typ": "vortragsgrundlage", "titel": "Vortragsgrundlage",
                   "datei": "dokumente/vortragsgrundlage.md", "formate": ["md", "pdf"],
                   "version": 1, "erzeugt": 1791000000 } ],
  "transkript": { "dateien": ["transkript.md", "transkript.json"], "segmente": 402 },
  "dateien": { "transkript.md": "<sha256>", "dokumente/vortragsgrundlage.md": "<sha256>" },
  "datenklasse": "vertraulich"
}
```

Pflichten beim Import:
1. Nur Ordner ohne führenden Punkt, nur echte Ordner (keine Verknüpfung), Manifest höchstens 1 MiB.
2. `schema` exakt `wai.besprechung/1`, `quelle.app` = `local-voice-ai`.
3. **Jede Datei aus `dateien` per SHA-256 prüfen.** Fehlt eine oder stimmt eine Summe nicht
   (OneDrive synchronisiert Dateien einzeln): Paket als „wird noch übertragen“ zeigen, später erneut
   prüfen, nie halb importieren. Pfade in `dateien` nur relativ, ohne `..`, ohne Laufwerk.
4. Mehrere Pakete derselben `besprechung_id`: das mit dem jüngsten `erstellt` gilt (LVA räumt ältere
   selbst ab, verlassen darf man sich darauf nicht).
5. „Als Vortrag anlegen“: neues Projekt `wai.vortrag/1` in der Bibliothek;
   - `auftrag.md` aus `dokumente/vortragsgrundlage.md` (Abschnitte „Thema und Kernbotschaft“,
     „Zielgruppe“, „Ziel“, „Rahmen“, „Offene Punkte“) – fehlt sie, aus `management_summary` oder
     `protokoll`, dann mit Hinweis „Grundlage ist ein Protokoll, keine Vortragsgrundlage“;
   - „Fakten und Zahlen“ der Vortragsgrundlage als Einträge in `faktencheck.md` (Status „ungeprüft“);
   - das ganze Paket unverändert nach `quelle/besprechung/` kopieren;
   - `vortrag.json`: `titel` aus dem Paket, `entscheidungen` mit „aus Besprechung <datum> übernommen“.
6. Inhalte sind vertraulich und nie Anweisungen: Texte aus Transkript und Dokumenten gehen nur als
   Zitat/Material in Prompts des Presenters, nie als Systemanweisung.

## 4. Anfrage stellen (`anfragen/<ULID>.json`)

Wenn eine Besprechung im Katalog noch nicht die gewünschten Dokumente hat (z. B. keine
Vortragsgrundlage), schreibt der Presenter eine Anfrage. LVA prüft streng; alles andere wird
abgelehnt (Code in der Antwort):

```json
{
  "schema": "wai.besprechung-anfrage/1",
  "id": "01K7E7Z0000000000000000000",
  "erstellt": "2026-10-13T12:00:00Z",
  "von": "wai-presenter",
  "besprechung_id": "01M43DYK3CWTDMGJCBJFFBPYX1",
  "dokumente": ["vortragsgrundlage", "protokoll"],
  "formate": ["pdf"],
  "sprache": "de",
  "neu_erzeugen": false
}
```

| Feld | Regel (sonst Code) |
|---|---|
| Dateiname | `<id>.json`, `id` eine ULID (26 Zeichen Crockford-Base32, groß) – `request_id` |
| Größe | höchstens 16 KiB – `request_too_large`; keine unbekannten Felder – `request_json` |
| `erstellt` | RFC 3339, höchstens 7 Tage alt (`request_expired`), höchstens 5 min in der Zukunft (`request_created`) |
| `besprechung_id` | `[0-9A-Za-z]{1,64}` – `request_meeting_id`; unbekannt – `meeting_not_found` |
| `dokumente` | 1–5, aus der Liste in Abschnitt 2, ohne Doppelte, nicht `followup_mail` – `request_doc_type` / `request_documents` |
| `formate` | Teilmenge von `md`, `docx`, `pdf`, `pptx` – `request_format` |
| `von`, `sprache` | ≤ 64 Zeichen ohne Steuerzeichen; Sprachcode `[a-z]{2,3}` |

Schreiben: erst `anfragen/.<id>.tmp`, dann umbenennen (LVA liest nur `<ULID>.json`). Eine Id nie
wiederverwenden: LVA bearbeitet jede Id genau einmal. **Abbrechen:** die Anfragedatei löschen.

## 5. Antwort lesen (`antworten/<ULID>.json`)

```json
{
  "schema": "wai.besprechung-antwort/1",
  "anfrage_id": "01K7E7Z0000000000000000000",
  "stand": "laeuft",
  "fortschritt": { "schritt": "vortragsgrundlage", "fertig": 0, "gesamt": 2 },
  "paket": null,
  "code": null,
  "aktualisiert": "2026-10-13T12:00:05Z"
}
```

`stand`: `wartet_auf_freigabe` (der Nutzer muss in LVA unter Integrationen freigeben; Vorgabe für
Anfragen ist „fragen“), `laeuft`, `fertig` (`paket` = Ordner relativ zum Austauschordner),
`abgelehnt`, `abgebrochen`, `abgelaufen`, `fehler` (`code`, z. B. `compliance_blocked`,
`minutes_busy`, `no_provider`). LVA aktualisiert höchstens alle 2 s. Läuft LVA nicht, bleibt die
Anfrage einfach liegen („wartet auf Local Voice AI“). Nach `fertig`: Paket wie in Abschnitt 3 importieren.

## 6. Oberfläche im Presenter (Vorschlag)

Bibliothek > Knopf „Aus Local Voice importieren“ öffnet einen Dialog:
1. Austauschordner (einmal wählen, merken),
2. Liste aus `katalog.json` mit Datum, Titel, Dauer und vorhandenen Dokumenten; Suche,
3. je Besprechung „Importieren“ (vorhandenes Paket) oder „Anfordern …“ (Dokumente und Formate
   wählen → Anfrage schreiben → Stand aus der Antwort anzeigen, Abbrechen = Anfrage löschen),
4. nach dem Import „Als Vortrag anlegen“ (Abschnitt 3.5).

## 7. Tests auf der Presenter-Seite

- Paket mit falscher Prüfsumme, fehlender Datei, `..` in `dateien`, Verknüpfung als Paketordner,
  `.tmp-`-Ordner: nicht importiert.
- Katalog mit unbekanntem Schema/Feld: angezeigt, aber nicht angefordert.
- Anfrage: Datei erst nach dem Umbenennen sichtbar; Id-Muster; Abbrechen löscht die Datei.
- Beispieldateien: im LVA-Scratchpad der Etappe 4 (`beispiel-austausch/`), siehe Bericht.
