# Vorschau D7-Rest: Cloud-Ausnahme, Unterordnerbaum, „nur online“ (Epic #395)

Statische HTML-Vorschau, keine App. Aufruf: `vorschau.html?s=<ansicht>&t=hell|dunkel` (`&z=<id>` scrollt den
offenen Dialog zu einem Abschnitt). Grundlage: Spezifikation `docs/superpowers/specs/2026-11-08-dokumentenablage.md`
(§3 Bestätigungsdialoge, §5.1/§5.2 Cloud-Ausnahme, §8 D7), Stil und Tokens der abgenommenen Vorschau `../ablage-d1/`
(Fluent 2, Segoe UI, gleiche CSS-Variablen hell/dunkel), Bestand der App in `src/components/ablage/`.
42 Bilder, alle aus EINEM Lauf über dieselbe `vorschau.html` (`belege.json` trägt deren SHA-256).
Die Hülle (Leiste, Kopf, Fuß) ist wie in D1 nur angedeutet. Alle Daten, Ordner und Anbieter sind erfunden
(`example.org`, „Beispiel-Cloud EU“, „Muster-KI Europa“, „Demo-Anbieter USA“).

| Bild | Breiten | Zeigt |
|---|---|---|
| `01-cloud-aus` | 1440, 390 | Ordnereinstellungen „Verträge“ (Klasse Vertraulich), Abschnitt **Cloud-Ausnahme**: Schalter „KI darf diesen Ordner über einen EU-Cloud-Anbieter nutzen“, Erklärung, Schalter aus |
| `02-cloud-anbieter` | 1440, 390 | Schalter an: Anbieterliste nach Regelwerk „EU“ (zwei wählbar, einer gesperrt mit Grund), Verweis auf Regelwerk |
| `03-cloud-bestaetigen` | 1440, 390 | Bestätigungsschritt nach „Speichern“: welche Dateien, welcher Anbieter, was es bedeutet; Fokus auf „Zurück“ |
| `04-cloud-chip` | 1440, 390 | Danach: Chip „Cloud-Ausnahme EU“ im Ordnerkopf (Knopf, öffnet die Einstellungen), Wolke-Haken im Baum, „Cloud: Ausnahme EU, geerbt“ in den Details einer Datei |
| `05-cloud-klassenwechsel` | 1440, 390 | Klasse auf „Intern“ gestellt: Schalter entfällt, Hinweis, dass die Ausnahme mit dem Speichern verfällt |
| `06-verknuepft-eigenschaften` | 1440, 390 | Verknüpfter Ordner: **kein** KI-Schalter, **keine** Cloud-Ausnahme; Erklärung warum, Datenklasse bleibt |
| `07-baum` | 1440, 390 | Verknüpfter Ordner mit aufklappbarem Unterordnerbaum (Dreieck, „nur lesen“, Symbol „verknüpft“); 390 als Ordnerblatt |
| `08-baum-nicht-erreichbar` | 1440, 390 | Wurzel „nicht erreichbar“: Warnzeichen, gedämpfter Name, zuletzt bekannte Unterordner; Seite mit Meldung, „Erneut prüfen“, „Verknüpfung lösen …“ |
| `09-nur-online-liste` | 1440, 390 | Dateiliste eines verknüpften Ordners mit Chip „Nur online“ (1440 Tabelle, 390 Karten) und Hinweiszeile |
| `09b-nur-online-tooltip` | 1440 | Tooltip am Chip (Hover/Fokus, `role=tooltip`, `aria-describedby`) |
| `10-nur-online-hinweis` | 1440, 390 | Details/Blatt der Datei: Hinweis „Auf diesem Gerät behalten, um zu indizieren“ (Anleitung, kein Download-Knopf) |

Bedienung (echt, nicht nur Bild): Schalter, Anbieterliste (Kontrollkästchen, Gesperrter ignoriert Klick), Speichern →
Bestätigung → „Ausnahme erteilen“ → Chip erscheint; Chip öffnet den Dialog; Klassenwechsel entfernt Schalter und Chip beim
Speichern; Baumdreiecke klappen auf/zu; Chip „Nur online“ öffnet die Details; Dialoge halten den Fokus, Escape schließt und gibt
den Fokus an den Auslöser zurück.

## Entscheidungen

