import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { open as dateiDialog } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  FolderOpen,
  FolderTree,
  FolderPlus,
  FolderUp,
  Pencil,
  RefreshCw,
  Settings2,
  Trash2,
  Unlink,
  Upload,
} from "lucide-react";
import type {
  AblageArt,
  AblageDatei,
  AblageFassung,
  AblageFundstelle,
  AblageImportZeile,
  AblageKollisionswahl,
  AblageKorb,
  AblageOrdner,
  AblageSicht,
  AblageSuchtreffer,
  AblageSyncStand,
  AblageSyncZustand,
} from "@/bindings";
import { PageShell } from "../ui/PageShell";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import { Dialog } from "../ui/Dialog";
import { useFileDrop } from "../library/useFileDrop";
import { api } from "./api";
import {
  BestaetigungDialog,
  NameDialog,
  OrdnerEinstellungen,
  type EinstellungenAenderung,
} from "./Dialoge";
import {
  DateiTabelle,
  Details,
  Hochladeliste,
  KORB,
  KorbTabelle,
  Meldung,
  OrdnerBaum,
  type UploadZeile,
} from "./Teile";
import { SuchErgebnisse } from "./Suche";
import {
  dateiname,
  fehlerTeile,
  formatBytes,
  nachNameFiltern,
  pfadZu,
  unterordner,
} from "./logik";
import "./ablage.css";

const SCHMAL = "(max-width: 1023px)";

function useSchmal(): boolean {
  const [schmal, setSchmal] = useState(
    () => typeof window !== "undefined" && window.matchMedia(SCHMAL).matches,
  );
  useEffect(() => {
    const q = window.matchMedia(SCHMAL);
    const f = () => setSchmal(q.matches);
    f();
    q.addEventListener("change", f);
    return () => q.removeEventListener("change", f);
  }, []);
  return schmal;
}

type Dlg =
  | { art: "ordner-neu" }
  | { art: "ordner-umbenennen"; id: string }
  | { art: "datei-umbenennen"; id: string }
  | { art: "einstellungen"; id: string }
  | { art: "endgueltig"; eintrag: AblageKorb }
  | { art: "leeren" }
  | { art: "freigaben" }
  | { art: "loesen"; id: string };

let schluessel = 0;
const neuerSchluessel = () => `u${++schluessel}`;

/**
 * Seite „Ablage“ (lva#407 D1, lva#408 D2): Ordnerbaum, Dateiliste, Details, Papierkorb, Suche in
 * Namen und Inhalten (ohne KI). Abgleich (D4) und KI-Zugriff (D3) folgen. Namen und Inhalte gehen
 * nie ins Log.
 */
