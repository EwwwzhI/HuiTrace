'use client';

import { useEffect, useState } from 'react';
import { Switch } from '@/components/ui/switch';
import { Loader2, ShieldCheck } from 'lucide-react';
import { computeConsentGate, readRecordingConsentState, setRememberRecordingPermission, RECORDING_CONSENT_CHANGED } from '@/lib/recordingConsent';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



export default function RecordingConsentSettings() {
  useUiTranslation();
  const [remember, setRemember] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      const state = await readRecordingConsentState();
      if (active) setRemember(!computeConsentGate(state));
    };
    void refresh();
    window.addEventListener(RECORDING_CONSENT_CHANGED, refresh);
    return () => { active = false; window.removeEventListener(RECORDING_CONSENT_CHANGED, refresh); };
  }, []);

  const toggle = async (enabled: boolean) => {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      await setRememberRecordingPermission(enabled);
      setRemember(enabled);
    } catch {
      setError(translateUI("Could not save your preference. Please try again."));
    } finally { setSaving(false); }
  };

  return (
    <div className="space-y-4">
      <div className="flex items-start gap-3">
        <div className="rounded-lg bg-primary/10 p-2 text-primary"><ShieldCheck className="h-5 w-5" aria-hidden="true" /></div>
        <div>
          <h3 className="text-base font-semibold text-foreground">{translateUI("Recording permission")}</h3>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">{translateUI("Choose whether to confirm before each recording on this device.")}</p>
        </div>
      </div>
      <div className="flex items-center justify-between gap-4 rounded-xl border border-border bg-muted/40 p-4">
        <div>
          <label htmlFor="remember-recording-permission" className="cursor-pointer text-sm font-medium text-foreground">{translateUI("Remember recording permission")}</label>
          <p id="recording-permission-description" className="mt-1 text-sm leading-6 text-muted-foreground">
            {remember ? translateUI("On · Recording starts without asking again.") : translateUI("Off · Confirm permission each time you start recording.")}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          {(saving || remember === null) && <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-label={translateUI("Loading")} />}
          <Switch id="remember-recording-permission" aria-describedby="recording-permission-description" checked={remember ?? false} disabled={saving || remember === null} onCheckedChange={toggle} />
        </div>
      </div>
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      <p className="text-xs leading-5 text-muted-foreground">{translateUI("You still need participants’ permission for every recording. This preference controls the in-app reminder; microphone and system audio access are managed by your operating system.")}</p>
    </div>
  );
}
