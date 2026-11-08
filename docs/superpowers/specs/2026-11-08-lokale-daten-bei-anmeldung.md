# Lokale Daten bei Anmeldung, Abmeldung und Kontowechsel

- Stand: 08.11.2026, Auftrag Patrick (Thread „Lokale Daten bei Anmeldung sicher übernehmen“)
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

## 2. Ziel

Nie geht ein Datum verloren, in keine Richtung. Nichts Lokales geht ohne ausdrückliche Wahl in ein Konto. Vor jeder
Löschung und vor jedem Überschreiben durch das Konto entsteht automatisch eine geprüfte Sicherung, die sich
wiederherstellen lässt. Gilt für Anmeldung, Abmeldung und Kontowechsel.

## 3. Entscheidungen

**U1 Freigabe vor dem ersten Lauf.** `sync_login` prüft Zugangsdaten und holt das Token wie bisher, speichert das
Konto aber mit `freigabe: false`. Solange die Freigabe fehlt, läuft kein Zyklus (kein Push, kein Pull). Bestehende
Konten ohne das Feld gelten als freigegeben (`serde(default = true)`), damit ein Update nichts anhält.

**U2 Bestand.** Nach dem Login ermittelt der Kern, welche lokalen Daten das Konto noch nicht kennt:
Seiten (Index), Bücher (`library/book_*`), dazu Größe und Zahl der Buch-Ordner-Dateien und Fortschritte. Bekannt sind
nur Objekte aus der **Herkunftsliste** (U6) desselben Kontos (gleiche `user_id` und Hub). Ist der Bestand leer, wird
ohne Rückfrage freigegeben (die Sicherung der Einstellungen aus U4 entsteht trotzdem). Sonst zeigt die App das Modal.

**U3 Modal „Lokale Daten gefunden“** (nicht per Esc oder Hintergrund schließbar, Entscheidung nötig):

| Wahl | Wirkung |
|---|---|
| **Übernehmen und zusammenführen** | Sicherung (U4), dann Freigabe. Lokales und Konto werden vereinigt; Kollisionen werden wie heute zur Konfliktkopie. |
| **Sichern und getrennt halten** | Sicherung, dann werden die gefundenen Seiten und Bücher als **nur auf diesem Gerät** markiert (U5) und nie hochgeladen. Kontodaten kommen dazu. |
| **Sichern und vom Gerät entfernen** | Sicherung, Prüfung, erst dann werden die gefundenen Seiten und Bücher lokal entfernt. Das Gerät zeigt danach nur die Kontodaten. Wiederherstellen jederzeit. |
| **Abbrechen** | Token am Hub widerrufen, Konto entfernen. Lokal ändert sich nichts. |

„Getrennt halten“ und „entfernen“ sind zwei Unterwahlen der Option „Sichern und …“ (Optionsfelder), damit das Modal
drei Hauptwege zeigt wie gewünscht. Beim **Kontowechsel** (Herkunft = anderes Konto) ist „getrennt halten“
vorausgewählt und der Text nennt beide Konten; „zusammenführen“ verlangt dort ein zusätzliches Häkchen
„Daten aus a@… in b@… kopieren“.

Besprechungen und Diktat-Verlauf erscheinen im Modal als „bleiben auf diesem Gerät, werden nicht synchronisiert“.
Keine Wahl berührt sie.

**U4 Sicherung.** Ordner `<daten>/sicherungen/konto-<ms>/` mit `manifest.json` (Pfad, Größe, SHA-256, Format wie
Buch-Ordner-Sicherung), Inhalt: `projects/` (ohne `_geloescht`), alle `library/book_*`, `library/fortschritt.json`,
`einstellungen.json` (synchronisierte Teilmenge), `info.json` (Anlass, Konto, Zahlen). Dateien ab 4 MiB als harter
Link, sonst Kopie (Rückfall auf Kopie, wenn der Link scheitert); Platzprüfung wie in `library_layout`. Erst nach
vollständiger Prüfung gegen das Manifest geht es weiter; scheitert sie, bricht die Wahl ab und nichts ändert sich.
Sicherungen werden nie automatisch gelöscht; die Oberfläche bietet Löschen ab 30 Tagen an.

**U5 Nur auf diesem Gerät.** `<daten>/sync-nur-lokal.json` mit Seiten- und Buchkennungen. Der Zyklus nimmt diese
Objekte aus dem Push: die Seite bzw. das Buch, dessen Dateien (`book_file` mit `book` = Kennung), Fortschritte und
Lesezeichen, deren Schlüssel die Kennung enthält. Kommt vom Hub ein Objekt mit einer markierten Kennung, wird es
nicht angewendet und mit Revision 0 verbucht (so kollidiert es bei einer späteren Übernahme sauber und wird zur
Konfliktkopie, statt die Kontofassung zu überschreiben). In „Konto & Geräte“: „N Einträge nur auf diesem Gerät ·
Ins Konto übernehmen“ (mit Sicherung).

**U6 Herkunftsliste.** Beim Abmelden und bei 401 schreibt der Kern `<daten>/sync-herkunft.json` mit `user_id`,
Hub, E-Mail und den Ledger-Schlüsseln. So erkennt die nächste Anmeldung, was aus demselben Konto stammt (kein Modal
für unveränderte Kontodaten) und was aus einem anderen Konto stammt (Kontowechsel).

**U7 Abmelden.** Dialog mit „Abmelden, Daten auf diesem Gerät behalten“ (vorausgewählt), „Abmelden und
Bibliothek/Seiten vom Gerät entfernen (vorher gesichert)“ und „Abbrechen“. Entfernen nur nach geprüfter Sicherung.

**U8 Wiederherstellen.** Liste der Sicherungen in „Konto & Geräte“. Wiederherstellen kopiert fehlende Seiten- und
Buchordner zurück, ergänzt Index und fehlende Fortschrittsschlüssel und überschreibt nie: Ist ein Ordner schon da,
bleibt er und der Bericht nennt ihn. Einstellungen werden nur auf eigenen Wunsch zurückgespielt. Bei freigegebenem
Konto gelangt Wiederhergestelltes durch den nächsten Lauf ins Konto (bei getrennter Haltung bleibt es nur lokal).

**U9 Löschschutz bleibt.** Keine Wahl erzeugt Grabsteine: Lokales Entfernen geschieht, solange die Freigabe fehlt
bzw. für Objekte, die nicht im Ledger stehen. Beim Abmelden-mit-Entfernen ist das Konto schon entfernt.

## 4. iPhone (zweites Paket)

Gleiches Verhalten in `LibrarySync.login` und `adopt`: `freigabe` im Schlüsselbund-Konto, Bestand aus
`PhoneLibrary` (Bücher, Fortschritte), Sicherung unter `Documents/Sicherungen/konto-<ms>/`, gleiches Modal als
Sheet, Herkunftsliste und Nur-lokal-Liste neben dem Ledger.

## 5. Abnahme

- Login mit lokalen Daten: kein Hub-Aufruf außer Login, bis gewählt ist (Test mit Hub-Nachbau zählt Aufrufe).
- Jede Wahl: Sicherung vorhanden und geprüft; Entfernen nur danach; Abbrechen ändert keine Datei.
- Kontowechsel A → B mit „getrennt halten“: kein Objekt aus A erscheint im Hub von B.
- Gleiches Konto erneut: unveränderte Kontodaten lösen kein Modal aus.
- Wiederherstellen überschreibt nichts.
- Absturz zwischen Sicherung und Entfernen: Neustart zeigt das Modal erneut, Daten vollständig (lokal oder in der
  Sicherung).
