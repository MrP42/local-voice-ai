# Abrufschutz verknüpfter Ordner und Ordner-Auslöser (lva#507, PR #512)

Abnahmeprotokoll zur geänderten Zusage und zum akzeptierten Restfenster.

## Zusage (Wortlaut in der Oberfläche)

> Local Voice überspringt erkannte, nur online gespeicherte Dateien. Durch gleichzeitige Änderungen des
> Synchronisationsdienstes kann beim Lesen dennoch ein automatischer Download ausgelöst werden.

Die frühere Formulierung „die App lädt nie selbst herunter“ gilt nicht mehr. Sie war als harte Garantie
formuliert, die der Code nicht einlösen kann.

Sichtbar ist die Einschränkung:

* beim Verknüpfen eines Ordners (Hinweis am Knopf „Ordner verknüpfen“, Schlüssel `ablage.verknuepft.cloudGrenze`),
* in der Meldung jedes verknüpften Ordners (`ablage-verknuepft-hinweis`) und in der Meldung zu nur online
  liegenden Einträgen (`ablage-verknuepft-nur-online`),
* in den Hinweisen „Nur online“ und „Auf diesem Gerät behalten“ an der Datei,
* im Hinweis der Automationen zu Platzhaltern (`automations.status.cloudHint`) und in `AUTOMATIONEN.md`.

## Akzeptiertes Restfenster (Befund 1, Abnahme Codex: Merge nur mit ausdrücklich akzeptiertem Schutz nach bestem Bemühen)

Der Schutz ist mehrstufig: Urteil über Attribute und Tag am Handle VOR jedem Lesezugriff, erneutes Urteil
unmittelbar vor dem Hochstufen auf Lesezugriff, Urteil und Datei-Kennung am Ergebnis; macOS zusätzlich die
Abrufsperre des Fadens (`EDEADLK` statt Download). Offen bleibt:

* Lagert der Synchronisationsdienst die Datei zwischen dem letzten Urteil und dem Öffnen mit Lesezugriff
  aus, kann dieses Öffnen den Abruf auslösen. Das Fenster ist ein Systemaufruf lang, hat aber keine zeitliche
  Obergrenze (der Faden kann dort beliebig lange pausieren). Eine Nachprüfung macht einen ausgelösten Download
  nicht rückgängig.
* Das Fenster wird im Code nicht geschlossen. Das ginge nur mit einer Sperre des Providers oder des
  Betriebssystems, die es auf Windows für OneDrive-Platzhalter nicht gibt.
* Ein einzelnes blockierendes `read` (z. B. Netzlaufwerk) lässt sich nicht unterbrechen; Frist und Abbruch
  des Hashens greifen zwischen den 1-MiB-Blöcken. Der Abbruch beim Beenden der App hängt an
  `WorkflowHub::shutdown`.

Entscheidung: Das Restfenster ist akzeptiert, solange die Oberfläche die Einschränkung nennt (siehe oben).

## Betriebs- und Auditprotokoll

Log, Berichte und Fehlertexte nennen nur beobachtete Zustände („übersprungen, weil Platzhalter“, „liegt nur
online“) und behaupten keine Downloadfreiheit. Geprüfte Stellen: `trigger/folder.rs` (Log und Bericht zu
Platzhaltern), `import.rs` (`CLOUD_ONLY_MESSAGE`), `ablage/verknuepft.rs` (`ablage.nur_online`,
`verknuepft.cloud`). Namen und Pfade stehen nicht im Log.

## Weitere Punkte der Fixrunde

Ahnenbindung (`Ordner` besitzt seinen Elternordner; unter Windows bleiben alle Ahnen ohne
`FILE_SHARE_DELETE` offen), faire Hashfortsetzung (reihum), Datei-Kennung nach dem Hash, exakter
Namensvergleich am Handle, kein Aufräumen nach Teilscan im Trigger: siehe PR #512.
