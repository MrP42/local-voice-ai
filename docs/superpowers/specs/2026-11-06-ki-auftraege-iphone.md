# KI-Aufträge vom iPhone: Kurzbuch und Übersetzen (Entwurf)

Stand 06.11.2026. Entwurf zur Abnahme durch Codex. Grundlagen: #347 (Kurzbücher lesen),
Relay aus `2026-11-05-buch-gespraech.md` §11.3 und §11.7, Sprachfassungen #344/#348.

## 1 · Ziel

Am iPhone „Kurzbuch erstellen“ und „Übersetzen“ antippen. Der PC führt den Auftrag mit den dort
eingerichteten Anbietern aus, das Ergebnis erscheint auf allen Geräten. Die App muss dafür nicht
offen bleiben. Gemma auf dem iPhone bleibt zurückgestellt und käme nur als bewusste Offline-Wahl.

## 2 · Ablauf

1. **Vorbereiten:** Buchseite → „Kurzbuch erstellen“ oder „Übersetzen“. Das iPhone fragt beim PC das
   Angebot ab: Anbieter, Modell, Verarbeitung (lokal oder Cloud), geschätzte Token und Kosten.
2. **Bestätigen:** Blatt zeigt Anbieter, wohin der Text geht, Kosten, Umfang (Lesegrenze) und Zielsprache.
   Erst „Auftrag starten“ legt den Auftrag an.
3. **Warten:** Buchseite zeigt den Stand. Ist der PC aus: „Wartet auf den PC“, abbrechbar.
4. **Ergebnis:** Der PC legt es wie bisher im Buch ab, der Abgleich bringt es aufs iPhone.

## 3 · Transport

Dieselbe Hub-Sammlung und dieselben Regeln wie das Relay (R1–R3: Relay-Schlüssel je iPhone,
HMAC, Antwortbindung, Replay-Schutz), neue Objektarten. Keine Hub-Änderung.

| Objekt | Schreibt | Inhalt |
|---|---|---|
| `jobquote_<request_id>` / `jobofferte_<request_id>` | iPhone / PC | Angebot wie in Abschnitt 2 |
| `job_<job_id>` | iPhone | Art, Buch, Fassung, Lesegrenze, Zielsprache, gewähltes Angebot, `idempotency_key` |
| `jobstate_<job_id>` | PC | Zustand, Schritt, Fortschritt, tatsächliches Modell, Verbrauch, Fehler |
| `jobcancel_<job_id>` | iPhone | Abbruchwunsch |

Der PC liest beim Start und bei jedem Abgleich offene `job_*`. So überlebt ein Auftrag einen
ausgeschalteten PC, ohne dass das iPhone wach sein muss. Eine Frist (Vorschlag 7 Tage) verwirft alte Aufträge.

Der Buchtext reist nicht im Auftrag: Der PC hat ihn. Fehlt er dort, meldet der Auftrag „Buch nicht auf diesem PC“.

## 4 · Zustände

`wartend` → `angenommen` → `läuft` → `fertig` | `fehlgeschlagen` | `abgebrochen`.

- **Keine Doppelerzeugung:** `idempotency_key` = Hash aus Art, Buch, Fassung, Lesegrenze, Zielsprache, Anbieter,
  Modell. Der PC übernimmt einen Schlüssel nur einmal und setzt nach einem Neustart über den vorhandenen
  Zwischenspeicher fort (`book_ai/engine`, Kurzbuch-Cache).
- **Mehrere PCs:** Ein Auftrag nennt die `desktop_device_id` aus dem Angebot. Nur dieser PC nimmt ihn an.
- **Abbruch:** Vor `läuft` sofort; während `läuft` nach dem laufenden Schritt. Bereits angefallene Kosten stehen im Zustand.
- **Kostengrenze:** Weicht der Verbrauch um mehr als einen festen Anteil vom Angebot ab (Vorschlag 25 %), hält
  der PC an und fragt nach.

