import React, { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Check,
  ChevronDown,
  ChevronRight,
  Clock,
  Cloud,
  CloudCheck,
  Eye,
  FileText,
  Folder,
  FolderSymlink,
  Link as LinkSymbol,
  Monitor,
  PauseCircle,
  Plus,
  RefreshCw,
  RotateCcw,
  Sparkles,
  Trash2,
  X,
  Lock,
  CircleDashed,
} from "lucide-react";
import type {
  AblageDatei,
  AblageFassung,
  AblageFundstelle,
  AblageImportStand,
  AblageKi,
  AblageKorb,
  AblageOrdner,
  AblageKollisionswahl,
  AblageSyncZustand,
  AblageVerknuepft,
} from "@/bindings";
import { IconButton } from "../ui/IconButton";
import { Button } from "../ui/Button";
import { Auszug, fundstelleText } from "./Suche";
import {
  baumBauen,
  cloudAlle,
  dateiArt,
  dateiKuerzel,
  formatBytes,
  letztesGlied,
  unterAnsicht,
  type BaumKnoten,
  type UnterKnoten,
} from "./logik";

export const KORB = "korb";

const Chip: React.FC<{
  art?: "ki" | "sperre" | "ok" | "cloud" | "warn";
  title?: string;
  children: React.ReactNode;
}> = ({ art, title, children }) => (
  <span
    className={`ablage-chip${art ? ` ablage-chip--${art}` : ""}`}
    title={title}
  >
    {children}
  </span>
);

/** Sync-Zustand eines Eintrags (D4): Zeichen, auf breiten Fenstern mit Text. */
export const SyncZeichen: React.FC<{ zustand: AblageSyncZustand }> = ({
  zustand,
}) => {
  const { t } = useTranslation();
  const Zeichen = {
    synchronisiert: Check,
    ausstehend: Clock,
    konflikt: AlertTriangle,
    wartet_auf_text: FileText,
    nur_geraet: Monitor,
    angehalten: PauseCircle,
  }[zustand];
  const text = t(`ablage.sync.${zustand}`);
  return (
    <span
      role="img"
      aria-label={text}
      title={text}
      className={`ablage-sync ablage-sync--${zustand}`}
      data-testid={`ablage-sync-${zustand}`}
    >
      <Zeichen size={16} aria-hidden="true" />
      <span className="ablage-mut ablage-nur-breit">{text}</span>
    </span>
  );
};

/**
 * Cloud-Ausnahme als Chip (D7-Rest): Text „Cloud-Ausnahme“, die Anbieter stehen im Titel und für
 * Hilfstechnik im Text. Nur geerbt: „(geerbt)“.
 */
export const CloudChip: React.FC<{
  namen: string[];
  geerbt?: boolean;
}> = ({ namen, geerbt = false }) => {
  const { t } = useTranslation();
  if (namen.length === 0) return null;
  const liste = namen.join(", ");
  return (
    <Chip
      art="cloud"
      title={t(
        geerbt ? "ablage.cloud.chipLabelGeerbt" : "ablage.cloud.chipLabel",
        {
          anbieter: liste,
        },
      )}
    >
      <CloudCheck size={14} aria-hidden="true" />
      {t(geerbt ? "ablage.cloud.chipGeerbt" : "ablage.cloud.chip")}
      <span className="ablage-sr"> ({liste})</span>
    </Chip>
  );
};

/** KI-Zustand und Datenklasse als Chips (Liste und Details). */
export const KiKlasseChips: React.FC<{
  ki: AblageKi;
  klasse: string;
  /** D7-Rest: Cloud-Ausnahme dieses Eintrags (Namen der Anbieter); `geerbt` = nur von oben. */
  cloudNamen?: string[];
  cloudGeerbt?: boolean;
}> = ({ ki, klasse, cloudNamen, cloudGeerbt }) => {
  const { t } = useTranslation();
  return (
    <>
      {ki.zustand === "an" && (
        <Chip art="ki">
          <Sparkles size={14} aria-hidden="true" />
          {t("ablage.ki.chipAn")}
        </Chip>
      )}
      {ki.zustand === "gesperrt" && (
        <Chip art="sperre">
          <Lock size={14} aria-hidden="true" />
          {t("ablage.ki.stufe.gesperrt")}
        </Chip>
      )}
      {ki.zustand === "unklar" && (
        <Chip art="sperre">{t("ablage.ki.chipUnklar")}</Chip>
      )}{" "}
      <Chip>
        {t(`ablage.klasse.kurz.${klasse}`, {
          defaultValue: t("ablage.klasse.kurz.unbekannt"),
        })}
      </Chip>
      {cloudNamen && cloudNamen.length > 0 && (
        <>
          {" "}
          <CloudChip namen={cloudNamen} geerbt={cloudGeerbt} />
        </>
      )}
    </>
  );
};

