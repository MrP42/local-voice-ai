/**
 * Kopie aus @wai/ui konto (wai-sdk PR #991, ADR-0024). Nicht vom Original abweichen; Änderungen dort machen
 * und neu kopieren. Abweichungen hier: `Icon` -> lucide-react (siehe `Icon.tsx`), `cx` -> `cx.ts`.
 */
import {
  Avatar,
  Button,
  Popover,
  PopoverSurface,
  PopoverTrigger,
  Tooltip,
} from "@fluentui/react-components";
import { useEffect, useState } from "react";
import "./konto.css";
import { Icon } from "./Icon";
import { cx } from "./cx";
import {
  type Anmeldestatus,
  avatarFarbe,
  istBrauchbaresBild,
  type KontoProfil,
  profilInitialen,
  profilName,
} from "./profil";

export interface KontoLabels {
  lokalesProfil: string;
  status: Record<Anmeldestatus, string>;
  suite: string;
  anmelden: string;
  erneutAnmelden: string;
  abmelden: string;
  abmeldenAlle: string;
  angemeldetBei: (anbieter: string) => string;
  meinProfil: string;
  kontoGeraete: string;
  profilHinzufuegen: string;
  profilWechseln: (name: string) => string;
  gesperrt: string;
  anmeldenHinweisTitel: string;
  anmeldenHinweis: string;
  abgelaufenHinweis: string;
  profilmenue: (name: string) => string;
  bildVon: (name: string) => string;
}

export const KONTO_LABELS: KontoLabels = {
  lokalesProfil: "Lokales Profil",
  status: {
    lokal: "Lokales Profil",
    angemeldet: "Angemeldet",
    abgelaufen: "Anmeldung abgelaufen",
  },
  suite: "WAI Suite",
  anmelden: "Anmelden",
  erneutAnmelden: "Erneut anmelden",
  abmelden: "Abmelden",
  abmeldenAlle: "Abmelden (alle Apps)",
  angemeldetBei: (anbieter) => `Angemeldet bei ${anbieter}`,
  meinProfil: "Mein Profil",
  kontoGeraete: "Konto & Geräte",
  profilHinzufuegen: "Profil hinzufügen",
  profilWechseln: (name) => `Zu ${name} wechseln`,
  gesperrt: "Gesperrt",
  anmeldenHinweisTitel: "Anmelden für mehr:",
  anmeldenHinweis:
    "Abgleich über Geräte und Funktionen über mehrere Apps. Lokal funktioniert alles weiter.",
  abgelaufenHinweis:
    "Lokal arbeiten Sie weiter. Abgleich und geteilte Funktionen pausieren.",
  profilmenue: (name) => `Profil: ${name}`,
  bildVon: (name) => `Profilbild von ${name}`,
};

function useBildUrl(bild: Blob | undefined): string | undefined {
  const [url, setUrl] = useState<string>();
  useEffect(() => {
    if (
      !istBrauchbaresBild(bild) ||
      typeof URL.createObjectURL !== "function"
    ) {
      setUrl(undefined);
      return;
    }
    const u = URL.createObjectURL(bild);
    setUrl(u);
    return () => URL.revokeObjectURL(u);
  }, [bild]);
  return url;
}

export interface ProfilBildProps {
  profil: KontoProfil;
  /** Pixelgröße (Fluent-Avatar-Stufen). */
  groesse?: 24 | 28 | 32 | 40 | 64 | 96;
  /** Statuspunkt unten rechts. */
  status?: Anmeldestatus;
  labels?: Partial<KontoLabels>;
}

