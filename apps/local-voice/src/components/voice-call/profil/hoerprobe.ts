/**
 * Hörprobe im Editor der Assistenten (lva#556, G2 Teil 2): einen Text mit der vorhandenen Sprachausgabe anhören. Reine Logik
 * ohne Tauri und ohne Oberfläche; Synthese und Wiedergabe kommen herein, damit Node-Tests jeden Fehlerfall nachstellen.
 *
 * Fehlerfälle: Die Synthese scheitert (Sprachdienst abgestürzt, nicht eingerichtet) oder die Wiedergabe (kein Ausgabegerät):
 * der Zustand geht auf `fehler`, nie bleibt ein Spinner stehen. Läuft ein Gespräch, gibt es keine Hörprobe (beide teilten sich
 * Sprachdienst und Ausgabe); beginnt eines während der Synthese, wird nicht mehr abgespielt. Eine neue Hörprobe oder `stopp`
 * macht jede ältere unwirksam (ein spätes Ergebnis spielt nie), und es klingt immer nur eine. Eine Synthese, die noch läuft,
 * wird dabei **wirklich abgebrochen** (`abbrechen` mit der Anfrage-Id, mit der sie begann), nicht nur ignoriert: der
 * Sprachdienst arbeitet nicht für eine Hörprobe weiter, die niemand mehr hört. Nach fertiger Synthese gibt es nichts mehr
 * abzubrechen (ein Abbruch ohne laufende Anfrage bliebe im Sprachdienst als vorgemerkter Abbruch liegen).
 *
 * `HoerprobeBesitzer` ist die EINE Hörprobe der Seite: mehrere Knöpfe (Begrüßung, Stimme) teilen sich eine `Hoerprobe` und
 * damit Sprachdienst und Ausgabe; ein zweiter Start stoppt den ersten, auch den eines anderen Knopfs, und fremde Wiedergaben
 * (die Stimmenvorschau) werden mitgestoppt und stoppen umgekehrt die Hörprobe. Beginn eines Gesprächs und Verlassen der
 * Seite stoppen alles.
 */

export type HoerprobeGrund = "leer" | "gespraech" | "synthese" | "wiedergabe";

export type HoerprobeZustand =
  | { art: "bereit" }
  | { art: "laedt" }
  | { art: "spielt" }
  | { art: "fehler"; grund: HoerprobeGrund };

/** Eine laufende Wiedergabe: `ende` erfüllt sich, wenn sie zu Ende ist (oder mit `stopp` beendet wurde). */
export interface Wiedergabe {
  stopp(): void;
  ende: Promise<void>;
}

export interface HoerprobeAbhaengig {
  /** Läuft ein Gespräch (der Sprachdienst und die Ausgabe sind dann belegt)? */
  gespraechLaeuft(): boolean;
  /** Der Text als WAV in Base64 (wirft bei einem Fehler). `anfrage` ist die Id, unter der `abbrechen` sie stoppen kann. */
  synthese(text: string, anfrage: string): Promise<string>;
  /** Bricht die laufende Synthese dieser Anfrage ab (ein Fehler dabei zählt nicht: die Synthese ist dann schon zu Ende). */
  abbrechen(anfrage: string): void | Promise<unknown>;
  /** Spielt das WAV ab (wirft, wenn es nicht beginnen kann). */
  spiele(wavBase64: string): Wiedergabe;
}

