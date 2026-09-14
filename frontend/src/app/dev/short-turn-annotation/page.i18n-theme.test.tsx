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
  open: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => path, invoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open }));
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
  fireEvent.click(screen.getByText(/Open existing annotation project|打开已有标注项目/));
  fireEvent.change(screen.getByLabelText(/Existing Meeting ID|已有项目 Meeting ID/), { target: { value: 'meeting-demo-001' } });
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
  mocks.open.mockReset();
  localStorage.clear();
  await uiI18n.changeLanguage('en');
});

it('renders the complete core workflow in Chinese', async () => {
  await uiI18n.changeLanguage('zh-CN');
  render(<ShortTurnAnnotationPage />);
  expect(screen.getByText('短会话标注工作台')).toBeTruthy();
  expect(screen.getByText('初始化标注项目')).toBeTruthy();
  expect(screen.getAllByText('盲标阶段').length).toBeGreaterThan(0);
  await loadWorkspace();
  expect(screen.getByRole('button', { name: '运行质量检查' })).toBeTruthy();
});

it('retains the English workflow', async () => {
  await uiI18n.changeLanguage('en');
  const { container } = render(<ShortTurnAnnotationPage />);
  expect(screen.getByText('Short-Turn Annotation Workspace')).toBeTruthy();
  expect(screen.getByText('Initialize annotation project')).toBeTruthy();
  await loadWorkspace();
  expect(screen.getByRole('button', { name: 'Run QA' })).toBeTruthy();

  const audio = container.querySelector('audio');
  expect(audio).toBeTruthy();
  Object.defineProperty(audio, 'duration', { configurable: true, value: 7.25 });
  expect(() => fireEvent.loadedMetadata(audio!)).not.toThrow();
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
  const workspace = container.querySelector('[data-testid="annotation-workspace"]');
  expect(workspace?.className).toContain('bg-background');
  expect(workspace?.className).toContain('h-full');
  expect(workspace?.className).toContain('overflow-y-auto');
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

it('adds and removes unused speakers with unique keys', async () => {
  const speakers = structuredClone(snapshot);
  speakers.session.speaker_map.push({ key: 'gt_speaker_03', description: 'Guest' });
  mocks.invoke.mockImplementation((command: string) => command === 'load_workspace' ? Promise.resolve(speakers) : Promise.resolve(null));
  render(<ShortTurnAnnotationPage />);
  await loadWorkspace();

  fireEvent.click(screen.getByRole('button', { name: 'Add speaker' }));
  expect(screen.getAllByText('gt_speaker_02').length).toBeGreaterThan(0);

  fireEvent.click(screen.getByRole('button', { name: 'Remove speaker gt_speaker_02' }));
  expect(screen.queryByRole('button', { name: 'Remove speaker gt_speaker_02' })).toBeNull();
  expect(screen.getByRole('button', { name: 'Remove speaker gt_speaker_03' })).toBeTruthy();
});

it('prevents deleting a speaker that is still assigned to annotation events', async () => {
  render(<ShortTurnAnnotationPage />);
  await loadWorkspace();

  fireEvent.click(screen.getByRole('button', { name: 'Remove speaker gt_speaker_01' }));
  expect(mocks.toastError).toHaveBeenCalledWith('Cannot remove this speaker because 1 annotation events use it. Reassign those events first.');
  expect(screen.getByRole('button', { name: 'Remove speaker gt_speaker_01' })).toBeTruthy();
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

it('enters Blind immediately after the prepared project is initialized', async () => {
  mocks.open.mockImplementation((options: { directory?: boolean; title?: string }) => {
    if (options.directory) return Promise.resolve('C:\\dataset');
    if (options.title?.includes('Source')) return Promise.resolve('C:\\meeting.wav');
    return Promise.resolve('C:\\meeting.production.json');
  });
  mocks.invoke.mockImplementation((command: string) => {
    if (command === 'api_inspect_short_turn_production_artifact') return Promise.resolve({
      meetingId: 'meeting-demo-001', artifactId: 'artifact-1', schemaVersion: 2,
      transcriptionRunId: 'run-1', durationMs: 5_000,
      asrBackend: 'whisper.cpp', asrModel: 'large-v3',
      diarizationBackend: 'sherpa-onnx', diarizationModel: 'campplus',
      transcriptCount: 1, diarizerTurnCount: 1, vadEventCount: 1,
    });
    if (command === 'api_prepare_short_turn_annotation_windows') return Promise.resolve({
      meetingId: 'meeting-demo-001', blindWindowCount: 1, reviewWindowCount: 1,
      candidateCount: 1, controlledProductionArtifact: 'C:\\dataset\\meeting-demo-001\\meeting-demo-001.production.json',
      blindManifest: 'blind.jsonl', reviewManifest: 'review.jsonl',
    });
    if (command === 'initialize_annotation_project') return Promise.resolve(snapshot);
    return Promise.resolve(null);
  });

  render(<ShortTurnAnnotationPage />);
  fireEvent.click(screen.getByRole('button', { name: 'Select folder' }));
  const fileButtons = screen.getAllByRole('button', { name: 'Select file' });
  fireEvent.click(fileButtons[0]);
  fireEvent.click(fileButtons[1]);
  await waitFor(() => expect((screen.getByLabelText(/^Meeting ID/) as HTMLInputElement).value).toBe('meeting-demo-001'));
  fireEvent.click(screen.getByRole('button', { name: 'Prepare annotation data' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Initialize and start Blind annotation' }));

  expect(await screen.findByText('meeting-demo-001-event-0001')).toBeTruthy();
  expect(screen.getAllByText('Blind annotation').length).toBeGreaterThan(0);
});
