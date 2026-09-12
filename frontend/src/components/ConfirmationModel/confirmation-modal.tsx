"use client";

import { ChevronDown } from 'lucide-react';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog';

interface ConfirmationModalProps {
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
  title: string;
  description: string;
  details?: string;
  detailsLabel?: string;
  isOpen: boolean;
  isBusy?: boolean;
}

export function ConfirmationModal({
  onConfirm,
  onCancel,
  title,
  description,
  details,
  detailsLabel = 'Learn about data deletion',
  isOpen,
  isBusy = false,
}: ConfirmationModalProps) {
  useUiTranslation();

  return (
    <Dialog
      open={isOpen}
      onOpenChange={(open) => {
        if (!open && !isBusy) onCancel();
      }}
    >
      <DialogContent
        className="sm:max-w-[440px]"
        aria-busy={isBusy}
        onEscapeKeyDown={(event) => {
          if (isBusy) event.preventDefault();
        }}
        onPointerDownOutside={(event) => {
          if (isBusy) event.preventDefault();
        }}
      >
        <DialogHeader className="space-y-2 pr-6">
          <DialogTitle>{translateUI(title)}</DialogTitle>
          <DialogDescription className="text-[15px] leading-6">
            {translateUI(description)}
          </DialogDescription>
        </DialogHeader>

        {details && (
          <details className="group rounded-lg border border-border/70 bg-muted/30 px-3.5 py-3 text-sm">
            <summary className="flex cursor-pointer list-none items-center justify-between gap-3 font-medium text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
              {translateUI(detailsLabel)}
              <ChevronDown className="h-4 w-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180" aria-hidden="true" />
            </summary>
            <p className="mt-2 leading-5 text-muted-foreground">{translateUI(details)}</p>
          </details>
        )}

        <DialogFooter className="mt-2 gap-2 sm:space-x-0">
          <button
            type="button"
            onClick={onCancel}
            disabled={isBusy}
            className="inline-flex h-10 items-center justify-center rounded-md px-4 text-sm font-medium text-foreground transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          >
            {translateUI('Cancel')}
          </button>
          <button
            type="button"
            onClick={() => void onConfirm()}
            disabled={isBusy}
            className="inline-flex h-10 min-w-20 items-center justify-center rounded-md bg-destructive px-4 text-sm font-medium text-destructive-foreground transition-colors hover:bg-destructive/90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-destructive focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          >
            {isBusy ? translateUI('Deleting…') : translateUI('Delete')}
          </button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
