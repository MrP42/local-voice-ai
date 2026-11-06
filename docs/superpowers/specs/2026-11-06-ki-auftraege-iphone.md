# KI-Aufträge vom iPhone: Kurzbuch und Übersetzen (Entwurf)

Stand 06.11.2026, zweite Fassung nach Codex. Grundlagen: #347 (Kurzbücher lesen),
Relay aus `2026-11-05-buch-gespraech.md` §11.3 und §11.7, Sprachfassungen #344/#348.

## 1 · Ziel

Am iPhone „Kurzbuch erstellen“ und „Übersetzen“ antippen. Der PC führt den Auftrag mit den dort
eingerichteten Anbietern aus, das Ergebnis erscheint auf allen Geräten. Die App muss dafür nicht
offen bleiben. Gemma auf dem iPhone bleibt zurückgestellt und käme nur als bewusste Offline-Wahl.

## 2 · Ablauf

1. **Vorbereiten:** Buchseite → „Kurzbuch erstellen“ oder „Übersetzen“. Das iPhone fragt beim PC das
   Angebot ab (§5). Dafür muss der PC erreichbar sein.
2. **Bestätigen:** Blatt zeigt Empfänger, Textumfang, Kosten, Lesegrenze und Zielsprache.
   Erst „Auftrag starten“ legt den Auftrag an.
3. **Warten:** Buchseite zeigt den Stand. Ist der PC aus: „Wartet auf den PC“, abbrechbar.
4. **Ergebnis:** Der PC legt es wie bisher im Buch ab, der Abgleich bringt es aufs iPhone.

## 3 · Transport und Sicherheit

Hub-Sammlung des Relays, keine Hub-Änderung, eigene Objektarten mit eigenen Aufräumregeln. Die kurzen Fristen von
`req_*`/`resp_*` erfassen Aufträge nicht.

**Die Relay-Regeln R1–R13 (`2026-11-05-buch-gespraech.md` §11.7) gelten vollständig.** Abweichungen sind einzeln genannt,
bisher nur: API-Nutzung erweitert R6 und braucht eine eigene Kostenfreigabe (§5). Zusätzlich:

- Eigene Funktionsfreigaben je Gerät und Modell („Kurzbuch“, „Übersetzen“), getrennt vom Buch-Gespräch.
- `nur_lokal` gilt auf beiden Seiten: Ist es am iPhone oder am PC gesetzt, gibt es kein Cloud-Angebot.
- Ausführung isoliert, ohne Werkzeuge, ohne Inhalte in Protokollen.
- **Alle** Objektarten sind per HMAC authentifiziert, auch Abbrüche. Zustände tragen eine monotone Sequenz; ältere werden verworfen.
- Duplikatsperren und Abbruchmarken bleiben bis Ablauf plus Sicherheitsfrist bestehen.
- Der PC prüft die Freigaben vor jedem Modellaufruf erneut. Widerruf stoppt den Auftrag.

| Objekt | Schreibt | Inhalt |
|---|---|---|
| `jobquote_<request_id>` | iPhone | Anfrage eines Angebots |
| `joboffer_<request_id>` | PC | Angebot nach §5, mit Ablaufzeit |
| `job_<job_id>` | iPhone | bindet das Angebot nach §4 |
| `jobstate_<job_id>` | PC | Zustand, Sequenz, Schritt, Fortschritt, tatsächliches Modell, Verbrauch, Fehler |
| `jobcancel_<job_id>` | iPhone | Abbruchwunsch |

Der PC liest offene `job_*` beim Start und bei jedem Abgleich. Der Buchtext reist nicht im Auftrag.

## 4 · Angebot, Auftrag, Zustände

**Bindung:** Der Auftrag authentifiziert iPhone und PC, Funktion, Quellfassung mit Inhaltshash, Lesegrenze, Parameter,
Modell, Preisbasis, Budget und Ablauf. Jede Änderung braucht ein neues Angebot und eine neue Bestätigung. Ohne erreichbaren
PC und gültiges Angebot startet nichts. Kein automatischer Wechsel auf ein anderes Modell oder einen anderen Anbieter.

