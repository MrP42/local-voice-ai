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
  /**
   * Die Vorschau ist gesperrt (etwa weil ein Gespraech laeuft: Sprachdienst und Ausgabe gehoeren dann dem Gespraech): der
   * Knopf ist aus, und eine Vorschau, die noch erzeugt wird, spielt danach nicht mehr. Ohne Angabe nie gesperrt.
   */
  gesperrt?: boolean;
  /** Der Nutzer startet eine Vorschau (Klick auf „Erzeugen“ oder „Abspielen“), noch bevor etwas erzeugt oder gespielt wird. */
  beiStart?: () => void;
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

/** Haelt die laufende Vorschau an (von aussen, etwa wenn ein Gespraech beginnt oder eine andere Hoerprobe startet). */
export const stoppeVorschau = stopCurrent;

export const VoicePreviewButton = ({
  voiceId,
  refreshKey,
  className = "",
  gesperrt = false,
  beiStart,
}: VoicePreviewButtonProps) => {
  const { t } = useTranslation();
  const [cached, setCached] = useState<VoiceSample | null>(null);
  const [generating, setGenerating] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  // Fuer das Ende einer Erzeugung: Sperre und Einhaengen gelten zu dem Zeitpunkt, an dem sie fertig wird, nicht beim Klick.
  const gesperrtRef = useRef(gesperrt);
  gesperrtRef.current = gesperrt;
  const lebtRef = useRef(true);

  useEffect(() => {
    let cancelled = false;
    void commands.ttsVoiceDemoCached(voiceId).then((sample) => {
      if (!cancelled) setCached(sample ?? null);
    });
    return () => {
      cancelled = true;
    };
  }, [voiceId, refreshKey]);

  useEffect(() => {
    lebtRef.current = true;
    return () => {
      lebtRef.current = false;
      if (current?.audio === audioRef.current) stopCurrent();
    };
  }, []);

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
    if (gesperrtRef.current) return;
    beiStart?.();
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
    // Hat sich unterdessen ein Gespraech gestartet oder wurde die Ansicht verlassen, bleibt es still (die Vorschau liegt dann
    // zwischengespeichert bereit).
    if (gesperrtRef.current || !lebtRef.current) return;
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
      disabled={generating || (gesperrt && !playing)}
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
