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

**Die Relay-Regeln R1–R13 (`2026-11-05-buch-gespraech.md` §11.7) gelten.** Ausdrückliche Ausnahmen für Aufträge:

| Regel | Buch-Gespräch | Aufträge |
|---|---|---|
| R6 Ausgabe | `max_tokens` ≤ 800 | Grenze je Schritt aus dem Angebot, im Budget enthalten |
| R6 Rate | 1 läuft, 30/Stunde, 200/Tag | 1 Auftrag je Gerät läuft, höchstens 5 offen, 20 je Tag |
| R6 Pfad | nur Abo | Abo; API nur mit Funktionsfreigabe und Kostenbestätigung (§5) |
| R8 Anzeige | satzweise Prüfung im iPhone-Kern | Gilt nicht: Kurzbuch und Übersetzung sind eigene Dokumente, keine Antworten mit Belegen. Stattdessen Kennzeichnung (§7) und Begrenzung der Quellen (§6). Texte erscheinen erst nach Ablage und Prüfung von Hash und Größe am iPhone |
| R11 Fristen | 90 s, Antwort binnen 75 s | Angebot 10 Min, Annahme 7 Tage ab Bestätigung; je Modellaufruf 10 Min, danach Prozess beenden |

Zusätzlich:

- Eigene Funktionsfreigaben je Gerät, Modell, Anbieter und Pfad (Abo oder API) für „Kurzbuch“ und „Übersetzen“, getrennt vom Buch-Gespräch.
  Modellfreigabe und Kostenbestätigung müssen zusammen die Übertragung des im Angebot genannten Buchumfangs an genau diesen Empfänger erlauben.
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

**Löschfristen** (Zeit nach dem genannten Ereignis; löscht der PC, das iPhone räumt eigene Reste beim Start ab):

| Objekt | Frist |
|---|---|
| `jobquote_*`, `joboffer_*` | 1 Tag nach Ablauf des Angebots |
| `job_*` | 30 Tage nach Endzustand |
| `jobstate_*` | 30 Tage nach Endzustand |
| `jobcancel_*` | mit dem Auftrag |
| Duplikatsperren, Abbruchmarken (lokal am PC) | Annahmefrist plus 7 Tage, mindestens 30 Tage nach Endzustand |

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

**Wiederanlauf:** Vor jedem Anbieteraufruf wird dauerhaft ein Checkpoint `aufruf_begonnen` geschrieben, danach `aufruf_beendet` samt Ergebnis und Verbrauch.
App- oder PC-Neustart setzt am letzten beendeten Schritt fort. Findet der Neustart `aufruf_begonnen` ohne Abschluss, ist der Ausgang unklar: Der Auftrag geht in
`Bestätigung nötig` und wiederholt den Aufruf nie ohne neue Bestätigung. Die Reserve aus §5 bleibt angerechnet; eine Wiederholung reserviert zusätzlich neu.

**Mehrere PCs:** Nur der im Angebot genannte PC nimmt an.

**Abbruch:** Der Wunsch wird sofort gespeichert und vor jedem Schritt geprüft; ein laufender Aufruf wird gestoppt. Die App zeigt die angefallenen
Kosten. Späte Ergebnisse werden nicht veröffentlicht.

## 5 · Transparenz, Datenfluss, Kosten

„Am PC“ heißt nicht „lokal“. Vor dem Start sichtbar und im Ergebnis gespeichert:

- Tatsächlicher Empfänger, Modell, Textumfang, Region soweit bekannt. Speicherung und Trainingsnutzung werden als unbekannt genannt, wenn sie es sind.
- Getrennt: Geld, Token, Abo-Kontingent. „Unbekannt“ ist kein Nullpreis.
- Ist die Geldschätzung bei einem kostenpflichtigen Pfad unbekannt, verlangt das Blatt ein numerisches Budget; ohne Budget ist der Start gesperrt.
- Ist die Preisbasis unbekannt, reicht ein Budget nicht: Ohne belastbare Kostenobergrenze für den nächsten Aufruf bleibt der kostenpflichtige Start gesperrt. Das gilt auch für jeden weiteren Aufruf eines laufenden Auftrags.
- Vor jedem Aufruf werden die maximal möglichen Kosten dauerhaft reserviert und gegen das Budget geprüft. Nach dem Aufruf ersetzt der tatsächliche Verbrauch die Reserve.
- Ist der Ausgang unklar, bleibt die Reserve angerechnet, auch nach Neustart und nach erneuter Bestätigung.
- Verbrauch und Reserven werden dauerhaft über alle Schritte und Neustarts aufsummiert.
- Budget: 125 % der Geldschätzung sind eine ausdrücklich bestätigte Obergrenze. Der PC prüft sie vor jedem Schritt einschließlich der maximalen
  Ausgabe; bei Überschreitung pausiert der Auftrag in `Bestätigung nötig`.
