import { describe, expect, it, vi } from 'vitest';
import { check } from '@tauri-apps/plugin-updater';
import { UpdateService } from './updateService';

vi.mock('@tauri-apps/plugin-updater', () => ({ check: vi.fn() }));
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }));
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn().mockResolvedValue('1.1.0') }));

describe('HuiTrace release isolation', () => {
  it('does not contact the upstream updater even on a forced check', async () => {
    const service = new UpdateService();
    expect(await service.checkForUpdates(true)).toEqual({
      available: false,
      currentVersion: '1.1.0',
    });
    expect(check).not.toHaveBeenCalled();
  });
});
