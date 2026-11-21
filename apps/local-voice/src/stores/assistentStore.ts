/**
 * Die Auswahl des Assistenten im Sprachgespräch (lva#556, Paket G2). Hält die geladenen Assistenten (Vorlagen und eigene),
 * die Auswahl und den Namen für die Platzhalter. Die Prüfung der Dateien steht in `profil/speicher.ts`, die Wirkung im
 * Gespräch in `profil/wirkung.ts`; dieser Store verbindet sie mit der Oberfläche und dem Gesprächsspeicher.
 *
 * Ohne geladenen Speicher (oder bei einem Lesefehler) gilt „Alles“: das Gespräch verhält sich wie vor dem Paket.
 */
import { create } from "zustand";
import {
  aktivesProfil,
  ladeAssistenten,
  nurVorlagen,
  waehleAssistent,
  SpeicherFehler,
  type Assistenten,
  type SpeicherCode,
  type Zugriff,
} from "@/components/voice-call/profil/speicher";
import { tauriZugriff } from "@/components/voice-call/profil/zugriff";
import {
  werteJetzt,
  wirkungFuer,
  type AssistentWirkung,
} from "@/components/voice-call/profil/wirkung";

/** So lange wartet ein Gesprächsschritt höchstens auf das Laden der Assistenten; danach gilt „Alles“ mit den Vorlagen. */
export const LADEN_WARTEZEIT_MS = 2000;
/** Ein Laden, das länger läuft, wird nicht mehr mitbenutzt (hängt der Speicher, soll ein neuer Versuch nicht daran hängen). */
const LADEN_GETEILT_MS = 1000;

export interface AssistentStore {
  assistenten: Assistenten;
  /** Das Laden ist einmal durchgelaufen (auch mit Fehler). */
  geladen: boolean;
  /** Der Name aus dem Konto für die Platzhalter; `null`, wenn keiner da ist. */
  name: string | null;
  /** Fehler der letzten Auswahl (nichts wurde geändert); `null` nach Erfolg. */
  wahlFehler: SpeicherCode | null;
  /**
   * Liest das Verzeichnis neu. Ein laufendes Laden wird mitbenutzt, außer mit `frisch`: dann beginnt ein neues, das jedes
   * ältere verwirft (nach einem Schreiben darf kein Stand von davor durchkommen).
   */
  laden: (frisch?: boolean) => Promise<void>;
  /** `null`, wenn geladen; sonst das Laden, auf das ein Gesprächsschritt wartet (höchstens `LADEN_WARTEZEIT_MS`). */
  bereit: () => Promise<void> | null;
  setzeName: (name: string | null) => void;
  /** Wählt einen Assistenten und merkt ihn sich (nur `aktiv.json`). `false`: nichts geändert, `wahlFehler` sagt warum. */
  waehlen: (id: string) => Promise<boolean>;
  /**
   * Führt einen Schreibvorgang der Verwaltung aus (anlegen, speichern, löschen): hinter den Auswahlen und anderen
   * Schreibvorgängen, gegen den aktuellen Stand, und liest danach frisch neu (auch bei einem Fehler: der Speicher kann sich
   * teilweise geändert haben). Die Auswahl bleibt, wie die Datei sie hält. Der Vorgang bekommt `neuLesen` für Wiederholungen.
   */
  schreiben: <T>(
    vorgang: (
      z: Zugriff,
      a: Assistenten,
      neuLesen: () => Promise<Assistenten>,
    ) => Promise<T>,
  ) => Promise<T>;
}

let ladeLauf: { promise: Promise<void>; seit: number } | null = null;
/** Nur das jüngste Laden setzt den Zustand: ein älteres, das später fertig wird, darf ihn nicht zurückdrehen. */
let ladeNummer = 0;
/** Auswahlen laufen nacheinander: der zuletzt gespeicherte Stand ist der zuletzt gewünschte. */
let wahlKette: Promise<unknown> = Promise.resolve();
/** Zählt gespeicherte Auswahlen: ein Laden, das während einer Auswahl las, überschreibt sie nicht mit dem älteren Stand. */
let wahlZaehler = 0;

/** Wartet auf `promise`, höchstens `ms`; ein Fehler oder Ausbleiben bricht das Warten nur ab. */
async function bisZu(promise: Promise<void> | undefined, ms: number) {
  if (!promise) return;
  let frist: ReturnType<typeof setTimeout> | undefined;
  await Promise.race([
    promise.catch(() => undefined),
    new Promise<void>((fertig) => {
      frist = setTimeout(fertig, ms);
    }),
  ]);
  clearTimeout(frist);
}

