/**
 * Hörprobe im Editor der Assistenten (lva#556, G2 Teil 2): einen Text mit der vorhandenen Sprachausgabe anhören. Reine Logik
 * ohne Tauri und ohne Oberfläche; Synthese und Wiedergabe kommen herein, damit Node-Tests jeden Fehlerfall nachstellen.
 *
 * Fehlerfälle: Die Synthese scheitert (Sprachdienst abgestürzt, nicht eingerichtet) oder die Wiedergabe (kein Ausgabegerät):
 * der Zustand geht auf `fehler`, nie bleibt ein Spinner stehen. Läuft ein Gespräch, gibt es keine Hörprobe (beide teilten sich
 * Sprachdienst und Ausgabe); beginnt eines während der Synthese, wird nicht mehr abgespielt. Eine neue Hörprobe oder `stopp`
 * macht jede ältere unwirksam (ein spätes Ergebnis spielt nie), und es klingt immer nur eine.
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
  /** Der Text als WAV in Base64 (wirft bei einem Fehler). */
  synthese(text: string): Promise<string>;
  /** Spielt das WAV ab (wirft, wenn es nicht beginnen kann). */
  spiele(wavBase64: string): Wiedergabe;
}

export class Hoerprobe {
  private nummer = 0;
  private aktuell: Wiedergabe | null = null;
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
  }

  /** Beginnt eine Hörprobe; die Zusage endet mit dem Ende der Wiedergabe (oder dem Fehler). */
  async starte(text: string): Promise<void> {
    const mein = ++this.nummer;
    this.beende();
    if (text.trim() === "") return this.setze({ art: "fehler", grund: "leer" });
    if (this.abhaengig.gespraechLaeuft())
      return this.setze({ art: "fehler", grund: "gespraech" });
    this.setze({ art: "laedt" });
    let wav: string;
    try {
      wav = await this.abhaengig.synthese(text);
    } catch {
      if (mein === this.nummer)
        this.setze({ art: "fehler", grund: "synthese" });
      return;
    }
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
