import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Plus, Upload, X } from "lucide-react";
import { useAssistentStore } from "@/stores/assistentStore";
import {
  useAssistentEditStore,
  type Eintrag,
  type Entwurf,
} from "@/stores/assistentEditStore";
import { useVoiceCallStore } from "@/stores/voiceCallStore";
import { VORLAGEN, kodiereProfil } from "@/components/voice-call/assistenten";
import type { Profil } from "@/components/voice-call/assistenten";
import type { EigenerAssistent } from "@/components/voice-call/profil/speicher";
import {
  gleicheAb,
  istGeaendert,
  kopieName,
  zuSchreiben,
} from "@/components/voice-call/profil/editor";
import type { Misserfolg } from "@/components/voice-call/profil/verwaltung";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { AbgewiesenAnsicht, VorlagenAnsicht } from "./Ansichten";
import { AssistentEditor } from "./AssistentEditor";
import { AssistentenListe } from "./AssistentenListe";
import { BestaetigenDialog, NameDialog } from "./Dialoge";
import { fehlerZeilen, grundText } from "./fehler";
import { useVerwaltung } from "./useVerwaltung";
import "./assistenten.css";

/** Was gerade als Rückfrage oder Namensdialog offen ist. */
type DialogZustand =
  | null
  | { art: "neu" }
  | { art: "kopie"; quelle: Profil; entwurfOffen: boolean }
  | { art: "umbenennen"; profil: Profil }
  | { art: "alsKopie" }
  | { art: "loeschen"; profil: Profil }
  | { art: "wechsel"; ziel: Eintrag | null; behalte: boolean }
  | { art: "verwerfen" };

const NAMENSDIALOGE: readonly NonNullable<DialogZustand>["art"][] = [
  "neu",
  "kopie",
  "umbenennen",
  "alsKopie",
];

const gleichEintrag = (a: Eintrag | null, b: Eintrag | null): boolean => {
  if (a === null || b === null) return a === b;
  if (a.art !== b.art) return false;
  return a.art === "abgewiesen"
    ? a.datei === (b as typeof a).datei
    : a.id === (b as { id: string }).id;
};

const entwurfAus = (e: EigenerAssistent): Entwurf => ({
  id: e.profil.id,
  basis: { profil: e.profil, text: e.text },
  profil: e.profil,
  konflikt: null,
});

/** Ein Misserfolg als ein Satz (der Grund und, wenn es sie gibt, die Felder), für den Dialog oder die Leiste des Editors. */
const satz = (
  t: ReturnType<typeof useTranslation>["t"],
  m: Misserfolg,
): string => [grundText(t, m.grund), ...fehlerZeilen(t, m.fehler)].join(" ");

/**
 * Die Seite „Assistenten“ (lva#556, G2 Teil 2): links die Liste (Vorlagen, eigene Assistenten, abgewiesene Dateien), rechts
 * die Ansicht des gewählten Eintrags, bei einem eigenen Assistenten der Editor. Auf dem Telefon steht eine Spalte: die Liste
 * oder die Ansicht, mit „Zurück“. Alles, was schreibt, läuft über die geprüfte Verwaltung; der Entwurf eines Editors liegt im
 * Store und überlebt den Wechsel in einen anderen Bereich. Kein Eintrag in der Seitenleiste: die Seite öffnet aus der
 * Auswahl des Assistenten im Gespräch („Assistenten verwalten …“).
 */
