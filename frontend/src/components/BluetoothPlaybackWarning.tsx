"use client";
import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert';
import { Speaker, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';


interface AudioOutputInfo {
  device_name: string;
  is_bluetooth: boolean;
  sample_rate: number | null;
  device_type: string;
}

interface BluetoothPlaybackWarningProps {
  /** Check interval in milliseconds (default: 5000ms / 5 seconds) */
  checkInterval?: number;
  /** Whether to show the warning (default: true for meeting playback pages) */
  enabled?: boolean;
}

export function BluetoothPlaybackWarning({
  checkInterval = 5000,
  enabled = true
}: BluetoothPlaybackWarningProps) {
  useUiTranslation();
  const [isBluetoothActive, setIsBluetoothActive] = useState(false);
  const [deviceName, setDeviceName] = useState<string>('');
  const [isDismissed, setIsDismissed] = useState(false);

  useEffect(() => {
    if (!enabled) return;

    const checkAudioOutput = async () => {
      try {
        const outputInfo = await invoke<AudioOutputInfo>('get_active_audio_output');

        if (outputInfo.is_bluetooth) {
          setIsBluetoothActive(true);
          setDeviceName(outputInfo.device_name);
        } else {
          setIsBluetoothActive(false);
          setIsDismissed(false); // Reset dismissal when switching to non-BT device
        }
      } catch (error) {
        console.error('Failed to check audio output device:', error);
        // Fail silently - don't show warning if we can't detect device
        setIsBluetoothActive(false);
      }
    };

    // Check immediately on mount
    checkAudioOutput();

    // Set up periodic checks
    const interval = setInterval(checkAudioOutput, checkInterval);

    return () => clearInterval(interval);
  }, [checkInterval, enabled]);

  // Don't show warning if Bluetooth not active, already dismissed, or not enabled
  if (!enabled || !isBluetoothActive || isDismissed) {
    return null;
  }

  return (
    <Alert
      className="mb-4 border-warning bg-warning/10 text-warning"
      role="alert"
      aria-live="polite"
    >
      <Speaker className="h-4 w-4 text-warning" />
      <div className="flex items-start justify-between w-full">
        <div className="flex-1">
          <AlertTitle className="text-warning font-semibold"> {translateUI("Bluetooth Playback Detected")} </AlertTitle>
          <AlertDescription className="text-warning mt-1"> {translateUI("You're using")} <strong>{deviceName}</strong> {translateUI("for playback. Recordings may sound distorted or sped up through Bluetooth devices. For accurate review, please use")} <strong>{translateUI("computer speakers")}</strong> {translateUI("or")}{' '}
            <strong>{translateUI("wired headphones")}</strong>.
            <br />
            <a
              href="https://github.com/aydogandagidir/mityu/blob/main/BLUETOOTH_PLAYBACK_NOTICE.md"
              target="_blank"
              rel="noopener noreferrer"
              className="underline hover:text-warning font-medium mt-2 inline-block"
            > {translateUI("Learn why this happens →")} </a>
          </AlertDescription>
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={() => setIsDismissed(true)}
          className="ml-4 h-6 w-6 text-warning hover:text-warning hover:bg-warning/10"
          aria-label={translateUI("Dismiss warning")}
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
    </Alert>
  );
}
