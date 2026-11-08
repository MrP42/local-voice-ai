# Lokale Daten bei Anmeldung, Abmeldung und Kontowechsel

- Stand: 08.11.2026, Fassung 3 (nach Codex-Gegenprüfungen ceeab27 und 4157acba), Auftrag Patrick
  (Thread „Lokale Daten bei Anmeldung sicher übernehmen“), Issue lva#400
- Gilt für: Local Voice AI Windows/macOS (Rust-Kern + Oberfläche), danach iPhone (`LibrarySync`)
- Bezug: Multi-Device-Spec 2026-09-14, Bibliothek-Sync 2026-11-02, Buch-Ordner 2026-11-07 (Löschschutz lva#377:
  „gelöscht wird nur ausdrücklich in der App“), wai-sdk ADR-0028 E3.6/E11 (lokale Daten erst nach erklärter
  Sync-Freigabe hochladen, Sicherung vorher, Rückweg).

## 1. Ist-Stand (geprüft im Code, main cb84a597)

| Fall | Heute |
|---|---|
| Anmeldung mit lokalen Daten | `sync_login` speichert das Konto, leert das Ledger und startet sofort einen Lauf. Alle Seiten, Bücher mit Dateien, Fortschritte, Lesezeichen, Buch-Ordner-Dateien und die 21 synchronisierten Einstellungen gehen mit Basisrevision 0 zum Hub. Keine Rückfrage. |
| Gleiche Seite lokal und im Konto | `superseded` → `apply_page` legt die lokale Fassung als „(Konflikt von …)“ an, die Kontofassung gewinnt. Kein Verlust. |
| Einstellungen | Kontowerte überschreiben die lokalen ohne Sicherung. |
| Besprechungen, Diktat-Verlauf | Nicht im Sync. Bleiben lokal. |
| Abmelden | Konto und Ledger weg, Daten bleiben lokal. |
| Token widerrufen (401) | Konto weg, Ledger bleibt, Daten bleiben. |
| Kontowechsel A → B | Alles Lokale, auch das aus Konto A geholte, geht in Konto B. **Vermischung.** |
| iPhone | `login`/`adopt`: Ledger leer, erst holen, dann alle lokalen Bücher, Fortschritte, Stimmwahl senden. Keine Rückfrage. |

## 2. Ziel und Geltungsbereich

Nie geht ein Datum verloren, in keine Richtung. Nichts Lokales geht ohne ausdrückliche Wahl in ein Konto. Vor jeder
Löschung und vor jedem Überschreiben durch das Konto entsteht automatisch eine geprüfte Sicherung, die sich
wiederherstellen lässt. Gilt für Anmeldung, Abmeldung und Kontowechsel.

**Betroffene Daten** (= alles, was der Sync trägt): Seiten (`page`), Bücher (`book`) mit Dateiablage, Dateien im
Buch-Ordner (`book_file`), Fortschritte (`progress`), Lesezeichen und Stimmwahl (`bookmark`), synchronisierte
Einstellungen (`settings`). **Nicht betroffen** und von keiner Wahl berührt: Besprechungen, Diktat-Verlauf, Stimmen,
Modelle, Integrationen, alle übrigen Einstellungen. Das Modal und die Dialoge sagen das ausdrücklich (f).

## 3. Zustände und Dateien

| Datei (`<daten>` = Datenordner) | Inhalt | Zweck |
|---|---|---|
| `sync.json` | Konto (wie heute) | Gibt es nur bei **freigegebenem** Konto. Ohne sie läuft nichts: kein Zyklus, kein Buch-Gespräch-Relay, keine Kopplung (alle lesen `account::load`). |
| `sync-anmeldung.json` (privat, wie `sync.json`) | Token, Schlüssel, Gerät, Hub, `user_id`, E-Mail, Bestand, Kollisionen | Anmeldung wartet auf Entscheidung. |
| `sync-uebergang.json` | Journal: `id`, Hub, `user_id`, Wahl, Phase, Sicherung, Ziel-Kennungen | Wiederanlauf (c). |
| `sync-ledger.json` | wie heute, **neu mit `konto` = `<user_id>@<hub>`** | Ein Ledger eines anderen Kontos gilt als leer (a). |
| `sync-herkunft.json` | Hub, `user_id`, E-Mail, Ledger-Einträge (Schlüssel → Hash, Revision) | Was das zuletzt verbundene Konto bestätigt hatte (b). |
| `sync-nur-lokal.json` | exakte Sync-Schlüssel + Einheiten (Seiten, Bücher) | Nie hochladen (U5). |
| `sync-widerruf.json` (privat) | Hub, Token | Ausstehender Token-Widerruf (g). |
| `sicherungen/konto-<ms>/` | Kopien + `manifest.json` + `info.json` + `einstellungen.json` | Sicherung (U4). |

