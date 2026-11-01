/**
 * Kopie aus @wai/ui konto (wai-sdk PR #991, ADR-0024). Nicht vom Original abweichen; Änderungen dort machen
 * und neu kopieren. Abweichungen hier: `Icon` -> lucide-react (siehe `Icon.tsx`), `cx` -> `cx.ts`.
 */
import { Button, Field, Input } from "@fluentui/react-components";
import { type FormEvent, useEffect, useId, useRef, useState } from "react";
import { Icon } from "./Icon";
import { KONTO_LABELS, type KontoLabels, ProfilBild } from "./Profilknopf";
import {
  type Anmeldestatus,
  aenderungen,
  feldGesperrt,
  type KontoFeld,
  type KontoProfil,
  MAX_BILD_BYTES,
  type ProfilAenderung,
  profilName,
} from "./profil";

export interface ProfilseiteLabels extends KontoLabels {
  beschreibung: string;
  bildAendern: string;
  bildEntfernen: string;
  bildZuGross: string;
  bildTyp: string;
  bildFehler: string;
  kontaktdaten: string;
  speichern: string;
  gespeichert: string;
  vonOrganisation: string;
  herkunft: (anbieter: string) => string;
  felder: Record<KontoFeld, string>;
  anmeldung: string;
  anmeldungLokal: string;
  anmeldungLokalNutzen: string;
  anmeldungAktiv: string;
  anmeldungAbgelaufen: string;
  anbieter: string;
  seit: string;
  apps: string;
  emailUngueltig: string;
}

export const PROFILSEITE_LABELS: ProfilseiteLabels = {
  ...KONTO_LABELS,
  beschreibung: "Ihre Angaben gelten in allen Apps der Suite.",
  bildAendern: "Bild ändern",
  bildEntfernen: "Bild entfernen",
  bildZuGross: "Das Bild ist größer als 256 KB.",
  bildTyp: "Bitte ein PNG-Bild wählen.",
  bildFehler: "Das Bild konnte nicht gespeichert werden.",
  kontaktdaten: "Kontaktdaten",
  speichern: "Speichern",
  gespeichert: "Gespeichert.",
  vonOrganisation: "Von Ihrer Organisation",
  herkunft: (anbieter) =>
    `Felder mit Schloss kommen von Ihrem Konto bei ${anbieter} und lassen sich nur dort ändern. Alle anderen Angaben pflegen Sie hier; sie gelten in allen Apps.`,
  felder: {
    anzeigename: "Anzeigename",
    vorname: "Vorname",
    nachname: "Nachname",
    email: "E-Mail",
    telefon: "Telefon",
    mobil: "Mobil",
    position: "Position",
    abteilung: "Abteilung",
    organisation: "Organisation",
    standort: "Standort",
  },
  anmeldung: "Anmeldung",
  anmeldungLokal: "Lokales Profil – Ihre Daten bleiben auf diesem PC.",
  anmeldungLokalNutzen:
    "Mit Anmeldung: Abgleich über Geräte, geteilte Unterhaltungen und Teamfunktionen – einmal anmelden, in allen Apps.",
  anmeldungAktiv: "Gilt für alle Apps der Suite auf diesem PC.",
  anmeldungAbgelaufen: "Abgelaufen – lokal geht alles weiter.",
  anbieter: "Anmeldedienst",
  seit: "Angemeldet seit",
  apps: "Apps",
  emailUngueltig: "Bitte eine gültige E-Mail-Adresse eingeben.",
};

const FELDER: readonly KontoFeld[] = [
  "anzeigename",
  "vorname",
  "nachname",
  "email",
  "telefon",
  "mobil",
  "position",
  "abteilung",
  "organisation",
  "standort",
];
/** `wai-konto` speichert nur PNG; andere Formate rechnet der Host vorher um oder lehnt sie ab. */
const TYPEN = ["image/png"];
const MAX_LAENGE: Record<KontoFeld, number> = {
  anzeigename: 100,
  vorname: 100,
  nachname: 100,
  email: 254,
  telefon: 40,
  mobil: 40,
  position: 100,
  abteilung: 100,
  organisation: 100,
  standort: 100,
};

export interface ProfilseiteProps {
  profil: KontoProfil;
  status: Anmeldestatus;
  /** Apps, die diese Anmeldung gerade nutzen (Anzeige). */
  apps?: readonly string[];
  /** Speichert nur geänderte, nicht schreibgeschützte Felder. Fehlertext zurückgeben oder werfen. */
  onSpeichern: (aenderung: ProfilAenderung) => Promise<void> | void;
  /** Bild wurde gewählt und vorgeprüft (Typ, Größe); der Host speichert es über `wai-konto`. */
  onBild?: (bild: File) => Promise<void> | void;
  onBildEntfernen?: () => void;
  onAnmelden?: () => void;
  onAbmelden?: () => void;
  onKontoGeraete?: () => void;
  /** Datumsformat für „Angemeldet seit“. */
  formatDatum?: (ms: number) => string;
  labels?: Partial<ProfilseiteLabels>;
}