/** Ordnerbaum: eigene Ordner, „Wiederhergestellt“, verknüpfte Ordner mit Unterbaum, Papierkorb. */
export const OrdnerBaum: React.FC<{
  ordner: AblageOrdner[];
  gewaehlt: string | null;
  korbAnzahl: number;
  onWahl(id: string): void;
  onNeu(): void;
  /** D7: verknüpfte Ordner (nur lesen, nur dieses Gerät). */
  verknuepft?: AblageVerknuepft[];
  onVerknuepfen?: () => void;
  /** D7-Rest: Unterordnerbaum je verknüpftem Ordner (nur lesen). */
  unterbaeume?: Map<string, UnterKnoten[]>;
  /** Aufgeklappte Knoten (Ansichts-Schlüssel: Kennung oder `<Kennung>|<Pfad>`). */
  offen?: Set<string>;
  onUmschalten?: (schluessel: string, name: string) => void;
  /** Namen der Anbieter einer Cloud-Ausnahme (für Titel der Zeichen). */
  anbieterNamen?: (ids: string[]) => string[];
  /** Kennung-Vorsatz für `aria-controls` (der Baum steht in Spalte und Blatt). */
  idVorsatz?: string;
}> = ({
  ordner,
  gewaehlt,
  korbAnzahl,
  onWahl,
  onNeu,
  verknuepft = [],
  onVerknuepfen,
  unterbaeume,
  offen,
  onUmschalten,
  anbieterNamen,
  idVorsatz = "baum",
}) => {
  const { t } = useTranslation();
  const { wurzeln, wiederhergestellt } = baumBauen(ordner);
  const zeile = (k: BaumKnoten): React.ReactNode => {
    const cloud = cloudAlle(k.ordner.cloud);
    const cloudTitel = (anbieterNamen?.(cloud) ?? cloud).join(", ");
    return (
      <React.Fragment key={k.ordner.id}>
        <button
          type="button"
          className="ablage-row"
          style={{ paddingLeft: 12 + k.tiefe * 18 }}
          aria-current={gewaehlt === k.ordner.id ? "true" : undefined}
          onClick={() => onWahl(k.ordner.id)}
          data-testid={`ablage-ordner-${k.ordner.id}`}
        >
          <Folder size={20} aria-hidden="true" />
          <span className="ablage-row__name">{k.ordner.name}</span>
          {k.ordner.ki.zustand === "an" && (
            <span
              role="img"
              aria-label={t("ablage.ki.fuerKiFreigegeben")}
              style={{ color: "var(--color-logo-primary)" }}
            >
              <Sparkles size={16} aria-hidden="true" />
            </span>
          )}
          {k.ordner.ki.zustand === "gesperrt" && (
            <span
              role="img"
              aria-label={t("ablage.ki.stufe.gesperrt")}
              style={{ color: "var(--abl-warn)" }}
            >
              <Lock size={16} aria-hidden="true" />
            </span>
          )}
          {cloud.length > 0 && (
            <span
              role="img"
              aria-label={`${t("ablage.cloud.baum")}: ${cloudTitel}`}
              title={`${t("ablage.cloud.baum")}: ${cloudTitel}`}
              style={{ color: "var(--color-logo-primary)" }}
              data-testid={`ablage-baum-cloud-${k.ordner.id}`}
            >
              <CloudCheck size={16} aria-hidden="true" />
            </span>
          )}
          {k.ordner.nur_geraet_wirksam && (
            <span
              role="img"
              aria-label={t("ablage.nurGeraet.titel")}
              className="ablage-row__ct"
            >
              <Monitor size={16} aria-hidden="true" />
            </span>
          )}
          <span className="ablage-row__ct">{k.ordner.dateien}</span>
        </button>
        {k.kinder.map(zeile)}
      </React.Fragment>
    );
  };
  const kennung = (schluessel: string) =>
    `${idVorsatz}-k-${encodeURIComponent(schluessel)}`;
  /** Aufklapp-Knopf (44 × 44 px) oder Platzhalter gleicher Breite. */
  const umschalter = (schluessel: string, name: string, hatKinder: boolean) => {
    if (!hatKinder) return <span className="ablage-tsp" aria-hidden="true" />;
    const auf = offen?.has(schluessel) ?? false;
    return (
      <button
        type="button"
        className="ablage-tw"
        aria-expanded={auf}
        aria-controls={kennung(schluessel)}
        aria-label={t(
          auf ? "ablage.baum.zuklappen" : "ablage.baum.aufklappen",
          {
            name,
          },
        )}
        onClick={() => onUmschalten?.(schluessel, name)}
        data-testid={`ablage-baum-umschalter-${schluessel}`}
      >
        {auf ? (
          <ChevronDown size={16} aria-hidden="true" />
        ) : (
          <ChevronRight size={16} aria-hidden="true" />
        )}
      </button>
    );
  };
  const unterZeile = (
    wurzel: string,
    k: UnterKnoten,
    tiefe: number,
  ): React.ReactNode => {
    const schluessel = unterAnsicht(wurzel, k.pfad);
    // Ordner, die nur online liegen, betritt die App nie: nicht aufklappbar.
    const hatKinder = !k.nurOnline && k.kinder.length > 0;
    const auf = offen?.has(schluessel) ?? false;
    return (
      <li key={schluessel}>
        <div
          className="ablage-trow"
          data-aktiv={gewaehlt === schluessel}
          style={{ "--lv": tiefe } as React.CSSProperties}
        >
          {umschalter(schluessel, k.name, hatKinder)}
          <button
            type="button"
            className="ablage-rm"
            aria-current={gewaehlt === schluessel ? "true" : undefined}
            onClick={() => onWahl(schluessel)}
            data-testid={`ablage-ordner-${schluessel}`}
          >
            <Folder size={20} aria-hidden="true" />
            <span className="ablage-rm__name">{k.name}</span>
            {k.nurOnline && (
              <span
                role="img"
                aria-label={t("ablage.baum.nurOnline")}
                title={t("ablage.baum.nurOnline")}
                className="ablage-mut"
              >
                <Cloud size={16} aria-hidden="true" />
              </span>
            )}
            {k.dateien > 0 && (
              <span className="ablage-row__ct">{k.dateien}</span>
            )}
          </button>
        </div>
        {hatKinder && (
          <ul id={kennung(schluessel)} className="ablage-tl" hidden={!auf}>
            {k.kinder.map((c) => unterZeile(wurzel, c, tiefe + 1))}
          </ul>
        )}
      </li>
    );
  };
  const verknuepftZeile = (v: AblageVerknuepft) => {
    const id = v.ordner.id;
    const baum = unterbaeume?.get(id) ?? [];
    const auf = offen?.has(id) ?? false;
    return (
      <li key={id}>
        <div
          className="ablage-trow"
          data-aktiv={gewaehlt === id}
          data-weg={!v.erreichbar}
          style={{ "--lv": 0 } as React.CSSProperties}
        >
          {umschalter(id, v.ordner.name, baum.length > 0)}
          <button
            type="button"
            className="ablage-rm"
            aria-current={gewaehlt === id ? "true" : undefined}
            onClick={() => onWahl(id)}
            title={t("ablage.verknuepft.hinweisKurz")}
            data-testid={`ablage-ordner-${id}`}
          >
            <FolderSymlink size={20} aria-hidden="true" />
            <span className="ablage-rm__name">{v.ordner.name}</span>
            <span
              role="img"
              aria-label={t("ablage.baum.nurLesen")}
              title={t("ablage.baum.nurLesen")}
              className="ablage-mut"
            >
              <Eye size={16} aria-hidden="true" />
            </span>
            {!v.erreichbar && (
              <span
                role="img"
                aria-label={t("ablage.verknuepft.nichtErreichbar")}
                title={t("ablage.verknuepft.nichtErreichbar")}
                style={{ color: "var(--abl-warn)" }}
              >
                <AlertTriangle size={16} aria-hidden="true" />
              </span>
            )}
            <span className="ablage-row__ct">{v.ordner.dateien}</span>
          </button>
        </div>
        {baum.length > 0 && (
          <ul id={kennung(id)} className="ablage-tl" hidden={!auf}>
            {baum.map((k) => unterZeile(id, k, 1))}
          </ul>
        )}
      </li>
    );
  };
  return (
    <nav className="ablage-col ablage-baum" aria-label={t("ablage.baum.label")}>
      <div className="ablage-head">
        <h2>{t("ablage.titel")}</h2>
        <div className="ablage-head__tools">
          <IconButton
            icon={<Plus size={20} aria-hidden="true" />}
            label={t("ablage.ordner.neu")}
            size="md"
            onClick={onNeu}
            data-testid="ablage-ordner-neu"
          />
        </div>
      </div>
      <div className="ablage-grp">{t("ablage.baum.meine")}</div>
      {wurzeln.map(zeile)}
      {wiederhergestellt.length > 0 && (
        <>
          <div className="ablage-grp">{t("ablage.baum.wiederhergestellt")}</div>
          {wiederhergestellt.map(zeile)}
        </>
      )}
      <div className="ablage-grp" data-testid="ablage-verknuepft-gruppe">
        {t("ablage.verknuepft.gruppe")}
      </div>
      <ul className="ablage-tl">{verknuepft.map(verknuepftZeile)}</ul>
      {onVerknuepfen && (
        <button
          type="button"
          className="ablage-row ablage-mut"
          onClick={onVerknuepfen}
          aria-haspopup="dialog"
          title={t("ablage.verknuepft.cloudGrenze")}
          data-testid="ablage-verknuepfen"
        >
          <LinkSymbol size={20} aria-hidden="true" />
          <span className="ablage-row__name">
            {t("ablage.verknuepft.verknuepfen")}
          </span>
        </button>
      )}
      <div className="ablage-grp">{t("ablage.baum.weitere")}</div>
      <button
        type="button"
        className="ablage-row"
        aria-current={gewaehlt === KORB ? "true" : undefined}
        onClick={() => onWahl(KORB)}
        data-testid="ablage-korb"
      >
        <Trash2 size={20} aria-hidden="true" />
        <span className="ablage-row__name">{t("ablage.papierkorb.titel")}</span>
        <span className="ablage-row__ct">{korbAnzahl}</span>
      </button>
    </nav>
  );
};

