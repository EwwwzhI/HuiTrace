'use client';

import { ThemeProvider as NextThemesProvider, useTheme } from 'next-themes';
import type { ComponentProps, CSSProperties } from 'react';
import { Toaster } from 'sonner';

/**
 * App theme provider. Toggles the `.dark` class on <html> so the semantic design
 * tokens in globals.css switch between the paper and night-ink palettes.
 * Defaults to following the OS setting; the user can override in Settings.
 */
export function ThemeProvider({ children, ...props }: ComponentProps<typeof NextThemesProvider>) {
  return <NextThemesProvider {...props}>{children}</NextThemesProvider>;
}

export function ThemeToaster() {
  const { resolvedTheme } = useTheme();
  return <Toaster position="bottom-center" theme={resolvedTheme === 'dark' ? 'dark' : 'light'} richColors closeButton style={{
    '--normal-bg': 'hsl(var(--popover))', '--normal-text': 'hsl(var(--popover-foreground))', '--normal-border': 'hsl(var(--border))',
    '--success-bg': 'hsl(var(--popover))', '--success-text': 'hsl(var(--success))', '--success-border': 'hsl(var(--success) / .3)',
    '--warning-bg': 'hsl(var(--popover))', '--warning-text': 'hsl(var(--warning))', '--warning-border': 'hsl(var(--warning) / .3)',
    '--error-bg': 'hsl(var(--popover))', '--error-text': 'hsl(var(--destructive))', '--error-border': 'hsl(var(--destructive) / .3)',
    '--info-bg': 'hsl(var(--popover))', '--info-text': 'hsl(var(--foreground))', '--info-border': 'hsl(var(--border))',
  } as CSSProperties} />;
}