## 4. Entscheidungen

**U1 Freigabe vor dem ersten Lauf.** `sync_login` prüft Zugangsdaten, leitet den Schlüssel ab und holt das Token wie
bisher, schreibt aber **nicht** `sync.json`, sondern `sync-anmeldung.json`. Solange diese Datei besteht, existiert
für die App kein Konto: nichts wird hochgeladen oder angewendet. Erst die Entscheidung schreibt `sync.json`. Alles
geschieht unter der Kontosperre des Sync-Laufs (`account_guard`), ein laufender Zyklus endet vorher (a).

**U2 Bestand und Kollisionen (b).** Mit dem neuen Schlüssel sammelt der Kern alle lokalen Sync-Objekte mit denselben
Funktionen wie der Zyklus (`collect`, `collect_library`, `book_files::collect`) und ihren Hashes. Ein Objekt gilt als
**bereits freigegeben** nur, wenn die Herkunftsliste vom **selben Konto** (gleiche `user_id` und Hub) den Schlüssel
mit **demselben Hash** enthält. Alles andere ist Bestand: neue und nach der Abmeldung geänderte Objekte, neue
Fortschritte und Lesezeichen, abweichende Einstellungen, Objekte, deren Prüfsumme im Budget nicht fertig wurde.
Dazu holt der Kern einmal lesend die Kennungen des Kontos (Pull ab Cursor 0, ohne Anwenden): **Kollision** = eine
Seite oder ein Buch des Bestands, das es im Konto mit gleicher Kennung gibt, und dessen Inhalt abweicht (Seiten:
entschlüsselt verglichen; Bücher: immer als abweichend gewertet). Ist der Bestand leer, wird ohne Modal freigegeben.

**U3 Modal „Lokale Daten gefunden“** (Esc und Hintergrund schließen nicht). Zeigt je Datenart die Zahl, die
Sicherungsgröße, die Kollisionen, und den Satz „Besprechungen, Diktate, Stimmen und Modelle bleiben auf diesem Gerät
und werden von keiner Wahl berührt.“

| Wahl | Wirkung |
|---|---|
| **Übernehmen und zusammenführen** | Sicherung, dann Freigabe. Lokales und Konto werden vereinigt nach den bestehenden Regeln (Seiten: Konfliktkopie; Bücher und Fortschritte: neuerer Stand; Einstellungen: Konto gewinnt). Alles Verdrängte liegt in der Sicherung. |
| **Sichern und … getrennt halten** – „bleibt auf diesem Gerät, wird nicht hochgeladen“ | Sicherung, dann werden alle Bestandsobjekte nur lokal (U5). Kollisionen siehe U5. Einstellungen bleiben lokal (nicht hochgeladen, Kontowerte nicht angewendet). Kontodaten kommen dazu. |
| **Sichern und … vom Gerät entfernen** | Sicherung, Prüfung, dann werden die Bestandsobjekte lokal entfernt (Seiten, Bücher mit Buch-Ordner, Fortschritte, Lesezeichen); Einstellungen nehmen die Kontowerte an. Die Bibliothek zeigt danach die Kontodaten; Besprechungen usw. bleiben. |
| **Abbrechen** | Keine Sicherung, Nutzdaten unverändert; `sync-anmeldung.json` wird gelöscht, Token widerrufen (U10). |

Beim **Kontowechsel** (Herkunft = anderes Konto) ist der ganze Weg „Sichern und … → getrennt halten“ vorausgewählt,
der Text nennt beide Konten, und „Übernehmen und zusammenführen“ braucht zusätzlich das Häkchen „Daten aus a@… in b@…
kopieren“.

**U4 Sicherung (c, harte Links).** Nur **Kopien**, keine harten Links. Umfang: `projects/` ohne `_geloescht`
(Seitenordner mit allen Dateien, `index.json`), alle `library/book_*` vollständig (inkl. Fassungsseiten, Buch-Ordner,
Audio), `library/fortschritt.json`, `einstellungen.json` (synchronisierte Teilmenge), `info.json` (Anlass, Konto,
Zahlen), dazu `sync-nur-lokal.json` und `sync-herkunft.json`, falls vorhanden. Ablauf: Platzprüfung (Kopien + 10 % +
1 MiB), jede Datei mit fsync, Manifest (Pfad, Größe, SHA-256) atomar, dann **vollständige Prüfung gegen das
Manifest**, dann fsync des Ordners. Während Bestandsaufnahme, Sicherung und Entfernen hält der Kern die Seiten- und
die Bibliothekssperre; Oberflächen-Schreibzugriffe warten. Sicherungen werden nie automatisch gelöscht.

