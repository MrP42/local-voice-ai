import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Loader2, Play, Square } from "lucide-react";
import { commands, type VoiceSample } from "@/bindings";
import { Button } from "@/components/ui/Button";

/**
 * Kompakte Hoerprobe einer Stimme: ein Knopf, der abspielt oder stoppt.
 * Liegt noch keine Hoerprobe vor, erzeugt der erste Klick sie (startet
 * einmalig die Sprach-Engine) und spielt sie danach ab.
 *
 * Ersetzt den vollen Player je Stimme: die Liste mit fuenf Stimmen war
 * anderthalb Bildschirme hoch, und man vergleicht Stimmen, indem man sie
 * nacheinander anklickt — nicht, indem man in ihnen spult.
 *
 * `refreshKey` erzwingt eine neue Cache-Pruefung, z. B. wenn sich der Seed
 * der Standardstimme aendert: das Backend legt je Seed eine eigene Datei ab,
 * die Oberflaeche wusste davon aber nichts und spielte den alten Seed.
 */
interface VoicePreviewButtonProps {
  voiceId: string;
  refreshKey?: string | number;
  className?: string;
}

// Eine Hoerprobe zur Zeit: ein neuer Klick stoppt die laufende.
let current: { audio: HTMLAudioElement; stop: () => void } | null = null;

const stopCurrent = () => {
  if (current) {
    current.audio.pause();
    current.stop();
    current = null;
  }
};

export const VoicePreviewButton = ({
  voiceId,
  refreshKey,
  className = "",
}: VoicePreviewButtonProps) => {
  const { t } = useTranslation();
  const [cached, setCached] = useState<VoiceSample | null>(null);
  const [generating, setGenerating] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    void commands.ttsVoiceDemoCached(voiceId).then((sample) => {
      if (!cancelled) setCached(sample ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [voiceId, refreshKey]);

  useEffect(
    () => () => {
      if (current?.audio === audioRef.current) stopCurrent();
    },
    [],
  );

  const play = (sample: VoiceSample) => {
    stopCurrent();
    const audio = new Audio(convertFileSrc(sample.wav_path, "asset"));
    audioRef.current = audio;
    const done = () => {
      setPlaying(false);
      if (current?.audio === audio) current = null;
    };
    audio.addEventListener("ended", done);
    audio.addEventListener("error", done);
    current = { audio, stop: () => setPlaying(false) };
    setPlaying(true);
    void audio.play().catch(done);
  };

  const onClick = async () => {
    setError(null);
    if (playing) {
      stopCurrent();
      return;
    }
    if (cached) {
      play(cached);
      return;
    }
    setGenerating(true);
    const result = await commands.ttsVoiceDemo(voiceId);
    setGenerating(false);
    if (result.status === "error") {
      setError(result.error);
      return;
    }
    setCached(result.data);
    play(result.data);
  };

  const label = generating
    ? t("tts.voices.previewGenerating")
    : playing
      ? t("tts.voices.previewStop")
      : cached
        ? t("tts.voices.previewPlay")
        : t("tts.voices.previewCreate");
  const hint =
    error ?? (cached ? undefined : t("tts.voices.previewNeedsEngine"));

  return (
    <Button
      size="sm"
      variant={playing ? "primary" : "secondary"}
      onClick={() => void onClick()}
      disabled={generating}
      title={hint}
      aria-label={label}
      className={`shrink-0 ${error ? "text-red-400" : ""} ${className}`}
    >
      {generating ? (
        <Loader2 width={14} height={14} className="animate-spin" />
      ) : playing ? (
        <Square width={14} height={14} />
      ) : (
        <Play width={14} height={14} />
      )}
      {label}
    </Button>
  );
};
