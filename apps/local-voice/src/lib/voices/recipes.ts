/**
 * Startpunkte fuer den Stimmen-Baukasten.
 *
 * Ein Rezept ist KEINE fertige Stimme — Fish-Speech kennt keine
 * Konditionierung auf eine Beschreibung, die Stimmidentitaet haengt allein am
 * Seed. Ein Rezept fuellt deshalb vor, was die Beschreibung wirklich steuern
 * kann: den Probesatz (dessen Prosodie sich beim Klonen ueberträgt), die
 * Emotions-Tags und die Metadaten. Gewuerfelt und ausgesucht wird danach.
 *
 * Ein Rezept ist damit ein Startpunkt und kein Ergebnis: es stellt die
 * Seed-Lotterie gut auf, es gewinnt sie nicht. Wer denselben Eintrag zweimal
 * durchlaufen laesst, bekommt zwei verschiedene Stimmen — ueber Timbre, Alter
 * und Geschlecht entscheidet allein der Seed, das Rezept nur ueber Sprechweise
 * und Stimmung. Eingeloest ist ein Rezept erst, wenn aus mehreren Kandidaten
 * einer ausgewaehlt und sein Seed festgehalten wurde.
 *
 * `color` ist ein Palette-Key aus `registry.rs` (siehe `palette.ts`). Die
 * Werte in `tags` sind `insert`-Werte aus der Tag-Registry
 * (`src/lib/tags/registry.ts`) — ein erfundenes Tag landet woertlich im
 * Probesatz und waere ein stiller Fehler.
 */
export interface VoiceRecipe {
  id: string;
  /** Vorgeschlagener Anzeigename — frei aenderbar. */
  name: string;
  /** Geht als Beschreibung in die Stimme und steuert den Probesatz-Vorschlag. */
  description: string;
  /** Referenzsatz, hoechstens etwa 150 Zeichen, in der Rolle gesprochen. */
  probeText: string;
  tags: string[];
  color: string;
}

