'use client';

/**
 * ReportHeader — the read.ai-style header for a meeting (see docs/DESIGN_READAI.md,
 * prototype at /design/report). Renders the meeting title, date + duration meta, and
 * an on-device overview-metrics strip (Duration / Words / Segments / Action items).
 *
 * All metrics are computed locally from the already-loaded transcript — nothing
 * leaves the device, no charisma/bias inference (per the four guardrails). Styled
 * entirely with semantic tokens, so it adapts light/dark.
 */

import { Clock, FileText, Hash, ListChecks } from 'lucide-react';
import type { Transcript } from '@/types';
import { translateUI, uiI18n } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



function formatDuration(sec: number): string {
  if (!sec || sec < 1) return '—';
  const h = Math.floor(sec / 3600);
  const m = Math.floor((sec % 3600) / 60);
  const s = Math.floor(sec % 60);
  if (h > 0) return `${h}${translateUI("hours short")} ${m}${translateUI("minutes short")}`;
  if (m > 0) return `${m}${translateUI("minutes short")} ${s}${translateUI("seconds short")}`;
  return `${s}${translateUI("seconds short")}`;
}

function StatTile({ icon: Icon, label, value, sub }: { icon: any; label: string; value: string; sub?: string }) {
  useUiTranslation();
  // Compact, content-hugging stat: icon tile + stacked value/label. Deliberately
  // NOT flex-1 — stretched, mostly-empty tiles read as dead space on wide windows.
  // shrink-0: a tile never squishes; at extreme widths the row scrolls instead.
  return (
    <div className="v2-stat inline-flex shrink-0 items-center gap-3 border border-border bg-card">
      <span className="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-accent text-accent-foreground">
        <Icon className="h-4 w-4" aria-hidden />
      </span>
      <span className="flex flex-col leading-tight">
        <span className="text-lg font-semibold tracking-tight text-foreground tabular-nums">
          {value}
          {sub && <span className="ml-1.5 text-xs font-normal text-muted-foreground">{sub}</span>}
        </span>
        <span className="text-xs font-medium text-muted-foreground">{label}</span>
      </span>
    </div>
  );
}

export function ReportHeader({
  title,
  createdAt,
  transcripts,
  actionItemCount,
}: {
  title: string;
  createdAt?: string;
  transcripts: Transcript[];
  actionItemCount?: number;
}) {
  useUiTranslation();
  const durationSec =
    transcripts.reduce((max, t) => Math.max(max, t.audio_end_time ?? 0), 0) ||
    transcripts.reduce((sum, t) => sum + (t.duration ?? 0), 0);

  const words = transcripts.reduce(
    (n, t) => n + (t.text ? t.text.trim().split(/\s+/).filter(Boolean).length : 0),
    0,
  );
  const segments = transcripts.length;

  let dateLabel: string | null = null;
  if (createdAt) {
    const d = new Date(createdAt);
    if (!Number.isNaN(d.getTime())) {
      dateLabel = d.toLocaleDateString(uiI18n.language, { weekday: 'short', day: 'numeric', month: 'short', year: 'numeric' });
    }
  }

  const wpm = durationSec > 0 ? Math.round(words / (durationSec / 60)) : 0;

  return (
    <header className="v2-report-header border-b border-border">
      {/* Full-width, left-aligned so the header shares a grid with the panels
          below (a centered max-w column over full-width panels reads off-grid).
          Title block left, stat tiles right; wraps on narrow windows. */}
      <div className="flex flex-wrap items-center justify-between gap-x-8 gap-y-3">
        <div className="min-w-0 max-w-full">
          <div className="text-xs font-medium uppercase tracking-[0.18em] text-primary">{translateUI("Meeting report")}</div>
          <h1 className="mt-0.5 break-words font-heading text-[28px] font-semibold leading-tight text-foreground">{title || translateUI("Untitled meeting")}</h1>
          <div className="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[13px] text-muted-foreground">
            {dateLabel && <span>{dateLabel}</span>}
            {dateLabel && durationSec > 0 && <span aria-hidden>·</span>}
            {durationSec > 0 && <span>{formatDuration(durationSec)}</span>}
            <span aria-hidden>·</span>
            <span>{segments} {translateUI("segments")}</span>
          </div>
        </div>

        {/* Stat tiles: wrap first (flex-wrap drops them under the title on
            narrow windows); min-w-0 + max-w-full keep the row inside the
            header, and overflow-x-auto is the never-clip backstop for widths
            where even a single tile can't fit. */}
        <div className="flex min-w-0 max-w-full flex-wrap items-center gap-2.5 overflow-x-auto [scrollbar-width:thin]">
          <StatTile icon={Clock} label={translateUI("Duration")} value={formatDuration(durationSec)} />
          <StatTile icon={FileText} label={translateUI("Words")} value={words.toLocaleString(uiI18n.language)} sub={wpm > 0 ? translateUI("~{{count}} wpm", { count: wpm }) : undefined} />
          <StatTile icon={Hash} label={translateUI("Segments")} value={String(segments)} />
          {actionItemCount != null && (
            <StatTile icon={ListChecks} label={translateUI("Action items")} value={String(actionItemCount)} />
          )}
        </div>
      </div>
    </header>
  );
}
