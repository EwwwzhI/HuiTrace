// @vitest-environment jsdom
import React from 'react';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import Editor from './Editor';

const state = vi.hoisted(() => ({ theme: 'light', mounted: vi.fn(), changed: vi.fn() }));
vi.mock('next-themes', () => ({ useTheme: () => ({ resolvedTheme: state.theme }) }));
vi.mock('@blocknote/react', () => ({ useCreateBlockNote: () => React.useMemo(() => ({ document: [], onChange: () => state.changed }), []) }));
vi.mock('@blocknote/shadcn', () => ({ BlockNoteView: ({ theme }: { theme: string }) => {
  React.useEffect(() => { state.mounted(); }, []);
  return <div data-testid="editor-surface" data-theme={theme}><textarea aria-label="Draft" defaultValue="Original" /></div>;
} }));
afterEach(cleanup);

it('updates the editor theme without remounting or losing an unsaved draft', () => {
  state.theme = 'light'; state.mounted.mockClear();
  const view = render(<Editor />);
  fireEvent.change(screen.getByRole('textbox'), { target: { value: 'Unsaved meeting notes' } });
  state.theme = 'dark'; view.rerender(<Editor />);
  expect(screen.getByTestId('editor-surface').getAttribute('data-theme')).toBe('dark');
  expect((screen.getByRole('textbox') as HTMLTextAreaElement).value).toBe('Unsaved meeting notes');
  state.theme = 'light'; view.rerender(<Editor />);
  expect(screen.getByTestId('editor-surface').getAttribute('data-theme')).toBe('light');
  expect(state.mounted).toHaveBeenCalledTimes(1);
});