/** Profilbild oder Initialen; ohne Namen das Personensymbol. */
export function ProfilBild({
  profil,
  groesse = 28,
  status,
  labels,
}: ProfilBildProps) {
  const l = { ...KONTO_LABELS, ...labels };
  const url = useBildUrl(profil.bild);
  const name = profilName(profil, l.lokalesProfil);
  const ini = profilInitialen(profil);
  return (
    <span className="wai-konto-bild" data-groesse={groesse}>
      <Avatar
        size={groesse}
        name={name}
        initials={ini}
        color={avatarFarbe(profil)}
        icon={ini ? undefined : <Icon name="user" />}
        image={url ? { src: url, alt: l.bildVon(name) } : undefined}
        aria-hidden
      />
      {status && (
        <span className="wai-konto-bild__status" data-status={status} />
      )}
    </span>
  );
}

export interface ProfilmenueProps {
  profil: KontoProfil;
  status: Anmeldestatus;
  /** Weitere Profile auf diesem Gerät (ohne das aktive). */
  weitereProfile?: readonly KontoProfil[];
  onMeinProfil?: () => void;
  onKontoGeraete?: () => void;
  onAnmelden?: () => void;
  onAbmelden?: () => void;
  onProfilWechseln?: (id: string) => void;
  onProfilHinzufuegen?: () => void;
  labels?: Partial<KontoLabels>;
}

/** Inhalt des Profilmenüs (Variante A, „Me Control“ wie Microsoft 365). */
export function Profilmenue({
  profil,
  status,
  weitereProfile = [],
  onMeinProfil,
  onKontoGeraete,
  onAnmelden,
  onAbmelden,
  onProfilWechseln,
  onProfilHinzufuegen,
  labels,
}: ProfilmenueProps) {
  const l = { ...KONTO_LABELS, ...labels };
  const name = profilName(profil, l.lokalesProfil);
  const kopf =
    status === "lokal"
      ? l.suite
      : status === "abgelaufen"
        ? l.status.abgelaufen
        : (profil.anmeldung?.organisation ??
          profil.anmeldung?.anbieter ??
          l.suite);
  const kopfAktion =
    status === "lokal"
      ? onAnmelden && { text: l.anmelden, tun: onAnmelden }
      : status === "abgelaufen"
        ? onAnmelden && { text: l.erneutAnmelden, tun: onAnmelden }
        : onAbmelden && { text: l.abmeldenAlle, tun: onAbmelden };
  return (
    <div className="wai-konto-menue" data-status={status}>
      <div className="wai-konto-menue__kopf">
        <span>{kopf}</span>
        {kopfAktion && (
          <Button
            appearance="transparent"
            size="small"
            onClick={kopfAktion.tun}
          >
            {kopfAktion.text}
          </Button>
        )}
      </div>
      <div className="wai-konto-menue__person">
        <ProfilBild profil={profil} groesse={64} labels={labels} />
        <div className="wai-konto-menue__text">
          <div className="wai-konto-menue__name">{name}</div>
          {profil.email && (
            <div className="wai-konto-menue__zusatz">{profil.email}</div>
          )}
          <span className="wai-profilseite__status" data-status={status}>
            <i aria-hidden="true" />
            {status === "angemeldet" && profil.anmeldung
              ? l.angemeldetBei(profil.anmeldung.anbieter)
              : l.status[status]}
          </span>
          <div className="wai-konto-menue__links">
            {onMeinProfil && (
              <Button
                appearance="transparent"
                size="small"
                onClick={onMeinProfil}
              >
                {l.meinProfil}
              </Button>
            )}
            {onKontoGeraete && (
              <Button
                appearance="transparent"
                size="small"
                onClick={onKontoGeraete}
              >
                {l.kontoGeraete}
              </Button>
            )}
          </div>
        </div>
      </div>
      {status === "lokal" && onAnmelden && (
        <div className="wai-konto-menue__hinweis">
          <p>
            <strong>{l.anmeldenHinweisTitel}</strong> {l.anmeldenHinweis}
          </p>
          <Button
            appearance="primary"
            icon={<Icon name="logIn" />}
            onClick={onAnmelden}
          >
            {l.anmelden}
          </Button>
        </div>
      )}
      {status === "abgelaufen" && (
        <div className="wai-konto-menue__hinweis" data-art="warnung">
          <p>{l.abgelaufenHinweis}</p>
        </div>
      )}
      {(weitereProfile.length > 0 || onProfilHinzufuegen) && (
        <div className="wai-konto-menue__profile">
          {weitereProfile.map((p) => {
            const n = profilName(p, l.lokalesProfil);
            return (
              <button
                key={p.id}
                type="button"
                className="wai-konto-menue__profil"
                aria-label={
                  p.gesperrt
                    ? `${l.profilWechseln(n)}, ${l.gesperrt}`
                    : l.profilWechseln(n)
                }
                onClick={() => onProfilWechseln?.(p.id)}
                disabled={!onProfilWechseln}
              >
                <ProfilBild profil={p} groesse={24} labels={labels} />
                <span className="wai-konto-menue__profiltext">
                  {n}
                  <small>{p.anmeldung ? p.email : l.status.lokal}</small>
                </span>
                {p.gesperrt && (
                  <Icon name="lock" className="wai-konto-menue__schloss" />
                )}
              </button>
            );
          })}
          {onProfilHinzufuegen && (
            <button
              type="button"
              className="wai-konto-menue__profil"
              onClick={onProfilHinzufuegen}
            >
              <Icon name="plus" className="wai-konto-menue__plus" />
              <span className="wai-konto-menue__profiltext">
                {l.profilHinzufuegen}
              </span>
            </button>
          )}
        </div>
      )}
    </div>
  );
}

