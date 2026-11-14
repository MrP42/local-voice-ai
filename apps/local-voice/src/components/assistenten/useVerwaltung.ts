/**
 * Die Vorgänge der Seite „Assistenten“ (lva#556, G2 Teil 2): anlegen, kopieren, umbenennen, speichern, löschen, importieren,
 * exportieren und im Gespräch verwenden. Die Logik steht in `voice-call/profil/` (geprüft, getestet); hier werden die
 * Vorgänge an den Speicher gehängt, ihr Ergebnis als Meldung gezeigt und die Oberfläche währenddessen gesperrt.
 */
import { useCallback, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { save } from "@tauri-apps/plugin-dialog";
import { commands } from "@/bindings";
import { useAssistentStore } from "@/stores/assistentStore";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { kodiereProfil } from "@/components/voice-call/assistenten";
import type { Profil, ProfilFehler } from "@/components/voice-call/assistenten";
import {
  legeNeuAn,
  loesche,
  ueberschreibe,
  type Ausgang,
  type Fehlergrund,
  type Misserfolg,
} from "@/components/voice-call/profil/verwaltung";
import {
  MAX_DATEI_BYTES,
  exportieren as baueAusgabe,
  importieren as liesImport,
  kopieren as kopiere,
  neuAnlegen,
  umbenennen as benenneUm,
} from "@/components/voice-call/profil/verwalten";
import { fehlerZeilen, grundText } from "./fehler";

/** Eine Meldung, die stehen bleibt, bis sie geschlossen wird (Fehler mit Liste, Hinweis zum Import). */
export interface Meldung {
  art: "fehler" | "hinweis";
  titel: string;
  zeilen: string[];
}

/** Ergebnis eines Vorgangs für die Seite: gelungen (mit dem Profil, wenn eines entstand) oder nicht. */
export type Vorgang = { ok: true; profil?: Profil } | Misserfolg;

const nein = (grund: Fehlergrund, fehler: ProfilFehler[] = []): Misserfolg => ({
  ok: false,
  grund,
  fehler,
});

export function useVerwaltung() {
  const { t } = useTranslation();
  const [laeuft, setLaeuft] = useState(false);
  const [meldung, setMeldung] = useState<Meldung | null>(null);

  const schliesseMeldung = useCallback(() => setMeldung(null), []);

  /** Führt einen Vorgang aus und sperrt währenddessen die Bedienung. */
  const lauf = useCallback(async <T>(vorgang: () => Promise<T>): Promise<T> => {
    setLaeuft(true);
    try {
      return await vorgang();
    } finally {
      setLaeuft(false);
    }
  }, []);

  /** Zeigt einen Misserfolg als Meldung (mit der Fehlerliste, wenn es eine gibt). */
  const zeigeFehler = useCallback(
    (titelKey: string, m: Misserfolg, name?: string) => {
      setMeldung({
        art: "fehler",
        titel: t(titelKey, { name }),
        zeilen: [grundText(t, m.grund), ...fehlerZeilen(t, m.fehler)],
      });
    },
    [t],
  );

  const neu = useCallback(
    (name: string): Promise<Vorgang> =>
      lauf(async () => {
        const r = await useAssistentStore
          .getState()
          .schreiben((z, a, neuLesen) =>
            legeNeuAn(z, a, neuLesen, (stand) => neuAnlegen(name, stand)),
          );
        if (!r.ok) {
          zeigeFehler("assistenten.meldung.neuFehler", r);
          return r;
        }
        setMeldung(null);
        toast.success(
          t("assistenten.meldung.angelegt", { name: r.profil.name }),
        );
        return { ok: true as const, profil: r.profil };
      }),
    [lauf, t, zeigeFehler],
  );

  const kopieren = useCallback(
    (quelle: Profil, name: string): Promise<Vorgang> =>
      lauf(async () => {
        const r = await useAssistentStore
          .getState()
          .schreiben((z, a, neuLesen) =>
            legeNeuAn(z, a, neuLesen, (stand) => kopiere(quelle, name, stand)),
          );
        if (!r.ok) {
          zeigeFehler("assistenten.meldung.kopierenFehler", r, quelle.name);
          return r;
        }
        setMeldung(null);
        toast.success(
          t("assistenten.meldung.kopiert", { name: r.profil.name }),
        );
        return { ok: true as const, profil: r.profil };
      }),
    [lauf, t, zeigeFehler],
  );

  const umbenennen = useCallback(
    (id: string, name: string): Promise<Vorgang> =>
      lauf(async () => {
        const r = await useAssistentStore
          .getState()
          .schreiben(async (z, a): Promise<Ausgang<{ profil: Profil }>> => {
            const alt = a.eigene.find((e) => e.profil.id === id);
            if (!alt) return nein("not_found");
            const neuesProfil = benenneUm(alt.profil, name);
            if (!neuesProfil.ok) return nein("ungueltig", neuesProfil.fehler);
            const s = await ueberschreibe(z, a, id, neuesProfil.profil);
            return s.ok ? { ok: true, profil: neuesProfil.profil } : s;
          });
        if (!r.ok) {
          zeigeFehler("assistenten.meldung.umbenennenFehler", r);
          return r;
        }
        setMeldung(null);
        toast.success(
          t("assistenten.meldung.umbenannt", { name: r.profil.name }),
        );
        return { ok: true as const, profil: r.profil };
      }),
    [lauf, t, zeigeFehler],
  );

  /** Speichert den Entwurf. Fehler zeigt der Editor selbst (am Feld, als Konflikt); hier bleibt es bei der Rückgabe. */
  const speichern = useCallback(
    (id: string, entwurf: Profil): Promise<Vorgang> =>
      lauf(async () => {
        const r = await useAssistentStore
          .getState()
          .schreiben((z, a) => ueberschreibe(z, a, id, entwurf));
        if (r.ok) {
          setMeldung(null);
          toast.success(
            t("assistenten.meldung.gespeichert", { name: entwurf.name }),
          );
          return { ok: true as const, profil: entwurf };
        }
        return r;
      }),
    [lauf, t],
  );

  const loeschen = useCallback(
    (id: string, name: string): Promise<Vorgang> =>
      lauf(async () => {
        const r = await useAssistentStore
          .getState()
          .schreiben((z, a) => loesche(z, a, id));
        if (!r.ok) {
          zeigeFehler("assistenten.meldung.loeschenFehler", r, name);
          return r;
        }
        setMeldung(null);
        toast.success(t("assistenten.meldung.geloescht", { name }));
        return { ok: true as const };
      }),
    [lauf, t, zeigeFehler],
  );

  /** Liest eine Datei als Assistent ein. Jede Abweisung kommt als Liste; nichts wird repariert. */
  const importieren = useCallback(
    (datei: File): Promise<Vorgang> =>
      lauf(async () => {
        // Zu große Dateien werden nicht gelesen (der Speicher nähme sie ohnehin nicht an).
        if (datei.size > MAX_DATEI_BYTES) {
          const m = nein("zu_gross");
          zeigeFehler("assistenten.meldung.importFehler", m, datei.name);
          return m;
        }
        let text: string;
        try {
          text = await datei.text();
        } catch {
          const m = nein("read_failed");
          zeigeFehler("assistenten.meldung.importFehler", m, datei.name);
          return m;
        }
        let hinweis: { ursprungsId: string; idGeaendert: boolean } | null =
          null;
        const r = await useAssistentStore
          .getState()
          .schreiben((z, a, neuLesen) =>
            legeNeuAn(z, a, neuLesen, (stand) => {
              const e = liesImport(text, stand);
              if (!e.ok) return nein(e.grund, e.fehler);
              hinweis = {
                ursprungsId: e.ursprungsId,
                idGeaendert: e.idGeaendert,
              };
              return { ok: true as const, profil: e.profil };
            }),
          );
        if (!r.ok) {
          zeigeFehler("assistenten.meldung.importFehler", r, datei.name);
          return r;
        }
        const h = hinweis as {
          ursprungsId: string;
          idGeaendert: boolean;
        } | null;
        if (h?.idGeaendert)
          setMeldung({
            art: "hinweis",
            titel: t("assistenten.meldung.importiertNeueId", {
              name: r.profil.name,
            }),
            zeilen: [
              t("assistenten.meldung.importiertNeueIdZeile", {
                alt: h.ursprungsId,
                neu: r.profil.id,
              }),
            ],
          });
        else setMeldung(null);
        toast.success(
          t("assistenten.meldung.importiert", { name: r.profil.name }),
        );
        return { ok: true as const, profil: r.profil };
      }),
    [lauf, t, zeigeFehler],
  );

  /** Fragt nach dem Ort (Speichern-Dialog) und schreibt die Datei atomar (Rust); abgebrochen ist kein Fehler. */
  const exportieren = useCallback(
    (profil: Profil): Promise<Vorgang | null> =>
      lauf(async () => {
        const ausgabe = baueAusgabe(profil);
        let ziel: string | null;
        try {
          ziel = await save({
            defaultPath: ausgabe.dateiname,
            filters: [
              { name: t("assistenten.export.filter"), extensions: ["json"] },
            ],
          });
        } catch {
          const m = nein("write_failed");
          zeigeFehler("assistenten.meldung.exportFehler", m, profil.name);
          return m;
        }
        if (typeof ziel !== "string") return null;
        let antwort;
        try {
          antwort = await commands.voiceAssistantExport(ziel, ausgabe.text);
        } catch {
          const m = nein("write_failed");
          zeigeFehler("assistenten.meldung.exportFehler", m, profil.name);
          return m;
        }
        if (antwort.status === "error") {
          const m = nein(antwort.error);
          zeigeFehler("assistenten.meldung.exportFehler", m, profil.name);
          return m;
        }
        setMeldung(null);
        toast.success(
          t("assistenten.meldung.exportiert", { name: profil.name }),
        );
        return { ok: true as const };
      }),
    [lauf, t, zeigeFehler],
  );

  /** Wählt den Assistenten für das Gespräch (beginnt ein neues, leeres Gespräch) und wechselt zum Gespräch. */
  const verwenden = useCallback(
    async (id: string): Promise<boolean> => {
      const ok = await useVoiceCallStore.getState().assistentWechseln(id);
      if (!ok) {
        const code = useAssistentStore.getState().wahlFehler;
        setMeldung({
          art: "fehler",
          titel: t("assistenten.meldung.verwendenFehler"),
          zeilen: [grundText(t, code ?? "write_failed")],
        });
        return false;
      }
      setMeldung(null);
      window.dispatchEvent(
        new CustomEvent("lv-navigate", { detail: { section: "home" } }),
      );
      return true;
    },
    [t],
  );

  return {
    laeuft,
    meldung,
    schliesseMeldung,
    setzeMeldung: setMeldung,
    neu,
    kopieren,
    umbenennen,
    speichern,
    loeschen,
    importieren,
    exportieren,
    verwenden,
    exportText: (p: Profil) => kodiereProfil(p),
  };
}
