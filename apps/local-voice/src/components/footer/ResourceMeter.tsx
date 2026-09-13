import React, { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { commands, type SystemMemory } from "@/bindings";

/** Wie oft die Fußleiste nachmisst. Drei Sekunden: träge genug, um nichts
 *  zu kosten, schnell genug, um einen Modellstart zu sehen. */
const POLL_MS = 3000;

const gb = (mb: number) => (mb / 1024).toFixed(1).replace(".", ",");

/**
 * RAM und GPU-Speicher in der Fußleiste — ohne Taskmanager. Beschriftet,
 * wie es gemessen wird: Auf Windows ist die GPU-Zahl das Budget des
 * Treibers und seine Belegung, kein "freier VRAM"; eine iGPU bekommt den
 * Zusatz „gemeinsam“. Ist nichts messbar, verschwindet die Anzeige, statt
 * eine Zahl zu erfinden.
 */
export const ResourceMeter: React.FC = () => {
  const { t } = useTranslation();
  const [memory, setMemory] = useState<SystemMemory | null>(null);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const result = await commands.systemMemory();
        if (!cancelled && result.status === "ok") setMemory(result.data);
      } catch {
        // Ohne Backend (Browser-Test) bleibt die Anzeige leer.
      }
    };
    void tick();
    const timer = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, []);

  if (!memory || memory.ram_total_mb === 0) return null;

  // Die erste Grafikkarte mit eigenem Speicher — die, auf der Modelle
  // laufen. Gibt es nur eine iGPU, wird die gezeigt, aber als gemeinsam.
  const gpu =
    memory.gpus.find((g) => !g.shared) ?? memory.gpus[0] ?? null;

  return (
    <div
      className="hidden md:flex items-center gap-3 text-text/60"
      data-resource-meter
      title={t("resourceMeter.hint")}
    >
      <span className="whitespace-nowrap">
        {t("resourceMeter.ram", {
          used: gb(memory.ram_used_mb),
          total: gb(memory.ram_total_mb),
        })}
      </span>
      {gpu && (
        <span className="whitespace-nowrap">
          {t(gpu.shared ? "resourceMeter.gpuShared" : "resourceMeter.gpu", {
            used: gb(gpu.used_mb),
            total: gb(gpu.budget_mb),
          })}
        </span>
      )}
    </div>
  );
};

export default ResourceMeter;