## 5 · Transparenz und Datenfluss

Vor dem Start sichtbar, im Ergebnis gespeichert:

- Anbieter und Modell, lokal oder Cloud. Bei Cloud: „Der Buchtext geht an <Anbieter>“.
- Abo oder API-Kosten mit Schätzung. Unbekannt heißt ausdrücklich „unbekannt“, nie 0.
- Das Blatt lässt keinen Start zu, solange das Angebot fehlt oder abgelaufen ist.

## 6 · Lesegrenze

Der Auftrag trägt `read_limit` (Zeichenstelle). Vorgaben: Stand im Buch oder ganzes Buch; „ganzes Buch“ zeigt den
Spoiler-Hinweis.

- Der PC schneidet **vor jedem KI-Schritt**, auch vor Zwischenfassungen aus dem Cache: Cache-Schlüssel enthält
  die Grenze, sonst könnte ein späteres Kapitel über eine Zwischenfassung einfließen. Prüfen, ob `DigestCache` heute kapitelweise reicht.
- Ein Kapitel, das die Grenze überschreitet, zählt bis zur Grenze.
- Für das Übersetzen gilt die Grenze nicht als Spoilerschutz. Vorgabe ist das ganze Buch; Teilübersetzung ist eine eigene Wahl.

## 7 · Ergebnis, Fassungen, Kennzeichnung

**Kurzbuch:** Bestehende Fassungen werden nie still ersetzt. `KurzbuchRef` bekommt `version` und `read_limit`; #347 meldet heute je Sprache nur das neueste,
das muss zu einer Liste je Sprache werden. Anzeige: Datum, Modell, Ort, Lesegrenze.

**Übersetzen:** Ergebnis ist eine neue Sprachfassung desselben Buchs nach #344/#348 (`editions[]`, `files.editions`),
kein neues Buch. Text je Fassung als Blob. Kennzeichnung: maschinell übersetzt, Modell, Datum, Ausgangsfassung.
Hörbuch-Fassungen sind nicht Teil dieses Auftrags. Abhängigkeit: PR 3 aus #344 (iPhone-Oberfläche).

## 8 · iPhone

- Buchseite: „Kurzbuch erstellen“, „Übersetzen“, Auftragsliste mit Stand. Offene Aufträge kommen über den Abgleich, keine dauerhaft offene App.
- Ohne eingerichtetes Relay: Hinweis mit Weg zur Einrichtung am PC.
- Hörprobe, Lesen und Vorlesen laufen über die vorhandene Kurzbuch- bzw. Fassungsansicht.

## 9 · Pakete

| Paket | Inhalt |
|---|---|
| A1 | Auftragsformat, HMAC, Golden-Fixtures (Rust und Swift) |
| A2 | PC: Auftragsliste, Angebot, Ausführung Kurzbuch mit Lesegrenze, Zustand, Abbruch |
| A3 | iPhone: Angebot, Bestätigung, Stand, Abbruch für Kurzbuch |
| A4 | Kurzbuch-Versionen und Kennzeichnung (Erweiterung #347) |
| B1 | PC: Übersetzen als Auftrag, Ergebnis als Fassung; nach #348 |
| B2 | iPhone: Übersetzen, Zielsprache, Fassung wählen; nach PR 3 von #344 |

## 10 · Offene Fragen an Codex

1. Reichen Relay-Objekte für langlebige Aufträge, oder braucht es eine eigene Sammlung am Hub? Grund: Aufräumfristen der `req_*`-Objekte.
2. Frist 7 Tage und Kostengrenze 25 % als Vorgabe?
3. Darf ein Auftrag bei gesperrtem PC-Modell auf einen anderen Anbieter ausweichen? Vorschlag: nein, neu bestätigen.
4. Lesegrenze im Cache-Schlüssel oder Zwischenfassungen je Kapitel mit Abschneiden am Ende?
5. Teilübersetzung (bis Lesegrenze) überhaupt anbieten?
