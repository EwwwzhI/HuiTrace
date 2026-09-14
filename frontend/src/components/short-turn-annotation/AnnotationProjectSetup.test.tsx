// @vitest-environment jsdom
import React, { useState } from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

import { AnnotationProjectSetup, type ProductionArtifactInspection } from './AnnotationProjectSetup';
import { uiI18n } from '@/i18n';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  open: vi.fn(),
  initialize: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open }));
vi.mock('sonner', () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const inspection: ProductionArtifactInspection = {
  meetingId: 'meeting-1', artifactId: 'artifact-1', schemaVersion: 2,
  transcriptionRunId: 'run-1', durationMs: 6_000,
  asrBackend: 'whisper.cpp', asrModel: 'large-v3',
  diarizationBackend: 'sherpa-onnx', diarizationModel: 'campplus',
  transcriptCount: 1, diarizerTurnCount: 1, vadEventCount: 1,
};

const preparation = {
  meetingId: 'meeting-1', blindWindowCount: 2, reviewWindowCount: 2,
  candidateCount: 1, controlledProductionArtifact: 'C:\\dataset\\meeting-1\\meeting-1.production.json',
  blindManifest: 'blind.jsonl', reviewManifest: 'review.jsonl',
};

function Harness() {
  const [datasetRoot, setDatasetRoot] = useState('');
  const [meetingId, setMeetingId] = useState('');
  return <AnnotationProjectSetup
    datasetRoot={datasetRoot}
    setDatasetRoot={setDatasetRoot}
    meetingId={meetingId}
    setMeetingId={setMeetingId}
    initialize={mocks.initialize}
  />;
}

beforeEach(async () => {
  await uiI18n.changeLanguage('en');
  mocks.open.mockImplementation((options: { directory?: boolean; title?: string }) => {
    if (options.directory) return Promise.resolve('C:\\dataset');
    if (options.title?.includes('Source')) return Promise.resolve('C:\\inputs\\meeting.wav');
    return Promise.resolve('C:\\inputs\\artifact.json');
  });
  mocks.invoke.mockImplementation((command: string) => {
    if (command === 'api_inspect_short_turn_production_artifact') return Promise.resolve(inspection);
    if (command === 'api_prepare_short_turn_annotation_windows') return Promise.resolve(preparation);
    return Promise.reject(new Error(`unexpected command ${command}`));
  });
  mocks.initialize.mockResolvedValue(true);
});

afterEach(() => {
  cleanup();
  mocks.invoke.mockReset();
  mocks.open.mockReset();
  mocks.initialize.mockReset();
});

it('keeps Prepare disabled and Initialize unavailable until their prerequisites complete', () => {
  render(<Harness />);
  expect((screen.getByRole('button', { name: 'Prepare annotation data' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.queryByRole('button', { name: 'Initialize and start Blind annotation' })).toBeNull();
});

async function selectAllInputs() {
  fireEvent.click(screen.getByRole('button', { name: 'Select folder' }));
  const fileButtons = screen.getAllByRole('button', { name: 'Select file' });
  fireEvent.click(fileButtons[0]);
  fireEvent.click(fileButtons[1]);
  await waitFor(() => expect((screen.getByLabelText(/^Meeting ID/) as HTMLInputElement).value).toBe('meeting-1'));
}

it('uses native pickers, derives Meeting ID, prepares, and initializes in two stages', async () => {
  render(<Harness />);
  await selectAllInputs();

  expect((screen.getByLabelText(/^Meeting ID/) as HTMLInputElement).readOnly).toBe(true);
  expect(screen.getByText('whisper.cpp / large-v3')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Prepare annotation data' }));
  expect(await screen.findByText('Blind windows')).toBeTruthy();
  expect(screen.getAllByText('2').length).toBeGreaterThanOrEqual(2);

  fireEvent.click(screen.getByRole('button', { name: 'Initialize and start Blind annotation' }));
  await waitFor(() => expect(mocks.initialize).toHaveBeenCalledWith(
    'C:\\inputs\\meeting.wav', preparation.controlledProductionArtifact,
  ));
  expect(mocks.invoke).toHaveBeenCalledWith('api_prepare_short_turn_annotation_windows', {
    request: {
      datasetRoot: 'C:\\dataset', sourceMedia: 'C:\\inputs\\meeting.wav',
      productionArtifact: 'C:\\inputs\\artifact.json', meetingId: 'meeting-1',
    },
  });
});

it('preserves a manually entered path when the picker is cancelled', async () => {
  mocks.open.mockResolvedValue(null);
  render(<Harness />);
  fireEvent.change(screen.getByLabelText('Dataset Root'), { target: { value: 'C:\\keep-me' } });
  fireEvent.click(screen.getByRole('button', { name: 'Select folder' }));
  await waitFor(() => expect((screen.getByLabelText('Dataset Root') as HTMLInputElement).value).toBe('C:\\keep-me'));
});

it('shows invalid Artifact errors and keeps preparation disabled', async () => {
  mocks.invoke.mockRejectedValue('artifact schema 99 is unsupported');
  render(<Harness />);
  fireEvent.click(screen.getAllByRole('button', { name: 'Select file' })[1]);
  expect((await screen.findByRole('alert')).textContent).toContain('Production Artifact is invalid');
  expect((screen.getByRole('button', { name: 'Prepare annotation data' }) as HTMLButtonElement).disabled).toBe(true);
});

it('prevents duplicate preparation while the command is running', async () => {
  let resolvePreparation!: (value: typeof preparation) => void;
  mocks.invoke.mockImplementation((command: string) => command.includes('inspect')
    ? Promise.resolve(inspection)
    : new Promise(resolve => { resolvePreparation = resolve; }));
  render(<Harness />);
  await selectAllInputs();
  const button = screen.getByRole('button', { name: 'Prepare annotation data' });
  fireEvent.click(button);
  fireEvent.click(button);
  expect(mocks.invoke.mock.calls.filter(([command]) => command === 'api_prepare_short_turn_annotation_windows')).toHaveLength(1);
  resolvePreparation(preparation);
  expect(await screen.findByText('Annotation data prepared')).toBeTruthy();
});
