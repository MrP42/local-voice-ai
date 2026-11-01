import React, { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Switch } from "@fluentui/react-components";
import { useSettings } from "@/hooks/useSettings";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import {
  clearPairs,
  deletePair,
  listPairs,
  refreshWords,
  setPairStatus,
  type LearnedPair,
} from "./speechLearning";
import { PairLine } from "./PairLine";
import { SetupDialog } from "./SetupDialog";

/** Reiter „Erkennung“ der Gesprächseinstellungen: Lernen aus Korrekturen und Einrichtung. */
export const RecognitionLearning: React.FC = () => {
  const { t } = useTranslation();
  const { getSetting, updateSetting } = useSettings();
  const enabled = getSetting("speech_learning_enabled") ?? false;
  const auto = getSetting("speech_learning_auto_apply") ?? false;
  const [pairs, setPairs] = useState<LearnedPair[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [setupOpen, setSetupOpen] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);

  const load = useCallback(async () => {
    try {
      setPairs(await listPairs());
      setError(null);
    } catch {
      setError(t("voiceCall.learning.loadError"));
    }
  }, [t]);

  useEffect(() => {
    void load();
  }, [load]);

  const act = async (job: () => Promise<unknown>) => {
    try {
      await job();
      await refreshWords();
    } catch (e) {
      setError(String(e));
    }
    await load();
  };

  return (
    <>
      <div className="flex flex-col gap-1">
        <span className="text-[13px] font-semibold">
          {t("voiceCall.learning.enable")}
        </span>
        <Switch
          checked={enabled}
          onChange={(_e, d) =>
            void updateSetting("speech_learning_enabled", d.checked)
          }
          aria-label={t("voiceCall.learning.enable")}
          aria-describedby="learning-enable-hint"
        />
        <span id="learning-enable-hint" className="text-xs text-text/60">
          {t("voiceCall.learning.enableHint")}
        </span>
      </div>
      <div className="flex flex-col gap-1">
        <span className="text-[13px] font-semibold">
          {t("voiceCall.learning.auto")}
        </span>
        <Switch
          checked={enabled && auto}
          disabled={!enabled}
          onChange={(_e, d) =>
            void updateSetting("speech_learning_auto_apply", d.checked)
          }
          aria-label={t("voiceCall.learning.auto")}
          aria-describedby="learning-auto-hint"
        />
        <span id="learning-auto-hint" className="text-xs text-text/60">
          {t("voiceCall.learning.autoHint")}
        </span>
      </div>
      <div className="rounded-lg border border-mid-gray/25 bg-mid-gray/10 px-3 py-2.5">
        <div className="font-semibold text-[13px]">
          {t("voiceCall.learning.privacyTitle")}
        </div>
        <div className="text-xs text-text/65 mt-0.5">
          {t("voiceCall.learning.privacy")}
        </div>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="text-[13px] font-semibold">
          {t("voiceCall.learning.setupTitle")}
        </span>
        <div>
          <Button type="button" onClick={() => setSetupOpen(true)}>
            {t("voiceCall.learning.setupButton")}
          </Button>
        </div>
        <span className="text-xs text-text/60">
          {t("voiceCall.learning.setupHint")}
        </span>
      </div>
      <div className="flex flex-col gap-2">
        <div className="flex items-center gap-2">
          <h3 className="m-0 text-[13px] font-semibold">
            {t("voiceCall.learning.listTitle")}
          </h3>
          {pairs && pairs.length > 0 && (
            <Button
              size="sm"
              variant="danger-ghost"
              type="button"
              className="ms-auto"
              onClick={() => setConfirmClear(true)}
            >
              {t("voiceCall.learning.clearAll")}
            </Button>
          )}
        </div>
        {error && (
          <p
            role="alert"
            className="m-0 text-xs text-red-600 dark:text-red-400"
          >
            {error}
          </p>
        )}
        {pairs && pairs.length === 0 && !error && (
          <p className="m-0 text-xs text-text/60">
            {t("voiceCall.learning.empty")}
          </p>
        )}
        {pairs && pairs.length > 0 && (
          <ul
            className="m-0 p-0 list-none flex flex-col gap-1.5"
            aria-label={t("voiceCall.learning.listTitle")}
          >
            {pairs.map((p) => (
              <PairLine
                key={p.id}
                pair={p}
                onSet={(status) => void act(() => setPairStatus(p.id, status))}
                onDelete={() => void act(() => deletePair(p.id))}
              />
            ))}
          </ul>
        )}
      </div>
      <Dialog
        open={confirmClear}
        onOpenChange={setConfirmClear}
        title={t("voiceCall.learning.clearTitle")}
        description={t("voiceCall.learning.clearBody")}
        closeLabel={t("voiceCall.learning.cancel")}
        footer={
          <>
            <Button
              variant="danger"
              type="button"
              onClick={() => {
                setConfirmClear(false);
                void act(clearPairs);
              }}
            >
              {t("voiceCall.learning.clearYes")}
            </Button>
            <Button type="button" onClick={() => setConfirmClear(false)}>
              {t("voiceCall.learning.cancel")}
            </Button>
          </>
        }
      >
        {null}
      </Dialog>
      <SetupDialog
        open={setupOpen}
        onClose={() => setSetupOpen(false)}
        onChanged={() => void load()}
      />
    </>
  );
};