export const useAssistentStore = create<AssistentStore>()((set, get) => {
  const laden = (frisch = false): Promise<void> => {
    const jetzt = Date.now();
    if (!frisch && ladeLauf && jetzt - ladeLauf.seit < LADEN_GETEILT_MS)
      return ladeLauf.promise;
    const nummer = ++ladeNummer;
    const vor = wahlZaehler;
    const promise = (async () => {
      let assistenten: Assistenten;
      try {
        assistenten = await ladeAssistenten(tauriZugriff);
      } catch {
        assistenten = nurVorlagen("read_failed");
      }
      if (nummer !== ladeNummer) return;
      set((s) => ({
        assistenten:
          vor === wahlZaehler
            ? assistenten
            : { ...assistenten, aktivId: s.assistenten.aktivId },
        geladen: true,
      }));
    })();
    const lauf = { promise, seit: jetzt };
    ladeLauf = lauf;
    void promise.finally(() => {
      if (ladeLauf === lauf) ladeLauf = null;
    });
    return promise;
  };

  return {
    assistenten: nurVorlagen(null),
    geladen: false,
    name: null,
    wahlFehler: null,

    laden,

    bereit: () => {
      if (get().geladen) return null;
      return new Promise<void>((fertig) => {
        // Hängt das Laden, gilt nach der Wartezeit „Alles“ mit den Vorlagen (wie bei einem Lesefehler); kommt das Laden
        // später doch an, setzt es den Zustand. So wartet kein weiterer Schritt noch einmal.
        const frist = setTimeout(() => {
          set((s) =>
            s.geladen
              ? s
              : { assistenten: nurVorlagen("read_failed"), geladen: true },
          );
          fertig();
        }, LADEN_WARTEZEIT_MS);
        void laden().finally(() => {
          clearTimeout(frist);
          fertig();
        });
      });
    },

    setzeName: (name) => {
      const sauber = name && name.trim() ? name.trim() : null;
      if (sauber === get().name) return;
      set({ name: sauber });
    },

    waehlen: (id) => {
      const lauf = wahlKette.then(async (): Promise<boolean> => {
        // Auf ein laufendes Laden warten (höchstens die Wartezeit): sonst prüfte die Auswahl gegen eine leere Liste.
        await bisZu(ladeLauf?.promise, LADEN_WARTEZEIT_MS);
        const a = get().assistenten;
        // Ist der Speicher nicht lesbar, wird nichts geschrieben: es bleibt bei „Alles“.
        if (a.ladefehler) {
          set({ wahlFehler: a.ladefehler });
          return false;
        }
        try {
          await waehleAssistent(tauriZugriff, a, id);
        } catch (e) {
          set({
            wahlFehler: e instanceof SpeicherFehler ? e.code : "write_failed",
          });
          return false;
        }
        wahlZaehler++;
        set((s) => ({
          assistenten: { ...s.assistenten, aktivId: id },
          wahlFehler: null,
        }));
        return true;
      });
      wahlKette = lauf.catch(() => undefined);
      return lauf;
    },

    schreiben: (vorgang) => {
      const lauf = wahlKette.then(async () => {
        // Auf ein laufendes Laden warten (höchstens die Wartezeit), damit der Vorgang gegen den neuesten Stand läuft.
        await bisZu(ladeLauf?.promise, LADEN_WARTEZEIT_MS);
        const neuLesen = async (): Promise<Assistenten> => {
          await laden(true);
          return get().assistenten;
        };
        try {
          return await vorgang(tauriZugriff, get().assistenten, neuLesen);
        } finally {
          await laden(true);
        }
      });
      wahlKette = lauf.catch(() => undefined);
      return lauf;
    },
  };
});

/** Wodurch sich der wirksame Assistent ändert: Id und, bei einem eigenen, der Text der Datei (Vorlagen sind unveränderlich). */
export function wirkungSignatur(a: Assistenten): string {
  const eigener = a.eigene.find((e) => e.profil.id === a.aktivId);
  return `${a.aktivId}\u0001${eigener?.text ?? ""}`;
}

/**
 * Der wirksame Assistent für die Engine; `null` mit „Alles“ (dann gilt, was die Nutzereinstellungen sagen). Die Platzhalter
 * werden erst beim Aufruf von `werte` gelesen (Beginn eines Gesprächs), nicht hier.
 */
export function aktuelleWirkung(
  zustand: Pick<AssistentStore, "assistenten"> = useAssistentStore.getState(),
): AssistentWirkung | null {
  return wirkungFuer(aktivesProfil(zustand.assistenten), () =>
    werteJetzt(useAssistentStore.getState().name),
  );
}
