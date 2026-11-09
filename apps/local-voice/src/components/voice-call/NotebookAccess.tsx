import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { useTranslation } from "react-i18next";
import { Switch, useId } from "@fluentui/react-components";
import { ChevronRight } from "lucide-react";
import {
  commands,
  type NotebookOutline,
  type NotebookOutlineSection,
} from "@/bindings";
import { useSettings } from "@/hooks/useSettings";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  type Auswahl,
  auswahlAus,
  eintraegeAus,
  frei,
  schreibkette,
  setzeAbschnitte,
  setzeGanz,
  startOffen,
} from "./notebookSelection";

/**
 * Bereich „Notizbuch“ im Reiter „Wissen“ (lva#396 N2, Spec lva#398 §3.2, Vorschau
 * `docs/vorschau/notizbuch-freigabe`): Schalter für den Bereich, darunter die Abschnitte je
 * Notizbuch. Standard: nichts freigegeben. Die Einstellung hält nur Einträge `abschnitt:<id>` und
 * `notizbuch:<id>`; die Gliederung kommt aus dem Notizbuch (nur Namen und Seitenzahlen).
 */

type Laden =
  | { art: "laedt" }
  | { art: "ok"; nbs: NotebookOutline[] }
  | { art: "kein" }
  | { art: "version" }
  | { art: "fehler" };

/** Fehlertexte des Kerns (`notizbuch::fehlertext`) auf eigene, übersetzte Zustände abbilden. */
function ladeArt(fehler: string): Laden {
  if (fehler.startsWith("Auf diesem Gerät gibt es noch kein Notizbuch"))
    return { art: "kein" };
  if (fehler.startsWith("Diese Notizbuch-Version wird nicht unterstützt"))
    return { art: "version" };
  return { art: "fehler" };
}

const SCHLUESSEL = "voice_call_notebook_sections";

const gleich = (a: readonly string[] | undefined, b: readonly string[]) =>
  !!a && a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * Speichern der Freigabe, einmal je App statt je Baum (Codex lva#418): Schaltet man den Bereich
 * aus und wieder ein, entsteht ein neuer Baum, aber dieselbe Kette. So überholt kein Schreiben
 * des alten Baums die neue Auswahl. `lokal` ist die zuletzt gewählte, noch nicht bestätigte
 * Auswahl; nach dem Ende der Kette gilt wieder der gespeicherte Stand (bei Fehler der alte).
 */
type Speicherstand = { lokal: string[] | null; fehler: boolean };
let speicherstand: Speicherstand = { lokal: null, fehler: false };
const hoerer = new Set<() => void>();
const melden = (s: Speicherstand) => {
  speicherstand = s;
  hoerer.forEach((h) => h());
};
const freigabeKette = schreibkette<string[]>(
  async (v) => {
    await useSettingsStore.getState().updateSetting(SCHLUESSEL, v as never);
    return gleich(
      useSettingsStore.getState().settings?.[SCHLUESSEL] as
        string[] | undefined,
      v,
    );
  },
  (ok) => melden({ lokal: null, fehler: !ok }),
);
function freigabeSpeichern(v: string[]) {
  melden({ lokal: v, fehler: false });
  freigabeKette.setze(v);
}
const abonnieren = (h: () => void) => {
  hoerer.add(h);
  return () => {
    hoerer.delete(h);
  };
};
const stand = () => speicherstand;

const Pfeil: React.FC<{ offen: boolean }> = ({ offen }) => (
  <ChevronRight
    aria-hidden="true"
    className={`w-4 h-4 shrink-0 transition-transform ${offen ? "rotate-90" : ""}`}
  />
);

/** Kontrollkästchen mit Teilauswahl (`indeterminate` lässt sich nur per DOM setzen). */
const Kaestchen: React.FC<{
  id: string;
  label: string;
  checked: boolean;
  teil: boolean;
  disabled?: boolean;
  onChange: (on: boolean) => void;
}> = ({ id, label, checked, teil, disabled, onChange }) => {
  const ref = React.useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.indeterminate = teil;
  }, [teil]);
  return (
    <label className="grid place-items-center min-w-11 min-h-11 shrink-0 cursor-pointer">
      <input
        ref={ref}
        id={id}
        type="checkbox"
        aria-label={label}
        className="w-[18px] h-[18px] m-0 accent-logo-primary"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
    </label>
  );
};

