// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import MeetingPage from './page';

const state = vi.hoisted(() => ({
  id: 'meeting-a',
  loading: false,
  currentMeeting: { id: 'meeting-a', title: 'Meeting A' },
  metadata: { id: 'meeting-a', title: 'Meeting A', created_at: '', updated_at: '' },
  transcripts: [{ id: 'segment', text: 'Transcript ready' }],
  setCurrentMeeting: vi.fn(), stopSummaryPolling: vi.fn(),
  requests: new Map<string, (value: unknown) => void>(),
}));
vi.mock('next/navigation', () => ({
  useSearchParams: () => new URLSearchParams({ id: state.id }),
  useRouter: () => ({ push: vi.fn() }),
}));
vi.mock('@/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => state }));
vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => ({ isAutoSummary: false }) }));
vi.mock('@/hooks/usePaginatedTranscripts', () => ({ usePaginatedTranscripts: () => ({
  metadata: state.metadata, transcripts: state.transcripts, isLoading: state.loading,
}) }));
vi.mock('@/lib/analytics', () => ({ default: { trackPageView: vi.fn() } }));
vi.mock('@/services/configService', () => ({ configService: {} }));
vi.mock('@/services/providerModelsService', () => ({ getOllamaModels: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: (_command: string, args: { meetingId: string }) =>
  new Promise(resolve => { state.requests.set(args.meetingId, resolve); }),
}));
vi.mock('./page-content', () => ({ default: ({ meeting, summaryData, isSummaryLoading }: any) =>
  <div>{meeting.title}<span>Transcript ready</span><span>{isSummaryLoading ? 'Loading summary' : summaryData?.markdown}</span></div>,
}));
afterEach(cleanup);

it('shows the selected title and stable panel placeholders while data loads', async () => {
  state.loading = true;
  const view = render(<MeetingPage />);
  expect(screen.getByRole('heading', { name: 'Meeting A' })).toBeTruthy();
  expect(screen.getByRole('status', { name: 'Loading transcript…' })).toBeTruthy();
  expect(screen.queryByText('Transcript ready')).toBeNull();
  state.loading = false;
  view.rerender(<MeetingPage />);
  await waitFor(() => expect(screen.getByText('Transcript ready')).toBeTruthy());
});

it('shows transcripts before the summary arrives and ignores an old meeting response', async () => {
  const view = render(<MeetingPage />);
  await waitFor(() => expect(screen.getByText('Transcript ready')).toBeTruthy());
  expect(screen.getByText('Loading summary')).toBeTruthy();
  state.id = 'meeting-b';
  state.metadata = { ...state.metadata, id: 'meeting-b', title: 'Meeting B' };
  view.rerender(<MeetingPage />);
  await waitFor(() => expect(screen.getByText('Meeting B')).toBeTruthy());
  await act(async () => { state.requests.get('meeting-b')!({ data: { markdown: 'Summary B' } }); });
  await waitFor(() => expect(screen.getByText('Summary B')).toBeTruthy());
  await act(async () => { state.requests.get('meeting-a')!({ data: { markdown: 'Old summary A' } }); });
  expect(screen.queryByText('Old summary A')).toBeNull();
  expect(screen.getByText('Summary B')).toBeTruthy();
});
