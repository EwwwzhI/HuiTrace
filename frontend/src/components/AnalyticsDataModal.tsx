'use client';

import { X, Info, Shield } from 'lucide-react';
import { APP_VERSION } from '@/lib/appVersion';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';


interface AnalyticsDataModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirmDisable: () => void;
}

export default function AnalyticsDataModal({ isOpen, onClose, onConfirmDisable }: AnalyticsDataModalProps) {
  useUiTranslation();
  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-card rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between p-6 border-b border-border">
          <div className="flex items-center gap-3">
            <Shield className="w-6 h-6 text-primary" />
            <h2 className="text-xl font-semibold text-foreground">{translateUI("What Analytics Collects")}</h2>
          </div>
          <button
            onClick={onClose}
            className="text-muted-foreground hover:text-muted-foreground transition-colors"
          >
            <X className="w-5 h-5" />
          </button>
        </div>

        {/* Content */}
        <div className="p-6 space-y-6">
          {/* Privacy Notice */}
          <div className="bg-success/10 dark:bg-success/10 border border-success/30 dark:border-success/25 rounded-lg p-4">
            <div className="flex items-start gap-3">
              <Info className="w-5 h-5 text-success dark:text-success mt-0.5 flex-shrink-0" />
              <div className="text-sm text-success dark:text-success">
                <p className="font-semibold mb-1">{translateUI("Analytics Is Optional and Content-Free")}</p>
                <p> {translateUI("Analytics is off by default. If you enable it, HuiTrace sends pseudonymous usage metrics under a random installation identifier. It never sends meeting content, meeting identifiers, names, file paths, raw errors, or account details.")} </p>
              </div>
            </div>
          </div>

          {/* Data Categories */}
          <div className="space-y-4">
            <h3 className="text-lg font-semibold text-foreground">{translateUI("Data We Collect When Enabled:")}</h3>

            {/* Model Families */}
            <div className="border border-border rounded-lg p-4">
              <h4 className="font-semibold text-foreground mb-2">{translateUI("1. Model Families")}</h4>
              <ul className="text-sm text-foreground space-y-1 ml-4">
                <li>{translateUI("• Transcription model family (e.g., \"whisper\", \"parakeet\")")}</li>
                <li>{translateUI("• Summary model family (e.g., \"llama\", \"claude\", \"custom\")")}</li>
                <li>{translateUI("• Model provider (e.g., \"Local\", \"Ollama\", \"OpenRouter\")")}</li>
              </ul>
              <p className="text-xs text-muted-foreground mt-2 italic">{translateUI("Exact or custom model names are reduced to a fixed family bucket before sending")}</p>
            </div>

            {/* Meeting Metrics */}
            <div className="border border-border rounded-lg p-4">
              <h4 className="font-semibold text-foreground mb-2">{translateUI("2. Aggregate Meeting Metrics")}</h4>
              <ul className="text-sm text-foreground space-y-1 ml-4">
                <li>{translateUI("• Recording duration (e.g., \"125 seconds\")")}</li>
                <li>{translateUI("• Pause duration (e.g., \"5 seconds\")")}</li>
                <li>{translateUI("• Number of transcript segments")}</li>
                <li>{translateUI("• Number of audio chunks processed")}</li>
              </ul>
              <p className="text-xs text-muted-foreground mt-2 italic">{translateUI("Helps us optimize performance and understand usage patterns")}</p>
            </div>

            {/* Device Types */}
            <div className="border border-border rounded-lg p-4">
              <h4 className="font-semibold text-foreground mb-2">{translateUI("3. Device Types (Not Names)")}</h4>
              <ul className="text-sm text-foreground space-y-1 ml-4">
                <li>{translateUI("• Microphone type: \"Bluetooth\" or \"Wired\" or \"Unknown\"")}</li>
                <li>{translateUI("• System audio type: \"Bluetooth\" or \"Wired\" or \"Unknown\"")}</li>
              </ul>
              <p className="text-xs text-muted-foreground mt-2 italic">{translateUI("Helps us improve compatibility, NOT the actual device names")}</p>
            </div>

            {/* Usage Patterns */}
            <div className="border border-border rounded-lg p-4">
              <h4 className="font-semibold text-foreground mb-2">{translateUI("4. App Usage Patterns")}</h4>
              <ul className="text-sm text-foreground space-y-1 ml-4">
                <li>{translateUI("• App started/stopped events")}</li>
                <li>{translateUI("• Session duration")}</li>
                <li>{translateUI("• Feature usage (e.g., \"settings changed\")")}</li>
                <li>{translateUI("• Success or error occurrence only, never the error message")}</li>
              </ul>
              <p className="text-xs text-muted-foreground mt-2 italic">{translateUI("Helps us improve user experience")}</p>
            </div>

            {/* Platform Info */}
            <div className="border border-border rounded-lg p-4">
              <h4 className="font-semibold text-foreground mb-2">{translateUI("5. Platform Information")}</h4>
              <ul className="text-sm text-foreground space-y-1 ml-4">
                <li>{translateUI("• Operating-system family (e.g., \"macOS\", \"Windows\")")}</li>
                <li>{translateUI("• App version (automatically included in all events)")}</li>
                <li>{translateUI("• Architecture (e.g., \"x86_64\", \"aarch64\")")}</li>
              </ul>
              <p className="text-xs text-muted-foreground mt-2 italic">{translateUI("Helps us prioritize platform support")}</p>
            </div>
          </div>

          {/* What We DON'T Collect */}
          <div className="bg-destructive/10 dark:bg-destructive/10 border border-destructive/30 dark:border-destructive/25 rounded-lg p-4">
            <h4 className="font-semibold text-destructive dark:text-destructive mb-2">{translateUI("What We DON'T Collect:")}</h4>
            <ul className="text-sm text-destructive dark:text-destructive space-y-1 ml-4">
              <li>{translateUI("• ❌ Meeting names or titles")}</li>
              <li>{translateUI("• ❌ File names, file paths, or meeting folders")}</li>
              <li>{translateUI("• ❌ Meeting transcripts or content")}</li>
              <li>{translateUI("• ❌ Audio recordings")}</li>
              <li>{translateUI("• ❌ Device names (only types: Bluetooth/Wired)")}</li>
              <li>{translateUI("• ❌ Meeting IDs or the random installation ID in custom event fields")}</li>
              <li>{translateUI("• ❌ Raw error messages, provider responses, or settings values")}</li>
              <li>{translateUI("• ❌ Exact OS versions, user-agent strings, email, or account details")}</li>
            </ul>
          </div>

          <p className="text-xs text-muted-foreground"> {translateUI("PostHog receives the random installation identifier only as the event's pseudonymous distinct ID so events from one installation can be grouped. The identifier is not a meeting ID, account ID, or email address, and it is not added to custom event fields.")} </p>

          {/* Example Event */}
          <div className="bg-muted border border-border rounded-lg p-4">
            <h4 className="font-semibold text-foreground mb-2">{translateUI("Example Event:")}</h4>
            <pre className="text-xs text-foreground overflow-x-auto">
              {`{
  "event": "meeting_ended",
  "app_version": "${APP_VERSION}",
  "transcription_provider": "parakeet",
  "transcription_model_family": "parakeet",
  "summary_provider": "ollama",
  "summary_model_family": "llama",
  "total_duration_seconds": "125.5",
  "microphone_device_type": "Wired",
  "system_audio_device_type": "Bluetooth",
  "chunks_processed": "150",
  "had_fatal_error": "false"
}`}
            </pre>
          </div>
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between gap-4 p-6 border-t border-border bg-muted">
          <button
            onClick={onClose}
            className="px-4 py-2 text-foreground bg-card border border-border rounded-md hover:bg-muted transition-colors"
          > {translateUI("Keep Analytics Enabled")} </button>
          <button
            onClick={onConfirmDisable}
            className="px-4 py-2 text-destructive-foreground bg-destructive rounded-md hover:bg-destructive transition-colors"
          > {translateUI("Confirm: Disable Analytics")} </button>
        </div>
      </div>
    </div>
  );
}
