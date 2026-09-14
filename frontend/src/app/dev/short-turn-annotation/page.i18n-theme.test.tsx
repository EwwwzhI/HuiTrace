// @vitest-environment jsdom
import React from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import ShortTurnAnnotationPage from './page';
import { uiI18n } from '@/i18n';
import { UiLanguageProvider } from '@/i18n/UiLanguageProvider';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  setTheme: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => path, invoke: mocks.invoke }));
vi.mock('sonner', () => ({ toast: { error: mocks.toastError, success: vi.fn() } }));
vi.mock('next-themes', () => ({ useTheme: () => ({ resolvedTheme: 'light', setTheme: mocks.setTheme }) }));
vi.mock('@/components/short-turn-annotation/WaveformTimeline', () => ({ WaveformTimeline: () => <div>controlled waveform</div> }));

const snapshot = {
  initialized: true,
  mode: 'blind',
  draft: {
    schema_version: 1,
    meeting_id: 'meeting-demo-001',
    events: [{
      event_id: 'meeting-demo-001-event-0001', start_ms: 100, end_ms: 500,
      kind: 'short_speech', speaker: 'gt_speaker_01', overlap: false,
      speaker_handoff: false, embedded: false, annotation_uncertain: false,
      expected_materialized: null, notes: 'Yes, I think so', annotation_status: 'pending',
    }],
  },
  session: {
    schema_version: 1, meeting_id: 'meeting-demo-001', source_media_path: 'meeting.wav',
    production_artifact_path: 'artifact.json', source_media_duration_ms: 5000,
    speaker_map: [{ key: 'gt_speaker_01', description: 'Host' }],
    window_status: {}, next_event_sequence: 2,
  },
  windows: [{ window_id: 'window-1', meeting_id: 'meeting-demo-001', source_start_ms: 0, source_end_ms: 5000, audio_path: 'meeting.wav', candidate_suggestions: [] }],
  reviewEvidence: { transcripts: [{ start_ms: 100, end_ms: 500, text: 'Yes, I think so' }], diarizer_turns: [], vad_events: [] },
};

async function loadWorkspace(pass: 'blind' | 'review' = 'blind') {
  fireEvent.change(screen.getByLabelText(/Meeting ID|会议 ID/), { target: { value: 'meeting-demo-001' } });
  fireEvent.click(screen.getByRole('button', { name: pass === 'blind' ? /Open blind pass|打开盲标阶段/ : /Review|复核/ }));
  await screen.findByText('meeting-demo-001-event-0001');
}

beforeEach(() => {
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'true');
  mocks.invoke.mockImplementation((command: string) => command === 'load_workspace' ? Promise.resolve(snapshot) : Promise.resolve(null));
});

afterEach(async () => {
  cleanup();
  vi.unstubAllEnvs();
  mocks.invoke.mockReset();
  mocks.setTheme.mockReset();
  mocks.toastError.mockReset();
  localStorage.clear();
  await uiI18n.changeLanguage('en');
});

it('renders the complete core workflow in Chinese', async () => {
  await uiI18n.changeLanguage('zh-CN');
  render(<ShortTurnAnnotationPage />);
  expect(screen.getByText('短会话标注工作台')).toBeTruthy();
  expect(screen.getByText('初始化标注项目')).toBeTruthy();
  expect(screen.getByText('盲标阶段')).toBeTruthy();
  await loadWorkspace();
  expect(screen.getByRole('button', { name: '运行质量检查' })).toBeTruthy();
});

it('retains the English workflow', async () => {
  await uiI18n.changeLanguage('en');
  render(<ShortTurnAnnotationPage />);
  expect(screen.getByText('Short-Turn Annotation Workspace')).toBeTruthy();
  expect(screen.getByText('Initialize annotation project')).toBeTruthy();
  await loadWorkspace();
  expect(screen.getByRole('button', { name: 'Run QA' })).toBeTruthy();
});

it('switches language and theme without changing annotation data', async () => {
  await uiI18n.changeLanguage('en');
  localStorage.setItem('huitrace.uiLanguage', 'en');
  const { container } = render(<UiLanguageProvider><ShortTurnAnnotationPage /></UiLanguageProvider>);
  await loadWorkspace('review');
  expect(screen.getByText('Yes, I think so')).toBeTruthy();
  expect(screen.getAllByText('gt_speaker_01').length).toBeGreaterThan(0);
  expect(screen.getByRole('radio', { name: 'English' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('radio', { name: '中文' }).getAttribute('aria-checked')).toBe('false');

  await act(async () => fireEvent.click(screen.getByRole('radio', { name: '中文' })));
  expect(await screen.findByText('短会话标注工作台')).toBeTruthy();
  expect(screen.getByRole('radio', { name: '中文' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('radio', { name: 'English' }).getAttribute('aria-checked')).toBe('false');
  expect(screen.getByText('Yes, I think so')).toBeTruthy();
  expect(screen.getByText('meeting-demo-001-event-0001')).toBeTruthy();
  expect(screen.getAllByText('gt_speaker_01').length).toBeGreaterThan(0);

  fireEvent.click(screen.getByRole('radio', { name: '深色' }));
  fireEvent.click(screen.getByRole('radio', { name: '浅色' }));
  expect(mocks.setTheme.mock.calls).toEqual([['dark'], ['light']]);
  expect(screen.getByText('meeting-demo-001-event-0001')).toBeTruthy();
  expect(container.querySelector('[data-testid="annotation-workspace"]')?.className).toContain('bg-background');
  expect(container.innerHTML).not.toContain('bg-slate-950');
});


it('refuses Blind completion until the annotator explicitly confirms the event', async () => {
  render(<ShortTurnAnnotationPage />);
  await loadWorkspace();
  fireEvent.click(screen.getByRole('button', { name: 'Confirm and complete this window' }));
  expect(mocks.toastError).toHaveBeenCalledWith('This window still contains 1 pending annotations. Confirm all event labels before completing the window.');
  fireEvent.click(screen.getByText('meeting-demo-001-event-0001'));
  fireEvent.click(screen.getByRole('button', { name: 'Confirm annotation' }));
  mocks.toastError.mockClear();
  fireEvent.click(screen.getByRole('button', { name: 'Confirm and complete this window' }));
  expect(mocks.toastError).not.toHaveBeenCalled();
});

it('keeps export disabled after successful QA when Review is only partial', async () => {
  const partial = structuredClone(snapshot);
  partial.mode = 'review';
  partial.draft.events[0].annotation_status = 'blind_confirmed';
  partial.session.window_status = { 'window-1': 'reviewed_second_pass' };
  partial.windows.push({ ...partial.windows[0], window_id: 'window-2' });
  mocks.invoke.mockImplementation((command: string) => Promise.resolve(command === 'load_workspace' ? partial
    : command === 'qa_workspace_command' ? { errors: [], possibleDuplicates: [], sourceDurationMs: 5000 } : null));
  render(<ShortTurnAnnotationPage />);
  await loadWorkspace('review');
  fireEvent.click(screen.getByRole('button', { name: 'Run QA' }));
  await screen.findByText('Review progress 1 / 2', { exact: false });
  expect((screen.getByRole('button', { name: 'Export benchmark manifest' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText('Complete all Review windows before exporting the Benchmark Manifest.', { exact: false })).toBeTruthy();
});
