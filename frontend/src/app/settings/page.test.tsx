// @vitest-environment jsdom
import React from 'react';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import SettingsPage from './page';
import { invoke } from '@tauri-apps/api/core';
import { uiI18n } from '@/i18n';

const mocks = vi.hoisted(() => ({ mounted: vi.fn(), setConfig: vi.fn(), push: vi.fn(), back: vi.fn() }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ back: mocks.back, push: mocks.push }) }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('@/contexts/ConfigContext', () => ({ useConfig: () => ({ transcriptModelConfig: {}, setTranscriptModelConfig: mocks.setConfig }) }));
vi.mock('@/components/PreferenceSettings', () => ({ PreferenceSettings: () => <div>General settings</div> }));
vi.mock('@/components/RecordingSettings', () => ({ RecordingSettings: () => <div>Recording settings</div> }));
vi.mock('@/components/SummaryModelSettings', () => ({ SummaryModelSettings: () => <div>Summary settings</div> }));
vi.mock('@/components/TranscriptSettings', () => ({ TranscriptSettings: () => {
  React.useEffect(() => { mocks.mounted(); }, []);
  return <input aria-label="Model draft" defaultValue="small" />;
} }));
afterEach(async () => { cleanup(); vi.unstubAllEnvs(); mocks.push.mockReset(); await uiI18n.changeLanguage('en'); });

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

it('shows the gated evaluation entry and navigates with the Next router', async () => {
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'true');
  render(<SettingsPage />);
  fireEvent.mouseDown(screen.getByRole('tab', { name: 'Evaluation Tools' }), { button: 0 });
  const open = await screen.findByRole('button', { name: /Open Annotation Workspace/i });
  fireEvent.click(open);
  expect(mocks.push).toHaveBeenCalledWith('/dev/short-turn-annotation');
});

it('does not render evaluation tools without the production opt-in', () => {
  vi.stubEnv('NODE_ENV', 'production');
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'false');
  render(<SettingsPage />);
  expect(screen.queryByRole('tab', { name: 'Evaluation Tools' })).toBeNull();
  expect(screen.queryByText('Short-Turn Annotation Workspace')).toBeNull();
});

it('localizes the evaluation tools entry with the shared UI language', async () => {
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'true');
  await uiI18n.changeLanguage('zh-CN');
  render(<SettingsPage />);
  fireEvent.mouseDown(screen.getByRole('tab', { name: '评测工具' }), { button: 0 });
  expect(await screen.findByText('短会话标注工作台')).toBeTruthy();
  expect(screen.getByRole('button', { name: '打开标注工作台' })).toBeTruthy();
});
