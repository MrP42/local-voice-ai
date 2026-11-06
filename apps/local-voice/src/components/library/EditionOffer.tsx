import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { commands, type LibraryBook } from "@/bindings";
import { Dialog } from "../ui/Dialog";
import { Button } from "../ui/Button";
import { editionsOf, langName } from "@/lib/library/editions";
import {
  describeBook,
  joinEditions,
  languagePairs,
} from "@/lib/library/editionOps";
import "./sprachfassungen.css";

type Choice = "edition" | "own" | "cancel";

interface Offer {
  added: LibraryBook;
  existing: LibraryBook;
  sure: boolean;
  resolve(choice: Choice): void;
}

/**
 * Import (sf-03): Passt das eben aufgenommene Buch als andere Sprache zu einem
 * vorhandenen, fragt diese Rückfrage, ob es dessen Sprachfassung wird. Ohne
 * Antwort geschieht nichts: das Buch bleibt ein eigenes Buch. Aufgerufen wird
 * sie nach dem Aufnehmen (die Sprache der Datei steht erst dann am Buch);
 * „Abbrechen“ lässt das Buch also stehen, es entfernt es nicht.
 */
export function useEditionOffer(onChanged: () => Promise<void>) {
  const { t } = useTranslation();
  const [offer, setOffer] = useState<Offer | null>(null);
  const [busy, setBusy] = useState(false);
  const [choice, setChoice] = useState<"edition" | "own">("edition");
  const pending = useRef<((c: Choice) => void) | null>(null);
  const gone = useRef(false);

  useEffect(() => {
    gone.current = false;
    return () => {
      gone.current = true;
      pending.current?.("cancel");
    };
  }, []);

  const offerFor = useCallback(
    /** Liefert die Kennung des Buchs, das nach der Antwort in der Bibliothek steht. */
    async (added: LibraryBook): Promise<string> => {
      if (gone.current) return added.id;
      const pairs = await languagePairs();
      const mine = pairs
        .filter((p) => p.a === added.id || p.b === added.id)
        .sort((x, y) => Number(y.sure) - Number(x.sure))[0];
      if (!mine) return added.id;
      const otherId = mine.a === added.id ? mine.b : mine.a;
      const list = await commands.libraryList();
      const existing =
        list.status === "ok"
          ? (list.data ?? []).find((b) => b.id === otherId)
          : undefined;
      if (!existing) return added.id;
      const answer = await new Promise<Choice>((resolve) => {
        pending.current?.("cancel");
        pending.current = resolve;
        setChoice("edition");
        setOffer({ added, existing, sure: mine.sure, resolve });
      });
      pending.current = null;
      setOffer(null);
      if (answer !== "edition") return added.id;
      setBusy(true);
      let joined = false;
      try {
        await joinEditions(existing.id, [added.id]);
        joined = true;
        toast(
          t("library.sf.offer.done", {
            title: added.title,
            lang: langName(editionsOf(added)[0]?.lang),
          }),
        );
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
        if (!gone.current) await onChanged();
      }
      return joined ? existing.id : added.id;
    },
    [onChanged, t],
  );

  const kinds = {
    audio: t("library.sf.kind.audio"),
    text: t("library.sf.kind.text"),
  };
  const lang = offer ? langName(editionsOf(offer.added)[0]?.lang) : "";
  const dialog = (
    <Dialog
      open={offer !== null}
      onOpenChange={(o) => !o && offer?.resolve("cancel")}
      title={t("library.sf.offer.title", { title: offer?.added.title ?? "" })}
      closeLabel={t("library.cancel")}
      footer={
        <>
          <Button
            onClick={() => offer?.resolve("cancel")}
            data-testid="sf-offer-cancel"
          >
            {t("library.cancel")}
          </Button>
          <Button
            variant="primary"
            disabled={busy}
            onClick={() => offer?.resolve(choice)}
            data-testid="sf-offer-confirm"
          >
            {t("library.sf.offer.confirm")}
          </Button>
        </>
      }
    >
      {offer && (
        <div className="sf-offer" data-testid="sf-offer-dialog">
          <p>
            {t(
              offer.sure ? "library.sf.offer.same" : "library.sf.offer.maybe",
              {
                title: offer.existing.title,
                info: describeBook(offer.existing, kinds),
                lang,
              },
            )}
          </p>
          <fieldset className="sf-offer__options">
            <legend className="sf-visually-hidden">
              {t("library.sf.offer.legend")}
            </legend>
            {(["edition", "own"] as const).map((value) => (
              <label
                key={value}
                className="sf-option"
                data-selected={choice === value || undefined}
              >
                <input
                  type="radio"
                  name="sf-offer"
                  value={value}
                  checked={choice === value}
                  onChange={() => setChoice(value)}
                  data-testid={`sf-offer-${value}`}
                />
                <span>
                  <b>{t(`library.sf.offer.${value}`, { lang })}</b>
                  <span className="sf-option__hint">
                    {t(`library.sf.offer.${value}Hint`)}
                  </span>
                </span>
              </label>
            ))}
          </fieldset>
        </div>
      )}
    </Dialog>
  );

  return { offerFor, dialog };
}
