'use client';

import { useEffect, useState } from 'react';
import { Mic, Monitor, Users, Loader2 } from 'lucide-react';
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



interface RecordingConsentDialogProps {
  open: boolean;
  busy?: boolean;
  error?: string | null;
  onConfirm: (dontShowAgain: boolean) => void;
  onCancel: () => void;
}

export function RecordingConsentDialog({ open, busy = false, error, onConfirm, onCancel }: RecordingConsentDialogProps) {
  useUiTranslation();
  const [dontShowAgain, setDontShowAgain] = useState(false);
  useEffect(() => { if (open) setDontShowAgain(false); }, [open]);

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!next && !busy) onCancel(); }}>
      <DialogContent className="w-[calc(100%-2rem)] max-w-md max-h-[calc(100dvh-2rem)] overflow-y-auto rounded-2xl border-border bg-card p-0 gap-0 text-card-foreground shadow-xl motion-reduce:animate-none">
        <DialogHeader className="px-6 pt-6 pb-5 text-left">
          <div className="mb-3 flex h-11 w-11 items-center justify-center rounded-xl bg-primary/10 text-primary">
            <Mic className="h-5 w-5" aria-hidden="true" />
          </div>
          <DialogTitle className="text-xl leading-7">{translateUI("Ready to record?")}</DialogTitle>
          <DialogDescription className="leading-6"> {translateUI("Confirm you have permission before capturing this conversation.")} </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 px-6 pb-5">
          <div className="rounded-xl border border-border bg-muted/40 p-4 space-y-4">
            <div className="flex items-start gap-3">
              <Monitor className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
              <div className="text-sm leading-6">
                <p className="font-medium">{translateUI("Your selected audio sources")}</p>
                <p className="text-muted-foreground">{translateUI("Recording uses the microphone and system audio selected in your recording settings.")}</p>
              </div>
            </div>
            <div className="flex items-start gap-3">
              <Users className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
              <div className="text-sm leading-6">
                <p className="font-medium">{translateUI("Let everyone know")}</p>
                <p className="text-muted-foreground">{translateUI("By continuing, you confirm participants are informed and you have any consent required to record.")}</p>
              </div>
            </div>
          </div>
          <label className="flex min-h-11 cursor-pointer items-start gap-3 rounded-lg py-2 text-sm">
            <input type="checkbox" checked={dontShowAgain} disabled={busy}
              onChange={(event) => setDontShowAgain(event.target.checked)}
              aria-describedby="recording-remember-hint"
              className="mt-0.5 h-4 w-4 shrink-0 accent-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-4 focus-visible:outline-ring" />
            <span>
              <span className="font-medium">{translateUI("Don’t ask again on this device")}</span>
              <span id="recording-remember-hint" className="mt-1 block text-xs leading-5 text-muted-foreground">{translateUI("Enables “Remember recording permission” in Settings → Recordings. You can turn it off anytime.")}</span>
            </span>
          </label>
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        </div>

        <DialogFooter className="gap-2 border-t border-border bg-muted/30 px-6 py-4 sm:space-x-0">
          <Button variant="outline" className="min-h-11" disabled={busy} onClick={onCancel}>{translateUI("Cancel")}</Button>
          <Button className="min-h-11" disabled={busy} onClick={() => onConfirm(dontShowAgain)}>
            {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin motion-reduce:animate-none" aria-hidden="true" /> : <Mic className="mr-2 h-4 w-4" aria-hidden="true" />}
            {busy ? translateUI("Confirming…") : translateUI("Allow & start recording")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export default RecordingConsentDialog;