function standardDatum(ms: number) {
  return new Intl.DateTimeFormat("de-DE", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(ms);
}

function entwurfAus(p: KontoProfil): Record<KontoFeld, string> {
  return Object.fromEntries(FELDER.map((f) => [f, p[f] ?? ""])) as Record<
    KontoFeld,
    string
  >;
}

/** Seite „Mein Profil“ (Variante A, Kartenspalte; ADR-0024 E6). Kein `h1` – den Titel setzt die Kopfleiste. */
export function Profilseite({
  profil,
  status,
  apps = [],
  onSpeichern,
  onBild,
  onBildEntfernen,
  onAnmelden,
  onAbmelden,
  onKontoGeraete,
  formatDatum = standardDatum,
  labels,
}: ProfilseiteProps) {
  const l = { ...PROFILSEITE_LABELS, ...labels };
  const [entwurf, setEntwurf] = useState(() => entwurfAus(profil));
  const [meldung, setMeldung] = useState<{
    art: "ok" | "fehler";
    text: string;
  } | null>(null);
  const [laeuft, setLaeuft] = useState(false);
  const datei = useRef<HTMLInputElement>(null);
  const id = useId();

  // Profilwechsel: alles neu. Fremdänderung (andere App): nur Felder übernehmen, die hier nicht bearbeitet werden.
  const vorher = useRef(profil);
  // biome-ignore lint/correctness/useExhaustiveDependencies: nur bei Wechsel von Profil oder Stammdaten
  useEffect(() => {
    const alt = vorher.current;
    vorher.current = profil;
    if (alt.id !== profil.id) {
      setEntwurf(entwurfAus(profil));
      return;
    }
    setEntwurf((e) => {
      const neu = { ...e };
      for (const f of FELDER)
        if (e[f] === (alt[f] ?? "")) neu[f] = profil[f] ?? "";
      return neu;
    });
  }, [profil.id, ...FELDER.map((f) => profil[f])]);

  const diff = aenderungen(profil, status, entwurf);
  const geaendert = Object.keys(diff).length > 0;
  const emailNeu = entwurf.email.trim();
  // Gleiche Regel wie `wai-konto` (genau ein @, beide Seiten nicht leer); nur geänderte, freie Werte prüfen.
  const emailFehler =
    !feldGesperrt(profil, status, "email") &&
    emailNeu !== "" &&
    emailNeu !== (profil.email ?? "") &&
    !/^[^\s@]+@[^\s@]+$/.test(emailNeu);
  const name = profilName(profil, l.lokalesProfil);

  async function speichern(e: FormEvent) {
    e.preventDefault();
    if (!geaendert || emailFehler) return;
    setLaeuft(true);
    setMeldung(null);
    try {
      await onSpeichern(diff);
      setMeldung({ art: "ok", text: l.gespeichert });
    } catch (err) {
      setMeldung({
        art: "fehler",
        text: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setLaeuft(false);
    }
  }

  async function bildGewaehlt(f: File | undefined) {
    if (datei.current) datei.current.value = "";
    if (!f || !onBild) return;
    if (!TYPEN.includes(f.type))
      return setMeldung({ art: "fehler", text: l.bildTyp });
    if (f.size > MAX_BILD_BYTES)
      return setMeldung({ art: "fehler", text: l.bildZuGross });
    setMeldung(null);
    try {
      await onBild(f);
    } catch (err) {
      setMeldung({
        art: "fehler",
        text: err instanceof Error ? err.message : l.bildFehler,
      });
    }
  }

  const statusText =
    status === "lokal"
      ? l.anmeldungLokal
      : status === "abgelaufen"
        ? l.anmeldungAbgelaufen
        : l.anmeldungAktiv;

  return (
    <div className="wai-profilseite">
      <p className="wai-profilseite__beschreibung">{l.beschreibung}</p>

      <section
        className="wai-profilseite__karte wai-profilseite__held"
        aria-label={name}
      >
        <ProfilBild profil={profil} groesse={96} labels={l} />
        <div className="wai-profilseite__wer">
          <div className="wai-profilseite__name">{name}</div>
          {profil.email && (
            <div className="wai-profilseite__email">{profil.email}</div>
          )}
          <span className="wai-profilseite__status" data-status={status}>
            <i aria-hidden="true" />
            {status === "angemeldet" && profil.anmeldung
              ? `${l.status.angemeldet} · ${profil.anmeldung.anbieter}`
              : l.status[status]}
          </span>
          {onBild && (
            <div className="wai-profilseite__aktionen">
              <Button
                icon={<Icon name="camera" />}
                onClick={() => datei.current?.click()}
              >
                {l.bildAendern}
              </Button>
              {profil.bild && onBildEntfernen && (
                <Button onClick={onBildEntfernen}>{l.bildEntfernen}</Button>
              )}
              <input
                ref={datei}
                type="file"
                accept={TYPEN.join(",")}
                hidden
                onChange={(e) => bildGewaehlt(e.target.files?.[0])}
                data-testid="profilbild-datei"
              />
            </div>
          )}
        </div>
      </section>

      <form
        className="wai-profilseite__karte"
        onSubmit={speichern}
        aria-labelledby={`${id}-kd`}
      >
        <div className="wai-profilseite__kopf">
          <h2 id={`${id}-kd`}>{l.kontaktdaten}</h2>
          <Button
            type="submit"
            appearance="primary"
            disabled={!geaendert || emailFehler || laeuft}
          >
            {l.speichern}
          </Button>
        </div>
        {profil.anmeldung && profil.anmeldung.idpFelder.length > 0 && (
          <p className="wai-profilseite__info" data-art="herkunft">
            <Icon name="lock" aria-hidden="true" />{" "}
            {l.herkunft(profil.anmeldung.anbieter)}
          </p>
        )}
        {meldung && (
          <p
            className="wai-profilseite__meldung"
            data-art={meldung.art}
            role={meldung.art === "fehler" ? "alert" : "status"}
          >
            {meldung.text}
          </p>
        )}
        {FELDER.map((f) => {
          const gesperrt = feldGesperrt(profil, status, f);
          return (
            <Field
              key={f}
              className="wai-profilseite__zeile"
              label={l.felder[f]}
              orientation="horizontal"
              hint={gesperrt ? l.vonOrganisation : undefined}
              validationState={
                f === "email" && emailFehler ? "error" : undefined
              }
              validationMessage={
                f === "email" && emailFehler ? l.emailUngueltig : undefined
              }
            >
              <Input
                value={entwurf[f]}
                readOnly={gesperrt}
                maxLength={MAX_LAENGE[f]}
                type={
                  f === "email"
                    ? "email"
                    : f === "telefon" || f === "mobil"
                      ? "tel"
                      : "text"
                }
                contentAfter={gesperrt ? <Icon name="lock" /> : undefined}
                onChange={(_, d) => setEntwurf((v) => ({ ...v, [f]: d.value }))}
              />
            </Field>
          );
        })}
      </form>

      <section className="wai-profilseite__karte" aria-labelledby={`${id}-an`}>
        <div className="wai-profilseite__kopf">
          <Icon name="shield" className="wai-profilseite__symbol" />
          <div>
            <h2 id={`${id}-an`}>{l.anmeldung}</h2>
            <div className="wai-profilseite__unter">{statusText}</div>
          </div>
          <div className="wai-profilseite__rechts">
            {status !== "angemeldet" && onAnmelden && (
              <Button
                appearance="primary"
                icon={<Icon name="logIn" />}
                onClick={onAnmelden}
              >
                {status === "abgelaufen" ? l.erneutAnmelden : l.anmelden}
              </Button>
            )}
            {status !== "lokal" && onAbmelden && (
              <Button icon={<Icon name="logOut" />} onClick={onAbmelden}>
                {l.abmelden}
              </Button>
            )}
          </div>
        </div>
        {status === "lokal" ? (
          <p className="wai-profilseite__info">{l.anmeldungLokalNutzen}</p>
        ) : (
          <dl className="wai-profilseite__liste">
            {profil.anmeldung && (
              <>
                <dt>{l.anbieter}</dt>
                <dd>{profil.anmeldung.anbieter}</dd>
              </>
            )}
            {profil.anmeldung?.seitMs !== undefined && (
              <>
                <dt>{l.seit}</dt>
                <dd>{formatDatum(profil.anmeldung.seitMs)}</dd>
              </>
            )}
            {apps.length > 0 && (
              <>
                <dt>{l.apps}</dt>
                <dd>{apps.join(", ")}</dd>
              </>
            )}
          </dl>
        )}
        {onKontoGeraete && (
          <div className="wai-profilseite__fuss">
            <Button
              appearance="transparent"
              icon={<Icon name="devices" />}
              onClick={onKontoGeraete}
            >
              {l.kontoGeraete}
            </Button>
          </div>
        )}
      </section>
    </div>
  );
}
