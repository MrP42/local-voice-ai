import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Server } from "lucide-react";
import { toast } from "sonner";
import { commands, type TtsStatus } from "@/bindings";
import { useTtsModelStore } from "@/stores/ttsModelStore";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

/**
 * Der Fish-Speech-Server in der Fußleiste, links neben dem Schild (bis 0.21.6
 * oben rechts auf der Vorlesen-Seite). Ein Element trägt Zustand UND Bedienung:
 * die Farbe sagt, woran man ist — grau (aus), gelb (fährt hoch), grün (läuft),
 * orange blinkend (Fehler) —, der Klick öffnet IMMER die Rückfrage. Starten
 * belegt rund 17 GB Grafikspeicher und zwei Minuten: eine Entscheidung, keine
 * Berührung. Ohne eingerichtetes Fish erscheint das Symbol nicht.
 */
export const FishServerButton: React.FC = () => {
  const { t } = useTranslation();
  const [status, setStatus] = useState<TtsStatus | null>(null);
  const [confirm, setConfirm] = useState(false);
  const runtime = useTtsModelStore((state) => state.runtime);
  const loadRuntime = useTtsModelStore((state) => state.loadRuntime);

  useEffect(() => {
    void loadRuntime();
    void commands
      .ttsServerStatus()
      .then((r) => {
        if (r.status === "ok") setStatus(r.data);
      })
      .catch(() => {
        // Ohne Backend (Browser-Test) bleibt der Zustand unbekannt.
      });
    const un = listen<TtsStatus>("tts-state-changed", (e) =>
      setStatus(e.payload),
    );
    return () => {
      void un.then((f) => f());
    };
  }, [loadRuntime]);

  const phase = status?.phase ?? "stopped";
  // Unbekannt gilt als vorhanden (kein Aufblitzen beim Start); ein laufender
  // oder startender Server (auch ein fremd gestarteter) gilt immer als
  // eingerichtet.
  const fishReady = runtime
    ? runtime.fish.ready ||
      phase === "starting" ||
      phase === "ready" ||
      phase === "speaking"
    : true;
  if (!fishReady) return null;

  const stopped = phase === "stopped" || phase === "error";

  const iconClass =
    phase === "starting"
      ? "text-yellow-400 animate-pulse"
      : phase === "error"
        ? "text-orange-500 animate-pulse"
        : phase === "stopped"
          ? "text-text/40"
          : "text-green-500";

  const title =
    phase === "stopped"
      ? t("tts.serverIconStart")
      : phase === "starting"
        ? t("tts.serverIconStarting")
        : phase === "error"
          ? (status?.message ?? t("tts.serverIconError"))
          : t("tts.serverIconStop");

  const start = async () => {
    const result = await commands.ttsServerStart();
    if (result.status === "error") toast.error(result.error);
  };

  /** Hart beenden: was auf dem Port lauscht. „Nichts gefunden“ ist ein
   *  Ergebnis, kein Fehler. */
  const kill = async (): Promise<boolean> => {
    const result = await commands.ttsServerKill();
    if (result.status === "error") {
      toast.error(result.error);
      return false;
    }
    toast(result.data);
    return true;
  };

  /** Neu starten = beenden und sofort wieder hochfahren (Server antwortet
   *  nicht mehr vernünftig, etwa mit 500). */
  const restart = async () => {
    if (await kill()) await start();
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setConfirm(true)}
        title={title}
        aria-label={title}
        className="p-0.5 rounded hover:bg-mid-gray/20 transition-colors cursor-pointer"
        data-fish-server={phase}
      >
        <Server
          width={16}
          height={16}
          className={iconClass}
          aria-hidden="true"
        />
      </button>
      <Dialog
        open={confirm}
        onOpenChange={setConfirm}
        title={stopped ? t("tts.serverStartTitle") : t("tts.stopConfirmTitle")}
        closeLabel={t("tts.stopConfirmCancel")}
        footer={
          stopped ? (
            <>
              <Button variant="secondary" onClick={() => setConfirm(false)}>
                {t("tts.stopConfirmCancel")}
              </Button>
              <Button
                onClick={() => {
                  setConfirm(false);
                  void start();
                }}
                data-testid="fish-server-start"
              >
                {t("tts.serverStart")}
              </Button>
            </>
          ) : (
            <>
              <Button variant="secondary" onClick={() => setConfirm(false)}>
                {t("tts.stopConfirmCancel")}
              </Button>
              <Button
                variant="secondary"
                onClick={() => {
                  setConfirm(false);
                  void restart();
                }}
              >
                {t("tts.stopConfirmRestart")}
              </Button>
              <Button
                variant="danger"
                onClick={() => {
                  setConfirm(false);
                  void kill();
                }}
                data-testid="fish-server-stop"
              >
                {t("tts.stopConfirmAccept")}
              </Button>
            </>
          )
        }
      >
        <p className="text-sm text-text/80">
          {stopped
            ? t("tts.serverStartBody")
            : phase === "starting"
              ? t("tts.stopConfirmBodyStarting")
              : t("tts.stopConfirmBody")}
        </p>
      </Dialog>
    </>
  );
};

export default FishServerButton;