export const VOICE_RECIPES: VoiceRecipe[] = [
  {
    id: "pyrion",
    name: "Pyrion",
    description:
      "Sehr tiefe, erwachsene Männerstimme mit viel Resonanz und ruhiger Autorität. " +
      "Alt, mächtig und würdevoll, als hätte er Jahrhunderte erlebt. Langsames bis " +
      "mittleres Tempo, klare Artikulation, kaum Hektik. Klanglich warm und dunkel, " +
      "mit leicht rauer, steiniger Textur — nicht dämonisch, nicht monströs. Selbst " +
      "wütend bleibt die Stimme kontrolliert und schwer statt schrill. Grundstimmung: " +
      "majestätisch, ernst, geheimnisvoll, erfahren. Ein uralter Wächter oder König " +
      "aus einem Fantasyfilm — gewaltig und respekteinflößend, aber vertrauenswürdig. " +
      "Keine Karikatur, kein übertriebenes Bösewicht-Lachen, kein Brüllen.",
    probeText:
      "Ich habe Königreiche kommen und vergehen sehen. Hört mir gut zu, denn ich sage es nur einmal.",
    tags: ["speaking slowly", "serious"],
    color: "amber",
  },
  {
    id: "erzaehler-maennlich",
    name: "Erzähler",
    description:
      "Erwachsene Männerstimme mittleren Alters, mitteltief, mit warmem, trockenem " +
      "Klang ohne Hall. Gleichmäßiges, eher ruhiges Tempo und saubere Artikulation, " +
      "Sätze werden zu Ende getragen. Grundhaltung: unaufgeregt, verlässlich, " +
      "freundlich distanziert — jemand, der die Geschichte kennt und sie nicht " +
      "ausschmücken muss. Keine Dramatik, kein Werbeton.",
    probeText:
      "An diesem Morgen begann alles ganz gewöhnlich, und niemand ahnte, wie der Tag enden würde.",
    tags: ["speaking slowly", "serious"],
    color: "slate",
  },
  {
    id: "erzaehlerin-weiblich",
    name: "Erzählerin",
    description:
      "Erwachsene Frauenstimme, mittlere Lage, weich und klar mit etwas Luft im Ton. " +
      "Ruhiges Tempo, deutliche Pausen an den Satzgrenzen, freundliche Melodie ohne " +
      "Singsang. Grundhaltung: aufmerksam, zugewandt, sicher — sie erzählt, statt " +
      "vorzulesen. Nicht säuselnd, nicht mütterlich überzeichnet.",
    probeText:
      "Setz dich einen Moment zu mir. Was ich dir erzähle, hat mir vor langer Zeit jemand anvertraut.",
    tags: ["soft tone", "sincere"],
    color: "teal",
  },
  {
    id: "gruselstimme",
    name: "Gruselstimme",
    description:
      "Erwachsene, geschlechtlich neutral gehaltene Stimme in tiefer Lage, dunkel " +
      "und belegt, fast gehaucht. Sehr langsames Tempo, lange Pausen, jedes Wort " +
      "einzeln gesetzt. Stimmung: unheimlich, lauernd, kalt — die Bedrohung liegt " +
      "in der Ruhe, nicht in der Lautstärke. Kein Schreien, keine Monsterstimme.",
    probeText:
      "Hörst du das auch? Es steht schon eine Weile hinter dir und wartet, dass du dich umdrehst.",
    tags: ["low voice", "whispering"],
    color: "violet",
  },
  {
    id: "opa",
    name: "Opa am Kamin",
    description:
      "Alte Männerstimme, tief und leicht brüchig, mit körnigem, rauem Rand. " +
      "Langsames Tempo, gemütliche Pausen, hier und da ein Schmunzeln im Ton. " +
      "Grundhaltung: herzlich, geduldig, ein wenig verschmitzt — jemand, der viel " +
      "erlebt hat und nichts mehr beweisen muss. Kein Zittern zur Karikatur.",
    probeText:
      "Komm her. Als ich so alt war wie du, gab es hier noch keine Straße, nur Wald und Wiese.",
    tags: ["speaking slowly", "comforting"],
    color: "orange",
  },
  {
    id: "oma",
    name: "Oma am Küchentisch",
    description:
      "Alte Frauenstimme, mittelhoch, warm und ein wenig rauchig, mit leichtem " +
      "Vibrato. Ruhiges Tempo, freundliche Melodie, viele kleine Betonungen. " +
      "Grundhaltung: fürsorglich, humorvoll, direkt — sie sagt geradeheraus, was " +
      "sie denkt, und meint es immer gut. Nicht piepsig, nicht klagend.",
    probeText:
      "Nun iss erst mal was Ordentliches, und dann erzählst du mir in Ruhe, was dich beschäftigt.",
    tags: ["soft tone", "delighted"],
    color: "rose",
  },
  {
    id: "eule",
    name: "Weise Eule",
    description:
      "Erwachsene, eher hohe und leichte Stimme mit rundem, hohlem Klang, wie durch " +
      "eine kleine Kammer gesprochen. Bedächtiges Tempo, kurze Sätze, oft eine " +
      "Pause vor dem entscheidenden Wort. Stimmung: nachdenklich, wohlwollend, ein " +
      "wenig belustigt. Ein sprechendes Tier, das mehr weiß, als es zugibt — kein " +
      "Uhu-Ruf, keine Tierlaute.",
    probeText:
      "Du fragst nach dem Weg. Ich frage lieber, wohin du wirklich möchtest. Das ist selten dasselbe.",
    tags: ["speaking slowly", "curious"],
    color: "sky",
  },
  {
    id: "rabe",
    name: "Rabe",
    description:
      "Erwachsene Stimme in tiefer bis mittlerer Lage, kratzig und trocken, mit " +
      "scharfen Konsonanten. Zügiges, abgehacktes Tempo, Sätze enden knapp. " +
      "Stimmung: spöttisch, klug, leicht respektlos — der Kommentator am Rand, der " +
      "immer schon Bescheid wusste. Kein Krächzen als Effekt, keine Tierlaute.",
    probeText:
      "Ich habe es dir gesagt. Zweimal sogar. Aber gut, mach weiter, ich schaue von hier oben zu.",
    tags: ["sarcastic", "low voice"],
    color: "slate",
  },
  {
    id: "baer",
    name: "Brummbär",
    description:
      "Sehr tiefe, große Stimme mit viel Brustresonanz und weichem, wolligem Klang. " +
      "Gemächliches Tempo, lange Vokale, gemütliches Brummen zwischen den Sätzen. " +
      "Grundhaltung: gutmütig, schwerfällig, freundlich — massig, aber nie " +
      "bedrohlich. Kein Grollen, kein Knurren als Dauerzustand.",
    probeText:
      "Nun mal langsam, mein Freund. Erst wird gefrühstückt, und dann sehen wir weiter.",
    tags: ["low voice", "relaxed"],
    color: "amber",
  },
  {
    id: "drache",
    name: "Drache",
    description:
      "Gewaltige, sehr tiefe Stimme mit rauchiger, heiserer Textur und langem " +
      "Nachklang. Langsames Tempo, gedehnte Zischlaute, schwere Betonungen. " +
      "Stimmung: uralt, hochmütig, ruhig gefährlich — Macht, die sich nicht " +
      "anstrengen muss. Kein Fauchen, kein Brüllen, keine Monsterkarikatur.",
    probeText:
      "Du stehst vor meinem Berg und nennst das Mut. Ich nenne es einen sehr kurzen Besuch.",
    tags: ["low voice", "serious"],
    color: "red",
  },
  {
    id: "magier",
    name: "Magier",
    description:
      "Ältere Männerstimme, mitteltief, klar und schlank, mit trockenem Timbre. " +
      "Wechselndes Tempo: ruhig erklärend, dann plötzlich scharf betont. " +
      "Grundhaltung: gelehrt, konzentriert, leicht ungeduldig mit Dummheit. " +
      "Formeln werden präzise gesprochen, nicht theatralisch geschrien.",
    probeText:
      "Ein Zauber ist kein Wunsch. Er ist eine Rechnung, und irgendjemand bezahlt sie immer.",
    tags: ["serious", "emphasis"],
    color: "violet",
  },
  {
    id: "elfe",
    name: "Elfe",
    description:
      "Erwachsene Frauenstimme, hell und klar, mit gläserner Oberfläche und wenig " +
      "Rauigkeit. Fließendes, gleichmäßiges Tempo, weiche Übergänge, kaum harte " +
      "Einsätze. Stimmung: kühl, anmutig, ein wenig fern — höflich, aber nicht " +
      "vertraulich. Nicht kindlich, nicht hauchig-verspielt.",
    probeText:
      "Wir messen die Zeit anders als ihr. Was dir eilig scheint, ist für uns kaum ein Atemzug.",
    tags: ["soft tone", "sincere"],
    color: "teal",
  },
  {
    id: "zwerg",
    name: "Zwerg",
    description:
      "Erwachsene Männerstimme, tief und gepresst, mit körnigem, erdigem Klang. " +
      "Kräftiges, zupackendes Tempo, harte Konsonanten, kurze Sätze. Grundhaltung: " +
      "stolz, direkt, stur und arbeitsam — ein Handwerker, der sein Werkzeug besser " +
      "kennt als seine Manieren. Kein Grunzen, kein Dialektklischee.",
    probeText:
      "Das hält. Das hält hundert Jahre. Und wenn nicht, sag mir ins Gesicht, dass ich mich irrte.",
    tags: ["low voice", "proud"],
    color: "orange",
  },
  {
    id: "ritter",
    name: "Ritter",
    description:
      "Erwachsene Männerstimme, mitteltief, kräftig und offen, mit klarer " +
      "Projektion. Zügiges, festes Tempo, aufrechte Betonung, keine " +
      "Verschleifungen. Grundhaltung: pflichtbewusst, geradlinig, mutig ohne " +
      "Prahlerei. Nicht gebrüllt, kein Kommandoton.",
    probeText:
      "Ich habe mein Wort gegeben, und mein Wort gilt. Tretet zurück, ich gehe zuerst hinein.",
    tags: ["confident", "serious"],
    color: "sky",
  },
  {
    id: "koenigin",
    name: "Königin",
    description:
      "Erwachsene Frauenstimme, mittlere bis tiefe Lage, voll und getragen, mit " +
      "sehr sauberer Artikulation. Ruhiges Tempo, klare Pausen, jede Silbe sitzt. " +
      "Grundhaltung: souverän, kühl höflich, gewohnt, dass man zuhört. Keine " +
      "Schärfe, keine Herablassung — Autorität ohne Lautstärke.",
    probeText:
      "Ihr habt Euer Anliegen vorgetragen. Nun hört meine Antwort und tragt sie unverfälscht zurück.",
    tags: ["confident", "serious"],
    color: "fuchsia",
  },
  {
    id: "schurke",
    name: "Schurke",
    description:
      "Erwachsene Männerstimme, mitteltief, geschmeidig und leicht ölig, mit " +
      "weichen Einsätzen. Langsames, genüssliches Tempo, gedehnte Betonungen, ein " +
      "Lächeln im Ton. Stimmung: hinterhältig, überlegen, charmant gefährlich. " +
      "Kein Bösewicht-Gelächter, kein Zischen.",
    probeText:
      "Aber natürlich helfe ich Euch. Die Frage ist nur, was Ihr mir dafür später schuldet.",
    tags: ["sneering", "low voice"],
    color: "red",
  },
  {
    id: "weiser-lehrer",
    name: "Weiser Lehrer",
    description:
      "Ältere Stimme mittlerer Lage, warm und ruhig, mit weichem Anschlag. Sehr " +
      "bedächtiges Tempo, viele kurze Pausen zum Nachdenken, gleichmäßige Melodie. " +
      "Grundhaltung: geduldig, ermutigend, ohne Belehrung — stellt lieber eine " +
      "Frage, als eine Antwort zu geben.",
    probeText:
      "Du hast dich geirrt, und das ist gut. Schau noch einmal hin und sag mir, was du siehst.",
    tags: ["speaking slowly", "sincere"],
    color: "green",
  },
  {
    id: "marktschreier",
    name: "Marktschreier",
    description:
      "Erwachsene Männerstimme, mittelhoch, laut und tragend, mit rauem Rand vom " +
      "vielen Rufen. Schnelles Tempo, starke Betonungen, steigende Melodie zum " +
      "Satzende. Stimmung: aufgekratzt, werbend, gut gelaunt — Lautstärke als " +
      "Beruf, nicht als Zorn.",
    probeText:
      "Herrschaften, hier und nur heute! Halber Preis, und wer zögert, geht leer nach Hause!",
    tags: ["loud", "excited"],
    color: "orange",
  },
  {
    id: "nachrichtensprecher",
    name: "Nachrichtensprecher",
    description:
      "Erwachsene Stimme mittlerer Lage, neutral und klar, ohne Färbung oder " +
      "Wärmeüberschuss. Gleichmäßiges, zügiges Tempo, exakte Artikulation, " +
      "sachliche Betonung der Kernwörter. Grundhaltung: distanziert, präzise, " +
      "glaubwürdig. Keine Dramatisierung, keine Emotion.",
    probeText:
      "Guten Abend. Wir beginnen mit der Meldung, die den heutigen Tag bestimmt hat.",
    tags: ["serious", "confident"],
    color: "slate",
  },
  {
    id: "hoerbuch-ruhig",
    name: "Hörbuchstimme ruhig",
    description:
      "Erwachsene Stimme mittlerer Lage, weich, trocken und sehr gleichmäßig. " +
      "Ruhiges Tempo über lange Strecken, kaum Dynamikspitzen, saubere Atempausen. " +
      "Grundhaltung: gelassen, geduldig, angenehm über Stunden — die Stimme tritt " +
      "hinter den Text zurück. Keine Betonungsspielereien.",
    probeText:
      "Das Zimmer lag still im Nachmittagslicht, und für einen Augenblick schien die Welt zu warten.",
    tags: ["speaking slowly", "relaxed"],
    color: "green",
  },
  {
    id: "maerchenerzaehler",
    name: "Märchenerzähler",
    description:
      "Erwachsene Stimme mittlerer Lage, warm und rund, mit leicht singender " +
      "Melodie. Gemächliches Tempo, deutliche Spannungspausen, sanfte " +
      "Betonungswellen. Stimmung: verzaubernd, geborgen, ein wenig altmodisch. " +
      "Nicht kindisch, nicht übertrieben verstellt.",
    probeText:
      "Es war einmal ein Königreich hinter sieben Hügeln, in dem seit hundert Jahren Frieden herrschte.",
    tags: ["soft tone", "comforting"],
    color: "amber",
  },
  {
    id: "geist",
    name: "Geist",
    description:
      "Erwachsene, körperlos wirkende Stimme in mittlerer Lage, dünn und luftig, " +
      "mit langem Nachhall. Schleppendes Tempo, verwehte Satzenden, unregelmäßige " +
      "Betonung. Stimmung: verloren, klagend, nicht ganz anwesend. Kein Heulen, " +
      "kein Stöhnen als Effekt.",
    probeText:
      "Ich gehe diesen Flur schon so lange. Manchmal vergesse ich, warum ich noch hier bin.",
    tags: ["whisper", "echo"],
    color: "violet",
  },
  {
    id: "roboter",
    name: "Roboter",
    description:
      "Erwachsene, geschlechtlich neutrale Stimme mittlerer Lage, flach und " +
      "metallisch trocken, ohne hörbare Atemgeräusche. Gleichmäßiges Tempo, " +
      "konstante Lautstärke, Betonung fast ohne Melodie. Grundhaltung: sachlich, " +
      "höflich, emotionslos — korrekt statt kalt. Keine Comic-Verzerrung.",
    probeText:
      "Anfrage verstanden. Ich bereite die Auswertung vor. Bitte bleiben Sie in Reichweite.",
    tags: ["indifferent", "serious"],
    color: "sky",
  },
  {
    id: "pirat",
    name: "Pirat",
    description:
      "Erwachsene Männerstimme, mitteltief, rau und salzig, mit lauten offenen " +
      "Vokalen. Schwungvolles Tempo, breite Betonungen, gelegentliches Lachen. " +
      "Stimmung: derb, aufgeräumt, verschlagen gut gelaunt. Kein dauerhaftes " +
      "Gegröle, keine Seemannsklischee-Laute.",
    probeText:
      "Setzt die Segel! Wer heute Abend trocken bleiben will, hat den falschen Kahn bestiegen.",
    tags: ["with strong accent", "amused"],
    color: "red",
  },
  {
    id: "detektiv",
    name: "Detektiv",
    description:
      "Erwachsene Männerstimme, tief und leise, mit rauchigem, müdem Klang. " +
      "Gedehntes Tempo, viele kleine Pausen, Betonung auf den beiläufigen " +
      "Beobachtungen. Stimmung: abgeklärt, wachsam, trocken ironisch — jemand, der " +
      "schon zu viel gesehen hat. Kein Krimi-Pathos.",
    probeText:
      "Die Tür war offen. Kein Kampf, kein Staub auf dem Griff. Jemand war hier erwartet worden.",
    tags: ["low voice", "curious"],
    color: "slate",
  },
  {
    id: "reisefuehrerin",
    name: "Reiseführerin",
    description:
      "Erwachsene Frauenstimme, mittlere Lage, hell und offen, gut verständlich " +
      "auch über Umgebungsgeräusche. Zügiges, freundliches Tempo, klare " +
      "Gliederung, betonte Zahlen und Namen. Grundhaltung: aufgeschlossen, " +
      "begeistert für die Sache, professionell. Kein Ansagerton.",
    probeText:
      "Bleiben Sie kurz stehen. Von hier aus sehen Sie die Altstadt wie die Händler vor dreihundert Jahren.",
    tags: ["interested", "confident"],
    color: "teal",
  },
  {
    id: "sportreporter",
    name: "Sportreporter",
    description:
      "Erwachsene Männerstimme, mittelhoch, kraftvoll und leicht heiser, mit " +
      "starker Projektion. Sehr schnelles Tempo, steigende Spannung, abrupte " +
      "Lautstärkesprünge an den entscheidenden Stellen. Stimmung: atemlos, " +
      "mitgerissen, live dabei. Kein Dauerschreien.",
    probeText:
      "Er nimmt den Ball an, dreht sich, zieht ab aus zwanzig Metern — und da kommt keiner mehr heran!",
    tags: ["in a hurry tone", "excited"],
    color: "orange",
  },
  {
    id: "gutenachtstimme",
    name: "Gutenachtstimme",
    description:
      "Erwachsene Stimme mittlerer Lage, sehr leise, weich und dunkel abgerundet. " +
      "Sehr langsames Tempo, lange Pausen, absinkende Satzenden, kaum Dynamik. " +
      "Grundhaltung: beruhigend, geborgen, einschläfernd im besten Sinne. Kein " +
      "Säuseln, keine Babysprache.",
    probeText:
      "Mach die Augen zu. Draußen ist es still geworden, und alles von heute darf liegen bleiben.",
    tags: ["soft tone", "speaking slowly"],
    color: "rose",
  },
  {
    id: "waldhexe",
    name: "Waldhexe",
    description:
      "Alte Frauenstimme, mittelhoch und schartig, mit brüchigen Kanten und " +
      "plötzlichem Kichern. Unregelmäßiges Tempo, abrupte Wechsel zwischen leise " +
      "und laut. Stimmung: listig, launisch, unberechenbar freundlich. Keine " +
      "Kreischerei, kein Karikaturenlachen am Satzende.",
    probeText:
      "Natürlich habe ich, was du suchst. Ob du es danach noch willst, ist eine andere Frage.",
    tags: ["sneering", "chuckle"],
    color: "fuchsia",
  },
  {
    id: "seherin",
    name: "Seherin",
    description:
      "Erwachsene Frauenstimme in tiefer Lage, ruhig und belegt, mit viel Luft im " +
      "Ton. Sehr langsames Tempo, gleichförmige Melodie, wie aus einer Trance " +
      "gesprochen. Stimmung: entrückt, ernst, unheilvoll ruhig. Kein Wispern als " +
      "Dauereffekt, keine Dramatik.",
    probeText:
      "Ich sehe zwei Wege und an ihrem Ende dieselbe Tür. Du wirst wählen müssen, ohne zu wissen.",
    tags: ["whispering", "serious"],
    color: "violet",
  },
];
