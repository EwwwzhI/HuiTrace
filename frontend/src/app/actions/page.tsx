'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { useRouter } from 'next/navigation';
import {
  AlertCircle,
  CalendarDays,
  ExternalLink,
  FileText,
  ListChecks,
  Loader2,
  RefreshCw,
  Shield,
  UserRound,
} from 'lucide-react';

import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import {
  ACTION_CENTER_PAGE_SIZE,
  listApprovedActionItems,
  type ApprovedActionItem,
} from '@/services/actionCenterService';
import { translateUI, uiI18n } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { ThemeToggle } from '@/components/ThemeToggle';



const LOAD_ERROR_MESSAGE = 'Approved action items could not be loaded from local storage.';

function formatRecordingTime(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) {
    return null;
  }

  const totalSeconds = Math.floor(seconds);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const remainingSeconds = totalSeconds % 60;

  if (hours > 0) {
    return [hours, minutes, remainingSeconds]
      .map((part) => part.toString().padStart(2, '0'))
      .join(':');
  }

  return `${minutes.toString().padStart(2, '0')}:${remainingSeconds
    .toString()
    .padStart(2, '0')}`;
}

function formatMeetingDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat(uiI18n.language, { dateStyle: 'medium' }).format(date);
}

function mergeUniqueItems(
  current: ApprovedActionItem[],
  incoming: ApprovedActionItem[],
): ApprovedActionItem[] {
  const seen = new Set(current.map((item) => item.id));
  return [...current, ...incoming.filter((item) => !seen.has(item.id))];
}

