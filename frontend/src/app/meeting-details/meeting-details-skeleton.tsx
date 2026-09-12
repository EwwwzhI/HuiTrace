'use client';

import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { useEffect, useState } from 'react';


export function NotePanelSkeleton({ labelKey = 'Loading summary…' }: { labelKey?: string }) {
  useUiTranslation();
  // Keep the first client render identical to the server. The language choice
  // is restored from browser/Tauri storage in an effect, so translating during
  // the initial render can otherwise change the skeleton text before hydration.
  const [isHydrated, setIsHydrated] = useState(false);
  useEffect(() => setIsHydrated(true), []);
  const label = isHydrated ? translateUI(labelKey) : labelKey;
  return (
    <div role="status" aria-label={label} className="w-full p-6">
      <span className="sr-only">{label}</span>
      <div aria-hidden="true" className="space-y-5 animate-pulse motion-reduce:animate-none">
        <div className="h-5 w-28 rounded bg-muted" />
        {[0, 1, 2].map(section => <div key={section} className="space-y-3 pt-3">
          <div className="h-3 w-full rounded bg-muted" />
          <div className="h-3 w-5/6 rounded bg-muted" />
          <div className="h-3 w-2/3 rounded bg-muted" />
        </div>)}
      </div>
    </div>
  );
}

export default function MeetingDetailsSkeleton({ meetingId }: { meetingId?: string | null }) {
  useUiTranslation();
  const { currentMeeting } = useSidebar();
  const title = currentMeeting && currentMeeting.id === meetingId ? currentMeeting.title : 'Meeting notes';
  return (
    <div className="flex h-screen flex-col bg-background" aria-busy="true">
      <header className="border-b border-border px-6 py-4">
        <h1 className="text-2xl font-semibold tracking-tight text-foreground">{title}</h1>
        <div aria-hidden="true" className="mt-3 h-4 w-44 rounded bg-muted" />
        <div aria-hidden="true" className="mt-4 flex gap-3 overflow-hidden">
          {[0, 1, 2, 3].map(tile => <div key={tile} className="h-16 w-32 shrink-0 rounded-xl border border-border bg-card" />)}
        </div>
      </header>
      <div className="flex flex-1 min-h-0">
        <div className="min-w-0 flex-1 md:border-r md:border-border"><NotePanelSkeleton labelKey="Loading transcript…" /></div>
        <div className="hidden w-1/2 min-w-[340px] max-w-[640px] md:flex"><NotePanelSkeleton /></div>
      </div>
    </div>
  );
}