**1 Cloud-Ausnahme**
- Ort: im Ordner-Dialog **unter der Datenklasse**, weil sie davon abhängt. Nur bei „Vertraulich“ ein Schalter; „Intern“/„Öffentlich“:
  Hinweis „braucht keine Ausnahme“; „Personenbezogen“: „nie Cloud“. Kein Schalter außerhalb von „Vertraulich“.
- Die Spezifikation führt Ausnahmen **je (Ordner, Anbieter)** (`freigabe.json`, `ablage_cloud_ausnahme_setzen(ordner, anbieter, an)`).
  Deshalb: ein Schalter, darunter eine Anbieterliste. Wählbar sind nur Anbieter, die das aktive Regelwerk „EU“ erfüllen
  (Verarbeitung in der EU, AVV, kein Training); andere stehen sichtbar gesperrt mit Grund (Texte aus `compliance.reasons`).
- Bestätigung als zweiter Schritt im selben Dialog nach „Speichern“ (gleiches Muster wie „Nur auf diesem Gerät“): nennt Dateizahl,
  Anbieter, „nur Auszüge“, „nur dieses Gerät“, „verfällt bei Klassenwechsel“, „Gesendetes nicht rückholbar“ und den
  DSFA-Hinweis aus Spezifikation §10. Fokus auf der sicheren Wahl („Zurück“). **Entziehen** (Schalter aus) braucht keine Bestätigung.
- Anzeige: Chip „Cloud-Ausnahme EU“ im Ordnerkopf als Knopf (44 px hoch, öffnet die Einstellungen, `aria-label` nennt den Anbieter);
  zusätzlich Wolke-Haken im Baum und eine Zeile in den Details geerbter Dateien.
- Klassenwechsel: weg von „Vertraulich“ → Schalter entfällt, Hinweis in Warnfarbe; die Ausnahme verfällt mit dem Speichern und
  lebt bei Rückwechsel **nicht** wieder auf (entspricht `ausnahmen_entziehen` und `ausnahme_gilt`).
- Verknüpfte Ordner: kein KI-Schalter, keine Ausnahme. Der Dialog sagt in drei Sätzen warum (nur lesen, gerätelokal, die KI greift nie
  zu, weder lokal noch Cloud) und nennt den Weg: Kopien in einen eigenen Ablage-Ordner legen und diesen freigeben.
  Wiederverwendet wird der bestehende Satz „Nicht verfügbar (verknüpfter Ordner)“.
- Verworfen: ein einzelner „alle EU-Anbieter“-Schalter ohne Liste (widerspricht „je Anbieter“); Pflicht-Kontrollkästchen
  „Ich habe verstanden“ im Bestätigungsschritt (Variante, falls die Reibung gewünscht ist).

**2 Unterordnerbaum**
- Nur verknüpfte Ordner sind aufklappbar (Dreieck als eigener 44×44-Knopf, `aria-expanded`, `aria-controls`); eigene Ordner bleiben
  wie in D1 immer offen. Einrückung 14 px je Ebene, Namen brechen auf bis zu zwei Zeilen um statt abzuschneiden.
- „Nur lesen erkennbar“: Gruppenüberschrift „Verknüpfte Ordner (nur lesen)“, Symbol Ordner mit Pfeil (verknüpft) an der Wurzel, Auge
  „Nur lesen“ rechts (bewusst kein Schloss: das Schloss steht für KI „Gesperrt“), im Kopf der Seite der Chip „Nur lesen“.
- Zahl der Dateien nur an eigenen Ordnern (Platz in der 240-px-Spalte); Unterordner zeigen keine.
- „Nur online“-Unterordner: Wolke, nicht aufklappbar (die App betritt sie nicht, `nur_online` in `AblageVerknuepft`).
- „Nicht erreichbar“: Warnzeichen an der Wurzel, Name gedämpft, der zuletzt bekannte Unterbaum bleibt; Seite mit Meldung (bestehender
  Text), „Erneut prüfen“ (= „Neu einlesen“) und „Verknüpfung lösen …“; Liste unter „Zuletzt bekannt“.
- Unter 1024 px ist der Baum wie in der App ein Ordnerblatt (Kopfknopf „Ordner wählen“), 390 px zeigt es scrollbar.

**3 „Nur online“**
- Symbol: Wolke im Umriss (wie der Platzhalter im Explorer) **mit Text** „Nur online“, nie nur Symbol. In verknüpften Ordnern heißt die
  Spalte „Stand“ (Im Index / Nur online), nicht „Sync“; so kollidiert die Wolke nicht mit „Nur in der Cloud“ der eigenen Ablage.
