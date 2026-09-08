import React, { useState } from "react";
import type { Meeting } from "@/bindings";
import { RecorderCard } from "./RecorderCard";
import { LiveTranscript } from "./LiveTranscript";
import { MeetingList } from "./MeetingList";
import { MeetingDetail } from "./MeetingDetail";

export const MeetingsSettings: React.FC = () => {
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
    <div className="w-full space-y-4">
      <RecorderCard />
      <LiveTranscript />
      <MeetingList onSelect={setSelected} />
    </div>
  );
};
