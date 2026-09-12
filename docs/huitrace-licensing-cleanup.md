# HuiTrace settings and commercial licensing cleanup

- Removed the empty Beta tab and upstream License tab from settings.
- Removed the trial banner and activation-dialog mounting. The compatibility context performs no IPC or network reads.
- Legacy native licensing commands remain registered for compatibility. Status uses the existing `licensed` wire value with no expiry/key/plan and `configured: false`; this means no activation is required, not that a Mityu commercial license was granted.
- All recording/import entry points keep their shared compatibility hook, which now succeeds independently of saved trial state. Status reads no longer evaluate trials or spawn Polar validation. Activation returns a not-required error; deactivation is a no-op.
- Existing database/keychain records, recording consent checks, upstream copyright/license notices, and development indicator configuration are unchanged.
- Legacy licensing implementation files remain as inactive upstream history. No runtime callers reach their trial evaluation or validation routines.

Validation: TypeScript check, seven frontend tests (settings, context, import, recording controls), and native licensing command status test passed. Native test used CPU/no-default-features; this does not constitute a live microphone or GPU test. Restart/rebuild the native app to load the new backend behavior.
