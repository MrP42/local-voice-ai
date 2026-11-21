import React, { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { MAX_NAME, sauberName } from "@/components/voice-call/profil/verwalten";
import { zeichen } from "@/components/voice-call/profil/editor";

interface NameDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titel: string;
  /** Erklärung unter dem Titel (optional). */
  hinweis?: string;
  /** Beschriftung des Knopfes, der bestätigt („Anlegen“, „Kopieren“, „Umbenennen“). */
  bestaetigen: string;
  initial: string;
  /** Führt den Vorgang aus; `true`, wenn er gelang (der Dialog schließt), sonst bleibt er offen und zeigt `vorgangFehler`. */
  onBestaetigen: (name: string) => Promise<boolean>;
  /** Warum der Vorgang scheiterte (ein Satz); steht im Dialog, denn die Seite dahinter ist dann nicht erreichbar. */
  vorgangFehler?: string | null;
}

/**
 * Nach einem Namen fragen (neuer Assistent, Kopie, Umbenennen). Die Länge (1 bis 60 Zeichen) steht am Feld, bevor etwas
 * geschrieben wird; die Id leitet sich beim Anlegen aus dem Namen ab und ändert sich beim Umbenennen nie.
 */
export const NameDialog: React.FC<NameDialogProps> = ({
  open,
  onOpenChange,
  titel,
  hinweis,
  bestaetigen,
  initial,
  onBestaetigen,
  vorgangFehler = null,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState(initial);
  const [laeuft, setLaeuft] = useState(false);
  // Der Fehler des Vorgangs gilt für den Namen, der ihn auslöste: wer den Namen ändert, sieht ihn nicht mehr.
  const [vorgangSichtbar, setVorgangSichtbar] = useState(true);
  const feld = useRef<HTMLInputElement>(null);
  const feldId = useId();
  const fehlerId = useId();

  useEffect(() => {
    if (open) {
      setName(initial);
      setLaeuft(false);
    }
  }, [open, initial]);
  useEffect(() => setVorgangSichtbar(true), [vorgangFehler]);

  const sauber = sauberName(name);
  const n = zeichen(sauber);
  const gueltig = n >= 1 && n <= MAX_NAME;
  const eingabeFehler =
    n > MAX_NAME ? t("assistenten.name.zuLang", { max: MAX_NAME }) : "";
  const fehler =
    eingabeFehler || (vorgangSichtbar ? (vorgangFehler ?? "") : "");

  const los = async () => {
    if (!gueltig || laeuft) return;
    setLaeuft(true);
    setVorgangSichtbar(true);
    const ok = await onBestaetigen(sauber);
    setLaeuft(false);
    if (ok) onOpenChange(false);
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={titel}
      description={hinweis}
      closeLabel={t("assistenten.dialog.schliessen")}
      className="max-w-md"
      initialFocusRef={feld}
      footer={
        <>
          <Button
            variant="primary"
            onClick={() => void los()}
            disabled={!gueltig || laeuft}
            data-testid="assistent-dialog-bestaetigen"
          >
            {bestaetigen}
          </Button>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("assistenten.dialog.abbrechen")}
          </Button>
        </>
      }
    >
      <div className="space-y-1" data-testid="assistent-name-dialog">
        <label htmlFor={feldId} className="text-sm font-semibold">
          {t("assistenten.name.label")}
        </label>
        <Input
          id={feldId}
          ref={feld}
          value={name}
          aria-invalid={fehler ? true : undefined}
          aria-describedby={fehler ? fehlerId : undefined}
          onChange={(e) => {
            setName(e.target.value);
            setVorgangSichtbar(false);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              void los();
            }
          }}
          data-testid="assistent-name-eingabe"
        />
        {fehler && (
          <p
            id={fehlerId}
            className="lv-asst-fehlerzeile"
            role="alert"
            data-testid="assistent-name-fehler"
          >
            {fehler}
          </p>
        )}
      </div>
    </Dialog>
  );
};

interface BestaetigenProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  titel: string;
  text: React.ReactNode;
  /** Der Knopf, der den Vorgang auslöst („Löschen“, „Verwerfen“). */
  bestaetigen: string;
  /** Der andere Knopf („Abbrechen“, „Weiter bearbeiten“). */
  abbrechen: string;
  gefaehrlich?: boolean;
  onBestaetigen: () => void | Promise<void>;
  testId: string;
}

/** Eine Rückfrage mit zwei Knöpfen. Der Fokus beginnt beim sicheren Knopf („Abbrechen“), nicht beim zerstörenden. */
export const BestaetigenDialog: React.FC<BestaetigenProps> = ({
  open,
  onOpenChange,
  titel,
  text,
  bestaetigen,
  abbrechen,
  gefaehrlich = false,
  onBestaetigen,
  testId,
}) => {
  const { t } = useTranslation();
  const sicher = useRef<HTMLButtonElement>(null);
  const [laeuft, setLaeuft] = useState(false);
  useEffect(() => {
    if (open) setLaeuft(false);
  }, [open]);
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={titel}
      closeLabel={t("assistenten.dialog.schliessen")}
      className="max-w-md"
      initialFocusRef={sicher}
      footer={
        <>
          <Button
            variant={gefaehrlich ? "danger" : "primary"}
            disabled={laeuft}
            onClick={() => {
              setLaeuft(true);
              void Promise.resolve(onBestaetigen()).finally(() =>
                setLaeuft(false),
              );
            }}
            data-testid={`${testId}-bestaetigen`}
          >
            {bestaetigen}
          </Button>
          <Button
            ref={sicher}
            variant="secondary"
            onClick={() => onOpenChange(false)}
            data-testid={`${testId}-abbrechen`}
          >
            {abbrechen}
          </Button>
        </>
      }
    >
      <div className="text-sm space-y-2" data-testid={testId}>
        {text}
      </div>
    </Dialog>
  );
};
