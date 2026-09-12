// @vitest-environment jsdom
import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import SettingsPage from './page';
import { invoke } from '@tauri-apps/api/core';

const mocks = vi.hoisted(() => ({ mounted: vi.fn(), setConfig: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ back: vi.fn() }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => ({ transcriptModelConfig: {}, setTranscriptModelConfig: mocks.setConfig }) }));
vi.mock('@/components/PreferenceSettings', () => ({ PreferenceSettings: () => <div>General settings</div> }));
vi.mock('@/components/RecordingSettings', () => ({ RecordingSettings: () => <div>Recording settings</div> }));
vi.mock('@/components/SummaryModelSettings', () => ({ SummaryModelSettings: () => <div>Summary settings</div> }));
vi.mock('@/components/TranscriptSettings', () => ({ TranscriptSettings: () => {
  React.useEffect(() => { mocks.mounted(); }, []);
  return <input aria-label="Model draft" defaultValue="small" />;
} }));
afterEach(cleanup);

it('mounts a tab only when visited and retains its draft when switching back', async () => {
  render(<SettingsPage />);
  expect(mocks.mounted).not.toHaveBeenCalled();
  expect(invoke).not.toHaveBeenCalled();
  const select = (name: string) => fireEvent.mouseDown(screen.getByRole('tab', { name }), { button: 0, ctrlKey: false });
  select('Transcription');
  await waitFor(() => expect(screen.getByRole('textbox', { name: 'Model draft' })).toBeTruthy());
  fireEvent.change(screen.getByRole('textbox', { name: 'Model draft' }), { target: { value: 'edited' } });
  select('General');
  expect(screen.queryByRole('textbox', { name: 'Model draft' })).toBeNull();
  select('Transcription');
  expect((screen.getByRole('textbox', { name: 'Model draft' }) as HTMLInputElement).value).toBe('edited');
  expect(mocks.mounted).toHaveBeenCalledTimes(1);
});

it('omits empty beta and upstream commercial licensing tabs', () => {
  render(<SettingsPage />);
  expect(screen.queryByRole('tab', { name: 'Beta' })).toBeNull();
  expect(screen.queryByRole('tab', { name: 'License' })).toBeNull();
  expect(screen.getAllByRole('tab')).toHaveLength(4);
});
