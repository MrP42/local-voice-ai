import type { TtsDownloadInfo } from "@/bindings";

export function selectablePiperVoices(downloads: TtsDownloadInfo[]) {
  const ready = downloads.some(
    (entry) =>
      entry.id === "piper-runtime" &&
      entry.is_downloaded &&
      !entry.is_downloading,
  );
  return ready
    ? downloads.filter((entry) => entry.kind === "voice" && entry.is_downloaded)
    : [];
}

export function voiceIsAvailable(
  value: string,
  fishInstalled: boolean,
  fishVoices: string[],
  piperVoices: { id: string }[],
  systemVoices: { name: string }[] = [],
) {
  if (value.startsWith("system:")) {
    return (
      systemVoices.length > 0 &&
      (value === "system:@default" ||
        systemVoices.some((voice) => value === "system:" + voice.name))
    );
  }
  return value.startsWith("piper:")
    ? piperVoices.some((voice) => value === `piper:${voice.id}`)
    : fishInstalled && (value === "@default" || fishVoices.includes(value));
}

export function moduleHelp(markdown: string, fishInstalled: boolean) {
  return markdown.replace(
    /<!-- module:fish -->([\s\S]*?)<!-- \/module:fish -->/g,
    (_match, content: string) => (fishInstalled ? content : ""),
  );
}
