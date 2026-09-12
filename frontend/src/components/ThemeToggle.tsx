'use client';

import { useEffect, useState, useRef } from 'react';
import { useTheme } from 'next-themes';
import { Monitor, Moon, Sun } from 'lucide-react';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';


const OPTIONS = [
  { value: 'system', get label() { return translateUI("System"); }, Icon: Monitor },
  { value: 'light', get label() { return translateUI("Light"); }, Icon: Sun },
  { value: 'dark', get label() { return translateUI("Dark"); }, Icon: Moon },
] as const;

/**
 * Segmented System / Light / Dark control backed by next-themes. Guarded with a
 * `mounted` flag: the resolved theme is only known on the client, so we render a
 * neutral skeleton on the server to avoid a hydration mismatch.
 */
export function ThemeToggle({ compact = false }: { compact?: boolean }) {
  useUiTranslation();
  const { theme, setTheme } = useTheme();
  const [mounted, setMounted] = useState(false);
  const transitionTimer = useRef<ReturnType<typeof setTimeout>>();
  useEffect(() => setMounted(true), []);
  useEffect(() => () => { clearTimeout(transitionTimer.current); document.documentElement.classList.remove('theme-changing'); }, []);
  const chooseTheme = (value: string) => {
    clearTimeout(transitionTimer.current);
    document.documentElement.classList.add('theme-changing');
    setTheme(value);
    transitionTimer.current = setTimeout(() => document.documentElement.classList.remove('theme-changing'), 260);
  };

  const active = mounted ? (theme ?? 'system') : undefined;

  return (
    <div
      role="radiogroup"
      aria-label={translateUI("Theme")}
      className={`v2-theme ${compact ? 'v2-theme-compact' : ''}`}
    >
      <span className="v2-theme-indicator" aria-hidden="true" style={{ opacity: mounted ? 1 : 0, transform: `translateX(${Math.max(0, OPTIONS.findIndex(option => option.value === active)) * 100}%)` }} />
      {OPTIONS.map(({ value, label, Icon }) => {
        const isActive = active === value;
        return (
          <button
            key={value}
            type="button"
            role="radio"
            aria-checked={isActive}
            aria-label={label}
            title={compact ? label : undefined}
            tabIndex={isActive || (!mounted && value === 'system') ? 0 : -1}
            onClick={() => chooseTheme(value)}
            onKeyDown={(event) => {
              const index = OPTIONS.findIndex(option => option.value === value);
              const next = event.key === 'Home' ? 0 : event.key === 'End' ? 2 : ['ArrowRight', 'ArrowDown'].includes(event.key) ? (index + 1) % 3 : ['ArrowLeft', 'ArrowUp'].includes(event.key) ? (index + 2) % 3 : -1;
              if (next < 0) return;
              event.preventDefault();
              chooseTheme(OPTIONS[next].value);
              event.currentTarget.parentElement?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
            }}
            className={[
              'relative z-10 inline-flex items-center justify-center gap-1.5 rounded-lg text-xs font-medium transition-colors',
              isActive
                ? 'text-foreground'
                : 'text-muted-foreground hover:text-foreground',
            ].join(' ')}
          >
            <Icon className="h-4 w-4" aria-hidden />
            {!compact && label}
          </button>
        );
      })}
    </div>
  );
}