**U5 Nur auf diesem Gerät (d).** `sync-nur-lokal.json` hält (1) **Einheiten** (Seiten- und Buchkennungen); alle
abhängigen Objekte, auch künftige, sind über ihre Einheit markiert, und (2) **exakte Schlüssel** nur für Objekte ohne
Einheit (`settings/app`, globale Stimmwahl `tts.voice.lang.*`). Abhängigkeit wird strukturiert ausgewertet:
`book_file.book`, Fortschritt `progress.text:<seite>:…`, `progress.audio:<buch>`, `progress.doc:<buch>:…`, Lesezeichen
`tts.resume.<seite>[:…]`, `tts.bookmarks.<seite>[:…]`, Stimme `tts.voice.book.<buch>.<sprache>` (exakter Vergleich der
Kennung, kein Teilwort). Der Zyklus nimmt markierte Objekte aus dem Push **und** zählt sie als vorhanden (nie
Grabstein). Eingehende Objekte mit markiertem Schlüssel werden weder angewendet noch verbucht.

**Markieren bereinigt das Ledger:** Wer einen Schlüssel oder eine Einheit markiert, entfernt im selben Schritt die
Ledger-Einträge dieser Objekte (bei der Anmeldung ist das Ledger ohnehin neu). Damit hat ein markiertes Objekt nie
eine Basisrevision.

**Kollision = getrennte Kennungen.** Eine **lose Seite** wird als ganzer Ordner unter neuer Kennung kopiert (Titel
„… (nur auf diesem Gerät)“), ihre Fortschritte und Lesezeichen werden unter der neuen Kennung angelegt, die **neue**
Kennung wird markiert, die alte Kennung samt Abhängigkeiten wird **freigegeben** und lokal entfernt (liegt in der
Sicherung und in der Kopie). So kommt die Kontofassung unter der alten Kennung an. Ein **Buch** (und eine Seite im
Buch-Ordner) lässt sich nicht gefahrlos umbenennen (Pfade und Verweise in `buch.json`): seine lokale Fassung bleibt
**nur in der Sicherung**, die Einheit wird freigegeben und lokal entfernt, auf dem Gerät erscheint die Kontofassung;
das Modal nennt diese Einträge einzeln. Taucht eine markierte Kennung erst später im Konto auf (anderes Gerät), hält
der Zyklus an und startet einen Übergang der Art `kollision` mit genau diesem Plan (U11); erst danach läuft er weiter.

**„Ins Konto übernehmen“** ist ein Übergang der Art `uebernahme`: Sicherung, dann Markierung entfernen und
ausdrücklich alle Ledger-Einträge dieser Objekte löschen, sodass der Push mit **Basis 0** geht. Der Hub antwortet bei
vorhandenem Objekt `superseded` mit dem aktuellen Stand, es gelten die Regeln von „zusammenführen“ (Seiten:
Konfliktkopie). Verdrängtes liegt in der Sicherung.

**U6 Herkunftsliste und Kontobindung (a, b).** Abmelden, 401 und jeder Kontowechsel schreiben zuerst
`sync-herkunft.json` aus Konto und Ledger (Schlüssel → Hash, Revision), erst danach werden Konto und Ledger entfernt.
Das Ledger trägt sein Konto; der Zyklus verwirft ein Ledger eines anderen Kontos.

**U7 Abmelden.** Dialog: „Abmelden, Daten auf diesem Gerät behalten“ (vorausgewählt), „Abmelden und Bibliothek und
Seiten vom Gerät entfernen (vorher gesichert)“, „Abbrechen“. Entfernen ist ein Übergang der Art `abmeldung` (U11):
Journal `entscheidung` mit Plan, Herkunft schreiben, Konto und Ledger entfernen (ab hier kein Zyklus, also keine
Grabsteine), Sicherung, `gesichert`, Umsetzung, `umgesetzt`, Journal löschen. Besprechungen usw. bleiben (Text sagt es).

**U8 Wiederherstellen (e).** Liste der Sicherungen in „Konto & Geräte“. Je Einheit (Seitenordner, Buchordner):
fehlt sie → zurückkopieren; gleich → überspringen; **abweicht** → lose Seite als **zusätzlicher Eintrag** unter neuer
Kennung („(aus Sicherung vom …)“), Buch bzw. Buchseite bleibt in der Sicherung und der Bericht nennt sie mit „Ordner
öffnen“. Fortschritte: fehlende Schlüssel ergänzen; abweichende Werte bleiben aktuell, die Sicherungswerte bleiben in
der Sicherung (Bericht nennt die Zahl). Alles Wiederhergestellte wird **nur lokal** markiert, auch bei
freigegebenem Konto; hochladen nur über „Ins Konto übernehmen“ für das angemeldete Konto. Einstellungen nur auf
eigenen Wunsch, vorher Sicherung der aktuellen.