export const AblagePage: React.FC = () => {
  const { t, i18n } = useTranslation();
  const schmal = useSchmal();
  const [sicht, setSicht] = useState<AblageSicht | null>(null);
  // D4: Sync-Zustand je Eintrag; ohne Konto oder bei Fehler bleibt die Liste wie in D1.
  const [syncStand, setSyncStand] = useState<AblageSyncStand | null>(null);
  const [ladefehler, setLadefehler] = useState<string | null>(null);
  const [ansicht, setAnsicht] = useState<string | null>(null);
  const [dateiId, setDateiId] = useState<string | null>(null);
  const [fassungen, setFassungen] = useState<AblageFassung[]>([]);
  const [uploads, setUploads] = useState<UploadZeile[]>([]);
  const [uploadOffen, setUploadOffen] = useState(false);
  const [suche, setSuche] = useState("");
  const [inhaltTreffer, setInhaltTreffer] = useState<
    AblageSuchtreffer[] | null
  >(null);
  const [suchFehler, setSuchFehler] = useState(false);
  const [fundstelle, setFundstelle] = useState<AblageFundstelle | null>(null);
  const [busy, setBusy] = useState(false);
  const [dlg, setDlg] = useState<Dlg | null>(null);
  const letzteEinstellungen = useRef<AblageOrdner | null>(null);
  const [ordnerBlatt, setOrdnerBlatt] = useState(false);
  const einstellungenAusloeser = useRef<HTMLElement | null>(null);
  const letzterAusloeser = useRef<HTMLElement | null>(null);
  const zaehler = useRef(0);

  const fehlerText = useCallback(
    (code: string | null, text?: string) => {
      if (!code) return text ?? "";
      const suffix = code.replace(/^ablage\./, "");
      return t(`ablage.fehler.${suffix}`, {
        defaultValue: text ?? t("ablage.fehler.allgemein"),
      });
    },
    [t],
  );
  const melden = useCallback(
    (e: unknown) => {
      const { code, text } = fehlerTeile(
        e instanceof Error ? e.message : String(e),
      );
      toast.error(fehlerText(code, text));
    },
    [fehlerText],
  );

  const ladeZaehler = useRef(0);
  const neuLaden = useCallback(async () => {
    const nr = ++ladeZaehler.current;
    try {
      const neu = await api.stand();
      if (nr !== ladeZaehler.current) return;
      setSicht(neu);
      setLadefehler(null);
      const sync = await api.syncStand().catch(() => null);
      if (nr !== ladeZaehler.current) return;
      setSyncStand(sync);
    } catch (e) {
      if (nr !== ladeZaehler.current) return;
      const { code, text } = fehlerTeile(
        e instanceof Error ? e.message : String(e),
      );
      setLadefehler(fehlerText(code, text));
    }
  }, [fehlerText]);

  useEffect(() => {
    void neuLaden();
  }, [neuLaden]);

  // D2: Der Textarbeiter meldet jede fertige Textfassung; dann Stand (und Suche) neu laden.
  useEffect(() => {
    let aus: (() => void) | null = null;
    let aktiv = true;
    listen("ablage-text", () => void neuLaden())
      .then((f) => (aktiv ? (aus = f) : f()))
      .catch(() => {});
    return () => {
      aktiv = false;
      aus?.();
    };
  }, [neuLaden]);

  // D4: Nach jedem Abgleich der Ablage Stand und Sync-Zustand neu laden.
  useEffect(() => {
    let aus: (() => void) | null = null;
    let aktiv = true;
    listen("ablage-sync", () => void neuLaden())
      .then((f) => (aktiv ? (aus = f) : f()))
      .catch(() => {});
    return () => {
      aktiv = false;
      aus?.();
    };
  }, [neuLaden]);

  const syncJe = useMemo(() => {
    if (!syncStand?.verbunden) return undefined;
    return new Map<string, AblageSyncZustand>(
      syncStand.eintraege.map((e) => [e.id, e.zustand]),
    );
  }, [syncStand]);

  // Inhaltssuche (verzögert); läuft erneut, wenn sich der Stand ändert (neue Textfassungen).
  const anfrage = suche.trim();
  const sucheAktiv = anfrage.length >= 2 && ansicht !== KORB;
  // D6: Wechsel der Suchart (semantisch an/aus, Vektoren fertig) wiederholt die Inhaltssuche.
  const [suchart, setSuchart] = useState(0);
  const suchartGeaendert = useCallback(() => setSuchart((n) => n + 1), []);
  useEffect(() => {
    if (!sucheAktiv) {
      setInhaltTreffer(null);
      setSuchFehler(false);
      return;
    }
    let aktuell = true;
    const zeit = window.setTimeout(() => {
      api
        .suchen(anfrage)
        .then((r) => {
          if (!aktuell) return;
          setInhaltTreffer(r);
          setSuchFehler(false);
        })
        .catch(() => {
          if (!aktuell) return;
          setInhaltTreffer([]);
          setSuchFehler(true);
        });
    }, 250);
    return () => {
      aktuell = false;
      window.clearTimeout(zeit);
    };
  }, [anfrage, sucheAktiv, sicht, suchart]);

  // Gültige Auswahl: Ordner (oder Papierkorb) und Datei.
  useEffect(() => {
    if (!sicht) return;
    const verknuepft = sicht.verknuepft ?? [];
    const hatOrdner =
      ansicht === KORB ||
      sicht.ordner.some((o) => o.id === ansicht) ||
      verknuepft.some((v) => v.ordner.id === ansicht);
    if (!hatOrdner) {
      const erster =
        sicht.ordner.find((o) => !o.eltern && !o.wiederhergestellt) ??
        sicht.ordner[0];
      setAnsicht(erster?.id ?? null);
    }
    if (
      dateiId &&
      !sicht.dateien.some((d) => d.id === dateiId) &&
      !(sicht.verknuepft_dateien ?? []).some((d) => d.id === dateiId)
    ) {
      setDateiId(null);
      setFundstelle(null);
    }
  }, [sicht, ansicht, dateiId]);

  const dateiVerknuepft = useMemo(
    () => (sicht?.verknuepft_dateien ?? []).some((d) => d.id === dateiId),
    [sicht, dateiId],
  );
  useEffect(() => {
    if (!dateiId || dateiVerknuepft) {
      setFassungen([]);
      return;
    }
    let aktuell = true;
    api
      .fassungen(dateiId)
      .then((f) => aktuell && setFassungen(f))
      .catch(() => aktuell && setFassungen([]));
    return () => {
      aktuell = false;
    };
  }, [dateiId, dateiVerknuepft, sicht]);

  /** Aktion mit Fehlermeldung; lädt danach den Stand neu. Liefert, ob sie gelang. */
  const tue = useCallback(
    async (f: () => Promise<unknown>, erfolg?: string): Promise<boolean> => {
      setBusy(true);
      try {
        await f();
        if (erfolg) toast.success(erfolg);
        return true;
      } catch (e) {
        melden(e);
        return false;
      } finally {
        await neuLaden();
        setBusy(false);
      }
    },
    [melden, neuLaden],
  );

  const ordnerListe = useMemo(() => sicht?.ordner ?? [], [sicht]);
  // D7: verknüpfte Ordner (nur lesen, nur dieses Gerät) mit ihren Dateien.
  const verknuepftListe = useMemo(() => sicht?.verknuepft ?? [], [sicht]);
  const alleOrdner = useMemo(
    () => [...ordnerListe, ...verknuepftListe.map((v) => v.ordner)],
    [ordnerListe, verknuepftListe],
  );
  const alleDateien = useMemo(
    () => [...(sicht?.dateien ?? []), ...(sicht?.verknuepft_dateien ?? [])],
    [sicht],
  );
  const wiederhergestellt = sicht?.freigabe_wiederhergestellt ?? false;
  const aktuellerOrdner = alleOrdner.find((o) => o.id === ansicht) ?? null;
  const aktVerknuepft =
    verknuepftListe.find((v) => v.ordner.id === ansicht) ?? null;
  const nurLesen = !!aktVerknuepft;
  const imKorb = ansicht === KORB;
  const pfad = pfadZu(alleOrdner, aktuellerOrdner?.id ?? null);
  const datei = alleDateien.find((d) => d.id === dateiId) ?? null;
  const unter = useMemo(
    () =>
      nachNameFiltern(
        aktuellerOrdner ? unterordner(ordnerListe, aktuellerOrdner.id) : [],
        suche,
      ),
    [ordnerListe, aktuellerOrdner, suche],
  );
  const dateienHier = useMemo(
    () =>
      nachNameFiltern(
        alleDateien.filter((d) => d.ordner === ansicht),
        suche,
      ),
    [alleDateien, ansicht, suche],
  );
  const pfadText = (id: string) =>
    pfadZu(alleOrdner, id)
      .map((o) => o.name)
      .join(" › ");
  // Namenstreffer über alle Ordner (die Liste darunter filtert nur den aktuellen Ordner).
  const namensTreffer = useMemo(
    () => (sucheAktiv ? nachNameFiltern(alleDateien, anfrage) : []),
    [alleDateien, anfrage, sucheAktiv],
  );

  // --- Hochladen -----------------------------------------------------------------------
  const zeilenAus = (z: AblageImportZeile[]): UploadZeile[] =>
    z.map((x) => ({
      key: neuerSchluessel(),
      name: dateiname(x.quelle),
      quelle: x.quelle,
      ordner: x.ordner,
      stand: x.stand,
      code: x.code,
    }));

  const hochladen = useCallback(
    async (pfade: string[]) => {
      if (!aktuellerOrdner || aktuellerOrdner.verknuepft || pfade.length === 0)
        return;
      const ziel = aktuellerOrdner.id;
      setUploadOffen(true);
      setDateiId(null);
      for (const pfadEintrag of pfade) {
        const key = neuerSchluessel();
        setUploads((u) => [
          ...u,
          {
            key,
            name: dateiname(pfadEintrag),
            quelle: pfadEintrag,
            ordner: ziel,
            stand: "laeuft",
            code: null,
          },
        ]);
        try {
          const zeilen = zeilenAus(
            await api.importieren(ziel, pfadEintrag, null),
          );
          setUploads((u) => u.flatMap((x) => (x.key === key ? zeilen : [x])));
        } catch (e) {
          const { code } = fehlerTeile(
            e instanceof Error ? e.message : String(e),
          );
          setUploads((u) =>
            u.map((x) => (x.key === key ? { ...x, stand: "fehler", code } : x)),
          );
        }
        await neuLaden();
      }
    },
    [aktuellerOrdner, neuLaden],
  );

  const kollisionWaehlen = useCallback(
    async (z: UploadZeile, wahl: AblageKollisionswahl | null) => {
      if (!wahl || !z.ordner) {
        setUploads((u) =>
          u.map((x) =>
            x.key === z.key ? { ...x, stand: "uebersprungen", code: null } : x,
          ),
        );
        return;
      }
      setUploads((u) =>
        u.map((x) => (x.key === z.key ? { ...x, stand: "laeuft" } : x)),
      );
      try {
        const zeilen = zeilenAus(
          await api.importieren(z.ordner, z.quelle, wahl),
        );
        setUploads((u) => u.flatMap((x) => (x.key === z.key ? zeilen : [x])));
      } catch (e) {
        const { code } = fehlerTeile(
          e instanceof Error ? e.message : String(e),
        );
        setUploads((u) =>
          u.map((x) => (x.key === z.key ? { ...x, stand: "fehler", code } : x)),
        );
      }
      await neuLaden();
    },
    [neuLaden],
  );

  const waehlen = async (ordnerWahl: boolean) => {
    try {
      const antwort = await dateiDialog({
        multiple: true,
        directory: ordnerWahl,
        title: t(
          ordnerWahl
            ? "ablage.hochladen.ordnerWaehlen"
            : "ablage.hochladen.dateienWaehlen",
        ),
      });
      if (!antwort) return;
      await hochladen(Array.isArray(antwort) ? antwort : [antwort]);
    } catch (e) {
      melden(e);
    }
  };

  const { over } = useFileDrop("[data-ablage-drop]", (pfade) => {
    if (!imKorb && !nurLesen && !dlg) void hochladen(pfade);
  });

  // --- Aktionen ------------------------------------------------------------------------
  const inKorb = (art: AblageArt, id: string) =>
    tue(() => api.inPapierkorb(art, id), t("ablage.toast.imPapierkorb"));
  const wiederherstellen = (e: AblageKorb) =>
    tue(
      () => api.wiederherstellen(e.art, e.id),
      t("ablage.toast.wiederhergestellt"),
    );

  const einstellungenSpeichern = async (a: EinstellungenAenderung) => {
    if (dlg?.art !== "einstellungen") return;
    const id = dlg.id;
    const e = a.einstellungen;
    const geaendert =
      e.datenklasse != null || e.nur_geraet != null || e.zielordner_ki != null;
    const gut = await tue(async () => {
      if (geaendert) await api.ordnerEinstellungen(id, e);
      if (a.ki !== undefined) await api.kiSetzen(id, a.ki);
    }, t("ablage.toast.gespeichert"));
    if (gut) schliesseEinstellungen();
  };

  // --- D7: verknüpfte Ordner ------------------------------------------------------------
  const verknuepfen = async () => {
    let pfadWahl: string | null = null;
    try {
      const antwort = await dateiDialog({
        multiple: false,
        directory: true,
        title: t("ablage.verknuepft.waehlen"),
      });
      if (!antwort) return;
      pfadWahl = Array.isArray(antwort) ? (antwort[0] ?? null) : antwort;
    } catch (e) {
      melden(e);
      return;
    }
    if (!pfadWahl) return;
    const quelle = pfadWahl;
    let neu: string | null = null;
    const gut = await tue(async () => {
      neu = await api.verknuepfen(quelle);
    }, t("ablage.verknuepft.toastVerknuepft"));
    if (gut && neu) wahl(neu);
  };
  const neuEinlesen = (id: string) =>
    void tue(
      () => api.verknuepftAbtasten(id),
      t("ablage.verknuepft.toastEingelesen"),
    );

  const gefundenAufnehmen = (dat: string) => {
    if (!aktuellerOrdner) return;
    void tue(
      () => api.gefundeneAufnehmen(dat, aktuellerOrdner.id),
      t("ablage.toast.aufgenommen"),
    );
  };

  const neuAufbauen = () =>
    void tue(async () => {
      const n = await api.bestandNeuAufbauen();
      toast.success(t("ablage.lage.aufgebaut", { count: n }));
    });

  // --- Ansicht -------------------------------------------------------------------------
  const korbAnzahl = sicht?.papierkorb.length ?? 0;
  const oeffneEinstellungen = (id: string) => {
    const aktiv = document.activeElement;
    einstellungenAusloeser.current =
      aktiv instanceof HTMLElement && aktiv !== document.body ? aktiv : null;
    setDlg({ art: "einstellungen", id });
  };
  const schliesseEinstellungen = () => {
    setDlg(null);
    const ziel = einstellungenAusloeser.current;
    einstellungenAusloeser.current = null;
    if (!ziel) return;
    // Der Dialog gibt den Fokus nicht in jedem Fall zurück: nach dem Schließen nachziehen.
    setTimeout(() => {
      const a = document.activeElement;
      if ((!a || a === document.body) && ziel.isConnected) ziel.focus();
    }, 50);
  };
  const schliesseDetails = () => {
    setDateiId(null);
    const ziel = letzterAusloeser.current;
    letzterAusloeser.current = null;
    if (ziel) requestAnimationFrame(() => ziel.isConnected && ziel.focus());
  };
  const wahl = (id: string) => {
    setAnsicht(id);
    setDateiId(null);
    setFundstelle(null);
    setSuche("");
  };

  /** Treffer öffnen: Ordner der Datei, Details mit Fundstelle; die Suche bleibt stehen. */
  const trefferOeffnen = (
    dat: string,
    ordner: string,
    f: AblageFundstelle | null,
    ausloeser: HTMLElement,
  ) => {
    letzterAusloeser.current = ausloeser;
    setUploadOffen(false);
    setAnsicht(ordner);
    setDateiId(dat);
    setFundstelle(f);
  };

  const detailInhalt = (id: string) => {
    if (uploadOffen) {
      return (
        <Hochladeliste
          zeilen={uploads}
          ordnerName={aktuellerOrdner?.name ?? ""}
          ueberschriftId={id}
          onWahl={kollisionWaehlen}
          onSchliessen={() => {
            setUploadOffen(false);
            setUploads([]);
          }}
          fehlerText={(code) => fehlerText(code)}
        />
      );
    }
    if (!datei) return null;
    return (
      <Details
        datei={datei}
        pfad={pfadText(datei.ordner)}
        fassungen={fassungen}
        busy={busy}
        ueberschriftId={id}
        onSchliessen={schliesseDetails}
        onUmbenennen={() => setDlg({ art: "datei-umbenennen", id: datei.id })}
        onPapierkorb={() => void inKorb("datei", datei.id)}
        onKi={() => oeffneEinstellungen(datei.ordner)}
        fundstelle={fundstelle}
        anfrage={anfrage}
      />
    );
  };
  const seiteOffen = uploadOffen || !!datei;

  if (!sicht) {
    return (
      <PageShell
        title={t("ablage.titel")}
        description={t("ablage.beschreibung")}
        fill
      >
        <div className="ablage-leer" role={ladefehler ? "alert" : "status"}>
          {ladefehler ?? t("ablage.laedt")}
        </div>
      </PageShell>
    );
  }

  if (sicht.lage) {
    const kannAufbauen = sicht.lage === "fehlt" || sicht.lage === "defekt";
    return (
      <PageShell
        title={t("ablage.titel")}
        description={t("ablage.beschreibung")}
        fill
      >
        <div className="ablage-leer" role="alert" data-testid="ablage-lage">
          <div>
            <div className="ablage-leer__gross">{t("ablage.lage.titel")}</div>
            <p>{t(`ablage.lage.${sicht.lage}`)}</p>
            {kannAufbauen && (
              <Button
                variant="primary"
                onClick={neuAufbauen}
                disabled={busy}
                data-testid="ablage-neu-aufbauen"
              >
                {t("ablage.lage.neuAufbauen")}
              </Button>
            )}
          </div>
        </div>
      </PageShell>
    );
  }

  const kopf = (
    <div className="ablage-head">
      <div className="ablage-crumb">
        {(pfad.length > 1 || imKorb) && (
          <IconButton
            icon={<ArrowLeft size={20} aria-hidden="true" />}
            label={t("ablage.zurueck")}
            size="md"
            onClick={() =>
              wahl(
                pfad.length > 1
                  ? pfad[pfad.length - 2].id
                  : (ordnerListe.find((o) => !o.eltern)?.id ?? ""),
              )
            }
            data-testid="ablage-zurueck"
          />
        )}
        {pfad.length > 1 && (
          <>
            <span className="ablage-nur-breit">
              {pfad[pfad.length - 2].name}
            </span>
            <span
              className="ablage-crumb__sep ablage-nur-breit"
              aria-hidden="true"
            >
              ›
            </span>
          </>
        )}
        <h2 data-testid="ablage-ordnername">
          {imKorb
            ? t("ablage.papierkorb.titel")
            : (aktuellerOrdner?.name ?? "")}
        </h2>
      </div>
      <div className="ablage-head__tools">
        <IconButton
          icon={<FolderTree size={20} aria-hidden="true" />}
          label={t("ablage.baum.knopf")}
          size="md"
          className="ablage-nur-schmal"
          aria-haspopup="dialog"
          onClick={() => setOrdnerBlatt(true)}
          data-testid="ablage-ordner-waehlen"
        />
        {imKorb ? (
          <Button
            variant="danger-ghost"
            disabled={busy || korbAnzahl === 0}
            onClick={() => setDlg({ art: "leeren" })}
            data-testid="ablage-leeren"
          >
            {t("ablage.papierkorb.leeren")}
          </Button>
        ) : aktVerknuepft ? (
          <>
            <IconButton
              icon={<RefreshCw size={20} aria-hidden="true" />}
              label={t("ablage.verknuepft.neuEinlesen")}
              size="md"
              disabled={busy}
              onClick={() => neuEinlesen(aktVerknuepft.ordner.id)}
              data-testid="ablage-verknuepft-einlesen"
            />
            <IconButton
              icon={<Settings2 size={20} aria-hidden="true" />}
              label={t("ablage.einstellungen.knopf")}
              size="md"
              aria-haspopup="dialog"
              onClick={() => oeffneEinstellungen(aktVerknuepft.ordner.id)}
              data-testid="ablage-ordner-einstellungen"
            />
            <IconButton
              icon={<Unlink size={20} aria-hidden="true" />}
              label={t("ablage.verknuepft.loesen")}
              size="md"
              aria-haspopup="dialog"
              disabled={busy}
              onClick={() =>
                setDlg({ art: "loesen", id: aktVerknuepft.ordner.id })
              }
              data-testid="ablage-verknuepfung-loesen"
            />
          </>
        ) : (
          aktuellerOrdner && (
            <>
              <Button
                variant="primary"
                icon={<Upload size={20} aria-hidden="true" />}
                onClick={() => void waehlen(false)}
                aria-haspopup="dialog"
                aria-label={t("ablage.hochladen.dateien")}
                disabled={busy}
                data-testid="ablage-hochladen"
              >
                <span className="ablage-nur-breit">
                  {t("ablage.hochladen.knopf")}
                </span>
              </Button>
              <IconButton
                icon={<FolderUp size={20} aria-hidden="true" />}
                label={t("ablage.hochladen.ordner")}
                size="md"
                onClick={() => void waehlen(true)}
                disabled={busy}
                data-testid="ablage-ordner-hochladen"
              />
              <IconButton
                icon={<FolderPlus size={20} aria-hidden="true" />}
                label={t("ablage.ordner.neu")}
                size="md"
                className="ablage-nur-schmal"
                onClick={() => setDlg({ art: "ordner-neu" })}
              />
              <IconButton
                icon={<Pencil size={20} aria-hidden="true" />}
                label={t("ablage.ordner.umbenennen")}
                size="md"
                onClick={() =>
                  setDlg({ art: "ordner-umbenennen", id: aktuellerOrdner.id })
                }
                data-testid="ablage-ordner-umbenennen"
              />
              <IconButton
                icon={<Trash2 size={20} aria-hidden="true" />}
                label={t("ablage.ordner.papierkorb")}
                size="md"
                disabled={busy}
                onClick={() => void inKorb("ordner", aktuellerOrdner.id)}
                data-testid="ablage-ordner-papierkorb"
              />
              <IconButton
                icon={<Settings2 size={20} aria-hidden="true" />}
                label={t("ablage.einstellungen.knopf")}
                size="md"
                aria-haspopup="dialog"
                onClick={() => oeffneEinstellungen(aktuellerOrdner.id)}
                data-testid="ablage-ordner-einstellungen"
              />
            </>
          )
        )}
        <IconButton
          icon={<Trash2 size={20} aria-hidden="true" />}
          label={t("ablage.papierkorb.oeffnen")}
          size="md"
          className="ablage-nur-schmal"
          onClick={() => wahl(KORB)}
        />
      </div>
    </div>
  );

  const meldungen = (
    <>
      {aktVerknuepft && (
        <Meldung testid="ablage-verknuepft-hinweis">
          <p>{t("ablage.verknuepft.hinweis")}</p>
        </Meldung>
      )}
      {aktVerknuepft && !aktVerknuepft.erreichbar && (
        <Meldung warn testid="ablage-verknuepft-unerreichbar">
          <p>{t("ablage.verknuepft.nichtErreichbarText")}</p>
        </Meldung>
      )}
      {aktVerknuepft &&
        (aktVerknuepft.ausgelassen > 0 || aktVerknuepft.zu_viele) && (
          <Meldung testid="ablage-verknuepft-ausgelassen">
            {aktVerknuepft.ausgelassen > 0 && (
              <p>
                {t("ablage.verknuepft.ausgelassen", {
                  count: aktVerknuepft.ausgelassen,
                })}
              </p>
            )}
            {aktVerknuepft.zu_viele && <p>{t("ablage.verknuepft.zuViele")}</p>}
          </Meldung>
        )}
      {sicht.freigabe_unklar && (
        <Meldung warn testid="ablage-freigabe-unklar">
          <p>{t("ablage.ki.unklarMeldung")}</p>
          {sicht.freigabe_unklar === "defekt" ? (
            <Button
              onClick={() => setDlg({ art: "freigaben" })}
              data-testid="ablage-freigaben-neu"
            >
              {t("ablage.ki.neuFestlegen")}
            </Button>
          ) : (
            <p>{t(`ablage.ki.unklar.${sicht.freigabe_unklar}`)}</p>
          )}
        </Meldung>
      )}
      {sicht.freigabe_wiederhergestellt && (
        <Meldung warn testid="ablage-freigabe-wiederhergestellt">
          <p>{t("ablage.ki.wiederhergestelltMeldung")}</p>
          <Button
            onClick={() => setDlg({ art: "freigaben" })}
            data-testid="ablage-freigaben-bestaetigen"
          >
            {t("ablage.ki.bestaetigen")}
          </Button>
        </Meldung>
      )}
      {sicht.gefunden.length > 0 && !imKorb && !nurLesen && (
        <Meldung warn testid="ablage-gefunden">
          <p>{t("ablage.gefunden.titel", { count: sicht.gefunden.length })}</p>
          {sicht.gefunden.map((g, i) => (
            <div
              key={`${g.dat}-${i}`}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                flexWrap: "wrap",
              }}
            >
              <span>
                {g.typ ? g.typ.toUpperCase() : t("ablage.gefunden.ohneTyp")} ·{" "}
                {formatBytes(g.groesse, i18n.language)}
              </span>
              <Button
                disabled={busy || !aktuellerOrdner || nurLesen}
                onClick={() => gefundenAufnehmen(g.dat)}
                data-testid="ablage-gefunden-aufnehmen"
              >
                {t("ablage.gefunden.aufnehmen")}
              </Button>
            </div>
          ))}
        </Meldung>
      )}
    </>
  );

  const hauptInhalt = imKorb ? (
    sicht.papierkorb.length === 0 ? (
      <div className="ablage-leer" role="status">
        <div>{t("ablage.papierkorb.leer")}</div>
      </div>
    ) : (
      <>
        <div className="ablage-hinweis">{t("ablage.papierkorb.hinweis")}</div>
        <KorbTabelle
          eintraege={sicht.papierkorb}
          busy={busy}
          onWiederherstellen={(e) => void wiederherstellen(e)}
          onLoeschen={(e) => setDlg({ art: "endgueltig", eintrag: e })}
        />
      </>
    )
  ) : unter.length === 0 && dateienHier.length === 0 ? (
    sucheAktiv &&
    (namensTreffer.length > 0 || (inhaltTreffer?.length ?? 0) > 0) ? null : (
      <div className="ablage-leer" data-testid="ablage-leer">
        <div>
          <FolderOpen
            size={48}
            aria-hidden="true"
            style={{ color: "var(--color-logo-primary)" }}
          />
          <div className="ablage-leer__gross">
            {suche
              ? t("ablage.leer.keineTreffer")
              : nurLesen
                ? t("ablage.verknuepft.leer")
                : t("ablage.leer.titel")}
          </div>
          {!suche && !nurLesen && (
            <>
              {t("ablage.leer.formate")}
              <br />
              {t("ablage.leer.lokal")}
              <div style={{ marginTop: 16 }}>
                <Button
                  variant="primary"
                  icon={<Upload size={20} aria-hidden="true" />}
                  onClick={() => void waehlen(false)}
                  aria-haspopup="dialog"
                  disabled={busy || !aktuellerOrdner}
                >
                  {t("ablage.hochladen.auswaehlen")}
                </Button>
              </div>
            </>
          )}
        </div>
      </div>
    )
  ) : (
    <DateiTabelle
      titel={aktuellerOrdner?.name ?? ""}
      unter={unter}
      dateien={dateienHier}
      gewaehlt={dateiId}
      sync={syncJe}
      onOrdner={wahl}
      onDatei={(id, ausloeser) => {
        letzterAusloeser.current = ausloeser;
        setUploadOffen(false);
        setDateiId(id);
        setFundstelle(null);
      }}
    />
  );

  const dlgOrdner = (id: string): AblageOrdner | null =>
    alleOrdner.find((o) => o.id === id) ?? null;
  // Beim Schließen bleibt der Dialog noch gemountet, damit der Fokus zum Auslöser zurückkehrt.
  const aktEinstellungen =
    dlg?.art === "einstellungen" ? dlgOrdner(dlg.id) : null;
  if (aktEinstellungen) letzteEinstellungen.current = aktEinstellungen;
  const einstellungenOrdner = aktEinstellungen ?? letzteEinstellungen.current;
  const dlgDatei = (id: string): AblageDatei | null =>
    alleDateien.find((d) => d.id === id) ?? null;

  return (
    <PageShell
      title={t("ablage.titel")}
      description={t("ablage.beschreibung")}
      search={{
        value: suche,
        onChange: setSuche,
        placeholder: t("ablage.suche"),
      }}
      fill
    >
      <div
        className={`ablage${seiteOffen && !schmal ? "" : " ablage--zwei"}`}
        data-testid="ablage-seite"
      >
        <OrdnerBaum
          ordner={ordnerListe}
          gewaehlt={ansicht}
          korbAnzahl={korbAnzahl}
          onWahl={wahl}
          onNeu={() => setDlg({ art: "ordner-neu" })}
          verknuepft={verknuepftListe}
          onVerknuepfen={() => void verknuepfen()}
        />
        <section
          className="ablage-col"
          aria-label={
            imKorb ? t("ablage.papierkorb.titel") : t("ablage.liste.label")
          }
          data-ablage-drop
        >
          {kopf}
          {meldungen}
          {sucheAktiv && (
            <SuchErgebnisse
              anfrage={anfrage}
              namen={namensTreffer}
              inhalt={inhaltTreffer}
              fehler={suchFehler}
              pfadText={pfadText}
              onSuchartGeaendert={suchartGeaendert}
              onOeffnen={trefferOeffnen}
            />
          )}
          {hauptInhalt}
          {over && !imKorb && !nurLesen && aktuellerOrdner && (
            <div
              className="ablage-drop"
              aria-hidden="true"
              data-testid="ablage-drop"
            >
              <div>
                <Upload size={40} aria-hidden="true" />
                <br />
                {t("ablage.drop.titel", { name: aktuellerOrdner.name })}
                <br />
                <span className="ablage-mut" style={{ fontWeight: 400 }}>
                  {t("ablage.drop.hinweis")}
                </span>
              </div>
            </div>
          )}
          <div className="ablage-fuss">
            <span className="ablage-fuss-belegt">
              {t("ablage.fuss.belegt", {
                size: formatBytes(sicht.belegt, i18n.language),
              })}
            </span>
            {syncStand?.verbunden ? (
              <>
                <span
                  className={
                    syncStand.kontingent_voll
                      ? "ablage-fuss-voll"
                      : "ablage-nur-breit"
                  }
                  role="status"
                  data-testid="ablage-fuss-cloud"
                >
                  {syncStand.kontingent_voll
                    ? t("ablage.sync.voll")
                    : syncStand.belegt != null && syncStand.kontingent != null
                      ? t("ablage.sync.cloud", {
                          used: formatBytes(syncStand.belegt, i18n.language),
                          quota: formatBytes(
                            syncStand.kontingent,
                            i18n.language,
                          ),
                        })
                      : null}
                </span>
                {syncStand.fehler && (
                  <span
                    className="ablage-fuss-voll"
                    role="status"
                    title={syncStand.fehler}
                    data-testid="ablage-fuss-fehler"
                  >
                    {t("ablage.sync.unvollstaendig")}
                  </span>
                )}
              </>
            ) : (
              <span className="ablage-nur-breit">{t("ablage.fuss.lokal")}</span>
            )}
          </div>
        </section>
        {seiteOffen && !schmal && (
          <section
            className="ablage-col ablage-seite"
            aria-labelledby="ablage-seite-titel"
            data-testid="ablage-seite-details"
          >
            {detailInhalt("ablage-seite-titel")}
          </section>
        )}
      </div>

      {schmal && (
        <Dialog
          open={seiteOffen}
          onOpenChange={(o) => {
            if (o) return;
            if (uploadOffen) {
              setUploadOffen(false);
              setUploads([]);
            } else setDateiId(null);
          }}
          title={
            uploadOffen
              ? t("ablage.hochladen.titel", { count: uploads.length })
              : (datei?.name ?? "")
          }
          closeLabel={t("ablage.details.schliessen")}
          className="ablage-sheet"
          contentFades={false}
        >
          <div className="ablage" style={{ display: "block", height: "auto" }}>
            {detailInhalt("ablage-blatt-titel")}
          </div>
        </Dialog>
      )}

      {schmal && (
        <Dialog
          open={ordnerBlatt}
          onOpenChange={setOrdnerBlatt}
          title={t("ablage.baum.label")}
          closeLabel={t("ablage.details.schliessen")}
          className="ablage-sheet"
          contentFades={false}
        >
          <div
            className="ablage ablage-ordnerblatt"
            style={{ display: "block", height: "auto" }}
          >
            <OrdnerBaum
              ordner={ordnerListe}
              gewaehlt={ansicht}
              korbAnzahl={korbAnzahl}
              onWahl={(id) => {
                wahl(id);
                setOrdnerBlatt(false);
              }}
              onNeu={() => {
                setOrdnerBlatt(false);
                setDlg({ art: "ordner-neu" });
              }}
              verknuepft={verknuepftListe}
              onVerknuepfen={() => {
                setOrdnerBlatt(false);
                void verknuepfen();
              }}
            />
          </div>
        </Dialog>
      )}

      <NameDialog
        open={dlg?.art === "ordner-neu"}
        titel={t("ablage.ordner.neu")}
        beschriftung={t("ablage.ordner.name")}
        start=""
        aktion={t("ablage.ordner.anlegen")}
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async (name) => {
          let neu: string | null = null;
          const gut = await tue(async () => {
            neu = await api.ordnerAnlegen(
              imKorb || nurLesen ? null : (aktuellerOrdner?.id ?? null),
              name,
            );
          });
          if (gut) {
            setDlg(null);
            if (neu) wahl(neu);
          }
        }}
      />
      <NameDialog
        open={dlg?.art === "ordner-umbenennen"}
        titel={t("ablage.ordner.umbenennen")}
        beschriftung={t("ablage.ordner.name")}
        start={
          dlg?.art === "ordner-umbenennen"
            ? (dlgOrdner(dlg.id)?.name ?? "")
            : ""
        }
        aktion={t("ablage.speichern")}
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async (name) => {
          if (dlg?.art !== "ordner-umbenennen") return;
          const id = dlg.id;
          if (await tue(() => api.ordnerUmbenennen(id, name))) setDlg(null);
        }}
      />
      <NameDialog
        open={dlg?.art === "datei-umbenennen"}
        titel={t("ablage.details.umbenennen")}
        beschriftung={t("ablage.liste.name")}
        start={
          dlg?.art === "datei-umbenennen" ? (dlgDatei(dlg.id)?.name ?? "") : ""
        }
        aktion={t("ablage.speichern")}
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async (name) => {
          if (dlg?.art !== "datei-umbenennen") return;
          const id = dlg.id;
          if (await tue(() => api.dateiUmbenennen(id, name))) setDlg(null);
        }}
      />
      <OrdnerEinstellungen
        open={dlg?.art === "einstellungen"}
        ordner={einstellungenOrdner}
        alle={ordnerListe}
        busy={busy}
        onSpeichern={(a) => void einstellungenSpeichern(a)}
        onClose={schliesseEinstellungen}
      />
      <BestaetigungDialog
        open={dlg?.art === "endgueltig"}
        titel={t("ablage.papierkorb.endgueltigTitel", {
          name: dlg?.art === "endgueltig" ? dlg.eintrag.name : "",
        })}
        text={t("ablage.papierkorb.endgueltigText")}
        aktion={t("ablage.papierkorb.endgueltig")}
        gefahr
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async () => {
          if (dlg?.art !== "endgueltig") return;
          const e = dlg.eintrag;
          if (
            await tue(
              () => api.endgueltigLoeschen(e.art, e.id),
              t("ablage.toast.geloescht"),
            )
          )
            setDlg(null);
        }}
      />
      <BestaetigungDialog
        open={dlg?.art === "leeren"}
        titel={t("ablage.papierkorb.leerenTitel", { count: korbAnzahl })}
        text={t("ablage.papierkorb.endgueltigText")}
        aktion={t("ablage.papierkorb.leerenAktion")}
        gefahr
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async () => {
          if (
            await tue(() => api.papierkorbLeeren(), t("ablage.toast.geleert"))
          )
            setDlg(null);
        }}
      />
      <BestaetigungDialog
        open={dlg?.art === "freigaben"}
        titel={t(
          wiederhergestellt
            ? "ablage.ki.bestaetigenTitel"
            : "ablage.ki.neuFestlegenTitel",
        )}
        text={
          <>
            <p>
              {t(
                wiederhergestellt
                  ? "ablage.ki.bestaetigenText"
                  : "ablage.ki.neuFestlegenText",
              )}
            </p>
            <ul style={{ margin: "8px 0 0 18px", listStyle: "disc" }}>
              {(wiederhergestellt
                ? alleOrdner.filter((o) => o.ki.vorgabe)
                : alleOrdner
              ).map((o) => (
                <li key={o.id}>
                  {pfadText(o.id)} — {t("ablage.ki.stufe.aus")}
                </li>
              ))}
            </ul>
          </>
        }
        aktion={t(
          wiederhergestellt
            ? "ablage.ki.bestaetigenAktion"
            : "ablage.ki.neuFestlegenAktion",
        )}
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async () => {
          if (
            await tue(
              () => api.freigabenNeuFestlegen(),
              t("ablage.toast.gespeichert"),
            )
          )
            setDlg(null);
        }}
      />
      <BestaetigungDialog
        open={dlg?.art === "loesen"}
        titel={t("ablage.verknuepft.loesenTitel", {
          name: dlg?.art === "loesen" ? (dlgOrdner(dlg.id)?.name ?? "") : "",
        })}
        text={t("ablage.verknuepft.loesenText")}
        aktion={t("ablage.verknuepft.loesenAktion")}
        busy={busy}
        onClose={() => setDlg(null)}
        onOk={async () => {
          if (dlg?.art !== "loesen") return;
          const id = dlg.id;
          if (
            await tue(
              () => api.verknuepfungLoesen(id),
              t("ablage.verknuepft.toastGeloest"),
            )
          )
            setDlg(null);
        }}
      />
    </PageShell>
  );
};