export interface ProfilknopfProps extends ProfilmenueProps {
  /** Name und Status neben dem Bild zeigen (breite Fenster). */
  mitName?: boolean;
  className?: string;
}

/**
 * Profilknopf ganz rechts in der Kopfleiste (Slot `profil` der AppShell, ADR-0024 E6). Öffnet das Profilmenü;
 * jede Aktion schließt es. Die App baut keinen eigenen Profilknopf.
 */
export function Profilknopf({
  mitName,
  className,
  ...menue
}: ProfilknopfProps) {
  const [offen, setOffen] = useState(false);
  const l = { ...KONTO_LABELS, ...menue.labels };
  const name = profilName(menue.profil, l.lokalesProfil);
  const zu = <A extends unknown[]>(f?: (...a: A) => void) =>
    f &&
    ((...a: A) => {
      setOffen(false);
      f(...a);
    });
  const tooltip = `${name} · ${l.status[menue.status]}`;
  return (
    <Popover
      open={offen}
      onOpenChange={(_, d) => setOffen(d.open)}
      positioning="below-end"
      trapFocus
    >
      <PopoverTrigger disableButtonEnhancement>
        <Tooltip content={tooltip} relationship="description" withArrow>
          <Button
            appearance="subtle"
            className={cx("wai-konto-knopf", className)}
            aria-label={l.profilmenue(name)}
            aria-expanded={offen}
            data-status={menue.status}
          >
            {mitName && (
              <span className="wai-konto-knopf__name">
                {name}
                <small>{l.status[menue.status]}</small>
              </span>
            )}
            <ProfilBild
              profil={menue.profil}
              groesse={28}
              status={menue.status}
              labels={menue.labels}
            />
          </Button>
        </Tooltip>
      </PopoverTrigger>
      <PopoverSurface
        className="wai-konto-menue__flaeche"
        aria-label={l.profilmenue(name)}
      >
        <Profilmenue
          {...menue}
          onMeinProfil={zu(menue.onMeinProfil)}
          onKontoGeraete={zu(menue.onKontoGeraete)}
          onAnmelden={zu(menue.onAnmelden)}
          onAbmelden={zu(menue.onAbmelden)}
          onProfilWechseln={zu(menue.onProfilWechseln)}
          onProfilHinzufuegen={zu(menue.onProfilHinzufuegen)}
        />
      </PopoverSurface>
    </Popover>
  );
}