/** Eine Id, die unter allen Hörproben dieses Prozesses (auch anderer Instanzen) einmalig ist. */
function neuerVorsatz(): string {
  const zufall =
    typeof crypto !== "undefined" && "randomUUID" in crypto
      ? crypto.randomUUID()
      : `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
  return `hoerprobe-${zufall}`;
}

export class Hoerprobe {
  private nummer = 0;
  private aktuell: Wiedergabe | null = null;
  /** Die Anfrage-Id der Synthese, die gerade läuft (`null`: keine). */
  private anfrage: string | null = null;
  private readonly vorsatz = neuerVorsatz();
  private _zustand: HoerprobeZustand = { art: "bereit" };
  private readonly abhaengig: HoerprobeAbhaengig;
  private readonly bei: (zustand: HoerprobeZustand) => void;

  constructor(
    abhaengig: HoerprobeAbhaengig,
    bei: (zustand: HoerprobeZustand) => void = () => undefined,
  ) {
    this.abhaengig = abhaengig;
    this.bei = bei;
  }

  get zustand(): HoerprobeZustand {
    return this._zustand;
  }

  private setze(zustand: HoerprobeZustand): void {
    this._zustand = zustand;
    this.bei(zustand);
  }

  private beende(): void {
    const w = this.aktuell;
    this.aktuell = null;
    try {
      w?.stopp();
    } catch {
      /* eine Wiedergabe, die sich nicht anhalten lässt, ist schon zu Ende */
    }
    this.bricheSyntheseAb();
  }

  /** Bricht die laufende Synthese ab, falls es eine gibt; ein Fehler dabei bleibt ohne Folge. */
  private bricheSyntheseAb(): void {
    const id = this.anfrage;
    this.anfrage = null;
    if (id === null) return;
    try {
      void Promise.resolve(this.abhaengig.abbrechen(id)).catch(() => undefined);
    } catch {
      /* der Abbruch selbst scheiterte: die Synthese ist ohnehin unwirksam (`nummer`) */
    }
  }

  /** Beginnt eine Hörprobe; die Zusage endet mit dem Ende der Wiedergabe (oder dem Fehler). */
  async starte(text: string): Promise<void> {
    const mein = ++this.nummer;
    this.beende();
    if (text.trim() === "") return this.setze({ art: "fehler", grund: "leer" });
    if (this.abhaengig.gespraechLaeuft())
      return this.setze({ art: "fehler", grund: "gespraech" });
    this.setze({ art: "laedt" });
    const id = `${this.vorsatz}-${mein}`;
    this.anfrage = id;
    let wav: string;
    try {
      wav = await this.abhaengig.synthese(text, id);
    } catch {
      if (this.anfrage === id) this.anfrage = null;
      if (mein === this.nummer)
        this.setze({ art: "fehler", grund: "synthese" });
      return;
    }
    // Die Synthese ist fertig: von nun an gibt es nichts mehr abzubrechen, nur noch die Wiedergabe anzuhalten.
    if (this.anfrage === id) this.anfrage = null;
    if (mein !== this.nummer) return;
    if (this.abhaengig.gespraechLaeuft())
      return this.setze({ art: "fehler", grund: "gespraech" });
    let wiedergabe: Wiedergabe;
    try {
      wiedergabe = this.abhaengig.spiele(wav);
    } catch {
      return this.setze({ art: "fehler", grund: "wiedergabe" });
    }
    this.aktuell = wiedergabe;
    this.setze({ art: "spielt" });
    try {
      await wiedergabe.ende;
    } catch {
      if (mein === this.nummer) {
        this.aktuell = null;
        this.setze({ art: "fehler", grund: "wiedergabe" });
      }
      return;
    }
    if (mein !== this.nummer) return;
    this.aktuell = null;
    this.setze({ art: "bereit" });
  }

  /** Hält an, was gerade klingt oder noch erzeugt wird. */
  stopp(): void {
    this.nummer += 1;
    this.beende();
    this.setze({ art: "bereit" });
  }
}

/** Wer die Hörprobe gerade hat (`quelle`: der Knopf, der sie gestartet hat) und in welchem Zustand sie ist. */
export interface BesitzerStand {
  quelle: string | null;
  zustand: HoerprobeZustand;
}

/**
 * Die eine Hörprobe der Seite. `fremde` sind Wiedergaben, die nicht über diese Hörprobe laufen (die Stimmenvorschau): sie
 * werden vor jeder Hörprobe und bei Gesprächsbeginn gestoppt. Der Stand ist unveränderlich und wird nur bei einer Änderung
 * ersetzt (für `useSyncExternalStore`).
 */
export class HoerprobeBesitzer {
  private readonly probe: Hoerprobe;
  private readonly fremde: readonly (() => void)[];
  private readonly abonnenten = new Set<() => void>();
  private quelle: string | null = null;
  private _stand: BesitzerStand = { quelle: null, zustand: { art: "bereit" } };

  constructor(
    abhaengig: HoerprobeAbhaengig,
    fremde: readonly (() => void)[] = [],
  ) {
    this.fremde = fremde;
    this.probe = new Hoerprobe(abhaengig, (zustand) => this.setze(zustand));
  }

  get stand(): BesitzerStand {
    return this._stand;
  }

  /** Meldet sich für Änderungen des Stands an; liefert die Abmeldung. */
  abonniere = (bei: () => void): (() => void) => {
    this.abonnenten.add(bei);
    return () => {
      this.abonnenten.delete(bei);
    };
  };

  private setze(zustand: HoerprobeZustand): void {
    if (zustand.art === "bereit") this.quelle = null;
    const alt = this._stand;
    if (
      alt.quelle === this.quelle &&
      alt.zustand.art === zustand.art &&
      (zustand.art !== "fehler" ||
        (alt.zustand.art === "fehler" && alt.zustand.grund === zustand.grund))
    )
      return;
    this._stand = { quelle: this.quelle, zustand };
    for (const bei of [...this.abonnenten]) bei();
  }

  private stoppeFremde(): void {
    for (const stoppe of this.fremde) {
      try {
        stoppe();
      } catch {
        /* eine fremde Wiedergabe, die sich nicht anhalten lässt, ist schon zu Ende */
      }
    }
  }

  /** Startet die Hörprobe für `quelle`: jede laufende (auch die eines anderen Knopfs) und jede fremde Wiedergabe endet. */
  spiele(quelle: string, text: string): Promise<void> {
    this.stoppeFremde();
    this.quelle = quelle;
    return this.probe.starte(text);
  }

  /** Hält an, was gerade klingt oder erzeugt wird (gleich, wer es gestartet hat). */
  stopp(): void {
    this.probe.stopp();
  }

  /** Hält nur an, wenn `quelle` die Hörprobe hat (ein Knopf, der verschwindet, stoppt nicht die eines anderen). */
  stoppFuer(quelle: string): void {
    if (this.quelle === quelle) this.probe.stopp();
  }

  /** Hält die Hörprobe (samt laufender Synthese) und jede fremde Wiedergabe an: beim Verlassen der Seite, beim Wechsel. */
  stoppAlles(): void {
    this.probe.stopp();
    this.stoppeFremde();
  }

  /** Ein Gespräch beginnt: alles endet (Sprachdienst und Ausgabe gehören dem Gespräch). */
  gespraechBegonnen(): void {
    this.stoppAlles();
  }

  /** Eine fremde Wiedergabe (Stimmenvorschau) beginnt: die Hörprobe endet, es klingt immer nur eines. */
  fremdeBeginnt(): void {
    this.probe.stopp();
  }
}