- Kein Start, solange das Angebot fehlt oder abgelaufen ist.

## 6 · Lesegrenze

`read_limit` ist ein Offset in UTF-16-Einheiten wie im Reader und im Fortschritt (`offset`), gebunden an die unveränderliche Quellfassung (Inhaltshash).

Ungültige Grenze: negativ, größer als die Textlänge, mitten in einem Surrogatpaar oder Quellhash passt nicht → der PC lehnt den Auftrag ab (`fehlgeschlagen`, Grund `ungültige_lesegrenze`), ohne einen Modellaufruf. Keine stille Korrektur.

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

## 10 · Abnahmetests (verbindlich)

| Fall | Erwartet |
|---|---|
| Bereinigung | Nach den Fristen aus §3 sind die Objekte weg, Sperren erst nach ihrer Frist; laufende Aufträge bleiben unberührt |
| Replay eines `job_*` | Keine zweite Ausführung, kein Modellaufruf, Zustand unverändert |
| Falscher HMAC oder fremdes Gerät, auch bei `jobcancel_*` | Verworfen, kein Zustandswechsel |
| Veralteter `jobstate_*` (kleinere Sequenz) | iPhone behält den neueren Stand |
| Widerruf während `läuft` | Vor dem nächsten Aufruf `abgebrochen`, kein weiterer Modellaufruf, spätes Ergebnis unveröffentlicht |
| Doppelte Zustellung desselben Auftrags | Eine Übernahme, ein Ergebnis |
| Absturz zwischen Anbieterantwort und Speicherung | Neustart findet `aufruf_begonnen`, Zustand `Bestätigung nötig`, kein automatischer Aufruf, Verbrauch gezählt |
| Budget überschritten, auch nur durch maximale Ausgabe des nächsten Schritts | Pause in `Bestätigung nötig` vor dem Aufruf |
| Kostenpflichtiger Pfad ohne Schätzung und ohne Budget | Start gesperrt |
| Unbekannte Preisbasis mit numerischem Budget | Kostenpflichtiger Start gesperrt, kein Modellaufruf |
| Preisbasis wird im laufenden Auftrag unbekannt | Vor dem nächsten Aufruf `Bestätigung nötig`, kein Aufruf |
| Reserve übersteigt das Restbudget | Pause vor dem Aufruf, keine Reserve gebucht |
| Normaler Aufruf | Reserve vorher dauerhaft gebucht, danach durch den tatsächlichen Verbrauch ersetzt |
| Unklarer Ausgang, dann Neustart | Reserve bleibt angerechnet und überlebt den Neustart |
| Wiederholung nach unklarem Ausgang und neuer Bestätigung | Alte Reserve bleibt, neue kommt dazu; passt die Summe nicht ins Budget, kein Aufruf |
| Abbruch gleichzeitig mit `fertig` | Genau ein Endzustand; bei Abbruch zuerst keine Veröffentlichung |
| Angebot abgelaufen, Parameter geändert, anderes Modell | Abgelehnt, neue Bestätigung nötig |
| PC aus | `wartend`, abbrechbar, nach 7 Tagen `abgelaufen`, nach PC-Start ohne Doppelung fortgesetzt |
| Ergebnis-Ablage scheitert | `Ergebnisabgleich ausstehend`, kein `fertig` am iPhone |
| Grenze im Kapitel, Text und Überschrift dahinter im Prompt | Kein Inhalt hinter der Grenze in Prompt, Zwischenfassung oder Cache |
| Gleicher Cache mit anderer Grenze | Eigener Eintrag, keine Wiederverwendung |
| Ungültige Grenze nach §6 | `fehlgeschlagen`, kein Modellaufruf |
| Neue Fassung beim Übersetzen scheitert mittendrin | Keine halbe Fassung sichtbar |

## 11 · Entscheidungen von Codex (06.11.2026)

Relay-Sammlung reicht; 7 Tage ab Bestätigung; 125 % als bestätigte Obergrenze; kein automatischer Wechsel; Lesegrenze im Cache-Schlüssel und vor der Verarbeitung; keine Teilübersetzung in V1.
