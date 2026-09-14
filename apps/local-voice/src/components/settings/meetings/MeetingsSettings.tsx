import React, { useState } from "react";
import { useTranslation } from "react-i18next";
import { PageShell } from "../../ui/PageShell";
import type { Meeting } from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { LiveTranscript } from "./LiveTranscript";
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
      <LiveTranscript />
      <MeetingList onSelect={setSelected} />
    </PageShell>
  );
};
