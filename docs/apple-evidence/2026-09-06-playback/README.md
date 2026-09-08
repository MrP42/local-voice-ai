# Originalaufnahmen und Watch-Lautstärke

Bisher war im Detail einer Sprachnotiz nur die erzeugte Antwort abspielbar. Die
Originaldatei blieb gespeichert und konnte am iPhone exportiert werden, hatte
aber keinen eigenen Player.

## Bedienung

- iPhone und Watch: Sprachnotiz im Verlauf öffnen, unter **Deine Aufnahme** auf
  Play tippen. Derselbe Knopf pausiert und setzt fort; daneben steht Stopp.
- Während der Wiedergabe werden Position und Dauer angezeigt. Die Antwort hat
  weiterhin ihren eigenen Abspielknopf.
- Watch: **Lautstärke** auf der Startseite oder in der Sprachnotiz öffnen und mit
  der Digital Crown die lokale Medienlautstärke anpassen. Die Wiedergabe läuft
  beim Navigieren in diese Ansicht weiter.

Die Lautstärkeansicht verwendet Apples `WKInterfaceVolumeControl(origin: .local)`;
sie steuert die Watch und nicht das gekoppelte iPhone. Es handelt sich um die
Medienlautstärke des aktiven Ausgabegeräts, nicht um Alarm-/Hinweislautstärke.
[Apple-Dokumentation](https://developer.apple.com/documentation/watchkit/wkinterfacevolumecontrol).

## Verhalten

Der Originalplayer verwendet AVAudioPlayer und liest ausschließlich die gespeicherte
Datei. Abspielen, Pause und Stopp verändern das Original nicht. Start einer Aufnahme,
explizite Antwortwiedergabe, Audio-Unterbrechung oder Verlassen der App beenden die
Originalwiedergabe. Automatisch eintreffende Antworten unterbrechen kein abgespieltes
oder pausiertes Original. Eine unlesbare Datei zeigt einen Hinweis direkt am Player;
Transkript und Original bleiben erhalten.

Dieser Ausbau betrifft die eigenen Sprachnotizen auf iPhone und Watch. Er ergänzt
keinen neuen Medieneditor und keinen Hintergrund-Audioplayer.

## Nachweise, 06.09.2026

- 84 Core-Tests bestanden.
- iPhone: Original abspielen/pausieren/fortsetzen/stoppen und Rückkehr nach Home;
  Fehler bei unlesbarer Datei mit erhaltenem Transkript; Freisprechzyklus als
  Regressionstest – alle drei Tests bestanden.
- Watch: Original abspielen/pausieren/fortsetzen/stoppen und Rückkehr nach Home;
  lokale Lautstärkeansicht während laufender Wiedergabe erreichbar – beide Tests
  bestanden.
- iPhone-Ansicht in Hell/Dunkel und Watch-Ansichten visuell geprüft.
- Wiedergabetests verwenden eine synthetische 20-Sekunden-AAC-Datei. Keine neue
  Nutzeraufnahme wird für diese Tests benötigt. Die Tests prüfen Player-Bedienung
  und native UI-Integration, nicht die akustische Lautstärke an echter Hardware.
- Lautsprecher-/Bluetooth-Hörprobe und Digital-Crown-Lautstärkeänderung an echter
  Hardware noch nicht gemessen. Der Simulator stellt den Systemregler dar, bildet
  aber das reale Audio-Ausgabegerät nicht verlässlich ab.

Code ist im Commit `c715ad6` auf `codex/watch-conversation-background` gesichert.
Der ursprüngliche Checkout und fremde Änderungen bleiben unangetastet; kein Push
und kein Merge nach main.

## Geräteinstallation

Frische signierte iPhone- und Watch-Builds erfolgreich. Das Update wurde auf beiden echten Geräten installiert. Die abschließenden Starts wurden jeweils ausdrücklich wegen der Gerätesperre abgewiesen. Zum Starten und für die akustische Kontrolle müssen iPhone und Watch entsperrt werden.
