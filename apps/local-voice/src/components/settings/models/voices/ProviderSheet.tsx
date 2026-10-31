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
import { onSampleStart, stopSample } from "@/lib/voices/samplePlayer";
import { Button } from "../../../ui/Button";
import { DialogPortalContext } from "../../../ui/Dialog";
import { ProviderDetails } from "../../tts/providers/ProviderDetails";
import { ProviderRow } from "../../tts/providers/ProviderRow";
import { onTryStart, stopTry } from "../../tts/providers/voiceTry";
import { useProviderPreview } from "../../tts/providers/useProviderPreview";
import type { TtsProvidersState } from "../../tts/providers/useTtsProviders";
import { ProviderErrors } from "./ProviderErrors";
import { PhononModels } from "../PhononModels";
import { PhononSetup } from "../../tts/providers/setup/PhononSetup";
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
  const { cancel: cancelPreview } = preview;

  // Seitenblatt zu: eine laufende Anbieter-Probestimme nicht weiterspielen lassen.
  useEffect(() => {
    if (!open) void cancelPreview();
  }, [open, cancelPreview]);

  // Eine Probe zur Zeit: startet eine Karten-Hoerprobe, endet die Anbieter-Probestimme.
  useEffect(() => {
    const offSample = onSampleStart(() => void cancelPreview());
    const offTry = onTryStart(() => void cancelPreview());
    return () => {
      offSample();
      offTry();
    };
  }, [cancelPreview]);

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
              <ProviderErrors
                providers={providers}
                testIdPrefix="voice-provider-sheet"
              />
              {id === "phonon" && (
                <PhononSetup entry={entry} providers={providers} />
              )}
              <div className="rounded-lg border border-mid-gray/20">
                <ProviderRow
                  entry={entry}
                  compact
                  approvalBySetup={id === "phonon"}
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
                    onPreview={(text, voice) => {
                      stopTry();
                      stopSample();
                      void preview.start(id, text, voice);
                    }}
                    onCancelPreview={() => void cancelPreview()}
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
                <details
                  className="group rounded-lg border border-mid-gray/20 px-3"
                  data-testid="phonon-downloads-details"
                >
                  <summary className="flex min-h-[44px] cursor-pointer list-none items-center gap-2 text-sm font-medium md:min-h-8">
                    <span
                      aria-hidden="true"
                      className="inline-block w-3 text-mid-gray transition-transform group-open:rotate-90"
                    >
                      ›
                    </span>
                    {t(
                      "settings.models.voiceManager.providers.downloadsSingle",
                    )}
                  </summary>
                  <section
                    className="pb-3"
                    aria-label={t(
                      "settings.models.voiceManager.providers.downloads",
                    )}
                  >
                    <PhononModels query="" />
                  </section>
                </details>
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