- Der Chip ist ein Knopf (44 px) und öffnet die Details; Tooltip bei Hover und Tastaturfokus, `aria-describedby`; Dateisymbol
  gedämpft. Auf dem Telefon gibt es kein Hover, dort trägt das Blatt die Erklärung.
- Hinweis „Auf diesem Gerät behalten, um zu indizieren“ als Textblock mit zwei Schritten (Explorer-Kontextmenü „Immer auf diesem Gerät
  behalten“). **Kein Download-Knopf**: die App lädt selbst nie herunter. Einzige Aktion: „Neu einlesen“.
- 390 px: Karten (Name, darunter Typ · Größe · Datum, rechts der Stand-Chip). Annahme: Karten ≤ 620 px für alle Dateilisten; D1 und
  die App zeigen dort bisher eine Tabelle mit zwei Spalten. Soll das bleiben, gilt der Kartenaufbau nur für verknüpfte Ordner.

## Offene Fragen

1. **Anbieterliste**: Quelle sind die konfigurierten Verbindungen? Fehlt ein EU-tauglicher Anbieter oder gilt „Nur lokal“, sollte der
   Schalter gesperrt sein und das Regelwerk nennen (hier nicht als Bild gezeigt).
2. Mehrere Anbieter gleichzeitig (so gezeigt) oder genau einer je Ordner?
3. Gilt die Ausnahme auch auf dem Projektweg? Der Bestätigungstext sagt „Voraussetzung bleibt die KI-Freigabe des Ordners“; für
   Projekte wäre es die Mitgliedschaft (Spezifikation §5.1).
4. KI-Freigabe „Gesperrt“ und Cloud-Schalter: ausblenden oder gesperrt mit Hinweis? (Hier unverändert sichtbar, „Gesperrt“ gewinnt.)
5. Wortlaut des Betriebssystems: Windows/OneDrive „Immer auf diesem Gerät behalten“; auf dem Mac heißt der Eintrag anders. Der Auftrag
   nennt „Auf diesem Gerät behalten“ (als Überschrift verwendet). Platzhalter oder je Plattform?
6. Liest der Dateibeobachter eine Datei automatisch ein, sobald OneDrive sie herunterlädt? Der Text sagt „liest sie ein, sonst
   hilft Neu einlesen“; das muss zur Umsetzung passen. Ein Knopf „Im Explorer zeigen“ ist bewusst nicht vorgesehen.
7. Anrede: die Vorschau nutzt „Sie“ wie die Ablage-Texte; die Compliance-Texte der App sagen „deinen“.

## Maße und Belege (`belege.json`)

Je Bild (Playwright, Chromium, Skalierung 1): `scrollX` = 0 und `ueberlauf` = 0 (kein waagerechtes Rollen, keine überlaufende Spalte);
`unter44` leer (jede sichtbare Bedienfläche mindestens 44 × 44 px: Links, Knöpfe, Eingabe, Auswahl, Schalter, Kontrollkästchen,
Baum-Dreiecke, Chips); `verschachtelteScroller` = 0 (keine Scrollleiste in einer Scrollleiste; hinter einem Dialog liegende
Listen und der Dialog sind Geschwister); `h1` = 1; `wai` und `ersatzschreibung` falsch (sichtbare Texte, `aria-label`, `title`).
Bei Dialog/Blatt: Fokus beim Öffnen im Dialog, 40 × Tab bleibt im Dialog, Escape gibt den Fokus an den Auslöser zurück.
Abläufe (`abläufe`): Cloud-Ausnahme bei 1440 und 390 (Schalter, Liste, gesperrter Anbieter, Fehler ohne Auswahl, Bestätigung,
Zurück, Erteilen, Chip 44 px, Chip öffnet Dialog, Klassenwechsel entfernt Chip) und Baum/Chip/Tooltip.
Kontrast (berechnet aus den Tokens, ≥ 4,5:1): hell Chip „Nur online“ 5,4, Cloud-Chip 4,8, Warn-Chip 5,9, „Im Index“ 6,3; dunkel 6,9 / 5,1 / 6,6 / 7,2.
Das gedämpfte Dateisymbol (Deckkraft 55 %) ist dekorativ (`aria-hidden`), der Name trägt die Information.

Nicht gemessen: die Abnahme mit echter App-Schrift (Segoe UI fehlt auf dem Linux-Rechner der Aufnahme; die Bilder zeigen die
Ersatzschrift, wie die D1-Bilder).
