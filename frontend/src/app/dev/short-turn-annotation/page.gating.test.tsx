// @vitest-environment jsdom
import React from 'react';
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import ShortTurnAnnotationPage from './page';

vi.mock('@tauri-apps/api/core', () => ({ convertFileSrc: (path: string) => path, invoke: vi.fn() }));

afterEach(() => { cleanup(); vi.unstubAllEnvs(); });

it('retains route-level protection in production', () => {
  vi.stubEnv('NODE_ENV', 'production');
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'false');
  render(<ShortTurnAnnotationPage />);
  expect(screen.getByText('Development evaluation route is disabled')).toBeTruthy();
  expect(screen.queryByText('Initialize annotation project')).toBeNull();
});
