# Abo-Vorschau im Buch-Gespräch: Lizenz- und Weitergabe-Notiz

Stand: lva#334, Phase 1 (nur Claude Code). Diese Notiz beschreibt, **was die App
technisch tut** und welche Punkte noch gegen die Anthropic-Bedingungen geprüft
werden müssen. Sie ist **keine Rechtsauskunft** und stellt nicht fest, dass die
Nutzung zulässig ist. Was hier nicht belegt ist, steht als offener Punkt.

## Was die App tut (technischer Ist-Zustand)

- Die App startet nur die **vom Nutzer selbst installierte, unveränderte
  Claude-Code-CLI** (native Binärdatei), lokal auf dem Rechner des Nutzers, mit
  **dessen eigenem Login**. Die App meldet nichts an und legt keine Zugangsdaten an.
- Die App **liest, speichert und vermittelt keine Tokens** und keine Anmelde-
  dateien der CLI. Die Anmeldung bleibt vollständig bei der CLI. (Das
  Prüfskript `tooling/buch-gespraech/isolation-check.sh`/`.ps1` kopiert
  die Anmeldedatei des Nutzers nur in ein temporäres Verzeichnis auf demselben
  Rechner, wird vom Nutzer selbst gestartet und löscht das Verzeichnis danach.)
- Es wird **kein `--bare`** verwendet, damit der OAuth-Login der CLI gilt.
- Die **Kopie** unter `<App-Daten>/booktalk-cli/<sha256>/claude[.exe]` ist ein
  **lokaler, nicht weitergegebener Cache derselben Datei** (byte-gleich, per
  SHA-256 geprüft). Sie dient nur dazu, zwischen Prüfung und Start keinen
  Austausch der Datei zuzulassen. Sie verlässt den Rechner nicht, wird nicht
  verändert und nicht mit der App verteilt; bei einem Update der CLI entsteht eine
  neue Kopie, die alte wird gelöscht.
- Die App **liefert die CLI nicht mit** und enthält keinen Teil davon.
- Ein Aufruf entspricht einem einzelnen `claude -p`-Lauf auf Anfrage des Nutzers
  (Buch-Frage). Die Antwort wird nur angezeigt, wenn alle Prüfungen bestehen.
- Auf Wunsch (Relay) beantwortet derselbe Rechner Fragen des eigenen iPhones des
  Nutzers über den eigenen Hub. Auch dort läuft die CLI lokal mit dem Login des
  Nutzers; Text geht nur an das eigene Gerät.
- Codex ist in der Vorschau gesperrt (Phase 2, eigenes Issue). Für Codex gilt
  diese Notiz nicht.

## Offene Punkte zur Prüfung gegen die Anthropic-Bedingungen (Checkliste für Patrick)

Nichts davon ist hier bewertet; jeder Punkt braucht eine Prüfung des aktuellen
Wortlauts.

- [ ] **Consumer Terms** (Claude Pro/Max): Ist die Nutzung der eigenen,
      installierten Claude-Code-CLI durch eine lokale Drittanwendung (diese App
      startet `claude -p` im Auftrag des Nutzers) mit dem Abo-Login zulässig?
      Gibt es Einschränkungen für automatisierten oder programmgesteuerten
      Zugriff auf das Abo?
- [ ] **Commercial Terms**: Falls der Nutzer ein Team- oder Enterprise-Konto
      verwendet, gelten andere Bedingungen. Prüfen, ob die Nutzung dort anders
      zu beurteilen ist.
- [ ] **Nutzungsrichtlinie (Usage Policy)**: Gibt es Aussagen zur Nutzung von
      Claude-Code-Abos in Drittanwendungen oder zum Weiterreichen von
      Abo-Zugang an andere Nutzer oder Geräte? Das Relay an das **eigene**
      iPhone des Nutzers einordnen.
- [ ] **Claude-Code-Bedingungen und Dokumentation**: Gibt es eigene Bedingungen
      für die CLI (Lizenz der Binärdatei), insbesondere zum **Kopieren** der
      Datei in einen lokalen Cache und zum Starten der Kopie statt der
      Originaldatei?
- [ ] **Weitergabe**: Bestätigen, dass die App die CLI nicht mitliefert und
      nicht verteilt (siehe oben) und dass die Kopie nur lokal liegt.
- [ ] **Ratenbegrenzung und Fair Use**: Prüfen, ob Limits des Abos für
      automatisierte Aufrufe gelten und wie Fehler (Limit erreicht) in der App
      angezeigt werden sollen.
- [ ] **Datenschutz**: Anfragen gehen über die CLI an Anthropic (Abo). Prüfen,
      ob der Risikohinweis im Einschaltdialog dazu ausreicht.
- [ ] **Änderungen**: Die Bedingungen können sich ändern. Stichtag der Prüfung
      und Wiedervorlage festlegen.

## Hinweis zur technischen Grenze der Kopie

Die Kopie ist schreibgeschützt (Datei und Ordner Unix 0500, Windows: Datei
schreibgeschützt, beim Lauf mit Handle ohne Schreib-, Umbenenn- und Löschrecht).
Derselbe Nutzer kann auf Unix per `chmod` die Rechte ändern; das Handle
verhindert dort keinen Austausch. Die App prüft den Hash der Kopie vor dem Start
und nach dem Lauf und verwirft die Antwort bei Abweichung. Das ist eine
Schutzmaßnahme gegen versehentliche Änderungen und Auto-Updates, kein Schutz
gegen einen böswilligen Prozess desselben Nutzers.
