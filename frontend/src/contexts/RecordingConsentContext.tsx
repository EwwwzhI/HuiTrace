'use client';

import React, { createContext, useCallback, useContext, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { RecordingConsentDialog } from '@/components/consent/RecordingConsentDialog';
import { setRememberRecordingPermission, shouldPromptBeforeRecording } from '@/lib/recordingConsent';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';


interface RecordingConsentContextType {
  ensureRecordingConsent: () => Promise<boolean>;
}
const RecordingConsentContext = createContext<RecordingConsentContextType | null>(null);
export function useRecordingConsent(): RecordingConsentContextType {
  const ctx = useContext(RecordingConsentContext);
  if (!ctx) throw new Error('useRecordingConsent must be used within a RecordingConsentProvider');
  return ctx;
}

export function RecordingConsentProvider({ children }: { children: React.ReactNode }) {
  useUiTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = useRef(false);
  const submitting = useRef(false);
  const resolver = useRef<((proceed: boolean) => void) | null>(null);

  useEffect(() => () => { resolver.current?.(false); resolver.current = null; }, []);

  const settle = useCallback((proceed: boolean) => {
    const resolve = resolver.current;
    resolver.current = null;
    active.current = false;
    setIsOpen(false);
    resolve?.(proceed);
  }, []);

  const ensureRecordingConsent = useCallback(async (): Promise<boolean> => {
    // Ignore duplicate starts, including while the preference is still loading.
    if (active.current) return false;
    active.current = true;
    try {
      if (!(await shouldPromptBeforeRecording())) {
        active.current = false;
        return true;
      }
      setError(null);
      setIsOpen(true);
      return new Promise<boolean>((resolve) => { resolver.current = resolve; });
    } catch {
      active.current = false;
      return false;
    }
  }, []);

  const handleConfirm = async (remember: boolean) => {
    if (submitting.current) return;
    submitting.current = true;
    setBusy(true);
    setError(null);
    try {
      if (remember) await setRememberRecordingPermission(true);
      // One-use native approval for this attempt; never remembered per process.
      await invoke('confirm_recording_consent');
      settle(true);
    } catch {
      setError(translateUI("Could not confirm recording permission. Please try again."));
    } finally {
      submitting.current = false;
      setBusy(false);
    }
  };

  return (
    <RecordingConsentContext.Provider value={{ ensureRecordingConsent }}>
      {children}
      <RecordingConsentDialog open={isOpen} busy={busy} error={error} onConfirm={handleConfirm} onCancel={() => { if (!submitting.current) settle(false); }} />
    </RecordingConsentContext.Provider>
  );
}
