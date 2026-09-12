// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ConfigProvider, useConfig } from './ConfigContext';
import { configService, type ModelConfig } from '@/services/configService';
import { getOllamaModels } from '@/services/providerModelsService';

vi.mock('@/services/configService', () => ({ configService: {
  getModelConfig: vi.fn(), getTranscriptConfig: vi.fn(), getSelectedDevices: vi.fn(),
} }));
vi.mock('@/services/providerModelsService', () => ({ getOllamaModels: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(null) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));
vi.mock('@/lib/analytics', () => ({ default: { track: vi.fn().mockResolvedValue(undefined) } }));

function Probe() {
  const { error, models, setModelConfig } = useConfig();
  return <>
    <div data-testid="error">{error}</div>
    <div data-testid="models">{models.map(model => model.name).join(',')}</div>
    <button onClick={() => setModelConfig(prev => ({ ...prev, provider: 'builtin-ai' }))}>Switch</button>
  </>;
}

beforeEach(() => { vi.clearAllMocks(); localStorage.clear(); });
afterEach(cleanup);

it('waits for saved configuration and skips Ollama for built-in AI', async () => {
  let resolveConfig!: (value: ModelConfig) => void;
  vi.mocked(configService.getModelConfig).mockReturnValue(new Promise(resolve => { resolveConfig = resolve; }));
  render(<ConfigProvider><Probe /></ConfigProvider>);
  expect(getOllamaModels).not.toHaveBeenCalled();
  await act(async () => { resolveConfig({ provider: 'builtin-ai', model: 'local', whisperModel: 'large-v3' }); });
  expect(getOllamaModels).not.toHaveBeenCalled();
});

it('uses the saved endpoint and exposes a Tauri string error without console.error', async () => {
  vi.mocked(configService.getModelConfig).mockResolvedValue({ provider: 'ollama', model: 'local', whisperModel: 'large-v3', ollamaEndpoint: 'http://localhost:12345' });
  vi.mocked(getOllamaModels).mockRejectedValue('Request timed out after 5 seconds.');
  const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  try {
    render(<ConfigProvider><Probe /></ConfigProvider>);
    await waitFor(() => expect(screen.getByTestId('error').textContent).toBe('Request timed out after 5 seconds.'));
    expect(getOllamaModels).toHaveBeenCalledExactlyOnceWith('http://localhost:12345');
    expect(errorSpy).not.toHaveBeenCalled();
  } finally { errorSpy.mockRestore(); }
});

it('ignores an old request after switching providers', async () => {
  vi.mocked(configService.getModelConfig).mockResolvedValue({ provider: 'ollama', model: 'local', whisperModel: 'large-v3' });
  let rejectModels!: (reason: string) => void;
  vi.mocked(getOllamaModels).mockReturnValue(new Promise((_, reject) => { rejectModels = reject; }));
  render(<ConfigProvider><Probe /></ConfigProvider>);
  await waitFor(() => expect(getOllamaModels).toHaveBeenCalledOnce());
  await act(async () => { screen.getByText('Switch').click(); });
  await act(async () => { rejectModels('Old endpoint timed out'); });
  expect(screen.getByTestId('error').textContent).toBe('');
});
