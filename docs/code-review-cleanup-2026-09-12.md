# Code review and cleanup — 2026-09-12

Scope: frontend unused-code analysis, references in the current worktree, and related build configuration. This is not a full Rust/native security or correctness audit.

## Cleaned up

- Removed obsolete sidebar model-settings state, save handlers, two configuration reads, and the model-update event subscription. The sidebar no longer renders the old settings dialog. The native tray already navigates to `/settings`, so its obsolete `window.openSettings` callback was also removed.
- Removed the unreferenced, empty Beta settings component and obsolete settings-test mocks.
- Removed unused imports using TypeScript's language service; preserved existing import formatting where possible.
- Removed the write-only onboarding completion state and duplicate, unused title keyboard handlers. The active textarea handler still supports Shift+Enter.
- Removed global route animations and settings-tab entrance animations in the preceding change.
- Consolidated Next build-cache ignore patterns and removed machine-specific performance-build type paths. Existing cache files were not deleted.

## Verification

- TypeScript standard type check passed.
- Frontend suite: 35 files, 228 tests passed.
- ESLint: no errors, 41 warnings, primarily hook dependency warnings.
- Git whitespace check passed.

## Remaining review items

- Strict unused-local/parameter analysis still reports legacy handlers, callback parameters and write-only state, particularly in recording and summary components. These need behavior-specific review rather than automatic deletion.
- Legacy commercial licensing UI/helpers remain unreachable from the current UI. The existing cleanup document explicitly retains upstream implementation history; shared licensing compatibility hooks still have live recording/import callers, so this pass does not remove that subsystem.
- Hook dependency warnings need targeted stale-state and lifecycle testing; adding every suggested dependency mechanically could change recording or polling behavior.
- No native build, microphone/GPU test, or desktop visual validation was performed in this pass.

Following review, the user requested committing and pushing the current source, assets, and documentation together. Local browser-debug output is excluded.
