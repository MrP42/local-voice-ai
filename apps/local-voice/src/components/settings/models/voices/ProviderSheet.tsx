import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button as FluentButton,
  DrawerBody,
  DrawerHeader,
  DrawerHeaderTitle,
  OverlayDrawer,
} from "@fluentui/react-components";
import { ExternalLink, X } from "lucide-react";
import { openSettingsTab } from "@/lib/openSettingsTab";
import { Button } from "../../../ui/Button";
import { DialogPortalContext } from "../../../ui/Dialog";
import { ProviderDetails } from "../../tts/providers/ProviderDetails";
import { ProviderRow } from "../../tts/providers/ProviderRow";
import { useProviderPreview } from "../../tts/providers/useProviderPreview";
import type { TtsProvidersState } from "../../tts/providers/useTtsProviders";
import { PhononModels } from "../PhononModels";
import { VoicesSection } from "../VoicesSection";

/**
 * Seitenblatt eines Anbieters: genau das, was bisher unter Einstellungen >
 * Ausgabe > Sprachausgabe-Anbieter stand (Zustand, Freigaben, Einstellungen,
 * Stimmenwahl, Probestimme, eigene Stimmen) -- dazu die Downloads des Anbieters
 * (Piper-Stimmen und -Programm, Phonon-Modelle), damit nichts nur noch ueber die
 * Karten einer Stimme erreichbar ist.
 */
export const ProviderSheet: React.FC<{
  providers: TtsProvidersState;
  providerId: string | null;
  onClose: () => void;
}> = ({ providers, providerId, onClose }) => {
  const { t } = useTranslation();
  const preview = useProviderPreview();
  const [expanded, setExpanded] = useState(true);
  const [surface, setSurface] = useState<HTMLElement | null>(null);
  const entry =
    providers.entries?.find((e) => e.descriptor.id === providerId) ?? null;

  useEffect(() => {
    setExpanded(true);
  }, [providerId]);

  const open = providerId !== null && entry !== null;
  const id = providerId ?? "";

  return (
    <OverlayDrawer
      open={open}
      position="end"
      surfaceMotion={null}
      backdropMotion={null}
      onOpenChange={(_e, data) => {
        if (!data.open) onClose();
      }}
      aria-label={entry?.descriptor.label}
      style={{ width: "min(100vw, 44rem)" }}
      className="voice-provider-sheet"
    >
      <DrawerHeader>
        <DrawerHeaderTitle
          action={
            <FluentButton
              appearance="subtle"
              aria-label={t("tts.providers.dialog.close")}
              data-testid="voice-provider-sheet-close"
              icon={<X size={20} aria-hidden="true" />}
              onClick={onClose}
            />
          }
        >
          {entry?.descriptor.label}
        </DrawerHeaderTitle>
      </DrawerHeader>
      <DrawerBody>
        {entry && (
          <DialogPortalContext.Provider value={surface}>
            <div
              ref={setSurface}
              className="space-y-4"
              data-testid="voice-provider-sheet"
            >
              <p className="text-xs text-text/60">
                {t("settings.models.voiceManager.providers.sheetHint")}
              </p>
              <div className="rounded-lg border border-mid-gray/20">
                <ProviderRow
                  entry={entry}
                  expanded={expanded}
                  busy={providers.busy === id}
                  onToggleExpanded={() => setExpanded((v) => !v)}
                  onEnable={(on) => void providers.enable(id, on)}
                  onRecheck={() => void providers.recheck(id)}
                  onApprove={(kind) => providers.setDialog({ kind, id })}
                  onRevokeRemote={() => void providers.revokeRemote(id)}
                >
                  <ProviderDetails
                    entry={entry}
                    preview={preview.state}
                    onPreview={(text, voice) =>
                      void preview.start(id, text, voice)
                    }
                    onCancelPreview={() => void preview.cancel()}
                    onChanged={providers.changed}
                  />
                </ProviderRow>
              </div>
              {id === "piper" && (
                <section
                  aria-label={t(
                    "settings.models.voiceManager.providers.downloads",
                  )}
                >
                  <VoicesSection query="" scope="piper" />
                </section>
              )}
              {id === "phonon" && (
                <section
                  aria-label={t(
                    "settings.models.voiceManager.providers.downloads",
                  )}
                >
                  <PhononModels query="" />
                </section>
              )}
              <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-dashed border-mid-gray/30 px-3 py-2 text-xs text-text/60">
                <span>
                  {t("settings.models.voiceManager.providers.moreSettings")}
                </span>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  className="min-h-[44px] md:min-h-8"
                  data-testid="voice-provider-open-output"
                  onClick={() => openSettingsTab("output")}
                >
                  {t("settings.models.voiceManager.openOutput")}
                  <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                </Button>
              </div>
            </div>
          </DialogPortalContext.Provider>
        )}
      </DrawerBody>
    </OverlayDrawer>
  );
};