const AbschnittZeile: React.FC<{
  id: string;
  a: NotebookOutlineSection;
  checked: boolean;
  ganz: boolean;
  onChange: (on: boolean) => void;
}> = ({ id, a, checked, ganz, onChange }) => {
  const { t } = useTranslation();
  return (
    <label
      className={`flex items-center gap-2.5 min-h-11 px-1 rounded-md ${ganz ? "text-text-muted" : "cursor-pointer hover:bg-mid-gray/10"}`}
    >
      <input
        id={id}
        type="checkbox"
        className="w-[18px] h-[18px] mx-[3px] my-0 shrink-0 accent-logo-primary"
        checked={checked}
        disabled={ganz}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="flex-1 min-w-0 truncate text-[13px]">{a.name}</span>
      <span className="text-xs text-text-muted whitespace-nowrap">
        {t("voiceCall.knowledge.notebook.pages", { count: a.pages })}
      </span>
    </label>
  );
};

function zustand(ids: string[], gewaehlt: (id: string) => boolean) {
  const k = ids.filter(gewaehlt).length;
  return { checked: k > 0 && k === ids.length, teil: k > 0 && k < ids.length };
}

const NotizbuchKarte: React.FC<{
  nb: NotebookOutline;
  auswahl: Auswahl;
  offen: boolean;
  ids: string;
  onOffen: (offen: boolean) => void;
  onAuswahl: (a: Auswahl) => void;
}> = ({ nb, auswahl, offen, ids, onOffen, onAuswahl }) => {
  const { t } = useTranslation();
  const ganz = auswahl.ganz.has(nb.id);
  const gewaehlt = (id: string) => ganz || auswahl.abschnitte.has(id);
  const alle = nb.sections.map((a) => a.id);
  const st = zustand(alle, gewaehlt);
  const anzahl = alle.filter(gewaehlt).length;
  const koerper = `${ids}-nbb-${nb.id}`;
  const ganzHinweis = `${ids}-gh-${nb.id}`;
  const ohneGruppe = nb.sections.filter((a) => !a.group);
  const gruppen = [
    ...new Set(nb.sections.flatMap((a) => (a.group ? [a.group] : []))),
  ];
  const zeile = (a: NotebookOutlineSection) => (
    <AbschnittZeile
      key={a.id}
      id={`${ids}-a-${a.id}`}
      a={a}
      checked={gewaehlt(a.id)}
      ganz={ganz}
      onChange={(on) => onAuswahl(setzeAbschnitte(auswahl, [a.id], on))}
    />
  );
  return (
    <div className="rounded-lg border border-mid-gray/25">
      <div className="flex items-center gap-1 ps-1 pe-2.5">
        <Kaestchen
          id={`${ids}-nb-${nb.id}`}
          label={t("voiceCall.knowledge.notebook.selectAll", { name: nb.name })}
          checked={st.checked}
          teil={st.teil}
          disabled={ganz}
          onChange={(on) => onAuswahl(setzeAbschnitte(auswahl, alle, on))}
        />
        <button
          type="button"
          id={`${ids}-ak-${nb.id}`}
          className="flex flex-1 min-w-0 items-center gap-2 min-h-11 p-0 border-0 bg-transparent cursor-pointer text-start"
          aria-expanded={offen}
          aria-controls={koerper}
          onClick={() => onOffen(!offen)}
        >
          <Pfeil offen={offen} />
          <span className="flex-1 min-w-0 truncate text-[13px] font-semibold">
            {nb.name}
          </span>
          <span className="text-xs text-text-muted whitespace-nowrap">
            {ganz
              ? t("voiceCall.knowledge.notebook.whole")
              : t("voiceCall.knowledge.notebook.countOf", {
                  count: anzahl,
                  total: alle.length,
                })}
          </span>
        </button>
      </div>
      <div
        id={koerper}
        hidden={!offen}
        className="px-2.5 pb-1.5 border-t border-mid-gray/20"
      >
        <div className="pt-2">
          <Switch
            checked={ganz}
            onChange={(_e, d) =>
              onAuswahl(setzeGanz(auswahl, nb.id, d.checked))
            }
            label={t("voiceCall.knowledge.notebook.wholeSwitch", {
              name: nb.name,
            })}
            aria-describedby={ganzHinweis}
          />
          <p
            id={ganzHinweis}
            className={`m-0 ps-[52px] -mt-1 pb-1.5 text-xs ${ganz ? "text-text-muted" : ""}`}
          >
            {ganz
              ? t("voiceCall.knowledge.notebook.wholeOn")
              : t("voiceCall.knowledge.notebook.wholeHint")}
          </p>
        </div>
        <div
          role="group"
          aria-label={t("voiceCall.knowledge.notebook.sectionsIn", {
            name: nb.name,
          })}
        >
          {ohneGruppe.map(zeile)}
          {gruppen.map((g) => {
            const liste = nb.sections.filter((a) => a.group === g);
            const gs = zustand(
              liste.map((a) => a.id),
              gewaehlt,
            );
            return (
              <React.Fragment key={g}>
                <div className="flex items-center min-h-11 -ms-1 text-[13px] font-semibold">
                  <Kaestchen
                    id={`${ids}-g-${nb.id}-${g}`}
                    label={t("voiceCall.knowledge.notebook.selectGroup", {
                      name: g,
                    })}
                    checked={gs.checked}
                    teil={gs.teil}
                    disabled={ganz}
                    onChange={(on) =>
                      onAuswahl(
                        setzeAbschnitte(
                          auswahl,
                          liste.map((a) => a.id),
                          on,
                        ),
                      )
                    }
                  />
                  <span>{g}</span>
                </div>
                <div
                  className="ms-[22px]"
                  role="group"
                  aria-label={t("voiceCall.knowledge.notebook.group", {
                    name: g,
                  })}
                >
                  {liste.map(zeile)}
                </div>
              </React.Fragment>
            );
          })}
        </div>
      </div>
    </div>
  );
};

