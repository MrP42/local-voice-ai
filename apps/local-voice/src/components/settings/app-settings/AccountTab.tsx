import React from "react";
import { KontoProfileCard } from "../KontoProfileCard";
import { SyncAccountCard } from "../SyncAccountCard";
import { PairPhoneCard } from "../PairPhoneCard";

/**
 * Konto & Geräte (ADR-0024 E6): die Verwaltung hinter dem Profilknopf oben rechts. Oben die
 * Profile auf diesem PC, darunter der Geräteabgleich (bisher im Reiter „Allgemein“) und
 * „Handy verbinden“ mit der Liste der verbundenen Geräte (local-voice-ai#289).
 */
export const AccountTab: React.FC = () => (
  <div className="w-full space-y-6">
    <KontoProfileCard />
    <SyncAccountCard />
    <PairPhoneCard />
  </div>
);