**U9 Löschschutz (f).** Kein Entfernen erzeugt Grabsteine: bei der Anmeldung ist das Ledger des neuen Kontos leer und
`sync.json` noch nicht geschrieben; beim Abmelden ist das Konto schon entfernt. Nach Neustart gilt dasselbe, weil das
neue Ledger die entfernten Kennungen nie kennt. Nur-lokale Objekte zählen im Zyklus als vorhanden.

**U10 Abbrechen (g).** Kein Nutzdatum wird verändert, keine Sicherung angelegt. Bereinigt wird nur der
Anmeldezustand: `sync-anmeldung.json` gelöscht, Token am Hub widerrufen. Scheitert der Widerruf, steht er in
`sync-widerruf.json` und wird bei jedem Start und jeder Anmeldung wiederholt; „Konto & Geräte“ zeigt den Hinweis.

**U11 Übergänge und Wiederanlauf (c).** Alle verändernden Abläufe laufen über ein Journal `sync-uebergang.json`
mit `art` (`anmeldung`, `abmeldung`, `kollision`, `uebernahme`), Konto, Wahl, Plan und Phase. Solange ein Journal
besteht, läuft kein Zyklus. Phasen bezeichnen **abgeschlossene Wirkung**, nicht Absicht:

| Phase | Bedeutet | Geschrieben |
|---|---|---|
| `entscheidung` | Plan steht fest, noch nichts verändert | bevor irgendetwas geschieht |
| `gesichert(<name>)` | Sicherung vollständig geschrieben, geprüft, fsync | **nach** erfolgreicher Prüfung |
| `umgesetzt` | Kopien, Entfernen, Fortschritte, Nur-lokal-Liste vollständig und mit fsync geschrieben | **nach** erfolgreicher Umsetzung |

Die Umsetzung ist idempotent (Kopien unter der Kennung aus dem Plan; entfernt wird nur, was noch da ist und
byte-gleich in der Sicherung liegt; Fortschritte nur, wenn der Wert dem gesicherten gleicht; Nur-lokal-Liste wird
vereinigt). Wiederanlauf beim Start und vor jedem Sync-Befehl:

| Phase beim Start | Weiter |
|---|---|
| `entscheidung`, Art `anmeldung` | Modal erneut; nichts wurde verändert. |
| `entscheidung`, andere Art | Sicherung neu anlegen und Ablauf fortsetzen (Abmeldung: Konto ist ggf. schon entfernt, das schadet nicht). |
| `gesichert`, Sicherung prüfbar | Umsetzung wiederholen, bis sie vollständig gelingt; erst dann `umgesetzt`. |
| `gesichert`, Sicherung nicht prüfbar | Nichts entfernen; neue Sicherung, Phase neu `gesichert`. Scheitert auch das: Hinweis, kein Zyklus. |
| `umgesetzt` | Abschluss je Art: `anmeldung` schreibt `sync.json` und Ledger (mit Kontobindung) und löscht `sync-anmeldung.json`; `kollision` und `uebernahme` bereinigen das Ledger; dann Journal löschen. |

Ein Absturz in jeder Phase verliert nichts: Entfernt wird erst nach `gesichert`, freigegeben erst nach `umgesetzt`.

## 5. iPhone (zweites Paket)

Gleiches Verhalten in `LibrarySync.login` und `adopt`: wartendes Konto getrennt vom Schlüsselbund-Konto, Bestand aus
`PhoneLibrary`, Sicherung unter `Documents/Sicherungen/konto-<ms>/`, gleiches Modal als Sheet, Herkunft, Nur-lokal,
Journal neben dem Ledger.

## 6. Abnahme (Tests)

- Login mit lokalen Daten: außer Login und lesendem Pull kein Hub-Aufruf bis zur Wahl (Hub-Nachbau zählt Aufrufe).
- Gleiches Konto, nichts geändert: kein Modal. Nach der Abmeldung geänderte Seite oder neuer Fortschritt: Modal.
- Kontowechsel A → B mit „getrennt halten“: kein Objekt aus A im Hub von B; Kollision lose Seite → Kopie lokal +
  Kontofassung; Kollision Buch → Kontofassung + Sicherung.
- Jede Wahl außer Abbrechen: Sicherung vorhanden und geprüft; Entfernen nur danach.
- Abbrechen: Nutzdaten byte-gleich, kein `sync.json`, Widerruf ausstehend bei Netzfehler.
- Absturz in jeder Phase (Fehler-Einspritzung): Wiederanlauf erreicht denselben Endstand, nichts verloren.
- Nach Entfernen und Neustart: kein Grabstein im Hub.
- Wiederherstellen: überschreibt nie; abweichende lose Seite als zusätzlicher Eintrag; Ergebnis nur lokal.
- Nur-lokal: kein Push, kein Grabstein, eingehende markierte Schlüssel nicht angewendet; „Ins Konto übernehmen“
  führt zu Konfliktkopie statt Überschreiben.
