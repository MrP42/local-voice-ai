import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../hooks/useSettings";

/**
 * Live microphone level, straight from the browser audio stack.
 *
 * Deliberately not fed by the backend's `mic-level` events: those only exist
 * while a dictation is being recorded, which is useless for the one job this
 * meter has — telling you whether the microphone you just picked hears
 * anything, before you dictate into it.
 *
 * Follows the microphone chosen in settings by matching its label, so the bar
 * reflects the device the app will actually record from.
 */
export const MicLevelMeter: React.FC<{ compact?: boolean }> = ({
  compact = false,
}) => {
  const { t } = useTranslation();
  const { getSetting } = useSettings();
  const selectedMic = getSetting("selected_microphone") as string | undefined;

  const [bars, setBars] = useState<number[]>(Array(24).fill(0));
  const [peak, setPeak] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const streamRef = useRef<MediaStream | null>(null);
  const ctxRef = useRef<AudioContext | null>(null);
  const rafRef = useRef<number | null>(null);
  const peakRef = useRef(0);

  useEffect(() => {
    let cancelled = false;

    const stop = () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
      streamRef.current?.getTracks().forEach((tr) => tr.stop());
      streamRef.current = null;
      ctxRef.current?.close().catch(() => {});
      ctxRef.current = null;
    };

    const start = async () => {
      try {
        // Resolve the configured device name to an id. Labels are only
        // populated once permission has been granted, so an unmatched name
        // simply falls back to the default device rather than failing.
        let deviceId: string | undefined;
        if (selectedMic && selectedMic !== "Default") {
          const devices = await navigator.mediaDevices.enumerateDevices();
          const match = devices.find(
            (d) => d.kind === "audioinput" && d.label === selectedMic,
          );
          deviceId = match?.deviceId;
        }

        const stream = await navigator.mediaDevices.getUserMedia({
          audio: deviceId ? { deviceId: { exact: deviceId } } : true,
        });
        if (cancelled) {
          stream.getTracks().forEach((tr) => tr.stop());
          return;
        }
        streamRef.current = stream;

        const ctx = new AudioContext();
        ctxRef.current = ctx;
        const source = ctx.createMediaStreamSource(stream);
        const analyser = ctx.createAnalyser();
        analyser.fftSize = 256;
        analyser.smoothingTimeConstant = 0.6;
        source.connect(analyser);

        const spectrum = new Uint8Array(analyser.frequencyBinCount);
        const time = new Uint8Array(analyser.fftSize);

        const tick = () => {
          analyser.getByteFrequencyData(spectrum);
          analyser.getByteTimeDomainData(time);

          // Speech lives in the lower bins; sampling the whole spectrum would
          // flatten the bars into noise.
          const used = Math.floor(spectrum.length * 0.55);
          const perBar = Math.max(1, Math.floor(used / 24));
          const next: number[] = [];
          for (let i = 0; i < 24; i++) {
            let sum = 0;
            for (let j = 0; j < perBar; j++) {
              sum += spectrum[i * perBar + j] ?? 0;
            }
            next.push(sum / perBar / 255);
          }
          setBars(next);

          // RMS of the waveform is the honest loudness number for the peak bar.
          let sumSquares = 0;
          for (let i = 0; i < time.length; i++) {
            const v = (time[i] - 128) / 128;
            sumSquares += v * v;
          }
          const rms = Math.sqrt(sumSquares / time.length);
          peakRef.current = Math.max(
            peakRef.current * 0.9,
            Math.min(1, rms * 4),
          );
          setPeak(peakRef.current);

          rafRef.current = requestAnimationFrame(tick);
        };
        tick();
        setError(null);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    };

    start();
    return () => {
      cancelled = true;
      stop();
    };
  }, [selectedMic]);

  if (error) {
    return (
      <p className="text-xs text-text-muted px-1">{t("micMeter.unavailable")}</p>
    );
  }

  return (
    <div className={compact ? "space-y-1.5" : "space-y-2"}>
      <div
        className="flex items-end gap-[2px]"
        style={{ height: compact ? 28 : 56 }}
      >
        {bars.map((v, i) => (
          <div
            key={i}
            className="flex-1 rounded-[2px]"
            style={{
              height: `${Math.max(6, Math.min(100, Math.pow(v, 0.6) * 130))}%`,
              background: "var(--color-logo-primary)",
              opacity: 0.35 + Math.min(0.65, v * 1.5),
              transition: "height 60ms linear",
            }}
          />
        ))}
      </div>
      <div className="h-1.5 w-full rounded-full bg-mid-gray/20 overflow-hidden">
        <div
          className="h-full rounded-full"
          style={{
            width: `${Math.min(100, peak * 100)}%`,
            background: "var(--color-logo-primary)",
            transition: "width 80ms linear",
          }}
        />
      </div>
    </div>
  );
};

export default MicLevelMeter;