export default function ActionsPage() {
  useUiTranslation();
  const router = useRouter();
  const { setCurrentMeeting } = useSidebar();
  const requestNonceRef = useRef(0);
  const sourceJumpNonceRef = useRef(0);
  const [items, setItems] = useState<ApprovedActionItem[]>([]);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadPage = useCallback(async (offset: number, replace: boolean) => {
    const requestNonce = ++requestNonceRef.current;
    setError(null);
    if (replace) {
      setIsLoading(true);
    } else {
      setIsLoadingMore(true);
    }

    try {
      const page = await listApprovedActionItems(offset, ACTION_CENTER_PAGE_SIZE);
      if (requestNonce !== requestNonceRef.current) return;

      setItems((current) => (replace ? page.items : mergeUniqueItems(current, page.items)));
      setNextOffset(page.hasMore ? page.nextOffset : null);
    } catch {
      if (requestNonce !== requestNonceRef.current) return;
      setError(LOAD_ERROR_MESSAGE);
    } finally {
      if (requestNonce === requestNonceRef.current) {
        setIsLoading(false);
        setIsLoadingMore(false);
      }
    }
  }, []);

  useEffect(() => {
    void loadPage(0, true);
    return () => {
      requestNonceRef.current += 1;
    };
  }, [loadPage]);

  const openSource = useCallback(
    (item: ApprovedActionItem) => {
      setCurrentMeeting({ id: item.meetingId, title: item.meetingTitle });
      const jumpNonce = ++sourceJumpNonceRef.current;
      const params = new URLSearchParams({
        id: item.meetingId,
        segment: item.sourceChunkId,
        source: 'action-center',
        jump: `${Date.now()}-${jumpNonce}`,
      });
      router.push(`/meeting-details?${params.toString()}`);
    },
    [router, setCurrentMeeting],
  );

  return (
    <div className="flex h-screen flex-col bg-background">
      <header className="v2-page-header sticky top-0 z-10">
        <div className="mx-auto w-full max-w-6xl">
          <div className="flex flex-wrap items-center gap-4">
            <div className="mt-1 flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-accent">
              <ListChecks className="h-5 w-5 text-primary" aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <h1 className="font-heading text-[28px] font-semibold text-foreground">{translateUI("Action Items")}</h1>
              <p className="mt-1 text-sm text-muted-foreground"> {translateUI("Review approved follow-ups from your meetings.")} </p>
            </div>
            <ThemeToggle compact />
          </div>
        </div>
      </header>

      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-6xl space-y-5 px-6 py-6 pr-8">
          <section
            className="flex gap-3 rounded-xl border border-primary/30 bg-accent px-4 py-3 text-sm text-primary"
            aria-label={translateUI("Action provenance")}
          >
            <Shield className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <div>
              <p className="font-semibold">{translateUI("AI-extracted · human approved")}</p>
              <p className="mt-0.5 text-primary"> {translateUI("This view is read-only. Each action links to its transcript source so you can verify it; approval is separate from future work-progress tracking.")} </p>
            </div>
          </section>

          {isLoading ? (
            <div
              className="flex min-h-64 items-center justify-center gap-3 rounded-xl border border-border bg-card text-sm text-muted-foreground"
              role="status"
              aria-live="polite"
            >
              <Loader2 className="h-5 w-5 animate-spin text-primary" aria-hidden="true" /> {translateUI("Loading approved action items…")} </div>
          ) : error && items.length === 0 ? (
            <div
              className="flex min-h-64 flex-col items-center justify-center gap-4 rounded-xl border border-destructive/30 bg-card px-6 text-center"
              role="alert"
            >
              <AlertCircle className="h-7 w-7 text-destructive" aria-hidden="true" />
              <div>
                <h2 className="font-semibold text-foreground">{translateUI("Unable to load Action Items")}</h2>
                <p className="mt-1 text-sm text-muted-foreground">{error}</p>
              </div>
              <button
                type="button"
                onClick={() => void loadPage(0, true)}
                className="inline-flex items-center gap-2 rounded-lg bg-secondary px-4 py-2 text-sm font-medium text-secondary-foreground transition-colors hover:bg-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
              >
                <RefreshCw className="h-4 w-4" aria-hidden="true" /> {translateUI("Retry")} </button>
            </div>
          ) : items.length === 0 ? (
            <div
              className="flex min-h-64 flex-col items-center justify-center rounded-xl border border-dashed border-border bg-card px-6 text-center"
              role="status"
            >
              <div className="flex h-12 w-12 items-center justify-center rounded-full bg-muted">
                <ListChecks className="h-6 w-6 text-muted-foreground" aria-hidden="true" />
              </div>
              <h2 className="mt-4 text-lg font-semibold text-foreground">{translateUI("No approved action items yet")}</h2>
              <p className="mt-1 max-w-lg text-sm leading-6 text-muted-foreground"> {translateUI("Source-linked actions will appear here after you explicitly approve them in a meeting summary.")} </p>
            </div>
          ) : (
            <>
              <ul className="v2-actions-grid" aria-label={translateUI("Approved action items")}>
                {items.map((item) => {
                  const recordingTime = formatRecordingTime(item.audioStartTime);
                  const sourceLabel = recordingTime || item.sourceTimestamp;

                  return (
                    <li key={item.id}>
                      <article className="bg-card p-5">
                        <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
                          <div className="min-w-0 flex-1">
                            <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
                              <span className="min-w-0 max-w-full break-words font-semibold text-foreground">
                                {item.meetingTitle}
                              </span>
                              <span aria-hidden="true">·</span>
                              <span>{formatMeetingDate(item.meetingCreatedAt)}</span>
                              <span className="rounded-full border border-primary/30 bg-accent px-2 py-0.5 font-medium text-primary"> {translateUI("Approved")} </span>
                            </div>

                            <h2 className="mt-3 whitespace-pre-wrap break-words text-base font-semibold leading-6 text-foreground">
                              {item.text}
                            </h2>

                            {(item.assignee || item.due) && (
                              <dl className="mt-4 flex flex-wrap gap-2 text-xs">
                                {item.assignee && (
                                  <div className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-lg border border-border bg-muted px-2.5 py-1.5">
                                    <UserRound className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />
                                    <dt className="shrink-0 font-medium text-muted-foreground">{translateUI("Assignee:")}</dt>
                                    <dd className="min-w-0 break-words text-foreground">
                                      {item.assignee}
                                    </dd>
                                  </div>
                                )}
                                {item.due && (
                                  <div className="inline-flex min-w-0 max-w-full items-center gap-1.5 rounded-lg border border-border bg-muted px-2.5 py-1.5">
                                    <CalendarDays className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />
                                    <dt className="shrink-0 font-medium text-muted-foreground">{translateUI("Due:")}</dt>
                                    <dd className="min-w-0 break-words text-foreground">{item.due}</dd>
                                  </div>
                                )}
                              </dl>
                            )}
                          </div>

                          <button
                            type="button"
                            onClick={() => openSource(item)}
                            className="inline-flex shrink-0 items-center justify-center gap-2 rounded-lg border border-border bg-card px-3 py-2 text-sm font-medium text-foreground transition-colors hover:border-primary/30 hover:bg-accent hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
                            aria-label={`Open source in ${item.meetingTitle} at ${sourceLabel}`}
                          >
                            <FileText className="h-4 w-4" aria-hidden="true" /> {translateUI("Source")} <ExternalLink className="h-3.5 w-3.5" aria-hidden="true" />
                          </button>
                        </div>

                        <div className="mt-4 border-t border-border pt-3 text-xs text-muted-foreground"> {translateUI("Transcript source ·")} {sourceLabel}
                        </div>
                      </article>
                    </li>
                  );
                })}
              </ul>

              {error && (
                <div
                  className="flex flex-col gap-3 rounded-lg border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive sm:flex-row sm:items-center sm:justify-between"
                  role="alert"
                >
                  <span>{error}</span>
                  <button
                    type="button"
                    onClick={() => nextOffset !== null && void loadPage(nextOffset, false)}
                    className="inline-flex items-center gap-2 self-start rounded-md border border-destructive/30 bg-card px-3 py-1.5 font-medium text-destructive hover:bg-destructive/10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-destructive"
                  >
                    <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" /> {translateUI("Retry")} </button>
                </div>
              )}

              {nextOffset !== null && !error && (
                <div className="flex justify-center pb-6">
                  <button
                    type="button"
                    disabled={isLoadingMore}
                    onClick={() => void loadPage(nextOffset, false)}
                    className="inline-flex items-center gap-2 rounded-lg border border-border bg-card px-4 py-2 text-sm font-medium text-foreground transition-colors hover:border-primary/30 hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2"
                  >
                    {isLoadingMore && (
                      <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
                    )}
                    {isLoadingMore ? translateUI("Loading…") : translateUI("Load more")}
                  </button>
                </div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
