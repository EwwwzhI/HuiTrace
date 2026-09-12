"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Save, Loader2 } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';



interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
  onFind?: () => void;
  onOpenFolder: () => Promise<void>;
  hasSummary: boolean;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
  onFind,
  onOpenFolder,
  hasSummary
}: SummaryUpdaterButtonGroupProps) {
  useUiTranslation();
  // Labels show from 2xl only: this group lives in the summary panel toolbar,
  // and that panel is capped at 640px wide (page-content), so the app-wide
  // `lg` label breakpoint would overflow it. Icon-only below 2xl; the title
  // tooltips keep the buttons self-explanatory. shrink-0 everywhere so the
  // toolbar's overflow-x-auto backstop scrolls instead of squishing buttons.
  return (
    <ButtonGroup className="shrink-0">
      {/* Save button */}
      <Button
        variant="outline"
        size="sm"
        className={`shrink-0 ${isDirty ? 'bg-success/10' : ""}`}
        title={isSaving ? translateUI("Saving") : translateUI("Save Changes")}
        onClick={() => {
          Analytics.trackButtonClick('save_changes', 'meeting_details');
          onSave();
        }}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="animate-spin" />
            <span className="hidden 2xl:inline">{translateUI("Saving...")}</span>
          </>
        ) : (
          <>
            <Save />
            <span className="hidden 2xl:inline">{translateUI("Save")}</span>
          </>
        )}
      </Button>

      {/* Copy button */}
      <Button
        variant="outline"
        size="sm"
        title={translateUI("Copy Summary")}
        onClick={() => {
          Analytics.trackButtonClick('copy_summary', 'meeting_details');
          onCopy();
        }}
        disabled={!hasSummary}
        className="shrink-0 cursor-pointer"
      >
        <Copy />
        <span className="hidden 2xl:inline">{translateUI("Copy")}</span>
      </Button>

      {/* Find button */}
      {/* {onFind && (
        <Button
          variant="outline"
          size="sm"
          title="Find in Summary"
          onClick={() => {
            Analytics.trackButtonClick('find_in_summary', 'meeting_details');
            onFind();
          }}
          disabled={!hasSummary}
          className="cursor-pointer"
        >
          <Search />
          <span className="hidden lg:inline">Find</span>
        </Button>
      )} */}
    </ButtonGroup>
  );
}
