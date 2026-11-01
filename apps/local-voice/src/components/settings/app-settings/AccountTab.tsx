import React from "react";
import { KontoProfileCard } from "../KontoProfileCard";
import { SyncAccountCard } from "../SyncAccountCard";

/**
 * Konto & Geräte (ADR-0024 E6): die Verwaltung hinter dem Profilknopf oben rechts. Oben die
 * Profile auf diesem PC, darunter der Geräteabgleich (bisher im Reiter „Allgemein“).
 */
export const AccountTab: React.FC = () => (
  <div className="w-full space-y-6">
    <KontoProfileCard />
    <SyncAccountCard />
  </div>
);
