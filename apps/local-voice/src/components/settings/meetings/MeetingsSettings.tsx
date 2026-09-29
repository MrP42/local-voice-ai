import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { PageShell } from "../../ui/PageShell";
import type { Meeting } from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { LiveTranscript } from "./LiveTranscript";
import { LiveNotesPad } from "./notes/LiveNotesPad";
import { MeetingList } from "./MeetingList";
import { MeetingDetail } from "./MeetingDetail";

export const MeetingsSettings: React.FC = () => {
  const { t } = useTranslation();
  const [selected, setSelected] = useState<Meeting | null>(null);

  if (selected) {
    return (
      <div className="w-full space-y-4">
        <MeetingDetail
          meeting={selected}
          onBack={() => setSelected(null)}
          onMeetingChange={setSelected}
        />
      </div>
    );
  }

  return (
    <PageShell
      title={t("workspace.recordings")}
      description={t("workspace.meetingsHint")}
      help="aufnahmen"
    >
      <RecorderCard />
      {/* Notizblock links, Transkript rechts (ab 1024 px), sonst untereinander.
          Ist nur eines von beiden sichtbar, nimmt es die ganze Breite. */}
      <div className="flex flex-col gap-4 empty:hidden lg:flex-row lg:items-start [&>*]:min-w-0 lg:[&>*]:flex-1">
        <LiveNotesPad />
        <LiveTranscript />
      </div>
      <MeetingList onSelect={setSelected} />
    </PageShell>
  );
};