const Symbol: React.FC<{ typ: string; aus?: boolean }> = ({ typ, aus }) => (
  <span
    className={`ablage-ic ablage-ic--${dateiArt(typ)}${aus ? " ablage-ic--aus" : ""}`}
    aria-hidden="true"
  >
    {dateiKuerzel(typ)}
  </span>
);

const datum = (ms: number, sprache: string) =>
  new Date(ms).toLocaleDateString(sprache, {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
  });

/**
 * „Nur online“ (OneDrive-Platzhalter) als Knopf mit Hinweis: Hover und Tastaturfokus zeigen ihn, Escape
 * schließt ihn (WCAG 1.4.13), der Zeiger darf in den Hinweis wechseln. Ein Klick öffnet die Details der
 * Datei, dort steht, wie sie sich lesbar machen lässt. Local Voice überspringt erkannte Platzhalter; ein Abruf durch gleichzeitige Änderungen des Sync-Dienstes bleibt möglich.
 */
export const NurOnlineChip: React.FC<{
  onOeffnen(ausloeser: HTMLElement): void;
  testid?: string;
}> = ({ onOeffnen, testid }) => {
  const { t } = useTranslation();
  const tipId = useId();
  const [zeiger, setZeiger] = useState(false);
  const [fokus, setFokus] = useState(false);
  const [zu, setZu] = useState(false);
  const sichtbar = (zeiger || fokus) && !zu;
  return (
    <span
      className="ablage-tipw"
      onMouseEnter={() => {
        setZeiger(true);
        setZu(false);
      }}
      onMouseLeave={() => setZeiger(false)}
    >
      <button
        type="button"
        className="ablage-chipbtn"
        aria-describedby={tipId}
        aria-haspopup="dialog"
        onFocus={() => {
          setFokus(true);
          setZu(false);
        }}
        onBlur={() => setFokus(false)}
        onKeyDown={(e) => {
          if (e.key === "Escape" && sichtbar) {
            e.stopPropagation();
            setZu(true);
          }
        }}
        onClick={(e) => onOeffnen(e.currentTarget)}
        data-testid={testid}
      >
        <span className="ablage-chip">
          <Cloud size={14} aria-hidden="true" />
          {t("ablage.stand.nurOnline")}
        </span>
      </button>
      <span
        role="tooltip"
        id={tipId}
        className="ablage-tip"
        data-offen={sichtbar}
        data-testid={testid ? `${testid}-tipp` : undefined}
      >
        <b>{t("ablage.stand.nurOnline")}</b>
        {t("ablage.verknuepft.nurOnlineTipp")}
      </span>
    </span>
  );
};

