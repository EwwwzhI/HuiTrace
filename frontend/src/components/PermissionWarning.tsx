import { AlertTriangle, Mic, Speaker, RefreshCw } from 'lucide-react';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { openSystemSettings } from '@/services/systemService';
import { useIsLinux } from '@/hooks/usePlatform';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



interface PermissionWarningProps {
  hasMicrophone: boolean;
  hasSystemAudio: boolean;
  onRecheck: () => void;
  isRechecking?: boolean;
}

export function PermissionWarning({
  hasMicrophone,
  hasSystemAudio,
  onRecheck,
  isRechecking = false
}: PermissionWarningProps) {
  useUiTranslation();
  const isLinux = useIsLinux();

  // Don't show on Linux - permission handling is not needed
  if (isLinux) {
    return null;
  }

  // Don't show if both permissions are granted
  if (hasMicrophone && hasSystemAudio) {
    return null;
  }

  const isMacOS = navigator.userAgent.includes('Mac');

  const openMicrophoneSettings = async () => {
    if (isMacOS) {
      try {
        await openSystemSettings('Privacy_Microphone');
      } catch (error) {
        console.error('Failed to open microphone settings:', error);
      }
    }
  };

  const openScreenRecordingSettings = async () => {
    if (isMacOS) {
      try {
        await openSystemSettings('Privacy_ScreenCapture');
      } catch (error) {
        console.error('Failed to open screen recording settings:', error);
      }
    }
  };

  return (
    <div className="max-w-md mb-4 space-y-3">
      {/* Combined Permission Warning - Show when either permission is missing */}
      {(!hasMicrophone || !hasSystemAudio) && (
        <Alert variant="destructive" className="border-warning bg-warning/10">
          <AlertTriangle className="h-5 w-5 text-warning" />
          <AlertTitle className="text-warning font-semibold">
            <div className="flex items-center gap-2">
              {!hasMicrophone && <Mic className="h-4 w-4" />}
              {!hasSystemAudio && <Speaker className="h-4 w-4" />}
              {!hasMicrophone && !hasSystemAudio ? translateUI("Permissions Required") : !hasMicrophone ? translateUI("Microphone Permission Required") : translateUI("System Audio Permission Required")}
            </div>
          </AlertTitle>
          {/* Action Buttons */}
          <div className="mt-4 flex flex-wrap gap-2">
            {isMacOS && !hasMicrophone && (
              <button
                onClick={openMicrophoneSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-warning-foreground bg-warning hover:bg-warning rounded-md transition-colors"
              >
                <Mic className="h-4 w-4" /> {translateUI("Open Microphone Settings")} </button>
            )}
            {isMacOS && !hasSystemAudio && (
              <button
                onClick={openScreenRecordingSettings}
                className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-primary-foreground bg-primary hover:bg-primary rounded-md transition-colors"
              >
                <Speaker className="h-4 w-4" /> {translateUI("Open Screen Recording Settings")} </button>
            )}
            <button
              onClick={onRecheck}
              disabled={isRechecking}
              className="inline-flex items-center gap-2 px-4 py-2 text-sm font-medium text-warning bg-warning/10 hover:bg-warning/10 rounded-md transition-colors disabled:opacity-50"
            >
              <RefreshCw className={`h-4 w-4 ${isRechecking ? 'animate-spin' : ''}`} /> {translateUI("Recheck")} </button>
          </div>
          <AlertDescription className="text-warning mt-2">
            {/* Microphone Warning */}
            {!hasMicrophone && (
              <>
                <p className="mb-3"> {translateUI("HuiTrace needs access to your microphone to record meetings. No microphone devices were detected.")} </p>
                <div className="space-y-2 text-sm mb-4">
                  <p className="font-medium">{translateUI("Please check:")}</p>
                  <ul className="list-disc list-inside ml-2 space-y-1">
                    <li>{translateUI("Your microphone is connected and powered on")}</li>
                    <li>{translateUI("Microphone permission is granted in System Settings")}</li>
                    <li>{translateUI("No other app is exclusively using the microphone")}</li>
                  </ul>
                </div>
              </>
            )}

            {/* System Audio Warning */}
            {!hasSystemAudio && (
              <>
                <p className="mb-3">
                  {hasMicrophone
                    ? translateUI("System audio capture is not available. You can still record with your microphone, but computer audio won't be captured.")
                    : translateUI("System audio capture is also not available.")}
                </p>
                {isMacOS && (
                  <div className="space-y-2 text-sm mb-4">
                    <p className="font-medium">{translateUI("To enable system audio on macOS:")}</p>
                    <ul className="list-disc list-inside ml-2 space-y-1">
                      <li>{translateUI("Install a virtual audio device (e.g., BlackHole 2ch)")}</li>
                      <li>{translateUI("Grant Screen Recording permission to HuiTrace")}</li>
                      <li>{translateUI("Configure your audio routing in Audio MIDI Setup")}</li>
                    </ul>
                  </div>
                )}
              </>
            )}


          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}
