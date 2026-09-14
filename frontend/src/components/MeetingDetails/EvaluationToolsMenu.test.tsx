// @vitest-environment jsdom
import React from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { uiI18n } from '@/i18n';

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  push: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('next/navigation', () => ({ useRouter: () => ({ push: mocks.push }) }));
vi.mock('sonner', () => ({ toast: { success: mocks.success, error: mocks.error } }));

const { EvaluationToolsMenu } = await import('./EvaluationToolsMenu');

type State = React.ComponentProps<typeof EvaluationToolsMenu>['diarizationState'];
const done = (turns: unknown[] = [{ speaker_label: 'Speaker 1' }]) => ({
  kind: 'done', diarizedAt: '2026-09-14T00:00:00Z', turns,
}) as State;

function renderMenu(state: State = { kind: 'ready' }, meetingId: string | undefined = 'meeting-1') {
  return render(<EvaluationToolsMenu meetingId={meetingId} diarizationState={state} />);
}

function openMenu() {
  fireEvent.pointerDown(screen.getByRole('button', { name: 'Evaluation Tools' }), {
    button: 0,
    ctrlKey: false,
  });
}

beforeEach(async () => {
  vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'true');
  await uiI18n.changeLanguage('en');
});

afterEach(() => {
  cleanup();
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  mocks.invoke.mockReset();
  mocks.push.mockReset();
  mocks.success.mockReset();
  mocks.error.mockReset();
});

describe('EvaluationToolsMenu', () => {
  it('does not appear when the existing evaluation feature gate is off', () => {
    vi.stubEnv('NODE_ENV', 'production');
    vi.stubEnv('NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION', 'false');
    renderMenu();
    expect(screen.queryByRole('button', { name: 'Evaluation Tools' })).toBeNull();
  });

  it('appears when the existing evaluation feature gate is on', () => {
    renderMenu();
    expect(screen.getByRole('button', { name: 'Evaluation Tools' })).toBeTruthy();
  });

  it.each([
    [{ kind: 'ready' }, 'Complete speaker analysis first.'],
    [{ kind: 'modelsMissing' }, 'Complete speaker analysis first.'],
    [{ kind: 'running' }, 'Speaker analysis is still running.'],
    [{ kind: 'queued' }, 'Speaker analysis is still running.'],
    [{ kind: 'done', diarizedAt: '2026-09-14T00:00:00Z', turns: [], job: 'running' } as State, 'Speaker analysis is still running.'],
  ] as const)('disables export for %s', (state, reason) => {
    renderMenu(state);
    openMenu();
    expect(screen.getByText(reason)).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /^Export Production Artifact/ })
      .hasAttribute('data-disabled')).toBe(true);
  });

  it('explains a missing meeting id and unavailable lifecycle state', () => {
    const view = renderMenu(null, '');
    openMenu();
    expect(screen.getByText('Meeting ID is unavailable.')).toBeTruthy();
    view.unmount();
    renderMenu(null, 'meeting-1');
    openMenu();
    expect(screen.getByText('Speaker analysis status is unavailable.')).toBeTruthy();
  });

  it.each([
    ['with speakers', done()],
    ['with zero speakers', done([])],
  ])('enables export after completed analysis %s', (_label, state) => {
    renderMenu(state);
    openMenu();
    expect(screen.getByRole('menuitem', { name: 'Export Production Artifact' })
      .hasAttribute('data-disabled')).toBe(false);
  });

  it('uses the existing backend command and reports a successful export', async () => {
    mocks.invoke.mockResolvedValue('C:\\Exports\\meeting-1.production.json');
    renderMenu(done());
    openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export Production Artifact' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith(
      'api_export_short_turn_production_artifact',
      { meetingId: 'meeting-1' },
    ));
    expect(mocks.success).toHaveBeenCalledWith('Production Artifact exported.');
  });

  it('treats Save dialog cancellation as a silent non-error', async () => {
    mocks.invoke.mockResolvedValue(null);
    renderMenu(done([]));
    openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export Production Artifact' }));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalled());
    expect(mocks.success).not.toHaveBeenCalled();
    expect(mocks.error).not.toHaveBeenCalled();
  });

  it('turns a missing production snapshot into actionable feedback and logs the original error', async () => {
    const original = 'build production artifact: meeting has no completed production short-turn snapshot';
    mocks.invoke.mockRejectedValue(original);
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    renderMenu(done());
    openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export Production Artifact' }));
    await waitFor(() => expect(mocks.error).toHaveBeenCalledWith(
      'Failed to export Production Artifact',
      { description: 'This meeting does not have a complete evaluation snapshot yet. Complete speaker analysis before exporting.' },
    ));
    expect(consoleError).toHaveBeenCalledWith('api_export_short_turn_production_artifact failed', original);
  });

  it('surfaces an unexpected backend error verbatim', async () => {
    mocks.invoke.mockRejectedValue('disk full');
    vi.spyOn(console, 'error').mockImplementation(() => {});
    renderMenu(done());
    openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: 'Export Production Artifact' }));
    await waitFor(() => expect(mocks.error).toHaveBeenCalledWith(
      'Failed to export Production Artifact',
      { description: 'disk full' },
    ));
  });

  it('opens the existing workspace route without creating a project', () => {
    renderMenu(done());
    openMenu();
    fireEvent.click(screen.getByRole('menuitem', { name: /Open Annotation Workspace/ }));
    expect(mocks.push).toHaveBeenCalledWith('/dev/short-turn-annotation');
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it('uses the shared Chinese translations', async () => {
    await act(() => uiI18n.changeLanguage('zh-CN'));
    renderMenu({ kind: 'running' });
    fireEvent.pointerDown(screen.getByRole('button', { name: '评测工具' }), { button: 0 });
    expect(screen.getByText('说话人分析仍在运行。')).toBeTruthy();
    expect(screen.getByRole('menuitem', { name: /打开标注工作台/ })).toBeTruthy();
  });
});