**Fristen:** Angebotsgültigkeit getrennt von der Annahmefrist: 7 Tage ab Bestätigung; danach `abgelaufen`.

**Zustände:** `wartend` (auch bei ausgeschaltetem PC) → `angenommen` → `läuft` → `Ergebnisabgleich ausstehend` → `fertig`.
Daneben `Bestätigung nötig`, `fehlgeschlagen`, `abgebrochen`, `abgelaufen`. `fertig` meldet der PC erst, wenn die Ablage am Hub bestätigt ist.

**Idempotenz:** `job_id` ist getrennt vom Ergebnis- und Cache-Schlüssel (Quellhash, Vorlage, Parameter, Pipeline-Version).
Übernahme und Ergebnisablage sind dauerhaft und atomar. Nach unklarem Ausgang beim Anbieter gibt es keine automatische
kostenpflichtige Wiederholung, sondern `Bestätigung nötig`.

**Wiederanlauf:** Persistente Checkpoints je Schritt. App- oder PC-Neustart setzt dort fort.

**Mehrere PCs:** Nur der im Angebot genannte PC nimmt an.

**Abbruch:** Der Wunsch wird sofort gespeichert und vor jedem Schritt geprüft; ein laufender Aufruf wird gestoppt. Die App zeigt die angefallenen
Kosten. Späte Ergebnisse werden nicht veröffentlicht.

## 5 · Transparenz, Datenfluss, Kosten

„Am PC“ heißt nicht „lokal“. Vor dem Start sichtbar und im Ergebnis gespeichert:

- Tatsächlicher Empfänger, Modell, Textumfang, Region soweit bekannt. Speicherung und Trainingsnutzung werden als unbekannt genannt, wenn sie es sind.
- Getrennt: Geld, Token, Abo-Kontingent. „Unbekannt“ ist kein Nullpreis.
- Budget: 125 % der Geldschätzung sind eine ausdrücklich bestätigte Obergrenze. Der PC prüft sie vor jedem Schritt einschließlich der maximalen
  Ausgabe; bei Überschreitung pausiert der Auftrag in `Bestätigung nötig`.
- Kein Start, solange das Angebot fehlt oder abgelaufen ist.

## 6 · Lesegrenze

`read_limit` ist ein eindeutiger Unicode-Offset, gebunden an die unveränderliche Quellfassung (Inhaltshash).

- Die Quellen werden **vor jeder Verarbeitung** begrenzt, Zwischenfassungen entstehen erst danach. Fertige Zusammenfassungen nachträglich abzuschneiden entfernt Spoiler nicht.
- Die Grenze gehört in den Cache-Schlüssel. Auch Kapitelüberschriften jenseits der Grenze fließen nicht ein.
- „Ganzes Buch“ zeigt den Spoiler-Hinweis.
- Übersetzungen sind in V1 immer vollständig, Teilübersetzung gibt es nicht.

## 7 · Ergebnis, Fassungen, Kennzeichnung

Ergebnisse sind unveränderlich versioniert.

**Kurzbuch:** Bestehende Fassungen werden nie still ersetzt. #347 meldet je Sprache das neueste; das wird eine Liste je Sprache mit `version` und
`read_limit`. Anzeige: Datum, Modell, Ort, Lesegrenze.

**Übersetzen:** Neue Sprachfassung desselben Buchs nach #344/#348 (`editions[]`, `files.editions`), atomar angelegt. Kennzeichnung: maschinell übersetzt, Modell, Datum,
Ausgangsfassung. Hörbuch-Fassungen sind nicht Teil dieses Auftrags. Abhängigkeit: PR 3 aus #344.

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

## 10 · Abnahmetests

Bereinigungsfristen, Replay, Widerruf während eines Laufs, doppelte Zustellung, Absturz zwischen Anbieterantwort und Speicherung, Budgetüberschreitung,
Abbruchrennen, Spoilerfälle im Cache und in Kapitelüberschriften.

## 11 · Entscheidungen von Codex (06.11.2026)

Relay-Sammlung reicht; 7 Tage ab Bestätigung; 125 % als bestätigte Obergrenze; kein automatischer Wechsel; Lesegrenze im Cache-Schlüssel und vor der Verarbeitung; keine Teilübersetzung in V1.
