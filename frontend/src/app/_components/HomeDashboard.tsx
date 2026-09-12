'use client';

/**
 * HomeDashboard — Phase C (docs/DESIGN_READAI.md): the "For You" analogue, shown on
 * the home route while idle (no recording in progress, no live transcript). Recent
 * meetings render as report cards; clicking one opens its meeting report. Purely
 * on-device: it reads the already-loaded meetings list, nothing else.
 *
 * The wire (`api_get_meetings`) carries only { id, title }, so the date meta is
 * best-effort parsed from auto-generated titles ("Meeting YYYY-MM-DD_HH-MM-SS");
 * renamed meetings simply show no date. Deliberately no fabricated metrics.
 */

import { useEffect, useState } from 'react';
import { useRouter } from 'next/navigation';
import { HomeOverview } from '@/components/HomeOverview';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { summaryDraftService, OpenActionItem } from '@/services/summaryDraftService';
import { isTauri } from '@/lib/isTauri';
import { uiI18n } from '@/i18n';
import { useImportDialog } from '@/contexts/ImportDialogContext';



function parseTitleDate(title: string): string | null {
  const m = title.match(/(\d{4})-(\d{2})-(\d{2})_(\d{2})-(\d{2})-(\d{2})/);
  if (!m) return null;
  const d = new Date(
    Number(m[1]), Number(m[2]) - 1, Number(m[3]),
    Number(m[4]), Number(m[5]), Number(m[6]),
  );
  if (Number.isNaN(d.getTime())) return null;
  return d.toLocaleDateString(uiI18n.language, {
    weekday: 'short', day: 'numeric', month: 'short',
  }) + ' · ' + d.toLocaleTimeString(uiI18n.language, { hour: '2-digit', minute: '2-digit' });
}

export function HomeDashboard({ recordingDisabled = false }: { recordingDisabled?: boolean }) {
  const router = useRouter();
  const { openImportDialog } = useImportDialog();
  const { meetings, setCurrentMeeting } = useSidebar();

  const recent = meetings.slice(0, 9);

  // Open action items across meetings (api_get_open_action_items). Failure is
  // silent by design — the dashboard degrades to meetings-only.
  const [actionItems, setActionItems] = useState<OpenActionItem[]>([]);
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    summaryDraftService
      .getOpenActionItems(8)
      .then((items) => { if (!cancelled) setActionItems(items); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [meetings.length]);

  const openMeeting = (id: string, title: string) => {
    setCurrentMeeting({ id, title });
    router.push(`/meeting-details?id=${id}`);
  };

  return <HomeOverview meetings={recent.map(m => ({ ...m, date: parseTitleDate(m.title) }))} total={meetings.length} actions={actionItems} onOpen={openMeeting}
    onRecord={() => window.dispatchEvent(new CustomEvent('start-recording-from-sidebar'))}
    recordingDisabled={recordingDisabled} onImport={() => openImportDialog()}
    onActions={() => router.push('/actions')} onSettings={() => router.push('/settings')} />;
}
