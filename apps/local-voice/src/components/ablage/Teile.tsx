import React from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  Check,
  Clock,
  FileText,
  Folder,
  FolderSymlink,
  Link as LinkSymbol,
  Monitor,
  PauseCircle,
  Plus,
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
  dateiArt,
  dateiKuerzel,
  formatBytes,
  type BaumKnoten,
} from "./logik";

export const KORB = "korb";

const Chip: React.FC<{
  art?: "ki" | "sperre" | "ok";
  children: React.ReactNode;
}> = ({ art, children }) => (
  <span className={`ablage-chip${art ? ` ablage-chip--${art}` : ""}`}>
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

/** KI-Zustand und Datenklasse als Chips (Liste und Details). */
export const KiKlasseChips: React.FC<{ ki: AblageKi; klasse: string }> = ({
  ki,
  klasse,
}) => {
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
    </>
  );
};

/** Ordnerbaum: eigene Ordner, „Wiederhergestellt“, Papierkorb. */
export const OrdnerBaum: React.FC<{
  ordner: AblageOrdner[];
  gewaehlt: string | null;
  korbAnzahl: number;
  onWahl(id: string): void;
  onNeu(): void;
  /** D7: verknüpfte Ordner (nur lesen, nur dieses Gerät). */
  verknuepft?: AblageVerknuepft[];
  onVerknuepfen?: () => void;
}> = ({
  ordner,
  gewaehlt,
  korbAnzahl,
  onWahl,
  onNeu,
  verknuepft = [],
  onVerknuepfen,
}) => {
  const { t } = useTranslation();
  const { wurzeln, wiederhergestellt } = baumBauen(ordner);
  const zeile = (k: BaumKnoten): React.ReactNode => (
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
      {verknuepft.map((v) => (
        <button
          key={v.ordner.id}
          type="button"
          className="ablage-row"
          aria-current={gewaehlt === v.ordner.id ? "true" : undefined}
          onClick={() => onWahl(v.ordner.id)}
          title={t("ablage.verknuepft.hinweisKurz")}
          data-testid={`ablage-ordner-${v.ordner.id}`}
        >
          <FolderSymlink size={20} aria-hidden="true" />
          <span className="ablage-row__name">{v.ordner.name}</span>
          {!v.erreichbar && (
            <span
              role="img"
              aria-label={t("ablage.verknuepft.nichtErreichbar")}
              style={{ color: "var(--abl-warn)" }}
            >
              <AlertTriangle size={16} aria-hidden="true" />
            </span>
          )}
          <span className="ablage-row__ct">{v.ordner.dateien}</span>
        </button>
      ))}
      {onVerknuepfen && (
        <button
          type="button"
          className="ablage-row ablage-mut"
          onClick={onVerknuepfen}
          aria-haspopup="dialog"
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

const Symbol: React.FC<{ typ: string }> = ({ typ }) => (
  <span className={`ablage-ic ablage-ic--${dateiArt(typ)}`} aria-hidden="true">
    {dateiKuerzel(typ)}
  </span>
);

const datum = (ms: number, sprache: string) =>
  new Date(ms).toLocaleDateString(sprache, {
    day: "2-digit",
    month: "2-digit",
    year: "numeric",
  });

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
}> = ({ titel, unter, dateien, gewaehlt, onOrdner, onDatei, sync }) => {
  const { t, i18n } = useTranslation();
  return (
    <table className="ablage-tabelle">
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
        {unter.map((o) => (
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
              <KiKlasseChips ki={o.ki} klasse={o.klasse} />
            </td>
            <td>
              {sync?.get(o.id) && <SyncZeichen zustand={sync.get(o.id)!} />}
            </td>
          </tr>
        ))}
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
                <Symbol typ={d.typ} />
                <span>{d.name}</span>
              </button>
            </td>
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
            {sync?.get(d.id) && !d.konflikt ? (
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
      <div
        className="ablage-vorschau"
        role="img"
        aria-label={t("ablage.details.vorschau")}
      >
        {t("ablage.details.vorschauHinweis")}
      </div>
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
        <div className="ablage-sw">
          <span>{t("ablage.details.textfassung")}</span>
          <Chip>
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
        {datei.text === "nur_online" && (
          <div className="ablage-mut" data-testid="ablage-nur-online-hinweis">
            {t("ablage.verknuepft.nurOnlineHinweis")}
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
