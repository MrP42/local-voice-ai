import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";

interface Props {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Legt das Profil an; wirft bei einem Fehler (Text wird im Dialog gezeigt). */
  onAnlegen: (anzeigename: string) => Promise<void>;
}

/**
 * „Profil hinzufügen“ (ADR-0024 E4). Der Hinweis auf fehlende Trennung steht schon hier,
 * nicht erst nach dem Anlegen (ADR A4).
 */
export const ProfilHinzufuegenDialog: React.FC<Props> = ({
  open,
  onOpenChange,
  onAnlegen,
}) => {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [fehler, setFehler] = useState("");
  const [laeuft, setLaeuft] = useState(false);
  const feld = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setName("");
      setFehler("");
    }
  }, [open]);

  const anlegen = async () => {
    const wert = name.trim();
    if (!wert || laeuft) return;
    setLaeuft(true);
    setFehler("");
    try {
      await onAnlegen(wert);
      onOpenChange(false);
    } catch (e) {
      setFehler(e instanceof Error ? e.message : String(e));
    } finally {
      setLaeuft(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={t("konto.hinzufuegen.titel")}
      closeLabel={t("konto.hinzufuegen.schliessen")}
      className="max-w-md"
      initialFocusRef={feld}
      footer={
        <>
          <Button
            variant="primary"
            onClick={() => void anlegen()}
            disabled={!name.trim() || laeuft}
            data-testid="konto-anlegen"
          >
            {t("konto.hinzufuegen.anlegen")}
          </Button>
          <Button variant="secondary" onClick={() => onOpenChange(false)}>
            {t("konto.hinzufuegen.abbrechen")}
          </Button>
        </>
      }
    >
      <div className="space-y-3" data-testid="konto-hinzufuegen">
        <label className="block space-y-1">
          <span className="text-sm">{t("konto.hinzufuegen.name")}</span>
          <Input
            ref={feld}
            value={name}
            maxLength={100}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void anlegen();
            }}
          />
        </label>
        <p
          className="text-sm text-text-muted"
          data-testid="konto-trennung-hinweis"
        >
          {t("konto.hinzufuegen.hinweis")}
        </p>
        {fehler && (
          <p className="text-sm text-red-500" role="alert">
            {fehler}
          </p>
        )}
      </div>
    </Dialog>
  );
};