/** Baum der Abschnitte samt Zustandsmeldungen (laden, kein Notizbuch, Version, Fehler). */
const Baum: React.FC<{ ids: string }> = ({ ids }) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const gespeichert = getSetting(SCHLUESSEL);
  // Während geschrieben wird, zeigt der Baum die zuletzt gewählte Auswahl; danach wieder den
  // gespeicherten Stand (bei einem Fehler also den alten).
  const { lokal, fehler: speicherFehler } = useSyncExternalStore(
    abonnieren,
    stand,
  );
  const geladen = useSettingsStore((s) => s.settings !== null);
  const roh = lokal ?? gespeichert ?? [];
  const auswahl = useMemo(() => auswahlAus(roh), [roh]);
  const [laden, setLaden] = useState<Laden>({ art: "laedt" });
  const [offen, setOffen] = useState<Record<string, boolean>>({});
  const start = useRef<Record<string, boolean> | null>(null);
  const aktiv = useRef(true);

  useEffect(() => {
    aktiv.current = true;
    commands
      .voiceNotebookOutline()
      .then((r) => {
        if (!aktiv.current) return;
        setLaden(
          r.status === "ok" ? { art: "ok", nbs: r.data } : ladeArt(r.error),
        );
      })
      .catch(() => aktiv.current && setLaden({ art: "fehler" }));
    return () => {
      aktiv.current = false;
    };
  }, []);

  const speichern = useCallback(
    (a: Auswahl) => {
      freigabeSpeichern(eintraegeAus(a, roh));
    },
    [roh],
  );

  // Ohne geladene Einstellungen gäbe es keine Grundlage: speichern würde sonst gespeicherte
  // Freigaben mit einer leeren Auswahl überschreiben.
  if (laden.art === "laedt" || !geladen)
    return (
      <div className="flex flex-col gap-2" aria-busy="true">
        <div className="h-11 rounded-lg bg-mid-gray/10" aria-hidden="true" />
        <div className="h-11 rounded-lg bg-mid-gray/10" aria-hidden="true" />
        <span role="status" className="text-xs text-text-muted">
          {t("voiceCall.knowledge.notebook.loading")}
        </span>
      </div>
    );
  if (laden.art !== "ok") {
    const warn = laden.art !== "kein";
    return (
      <div
        role="status"
        className={`rounded-lg px-3 py-2.5 text-xs ${warn ? "bg-amber-500/15" : "border border-mid-gray/30 bg-mid-gray/10"}`}
      >
        <strong className="block text-[13px] mb-0.5">
          {t(`voiceCall.knowledge.notebook.${laden.art}.title`)}
        </strong>
        {t(`voiceCall.knowledge.notebook.${laden.art}.text`)}
      </div>
    );
  }

  const nbs = laden.nbs;
  const summe = nbs.reduce((s, n) => s + frei(auswahl, n), 0);
  const alle = nbs.reduce((s, n) => s + n.sections.length, 0);
  // Einmal beim ersten Anzeigen; danach klappt nur der Knopf auf und zu.
  start.current ??= startOffen(nbs, auswahl);
  const anfang = start.current;
  const istOffen = (n: NotebookOutline) => offen[n.id] ?? !!anfang[n.id];

  return (
    <>
      <span role="status" className="text-xs text-text-muted">
        {summe === 0
          ? t("voiceCall.knowledge.notebook.none")
          : t("voiceCall.knowledge.notebook.summary", {
              count: summe,
              total: alle,
            })}
      </span>
      {speicherFehler && (
        <span role="alert" className="text-xs text-red-600 dark:text-red-400">
          {t("voiceCall.knowledge.notebook.saveError")}
        </span>
      )}
      {nbs.map((n) => (
        <NotizbuchKarte
          key={n.id}
          nb={n}
          ids={ids}
          auswahl={auswahl}
          offen={istOffen(n)}
          onOffen={(o) => setOffen((x) => ({ ...x, [n.id]: o }))}
          onAuswahl={speichern}
        />
      ))}
      <p className="m-0 text-xs text-text-muted">
        {t("voiceCall.knowledge.notebook.rule")}
      </p>
    </>
  );
};

export const NotebookAccess: React.FC<{
  on: boolean;
  onToggle: (on: boolean) => void;
}> = ({ on, onToggle }) => {
  const { t } = useTranslation();
  const ids = useId("notebook");
  return (
    <section
      aria-labelledby={`${ids}-titel`}
      className="flex flex-col gap-2 border-t border-mid-gray/20 pt-3"
    >
      <div className="flex flex-col">
        <Switch
          id={`${ids}-titel`}
          checked={on}
          onChange={(_e, d) => onToggle(d.checked)}
          label={t("voiceCall.knowledge.areas.notebook.label")}
          aria-describedby={`${ids}-hint`}
        />
        <span
          id={`${ids}-hint`}
          className="ps-[52px] -mt-1 text-xs text-text-muted"
        >
          {t("voiceCall.knowledge.areas.notebook.hint")}
        </span>
      </div>
      <div
        className="ms-6 flex flex-col gap-2"
        role="group"
        aria-label={t("voiceCall.knowledge.notebook.groupLabel")}
      >
        {on ? (
          <Baum ids={ids} />
        ) : (
          <span role="status" className="text-xs text-text-muted">
            {t("voiceCall.knowledge.notebook.off")}
          </span>
        )}
      </div>
    </section>
  );
};