/** Liste des Ordners: Unterordner zuerst, dann Dateien. */
export const DateiTabelle: React.FC<{
  titel: string;
  unter: AblageOrdner[];
  dateien: AblageDatei[];
  gewaehlt: string | null;
  onOrdner(id: string): void;
  onDatei(id: string, ausloeser: HTMLElement): void;
  /** Sync-Zustand je Kennung (D4); fehlt ohne Konto. */
  sync?: Map<string, AblageSyncZustand>;
  /** Namen der Anbieter einer Cloud-Ausnahme (Chip am Ordner). */
  anbieterNamen?: (ids: string[]) => string[];
  /** D7-Rest: Liste eines verknüpften Ordners (nur lesen): Unterordner als Pfade, Dateien mit Namen
   *  ohne Pfad, Stand „Im Index“ oder „Nur online“, auf schmalen Fenstern als Karten. */
  verknuepft?: {
    wurzel: string;
    klasse: string;
    unterPfade: UnterKnoten[];
    onUnterPfad(schluessel: string): void;
    /** Der Ordner war nicht erreichbar oder liegt nur online: die Dateien sind zuletzt bekannte. */
    zuletztBekannt: boolean;
  };
}> = ({
  titel,
  unter,
  dateien,
  gewaehlt,
  onOrdner,
  onDatei,
  sync,
  anbieterNamen,
  verknuepft,
}) => {
  const { t, i18n } = useTranslation();
  const vk = verknuepft;
  /** Stand einer verknüpften Datei: im Index, nur online (mit Hinweis) oder Textstand. */
  const standVerknuepft = (d: AblageDatei): React.ReactNode => {
    if (vk?.zuletztBekannt)
      return (
        <span className="ablage-mut" data-testid={`ablage-zuletzt-${d.id}`}>
          {t("ablage.stand.zuletztBekannt")}
        </span>
      );
    if (d.text === "nur_online")
      return (
        <NurOnlineChip
          onOeffnen={(a) => onDatei(d.id, a)}
          testid={`ablage-nur-online-${d.id}`}
        />
      );
    if (d.text === "ok")
      return (
        <span className="ablage-stn ablage-stn--ok">
          <span role="img" aria-label={t("ablage.stand.imIndexLabel")}>
            <Check size={16} aria-hidden="true" />
          </span>
          {t("ablage.stand.imIndex")}
        </span>
      );
    if (d.text)
      return (
        <Chip art={d.text === "fehler" ? "warn" : undefined}>
          {t(`ablage.details.text.${d.text}`, { defaultValue: d.text })}
        </Chip>
      );
    return <span className="ablage-mut">—</span>;
  };
  return (
    <table className={`ablage-tabelle${vk ? " ablage-karten" : ""}`}>
      <caption className="ablage-sr">
        {t("ablage.liste.caption", { name: titel })}
      </caption>
      <thead>
        <tr>
          <th scope="col">{t("ablage.liste.name")}</th>
          <th scope="col" className="ablage-spalte-datum">
            {t("ablage.liste.geaendert")}
          </th>
          <th scope="col" className="ablage-spalte-groesse">
            {t("ablage.liste.groesse")}
          </th>
          <th scope="col" className="ablage-spalte-klasse">
            {t("ablage.liste.klasse")}
          </th>
          <th scope="col">{t("ablage.liste.stand")}</th>
        </tr>
      </thead>
      <tbody>
        {unter.map((o) => {
          const cloud = cloudAlle(o.cloud);
          return (
            <tr key={o.id}>
              <td className="ablage-name">
                <button
                  type="button"
                  className="ablage-lnk"
                  onClick={() => onOrdner(o.id)}
                  data-testid={`ablage-unterordner-${o.id}`}
                >
                  <span className="ablage-ic ablage-ic--dir" aria-hidden="true">
                    <Folder size={20} />
                  </span>
                  <span>{o.name}</span>
                </button>
              </td>
              <td className="ablage-spalte-datum ablage-mut">—</td>
              <td className="ablage-spalte-groesse ablage-mut">
                {t("ablage.liste.dateien", { count: o.dateien })}
              </td>
              <td className="ablage-spalte-klasse">
                <KiKlasseChips
                  ki={o.ki}
                  klasse={o.klasse}
                  cloudNamen={anbieterNamen?.(cloud) ?? cloud}
                  cloudGeerbt={(o.cloud?.anbieter.length ?? 0) === 0}
                />
              </td>
              <td>
                {sync?.get(o.id) && <SyncZeichen zustand={sync.get(o.id)!} />}
              </td>
            </tr>
          );
        })}
        {vk?.unterPfade.map((k) => {
          const schluessel = unterAnsicht(vk.wurzel, k.pfad);
          return (
            <tr key={schluessel}>
              <td className="ablage-name">
                <button
                  type="button"
                  className="ablage-lnk"
                  onClick={() => vk.onUnterPfad(schluessel)}
                  data-testid={`ablage-unterordner-${schluessel}`}
                >
                  <span className="ablage-ic ablage-ic--dir" aria-hidden="true">
                    <Folder size={20} />
                  </span>
                  <span>{k.name}</span>
                </button>
              </td>
              <td className="ablage-meta">
                {t("ablage.liste.dateien", { count: k.dateien })}
              </td>
              <td className="ablage-spalte-datum ablage-mut">—</td>
              <td className="ablage-spalte-groesse ablage-mut">
                {t("ablage.liste.dateien", { count: k.dateien })}
              </td>
              <td className="ablage-spalte-klasse">
                <KiKlasseChips
                  ki={{
                    zustand: "aus",
                    von: null,
                    von_name: null,
                    vorgabe: true,
                  }}
                  klasse={vk.klasse}
                />
              </td>
              <td className="ablage-stand">
                {k.nurOnline ? (
                  <Chip>
                    <Cloud size={14} aria-hidden="true" />
                    {t("ablage.stand.nurOnline")}
                  </Chip>
                ) : (
                  <span className="ablage-mut">—</span>
                )}
              </td>
            </tr>
          );
        })}
        {dateien.map((d) => (
          <tr key={d.id} data-gewaehlt={gewaehlt === d.id}>
            <td className="ablage-name">
              <button
                type="button"
                className="ablage-lnk"
                aria-current={gewaehlt === d.id ? "true" : undefined}
                onClick={(e) => onDatei(d.id, e.currentTarget)}
                data-testid={`ablage-datei-${d.id}`}
              >
                <Symbol typ={d.typ} aus={vk && d.text === "nur_online"} />
                <span title={vk ? d.name : undefined}>
                  {vk ? letztesGlied(d.name) : d.name}
                </span>
              </button>
            </td>
            {vk && (
              <td className="ablage-meta">
                {[
                  d.typ.toUpperCase(),
                  formatBytes(d.groesse, i18n.language),
                  datum(d.geaendert, i18n.language),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </td>
            )}
            <td className="ablage-spalte-datum ablage-mut">
              {datum(d.geaendert, i18n.language)}
            </td>
            <td className="ablage-spalte-groesse ablage-mut">
              {formatBytes(d.groesse, i18n.language)}
            </td>
            <td className="ablage-spalte-klasse">
              <KiKlasseChips
                ki={d.verknuepft ? { ...d.ki, zustand: "aus" } : d.ki}
                klasse={d.klasse}
              />
            </td>
            {vk ? (
              <td className="ablage-stand">{standVerknuepft(d)}</td>
            ) : sync?.get(d.id) && !d.konflikt ? (
              <td>
                <SyncZeichen zustand={sync.get(d.id)!} />
              </td>
            ) : (
              <td style={{ color: d.konflikt ? "var(--abl-warn)" : undefined }}>
                {d.konflikt && (
                  <span
                    role="img"
                    aria-label={t("ablage.stand.konflikt")}
                    style={{
                      display: "inline-flex",
                      alignItems: "center",
                      gap: 4,
                    }}
                  >
                    <AlertTriangle size={16} aria-hidden="true" />
                    <span className="ablage-mut ablage-nur-breit">
                      {t("ablage.stand.konflikt")}
                    </span>
                  </span>
                )}
                {!d.konflikt && sync && (
                  <span
                    role="img"
                    aria-label={t("ablage.sync.nicht_erfasst")}
                    title={t("ablage.sync.nicht_erfasst")}
                    className="ablage-mut"
                    data-testid="ablage-sync-nicht-erfasst"
                  >
                    <CircleDashed size={16} aria-hidden="true" />
                  </span>
                )}
                {!d.konflikt && !sync && d.nur_geraet && (
                  <span
                    role="img"
                    aria-label={t("ablage.nurGeraet.titel")}
                    className="ablage-mut"
                  >
                    <Monitor size={16} aria-hidden="true" />
                  </span>
                )}
              </td>
            )}
          </tr>
        ))}
      </tbody>
    </table>
  );
};

/** Papierkorb: 30 Tage, Wiederherstellen, endgültig löschen. */
export const KorbTabelle: React.FC<{
  eintraege: AblageKorb[];
  busy: boolean;
  onWiederherstellen(e: AblageKorb): void;
  onLoeschen(e: AblageKorb): void;
}> = ({ eintraege, busy, onWiederherstellen, onLoeschen }) => {
  const { t, i18n } = useTranslation();
  return (
    <table className="ablage-tabelle">
      <caption className="ablage-sr">{t("ablage.papierkorb.caption")}</caption>
      <thead>
        <tr>
          <th scope="col">{t("ablage.liste.name")}</th>
          <th scope="col" className="ablage-spalte-datum">
            {t("ablage.papierkorb.geloescht")}
          </th>
          <th scope="col" className="ablage-spalte-klasse">
            {t("ablage.papierkorb.ursprung")}
          </th>
          <th scope="col">
            <span className="ablage-sr">{t("ablage.papierkorb.aktion")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {eintraege.map((e) => (
          <tr key={`${e.art}-${e.id}`}>
            <td className="ablage-name">
              <span className="ablage-lnk" title={e.name}>
                {e.art === "ordner" ? (
                  <span className="ablage-ic ablage-ic--dir" aria-hidden="true">
                    <Folder size={20} />
                  </span>
                ) : (
                  <Symbol typ={e.typ} />
                )}
                <span>{e.name}</span>
              </span>
            </td>
            <td className="ablage-spalte-datum ablage-mut">
              {datum(e.geloescht, i18n.language)}
              <br />
              {t("ablage.papierkorb.tageRest", { count: e.tage_rest })}
            </td>
            <td className="ablage-spalte-klasse ablage-mut">
              {e.ursprung || t("ablage.papierkorb.ohneUrsprung")}
            </td>
            <td>
              <Button
                className="ablage-btn-hoch"
                disabled={busy}
                aria-label={t("ablage.papierkorb.wiederherstellenName", {
                  name: e.name,
                })}
                onClick={() => onWiederherstellen(e)}
                icon={<RotateCcw size={16} aria-hidden="true" />}
                data-testid={`ablage-wiederherstellen-${e.id}`}
              >
                {t("ablage.papierkorb.wiederherstellen")}
              </Button>{" "}
              <IconButton
                icon={<Trash2 size={16} aria-hidden="true" />}
                label={t("ablage.papierkorb.endgueltigName", { name: e.name })}
                size="md"
                disabled={busy}
                onClick={() => onLoeschen(e)}
                data-testid={`ablage-endgueltig-${e.id}`}
              />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
};

/** Details einer Datei: Vorschau, „Für die KI“, Fassungen, Angaben. */
export const Details: React.FC<{
  datei: AblageDatei;
  pfad: string;
  fassungen: AblageFassung[];
  busy: boolean;
  onUmbenennen(): void;
  onPapierkorb(): void;
  onKi(): void;
  onSchliessen?: () => void;
  ueberschriftId: string;
  /** D2: Fundstelle aus der Inhaltssuche (Anker, Pfad, Auszug). */
  fundstelle?: AblageFundstelle | null;
  anfrage?: string;
  /** Im Blatt (schmales Fenster) trägt der Dialog Titel und Schließen-Knopf: kein zweiter Kopf. */
  imBlatt?: boolean;
  /** D7-Rest: Cloud-Ausnahmen, die für die Datei gelten (Anbietername, Ordner, von dem sie stammen). */
  cloud?: { name: string; von: string | null }[];
  /** D7-Rest: verknüpfter Ordner neu einlesen (Hinweis „Auf diesem Gerät behalten“). */
  onNeuEinlesen?: () => void;
}> = ({
  datei,
  pfad,
  fassungen,
  busy,
  onUmbenennen,
  onPapierkorb,
  onKi,
  onSchliessen,
  ueberschriftId,
  fundstelle,
  anfrage,
  imBlatt = false,
  cloud = [],
  onNeuEinlesen,
}) => {
  const { t, i18n } = useTranslation();
  const textFehler = datei.text_code
    ? t(
        `ablage.details.textFehler.${datei.text_code.replace(/^(extrakt|ablage|verknuepft)\./, "")}`,
        {
          defaultValue: t("ablage.details.textFehler.allgemein"),
        },
      )
    : null;
  const kiText = (k: AblageKi) =>
    k.zustand === "unklar"
      ? t("ablage.ki.unklarKurz")
      : t(`ablage.ki.zustand.${k.zustand}`) +
        (k.von_name
          ? ` · ${t("ablage.ki.geerbtVon", { name: k.von_name })}`
          : "");
  return (
    <>
      {!imBlatt && (
        <div className="ablage-head">
          <h2 id={ueberschriftId} title={datei.name}>
            {datei.name}
          </h2>
          {onSchliessen && (
            <div className="ablage-head__tools">
              <IconButton
                icon={<X size={20} aria-hidden="true" />}
                label={t("ablage.details.schliessen")}
                size="md"
                onClick={onSchliessen}
              />
            </div>
          )}
        </div>
      )}
      {fundstelle && (
        <div
          className="ablage-sec ablage-fundstelle"
          data-testid="ablage-fundstelle"
        >
          <h3>{t("ablage.suchergebnis.fundstelle")}</h3>
          <div className="ablage-fundstelle__anker">
            {fundstelleText(t, fundstelle)}
          </div>
          <p className="ablage-suche__auszug">
            <Auszug text={fundstelle.auszug} anfrage={anfrage ?? ""} />
          </p>
        </div>
      )}
      {datei.text === "nur_online" ? (
        <>
          <div
            className="ablage-vorschau ablage-vorschau--leer"
            role="img"
            aria-label={t("ablage.verknuepft.keineVorschauLabel")}
          >
            <div>
              <Cloud size={36} aria-hidden="true" />
              <div>{t("ablage.verknuepft.keineVorschau")}</div>
            </div>
          </div>
          <div
            className="ablage-hint"
            role="note"
            data-testid="ablage-nur-online-hinweis"
          >
            <div className="ablage-hint__kopf">
              <Cloud size={20} aria-hidden="true" />
              <b>{t("ablage.verknuepft.behaltenTitel")}</b>
            </div>
            <p>{t("ablage.verknuepft.behaltenText")}</p>
            <ol>
              <li>{t("ablage.verknuepft.behaltenSchritt1")}</li>
              <li>{t("ablage.verknuepft.behaltenSchritt2")}</li>
            </ol>
            <p>{t("ablage.verknuepft.behaltenDanach")}</p>
            {onNeuEinlesen && (
              <Button
                onClick={onNeuEinlesen}
                disabled={busy}
                icon={<RefreshCw size={16} aria-hidden="true" />}
                data-testid="ablage-nur-online-einlesen"
              >
                {t("ablage.verknuepft.neuEinlesen")}
              </Button>
            )}
          </div>
        </>
      ) : (
        <div
          className="ablage-vorschau"
          role="img"
          aria-label={t("ablage.details.vorschau")}
        >
          {t("ablage.details.vorschauHinweis")}
        </div>
      )}
      <div className="ablage-sec">
        <h3>{t("ablage.ki.fuerDieKi")}</h3>
        {datei.verknuepft ? (
          // Die KI erreicht verknüpfte Ordner nie: nur ein Hinweis, kein Schalter.
          <div className="ablage-sw">
            <span
              className="ablage-mut"
              data-testid="ablage-ki-nicht-verfuegbar"
            >
              {t("ablage.ki.verknuepftNichtVerfuegbar")}
            </span>
          </div>
        ) : (
          <div className="ablage-sw">
            <span>
              {kiText(datei.ki)}
              <br />
              <span className="ablage-mut">{t("ablage.ki.nurAuszuege")}</span>
            </span>
            <Button
              onClick={onKi}
              disabled={busy}
              data-testid="ablage-ki-aendern"
            >
              {t("ablage.ki.aendern")}
            </Button>
          </div>
        )}
        <div className="ablage-sw">
          <span>{t("ablage.klasse.titel")}</span>
          <KiKlasseChips
            ki={{ ...datei.ki, zustand: "aus" }}
            klasse={datei.klasse}
          />
        </div>
        {cloud.length > 0 && (
          <div className="ablage-sw" data-testid="ablage-details-cloud">
            <span>
              {t("ablage.cloud.details")}
              {cloud.map((c) => (
                <React.Fragment key={c.name}>
                  <br />
                  <span className="ablage-mut">
                    {c.von
                      ? t("ablage.cloud.detailsQuelle", {
                          anbieter: c.name,
                          ordner: c.von,
                        })
                      : c.name}
                  </span>
                </React.Fragment>
              ))}
            </span>
            <Chip art="cloud">
              <CloudCheck size={14} aria-hidden="true" />
              {t("ablage.cloud.detailsWert")}
            </Chip>
          </div>
        )}
        <div className="ablage-sw">
          <span>{t("ablage.details.textfassung")}</span>
          <Chip>
            {datei.text === "nur_online" && (
              <Cloud size={14} aria-hidden="true" />
            )}
            {datei.text
              ? t(`ablage.details.text.${datei.text}`, {
                  defaultValue: datei.text,
                })
              : t("ablage.details.textOhne")}
          </Chip>
        </div>
        {textFehler && (
          <div className="ablage-mut" data-testid="ablage-textfehler">
            {textFehler}
          </div>
        )}
      </div>
      {!datei.verknuepft && (
        <div className="ablage-sec">
          <h3>{t("ablage.details.fassungen")}</h3>
          <dl className="ablage-kv">
            {fassungen.map((f) => (
              <React.Fragment key={f.fas}>
                <dt>
                  {f.aktuell
                    ? t("ablage.details.fassungAktuell", { nummer: f.nummer })
                    : f.nummer}
                </dt>
                <dd>
                  {new Date(f.zeit).toLocaleString(i18n.language, {
                    day: "2-digit",
                    month: "2-digit",
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                  {" · "}
                  {formatBytes(f.groesse, i18n.language)}
                  {f.abgeloest && ` · ${t("ablage.details.abgeloest")}`}
                </dd>
              </React.Fragment>
            ))}
          </dl>
        </div>
      )}
      <div className="ablage-sec">
        <h3>{t("ablage.details.angaben")}</h3>
        <dl className="ablage-kv">
          <dt>{t("ablage.liste.groesse")}</dt>
          <dd>{formatBytes(datei.groesse, i18n.language)}</dd>
          <dt>{t("ablage.details.ordner")}</dt>
          <dd>{pfad}</dd>
          <dt>{t("ablage.details.ablage")}</dt>
          <dd
            data-testid={
              datei.verknuepft ? "ablage-details-verknuepft" : undefined
            }
          >
            {datei.verknuepft
              ? t("ablage.verknuepft.angabe")
              : t("ablage.details.nurDiesesGeraet")}
          </dd>
        </dl>
      </div>
      {!datei.verknuepft && (
        <div
          className="ablage-sec"
          style={{ display: "flex", flexWrap: "wrap", gap: 8, borderBottom: 0 }}
        >
          <Button
            onClick={onUmbenennen}
            disabled={busy}
            data-testid="ablage-umbenennen"
          >
            {t("ablage.details.umbenennen")}
          </Button>
          <Button
            onClick={onPapierkorb}
            disabled={busy}
            icon={<Trash2 size={16} aria-hidden="true" />}
            data-testid="ablage-datei-papierkorb"
          >
            {t("ablage.details.papierkorb")}
          </Button>
        </div>
      )}
    </>
  );
};

export interface UploadZeile {
  key: string;
  name: string;
  quelle: string;
  ordner: string | null;
  stand: AblageImportStand | "laeuft";
  code: string | null;
}

const BAR: Record<UploadZeile["stand"], number> = {
  laeuft: 55,
  abgelegt: 100,
  neue_fassung: 100,
  kollision: 100,
  uebersprungen: 100,
  fehler: 100,
};

/** Hochladeliste mit Prüfung und Kollisionswahl (nichts wird überschrieben). */
export const Hochladeliste: React.FC<{
  zeilen: UploadZeile[];
  ordnerName: string;
  ueberschriftId: string;
  onWahl(z: UploadZeile, wahl: AblageKollisionswahl | null): void;
  onSchliessen(): void;
  fehlerText(code: string | null): string;
  /** Im Blatt (schmales Fenster) trägt der Dialog Titel und Schließen-Knopf: kein zweiter Kopf. */
  imBlatt?: boolean;
}> = ({
  zeilen,
  ordnerName,
  ueberschriftId,
  onWahl,
  onSchliessen,
  fehlerText,
  imBlatt = false,
}) => {
  const { t } = useTranslation();
  return (
    <>
      {!imBlatt && (
        <div className="ablage-head">
          <h2 id={ueberschriftId}>
            {t("ablage.hochladen.titel", { count: zeilen.length })}
          </h2>
          <div className="ablage-head__tools">
            <IconButton
              icon={<X size={20} aria-hidden="true" />}
              label={t("ablage.hochladen.schliessen")}
              size="md"
              onClick={onSchliessen}
            />
          </div>
        </div>
      )}
      <div className="ablage-up" aria-live="polite">
        {zeilen.map((z) => (
          <div key={z.key} data-testid={`ablage-upload-${z.stand}`}>
            <div className="ablage-up__zeile">
              <span title={z.name}>{z.name}</span>
              <span className="ablage-mut">
                {z.stand === "fehler" || z.stand === "uebersprungen"
                  ? fehlerText(z.code) || t(`ablage.hochladen.stand.${z.stand}`)
                  : t(`ablage.hochladen.stand.${z.stand}`)}
              </span>
            </div>
            <div
              className="ablage-up__bar"
              role="progressbar"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={BAR[z.stand]}
              aria-label={z.name}
            >
              <i
                style={{
                  width: `${BAR[z.stand]}%`,
                  background:
                    z.stand === "fehler"
                      ? "var(--abl-warn)"
                      : z.stand === "abgelegt" || z.stand === "neue_fassung"
                        ? "var(--abl-ok)"
                        : undefined,
                }}
              />
            </div>
            {z.stand === "kollision" && (
              <>
                <div className="ablage-mut" style={{ margin: "6px 0" }}>
                  {t("ablage.hochladen.kollisionHinweis")}
                </div>
                <div
                  className="ablage-up__aktion"
                  role="group"
                  aria-label={t("ablage.hochladen.kollisionGruppe", {
                    name: z.name,
                  })}
                >
                  <Button
                    variant="primary"
                    onClick={() => onWahl(z, "neue_fassung")}
                    data-testid="ablage-kollision-fassung"
                  >
                    {t("ablage.hochladen.neueFassung")}
                  </Button>
                  <Button
                    onClick={() => onWahl(z, "beide_behalten")}
                    data-testid="ablage-kollision-beide"
                  >
                    {t("ablage.hochladen.beideBehalten")}
                  </Button>
                  <Button
                    onClick={() => onWahl(z, null)}
                    data-testid="ablage-kollision-ueberspringen"
                  >
                    {t("ablage.hochladen.ueberspringen")}
                  </Button>
                </div>
              </>
            )}
          </div>
        ))}
      </div>
      <div className="ablage-hinweis">
        {t("ablage.hochladen.erben", { name: ordnerName })}
      </div>
    </>
  );
};

/** Meldung oben in der Liste: Ablage nicht lesbar, KI-Freigabe unklar, gefundene Dateien. */
export const Meldung: React.FC<{
  warn?: boolean;
  children: React.ReactNode;
  testid?: string;
}> = ({ warn, children, testid }) => (
  <div
    className={`ablage-hinweis${warn ? " ablage-hinweis--warn" : ""}`}
    role="status"
    data-testid={testid}
  >
    {warn && (
      <AlertTriangle
        size={16}
        aria-hidden="true"
        style={{ verticalAlign: -3, marginRight: 6 }}
      />
    )}
    {children}
  </div>
);