export const AssistentenPage: React.FC = () => {
  const { t } = useTranslation();
  const assistenten = useAssistentStore((s) => s.assistenten);
  const geladen = useAssistentStore((s) => s.geladen);
  const offen = useAssistentEditStore((s) => s.offen);
  const entwurf = useAssistentEditStore((s) => s.entwurf);
  const hatEntwurf = entwurf !== null;
  const gespraechLaeuft = useVoiceCallStore((s) => s.state !== "idle");
  const verw = useVerwaltung();
  const [dialog, setDialog] = useState<DialogZustand>(null);
  const [dialogFehler, setDialogFehler] = useState<string | null>(null);
  const [speicherFehler, setSpeicherFehler] = useState<string | null>(null);
  const importFeld = useRef<HTMLInputElement>(null);
  const detail = useRef<HTMLDivElement>(null);
  const liste = useRef<HTMLDivElement>(null);
  /** Der Text, den das eigene Speichern geschrieben hat: steht er in der Datei, ist das kein fremder Eingriff. */
  const eigeneSpeicherung = useRef<string | null>(null);
  /** Vorheriger Eintrag, um nur bei einem Wechsel den Fokus zu setzen (nicht beim ersten Anzeigen). */
  const vorher = useRef<Eintrag | null>(offen);

  const ladefehler = assistenten.ladefehler;
  const lesbar = geladen && ladefehler === null;
  const schreibt = lesbar && !verw.laeuft;

  // Beim Öffnen der Seite frisch lesen (außerhalb kann sich etwas geändert haben).
  useEffect(() => {
    void useAssistentStore.getState().laden(true);
  }, []);

  // Den Entwurf mit dem Speicher abgleichen: anlegen, wenn ein eigener Assistent offen ist; übernehmen, wenn die Datei
  // sich ändert, ohne dass der Entwurf Änderungen hat; sonst Konflikt. Ist der Speicher nicht lesbar, folgt daraus nichts
  // (kein Eintrag „verschwindet“, kein Entwurf geht verloren).
  useEffect(() => {
    if (!lesbar) return;
    const ed = useAssistentEditStore.getState();
    const { offen: o, entwurf: e } = ed;
    if (o === null) {
      if (e) ed.setzeEntwurf(null);
      return;
    }
    if (o.art === "vorlage") {
      if (e) ed.setzeEntwurf(null);
      if (!VORLAGEN.some((p) => p.id === o.id)) ed.oeffne(null);
      return;
    }
    if (o.art === "abgewiesen") {
      if (e) ed.setzeEntwurf(null);
      if (!assistenten.abgewiesen.some((x) => x.datei === o.datei))
        ed.oeffne(null);
      return;
    }
    const datei = assistenten.eigene.find((x) => x.profil.id === o.id);
    if (!e || e.id !== o.id) {
      if (datei) ed.setzeEntwurf(entwurfAus(datei));
      else ed.oeffne(null);
      return;
    }
    if (!datei) {
      if (e.konflikt !== "fehlt") ed.setzeKonflikt("fehlt");
      return;
    }
    const jetzt = { profil: datei.profil, text: datei.text };
    if (
      eigeneSpeicherung.current === datei.text &&
      datei.text !== e.basis.text
    ) {
      ed.setzeEntwurf({ ...e, basis: jetzt, konflikt: null });
      return;
    }
    const r = gleicheAb(e.basis, jetzt, e.profil);
    if (r.art === "gleich") {
      if (e.konflikt !== null) ed.setzeKonflikt(null);
    } else if (r.art === "konflikt") {
      if (e.konflikt !== "geaendert") ed.setzeKonflikt("geaendert");
    } else {
      ed.setzeEntwurf({
        ...e,
        basis: jetzt,
        profil: r.entwurf,
        konflikt: null,
      });
    }
  }, [assistenten, lesbar, offen, hatEntwurf]);

  // Schmal: wer einen Eintrag öffnet, landet in der Ansicht, wer zurückgeht, in der Liste (der Fokus folgt).
  useEffect(() => {
    const davor = vorher.current;
    vorher.current = offen;
    if (gleichEintrag(davor, offen)) return;
    if (!window.matchMedia("(max-width: 767px)").matches) return;
    (offen ? detail.current : liste.current)?.focus();
  }, [offen]);

  const geaendert =
    entwurf !== null && istGeaendert(entwurf.basis.profil, entwurf.profil);

  /**
   * Wechselt den offenen Eintrag. Die Meldung des letzten Vorgangs bleibt, wenn der Wechsel dessen Folge ist (etwa der
   * Hinweis zu einem Import, der den neuen Assistenten öffnet); wer selbst einen Eintrag wählt, schließt sie.
   */
  const wechsle = (ziel: Eintrag | null, behalte = false) => {
    const ed = useAssistentEditStore.getState();
    ed.setzeEntwurf(null);
    ed.oeffne(ziel);
    setSpeicherFehler(null);
    if (!behalte) verw.schliesseMeldung();
  };

  /** Öffnet einen Eintrag; ungespeicherte Änderungen werden nie still verworfen, davor fragt die Seite. */
  const oeffne = (ziel: Eintrag | null, behalte = false) => {
    const ed = useAssistentEditStore.getState();
    if (gleichEintrag(ziel, ed.offen)) return;
    const e = ed.entwurf;
    if (e && istGeaendert(e.basis.profil, e.profil)) {
      setDialog({ art: "wechsel", ziel, behalte });
      return;
    }
    wechsle(ziel, behalte);
  };

  const oeffneEigenen = (id: string) => oeffne({ art: "eigen", id }, true);

  const schliesseDialog = () => {
    if (dialogFehler) verw.schliesseMeldung();
    setDialogFehler(null);
    setDialog(null);
  };

  const speichern = async () => {
    const e = useAssistentEditStore.getState().entwurf;
    if (!e) return;
    setSpeicherFehler(null);
    // Geschrieben wird `zuSchreiben(entwurf)`: eine leere Liste der Aussprache fällt weg, es wird nie `[]` geschrieben.
    const profil = zuSchreiben(e.profil);
    try {
      eigeneSpeicherung.current = kodiereProfil(profil);
    } catch {
      eigeneSpeicherung.current = null;
    }
    const r = await verw.speichern(e.id, profil);
    if (!r.ok) {
      eigeneSpeicherung.current = null;
      setSpeicherFehler(satz(t, r));
    }
  };

  const verwenden = (id: string) => void verw.verwenden(id);

  const importiere = async (datei: File | undefined) => {
    if (!datei) return;
    const r = await verw.importieren(datei);
    if (r.ok && r.profil) oeffneEigenen(r.profil.id);
  };

  const loeschen = async (p: Profil) => {
    const r = await verw.loeschen(p.id, p.name);
    setDialog(null);
    if (!r.ok) return;
    const ed = useAssistentEditStore.getState();
    if (ed.offen?.art === "eigen" && ed.offen.id === p.id) {
      ed.setzeEntwurf(null);
      ed.oeffne(null);
    }
  };

  /** Ergebnis eines Vorgangs im Namensdialog: schließt bei Erfolg, sonst steht der Grund im Dialog. */
  const imDialog = async (
    vorgang: Promise<{ ok: true; profil?: Profil } | Misserfolg>,
    danach: (profil: Profil) => void,
  ): Promise<boolean> => {
    const r = await vorgang;
    if (!r.ok) {
      setDialogFehler(satz(t, r));
      return false;
    }
    setDialogFehler(null);
    if (r.profil) danach(r.profil);
    return true;
  };

  const eigenerAbgewiesen = (o: Extract<Eintrag, { art: "abgewiesen" }>) =>
    assistenten.abgewiesen.find((x) => x.datei === o.datei);

  const verwendenGrund = !lesbar
    ? t("assistenten.kopf.verwendenGesperrt")
    : gespraechLaeuft
      ? t("assistenten.kopf.verwendenGespraech")
      : null;

  const gewaehltId = assistenten.aktivId;

  // ---------------------------------------------------------------- Namensdialog je nach Vorgang
  const name = ((): {
    titel: string;
    hinweis?: string;
    bestaetigen: string;
    initial: string;
    los: (n: string) => Promise<boolean>;
  } | null => {
    if (dialog === null) return null;
    if (dialog.art === "neu")
      return {
        titel: t("assistenten.dialog.neuTitel"),
        hinweis: t("assistenten.dialog.neuHinweis"),
        bestaetigen: t("assistenten.dialog.neuBestaetigen"),
        initial: "",
        los: (n) => imDialog(verw.neu(n), (p) => oeffneEigenen(p.id)),
      };
    if (dialog.art === "kopie") {
      const quelle = dialog.quelle;
      return {
        titel: t("assistenten.dialog.kopieTitel", { name: quelle.name }),
        hinweis: dialog.entwurfOffen
          ? t("assistenten.dialog.kopieGespeichert")
          : t("assistenten.dialog.kopieHinweis"),
        bestaetigen: t("assistenten.dialog.kopieBestaetigen"),
        initial: kopieName(quelle.name, t("assistenten.kopie")),
        los: (n) =>
          imDialog(verw.kopieren(quelle, n), (p) => oeffneEigenen(p.id)),
      };
    }
    if (dialog.art === "umbenennen") {
      const p = dialog.profil;
      return {
        titel: t("assistenten.dialog.umbenennenTitel", { name: p.name }),
        hinweis: t("assistenten.dialog.umbenennenHinweis"),
        bestaetigen: t("assistenten.dialog.umbenennenBestaetigen"),
        initial: p.name,
        los: (n) => imDialog(verw.umbenennen(p.id, n), () => undefined),
      };
    }
    if (dialog.art === "alsKopie") {
      const quelle = entwurf?.profil;
      if (!quelle) return null;
      return {
        titel: t("assistenten.dialog.alsKopieTitel"),
        hinweis: t("assistenten.dialog.alsKopieHinweis"),
        bestaetigen: t("assistenten.dialog.kopieBestaetigen"),
        initial: kopieName(quelle.name, t("assistenten.kopie")),
        los: (n) =>
          imDialog(verw.kopieren(quelle, n), (p) => {
            // Der Entwurf lebt jetzt in der Kopie; der alte Eintrag bleibt, wie er in der Datei steht.
            wechsle({ art: "eigen", id: p.id }, true);
          }),
      };
    }
    return null;
  })();

  const ansicht = offen !== null ? "editor" : "liste";

  return (
    <PageShell
      title={t("assistenten.titel")}
      description={t("assistenten.seitenbeschreibung")}
      actions={
        <>
          <Button
            variant="secondary"
            size="sm"
            className="lv-asst-aktion"
            type="button"
            onClick={() => importFeld.current?.click()}
            disabledFocusable={!schreibt}
            data-testid="assistent-importieren"
          >
            <Upload width={14} height={14} aria-hidden="true" />
            {t("assistenten.aktion.importieren")}
          </Button>
          <Button
            variant="primary"
            size="sm"
            className="lv-asst-aktion"
            type="button"
            onClick={() => setDialog({ art: "neu" })}
            disabledFocusable={!schreibt}
            data-testid="assistent-neu"
          >
            <Plus width={14} height={14} aria-hidden="true" />
            {t("assistenten.aktion.neu")}
          </Button>
        </>
      }
    >
      <input
        ref={importFeld}
        type="file"
        accept=".json,application/json"
        hidden
        tabIndex={-1}
        aria-hidden="true"
        data-testid="assistent-import-datei"
        onChange={(e) => {
          const datei = e.target.files?.[0];
          e.target.value = "";
          void importiere(datei);
        }}
      />

      {geladen && ladefehler !== null && (
        <div
          className="lv-asst-meldung mb-3"
          data-art="fehler"
          role="alert"
          data-testid="assistenten-ladefehler"
        >
          <h3>{t("assistenten.ladefehler.titel")}</h3>
          <p className="m-0">
            {t(
              ladefehler === "unsafe_path"
                ? "voiceCall.assistant.loadUnsafe"
                : "voiceCall.assistant.loadFailed",
            )}
          </p>
        </div>
      )}

      {verw.meldung && (
        <div
          className="lv-asst-meldung mb-3"
          data-art={verw.meldung.art}
          role={verw.meldung.art === "fehler" ? "alert" : "status"}
          data-testid="assistent-meldung"
        >
          <div className="lv-asst-meldung__kopf">
            <h3>{verw.meldung.titel}</h3>
            <Button
              variant="ghost"
              size="sm"
              type="button"
              icon={<X width={14} height={14} aria-hidden="true" />}
              aria-label={t("assistenten.meldung.schliessen")}
              onClick={verw.schliesseMeldung}
              data-testid="assistent-meldung-schliessen"
            />
          </div>
          {verw.meldung.zeilen.length > 0 && (
            <ul>
              {verw.meldung.zeilen.map((z) => (
                <li key={z}>{z}</li>
              ))}
            </ul>
          )}
        </div>
      )}

      <div
        className="lv-asst"
        data-ansicht={ansicht}
        data-testid="assistenten-seite"
      >
        <div
          className="lv-asst__liste"
          ref={liste}
          tabIndex={-1}
          aria-busy={!geladen}
        >
          <AssistentenListe
            assistenten={assistenten}
            offen={offen}
            gewaehltId={gewaehltId}
            entwurfId={geaendert ? (entwurf?.id ?? null) : null}
            schreibt={schreibt}
            onOeffne={oeffne}
            onKopieren={(p) =>
              setDialog({
                art: "kopie",
                quelle: p,
                entwurfOffen: geaendert && entwurf?.id === p.id,
              })
            }
            onUmbenennen={(p) => setDialog({ art: "umbenennen", profil: p })}
            onExportieren={(p) => void verw.exportieren(p)}
            onLoeschen={(p) => setDialog({ art: "loeschen", profil: p })}
          />
        </div>
        <div className="lv-asst__detail" ref={detail} tabIndex={-1}>
          {!geladen ? (
            <p className="lv-asst-leer" role="status">
              {t("assistenten.laedt")}
            </p>
          ) : offen === null ? (
            <p className="lv-asst-leer" data-testid="assistent-leer">
              {t("assistenten.waehle")}
            </p>
          ) : offen.art === "vorlage" ? (
            (() => {
              const p = VORLAGEN.find((x) => x.id === offen.id);
              return p ? (
                <VorlagenAnsicht
                  profil={p}
                  gewaehlt={p.id === gewaehltId}
                  schreibt={schreibt}
                  verwendenGrund={verwendenGrund}
                  onZurueck={() => oeffne(null)}
                  onKopieren={() =>
                    setDialog({ art: "kopie", quelle: p, entwurfOffen: false })
                  }
                  onVerwenden={() => verwenden(p.id)}
                  onExportieren={() => void verw.exportieren(p)}
                />
              ) : null;
            })()
          ) : offen.art === "abgewiesen" ? (
            (() => {
              const x = eigenerAbgewiesen(offen);
              return x ? (
                <AbgewiesenAnsicht datei={x} onZurueck={() => oeffne(null)} />
              ) : null;
            })()
          ) : entwurf && entwurf.id === offen.id ? (
            <AssistentEditor
              eigener={entwurf.basis}
              entwurf={entwurf}
              gewaehlt={entwurf.id === gewaehltId}
              schreibt={lesbar}
              laeuft={verw.laeuft}
              speicherFehler={speicherFehler}
              onSpeichern={() => void speichern()}
              onVerwerfen={() => setDialog({ art: "verwerfen" })}
              onNeuLaden={() => {
                setSpeicherFehler(null);
                useAssistentEditStore.getState().setzeEntwurf(null);
              }}
              onAlsKopie={() => setDialog({ art: "alsKopie" })}
              onZurueck={() => oeffne(null)}
              onVerwenden={() => verwenden(entwurf.id)}
              onKopieren={() =>
                setDialog({
                  art: "kopie",
                  quelle: entwurf.basis.profil,
                  entwurfOffen: geaendert,
                })
              }
              onUmbenennen={() =>
                setDialog({ art: "umbenennen", profil: entwurf.basis.profil })
              }
              onExportieren={() => void verw.exportieren(entwurf.basis.profil)}
              onLoeschen={() =>
                setDialog({ art: "loeschen", profil: entwurf.basis.profil })
              }
            />
          ) : (
            <p className="lv-asst-leer" role="status">
              {t("assistenten.laedt")}
            </p>
          )}
        </div>
      </div>

      <NameDialog
        open={name !== null}
        onOpenChange={(open) => {
          if (open) return;
          // Hat der Vorgang gerade eine Rückfrage geöffnet (etwa nach dem Anlegen mit ungespeichertem Entwurf), bleibt sie.
          if (dialogFehler) verw.schliesseMeldung();
          setDialogFehler(null);
          setDialog((d) => (d && NAMENSDIALOGE.includes(d.art) ? null : d));
        }}
        titel={name?.titel ?? ""}
        hinweis={name?.hinweis}
        bestaetigen={name?.bestaetigen ?? ""}
        initial={name?.initial ?? ""}
        onBestaetigen={(n) => {
          setDialogFehler(null);
          return name ? name.los(n) : Promise.resolve(false);
        }}
        vorgangFehler={dialogFehler}
      />

      <BestaetigenDialog
        open={dialog?.art === "loeschen"}
        onOpenChange={(open) => {
          if (!open) schliesseDialog();
        }}
        titel={t("assistenten.loeschen.titel", {
          name: dialog?.art === "loeschen" ? dialog.profil.name : "",
        })}
        text={
          dialog?.art === "loeschen" ? (
            <>
              <p className="m-0">{t("assistenten.loeschen.text")}</p>
              {dialog.profil.id === gewaehltId && (
                <p className="m-0" data-testid="assistent-loeschen-gewaehlt">
                  {t("assistenten.loeschen.gewaehlt")}
                </p>
              )}
              {entwurf?.id === dialog.profil.id && geaendert && (
                <p className="m-0">{t("assistenten.loeschen.entwurf")}</p>
              )}
            </>
          ) : null
        }
        bestaetigen={t("assistenten.aktion.loeschen")}
        abbrechen={t("assistenten.dialog.abbrechen")}
        gefaehrlich
        onBestaetigen={async () => {
          if (dialog?.art === "loeschen") await loeschen(dialog.profil);
        }}
        testId="assistent-loeschen"
      />

      <BestaetigenDialog
        open={dialog?.art === "wechsel"}
        onOpenChange={(open) => {
          if (!open) schliesseDialog();
        }}
        titel={t("assistenten.wechsel.titel")}
        text={<p className="m-0">{t("assistenten.wechsel.text")}</p>}
        bestaetigen={t("assistenten.wechsel.verwerfen")}
        abbrechen={t("assistenten.wechsel.weiter")}
        gefaehrlich
        onBestaetigen={() => {
          if (dialog?.art === "wechsel") wechsle(dialog.ziel, dialog.behalte);
          setDialog(null);
        }}
        testId="assistent-wechsel"
      />

      <BestaetigenDialog
        open={dialog?.art === "verwerfen"}
        onOpenChange={(open) => {
          if (!open) schliesseDialog();
        }}
        titel={t("assistenten.verwerfen.titel")}
        text={<p className="m-0">{t("assistenten.verwerfen.text")}</p>}
        bestaetigen={t("assistenten.aktion.verwerfen")}
        abbrechen={t("assistenten.wechsel.weiter")}
        gefaehrlich
        onBestaetigen={() => {
          const ed = useAssistentEditStore.getState();
          if (ed.entwurf)
            ed.setzeEntwurf({
              ...ed.entwurf,
              profil: ed.entwurf.basis.profil,
              konflikt: null,
            });
          setSpeicherFehler(null);
          setDialog(null);
        }}
        testId="assistent-verwerfen"
      />
    </PageShell>
  );
};

export default AssistentenPage;
