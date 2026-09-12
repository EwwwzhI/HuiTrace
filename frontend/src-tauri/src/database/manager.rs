use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Result, Sqlite, SqlitePool, Transaction};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tauri::Manager;
use tokio::sync::Mutex;

use crate::context::AuthContext;
use crate::database::deletion::ArtifactEraseReport;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RecordingCleanupStatus {
    Removed,
    #[default]
    Absent,
    RetainedUntrusted,
    RetainedShared,
    RetainedOwnershipMismatch,
}

impl RecordingCleanupStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Absent => "absent",
            Self::RetainedUntrusted => "retained_untrusted",
            Self::RetainedShared => "retained_shared",
            Self::RetainedOwnershipMismatch => "retained_ownership_mismatch",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct VerifiedMeetingDeletion {
    pub already_absent: bool,
    pub artifacts: ArtifactEraseReport,
    pub recording_cleanup: RecordingCleanupStatus,
    /// Logical rows are gone, but the best-effort FTS/WAL/free-page compaction
    /// still needs a later retry. Recording cleanup is reported independently.
    pub maintenance_pending: bool,
}

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
    /// VACUUM and WAL truncation are whole-database operations. Serialize them
    /// so concurrent delete commands cannot make each other's verification busy.
    deletion_lock: Arc<Mutex<()>>,
    /// Records which branch of the ADR-0014 at-rest encryption decision actually ran
    /// for this open, so the UI can warn on a plaintext fallback (follow-up to
    /// ADR-0014). `true` = the pool was opened with the SQLCipher key (data encrypted
    /// at rest); `false` = a fail-open plaintext branch was taken because the keychain
    /// key was unavailable on a plaintext/fresh DB. This does NOT influence the open
    /// logic; it only mirrors the outcome recorded in `DatabaseManager::new`.
    at_rest_encrypted: bool,
}

impl DatabaseManager {
    pub async fn new(tauri_db_path: &str, backend_db_path: &str) -> Result<Self> {
        if let Some(parent_dir) = Path::new(tauri_db_path).parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).map_err(sqlx::Error::Io)?;
            }
        }

        // Legacy import stays plaintext on disk here; the SQLCipher conversion
        // below encrypts whatever plaintext file we end up with (copied legacy or a
        // pre-existing plaintext .sqlite) before the keyed pool opens.
        if !Path::new(tauri_db_path).exists() && Path::new(backend_db_path).exists() {
            log::info!(
                "Copying database from {} to {}",
                backend_db_path,
                tauri_db_path
            );
            fs::copy(backend_db_path, tauri_db_path).map_err(sqlx::Error::Io)?;
        }

        // At-rest encryption (BACKLOG B3, docs/SECURITY_PRIVACY.md "Encryption",
        // ADR-0014) with a GUARDRAILED local-first fallback. The 256-bit DB key lives
        // in the OS keychain (`secrets::db`). The posture depends on the on-disk state
        // of the file, decided BEFORE the key is touched:
        //
        //   * ENCRYPTED file  → FAIL CLOSED. Read the key with the NON-creating
        //     `get_hex()` (a missing entry must NEVER mint a fresh key over live
        //     ciphertext — that would strand the data behind a key that cannot decrypt
        //     it). If the key is present we open keyed as normal; if it is missing or
        //     the store errored we return an error and DO NOT open — never a keyless
        //     open of ciphertext, never a regenerated key. Local-first is preserved by
        //     "retry next launch / restore the key", not by exposing plaintext.
        //
        //   * PLAINTEXT or ABSENT file → fail-open is permitted (CLAUDE.md §0.1: the
        //     capture→transcript→summary→store path must stay functional). Use
        //     `get_or_create_hex()` (mint-on-first-run is correct here), then convert
        //     via `ensure_encrypted`. If the key store is unavailable, or the one-time
        //     conversion fails (e.g. a SQLCipher-less runtime where `sqlcipher_export`
        //     is missing), we log LOUDLY and open the file UNENCRYPTED, retried next
        //     launch. This is safe because the file was plaintext anyway — no
        //     confidentiality is lost relative to the pre-B3 state, and no ciphertext
        //     is ever opened keyless.
        //
        // Net accepted tradeoff: brand-new plaintext data is possible ONLY on a
        // machine that was never successfully encrypted (first run with a broken
        // keychain); it heals on the next keyed launch. An in-app downgrade WARNING is
        // tracked as follow-up (ADR-0014). `Zeroizing<String>` scrubs the key from
        // memory on drop (below, right after the pool opens).
        let db_path = Path::new(tauri_db_path);
        let is_plaintext = crate::database::encryption::is_plaintext_db(db_path)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(format!("{e:#}"))))?;
        let on_disk_encrypted = db_path.exists() && !is_plaintext;

        // `key_hex` holds the key material only if a keyed open is actually happening;
        // `None` means "open plaintext" (fail-open branch only, never for ciphertext).
        let key_hex: Option<zeroize::Zeroizing<String>> = if on_disk_encrypted {
            // FAIL CLOSED path: non-creating read; a missing/errored key must abort.
            match crate::secrets::db::get_hex() {
                Ok(Some(key)) => {
                    // `ensure_encrypted` is a no-op on an already-encrypted file; call
                    // it for symmetry, then open keyed.
                    crate::database::encryption::ensure_encrypted(db_path, &key)
                        .await
                        .map_err(|e| {
                            sqlx::Error::Configuration(
                                format!("database encryption conversion failed: {e:#}").into(),
                            )
                        })?;
                    Some(key)
                }
                Ok(None) | Err(_) => {
                    // Encrypted on disk but no usable key: DO NOT open, DO NOT
                    // regenerate. This is NOT corruption — return a distinct
                    // Configuration error so the recovery path in
                    // `new_from_app_handle` does not misclassify it and delete the WAL.
                    return Err(sqlx::Error::Configuration(
                        "local database is encrypted but its key is unavailable (the OS keychain \
                         is locked, or the 'db-key' entry is missing); refusing to open — retry \
                         once the keychain is unlocked, or restore the key"
                            .into(),
                    ));
                }
            }
        } else {
            // FAIL OPEN path (plaintext or absent file only). Mint-on-first-run is
            // correct here; a store/conversion failure degrades to a plaintext open.
            match crate::secrets::db::get_or_create_hex() {
                Ok(key) => match crate::database::encryption::ensure_encrypted(db_path, &key).await
                {
                    Ok(()) => Some(key),
                    Err(e) => {
                        // The file is still plaintext (conversion failed); open it
                        // UNENCRYPTED for now. Do NOT claim it is encrypted/safe.
                        log::error!(
                            "At-rest DB encryption conversion FAILED ({e:#}); opening the local \
                             database UNENCRYPTED (plaintext at rest) for now — will retry the \
                             conversion on the next launch. No data is lost; the file was already \
                             plaintext."
                        );
                        None
                    }
                },
                Err(e) => {
                    // Keychain unavailable on a fresh/plaintext file: open plaintext,
                    // retry next launch. Accurate wording — this is UNENCRYPTED.
                    log::error!(
                        "DB encryption key unavailable ({e:#}); opening the local database \
                         UNENCRYPTED (plaintext at rest) for now — will retry acquiring the key \
                         and encrypting on the next launch."
                    );
                    None
                }
            }
        };

        // Keyed pool ONLY when a key is in hand (encrypted file with a good key, or a
        // freshly converted/created file); otherwise a plaintext open on the fail-open
        // branch so a first-run encryption hiccup never locks the user out of their
        // own local data. A keyless open of an ENCRYPTED file cannot happen here — that
        // branch returned above. `PRAGMA key` is a reserved slot sqlx executes FIRST
        // when present. WAL mode is preserved explicitly; a fresh file is created
        // already-encrypted (or plaintext) via create_if_missing.
        let mut options = SqliteConnectOptions::new()
            .filename(tauri_db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(15))
            // SQLite core secure-delete does not cover FTS shadow tables; the
            // persistent FTS5 option is installed by migration 20260714010000.
            .pragma("secure_delete", "ON");
        if let Some(k) = &key_hex {
            options = options.pragma("key", crate::secrets::db::pragma_key_value(k));
        }
        // Record the encryption outcome BEFORE `key_hex` is dropped. A key in hand
        // means the pool is opening keyed (encrypted file + good key, or a
        // freshly created/converted file); `None` is only ever the fail-open plaintext
        // branch (both keyless-open paths above logged loudly). This is purely a
        // read-out of the decision already made — it does not alter the open logic.
        let at_rest_encrypted = key_hex.is_some();

        let pool = SqlitePool::connect_with(options).await?;
        // Key material is no longer needed once the pool holds an open connection;
        // drop it now to zeroize it promptly (defense in depth).
        drop(key_hex);

        sqlx::migrate!("./migrations").run(&pool).await?;

        // One-time, idempotent migration of any legacy plaintext BYOK API keys out
        // of the settings/transcript_settings columns and into the OS credential
        // store (CLAUDE.md §0.7/§3, docs/SECURITY_PRIVACY.md "Secrets"). Runs on
        // every init path (fresh, legacy-import, recovery) and fully offline.
        // NON-FATAL: a locked/unavailable keychain must never block opening the DB
        // or offline operation (local-first invariant). Legacy plaintext is still
        // scrubbed in that case and the affected provider requires user re-entry.
        let startup_ctx = crate::context::current();
        let removed_plaintext = crate::database::repositories::setting::SettingsRepository::migrate_plaintext_keys_to_keychain(
            &pool,
            &startup_ctx,
        )
        .await?;
        if removed_plaintext > 0 {
            log::info!(
                "Removed {} legacy plaintext API key(s) from SQLite",
                removed_plaintext
            );
        }

        let manager = DatabaseManager {
            pool,
            deletion_lock: Arc::new(Mutex::new(())),
            at_rest_encrypted,
        };

        // New v1.0.4 databases and upgrades receive one historical FTS/free-page
        // compaction. A failure is non-fatal for local-first startup; the durable,
        // content-free marker stays pending and every later delete retries it.
        if let Err(error) = manager.resume_pending_privacy_maintenance().await {
            log::warn!("Local privacy maintenance remains pending and will be retried: {error:#}");
        }

        Ok(manager)
    }

    // NOTE: So for the first time users they needs to start the application
    // after they can just delete the existing .sqlite file and then copy the existing .db file to
    // the current app dir, So the system detects legacy db and copy it and starts with that data
    // (Newly created .sqlite with the copied content from .db)
    pub async fn new_from_app_handle(app_handle: &tauri::AppHandle) -> Result<Self> {
        // Resolve the app's data directory
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Define database paths
        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();
        // Legacy backend DB path (for auto-migration if exists)
        let backend_db_path = app_data_dir
            .join("meeting_minutes.db")
            .to_string_lossy()
            .to_string();

        // WAL file paths for defensive cleanup
        let wal_path = app_data_dir.join("meeting_minutes.sqlite-wal");
        let shm_path = app_data_dir.join("meeting_minutes.sqlite-shm");

        log::info!("Tauri DB path: {}", tauri_db_path);
        log::info!("Legacy backend DB path: {}", backend_db_path);

        // Try to open database with defensive WAL handling
        match Self::new(&tauri_db_path, &backend_db_path).await {
            Ok(db_manager) => {
                log::info!("Database opened successfully");
                Ok(db_manager)
            }
            Err(e) => {
                // Check if error is due to corrupted WAL file
                let error_msg = e.to_string();
                if error_msg.contains("malformed") || error_msg.contains("corrupt") {
                    log::warn!("Database appears corrupted, likely due to orphaned WAL file. Attempting recovery...");
                    log::warn!("Error details: {}", error_msg);

                    // Delete potentially corrupted WAL/SHM files
                    if wal_path.exists() {
                        match fs::remove_file(&wal_path) {
                            Ok(_) => log::info!("Removed orphaned WAL file: {:?}", wal_path),
                            Err(e) => log::warn!("Failed to remove WAL file: {}", e),
                        }
                    }
                    if shm_path.exists() {
                        match fs::remove_file(&shm_path) {
                            Ok(_) => log::info!("Removed orphaned SHM file: {:?}", shm_path),
                            Err(e) => log::warn!("Failed to remove SHM file: {}", e),
                        }
                    }

                    // Retry connection without WAL files
                    log::info!("Retrying database connection after WAL cleanup...");
                    match Self::new(&tauri_db_path, &backend_db_path).await {
                        Ok(db_manager) => {
                            log::info!("Database opened successfully after WAL recovery");
                            Ok(db_manager)
                        }
                        Err(retry_err) => {
                            log::error!(
                                "Database connection failed even after WAL cleanup: {}",
                                retry_err
                            );
                            Err(retry_err)
                        }
                    }
                } else {
                    // Not a WAL-related error, propagate original error
                    log::error!("Database connection failed: {}", error_msg);
                    Err(e)
                }
            }
        }
    }

    /// Check if this is the first launch (sqlite database doesn't exist yet)
    pub async fn is_first_launch(app_handle: &tauri::AppHandle) -> Result<bool> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        let tauri_db_path = app_data_dir.join("meeting_minutes.sqlite");

        Ok(!tauri_db_path.exists())
    }

    /// Import a legacy database from the specified path and initialize
    pub async fn import_legacy_database(
        app_handle: &tauri::AppHandle,
        legacy_db_path: &str,
    ) -> Result<Self> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Copy legacy database to app data directory as meeting_minutes.db
        let target_legacy_path = app_data_dir.join("meeting_minutes.db");
        log::info!(
            "Copying legacy database from {} to {}",
            legacy_db_path,
            target_legacy_path.display()
        );

        fs::copy(legacy_db_path, &target_legacy_path).map_err(|e| sqlx::Error::Io(e))?;

        // Now use the standard initialization which will detect and migrate the legacy db
        Self::new_from_app_handle(app_handle).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Whether the local database was opened ENCRYPTED at rest (SQLCipher key applied)
    /// for this session. `false` means the ADR-0014 fail-open plaintext branch was
    /// taken (keychain key unavailable on a plaintext/fresh DB), so the file is
    /// plaintext at rest and the UI should warn. Reflects the decision made in `new`;
    /// it does not re-check the file on disk.
    pub fn is_at_rest_encrypted(&self) -> bool {
        self.at_rest_encrypted
    }

    async fn resume_pending_privacy_maintenance(&self) -> anyhow::Result<()> {
        if crate::database::deletion::privacy_maintenance_required(&self.pool).await? {
            crate::database::deletion::complete_privacy_maintenance(&self.pool).await?;
        }
        Ok(())
    }

    /// Delete tenant-scoped meeting/search rows and, when the recording folder
    /// can be proven safe, its HuiTrace-managed artifacts. A folder that is
    /// shared, outside the trusted root, linked, or owned by another workspace
    /// is left untouched and reported separately; it never blocks logical
    /// meeting deletion. Operational filesystem failures remain fatal so a
    /// partially completed cleanup is never reported as successful.
    pub async fn delete_meeting_verified(
        &self,
        ctx: &AuthContext,
        meeting_id: &str,
        allowed_recording_roots: Vec<PathBuf>,
    ) -> anyhow::Result<VerifiedMeetingDeletion> {
        let _guard = self.deletion_lock.lock().await;

        // A previous crash may have committed logical deletion before VACUUM.
        // Retry it first, but do not let a persistent checkpoint/VACUUM failure
        // make every future meeting undeletable. The content-free marker stays
        // pending and this request performs its own tenant-scoped logical delete.
        let mut maintenance_pending = false;
        if self.resume_pending_privacy_maintenance().await.is_err() {
            log::warn!(
                "Local privacy maintenance is still pending; continuing with logical meeting deletion"
            );
            maintenance_pending = true;
        }

        let meeting =
            crate::database::repositories::meeting::MeetingsRepository::get_meeting_metadata(
                &self.pool, ctx, meeting_id,
            )
            .await?;
        let Some(meeting) = meeting else {
            // Idempotent and non-enumerating: an absent or foreign-tenant id has
            // the same externally visible result and reveals no row existence.
            return Ok(VerifiedMeetingDeletion {
                already_absent: true,
                artifacts: ArtifactEraseReport::default(),
                recording_cleanup: RecordingCleanupStatus::Absent,
                maintenance_pending,
            });
        };

        let mut artifacts = ArtifactEraseReport::default();
        let mut recording_cleanup = RecordingCleanupStatus::Absent;
        if let Some(folder_path) = meeting.folder_path.filter(|path| !path.trim().is_empty()) {
            let shared = crate::database::repositories::meeting::MeetingsRepository::recording_folder_has_other_same_workspace_reference(
                &self.pool,
                ctx,
                meeting_id,
                &folder_path,
            )
            .await
            .map_err(|error| anyhow::anyhow!("check for a shared recording folder: {error}"))?;

            if shared {
                recording_cleanup = RecordingCleanupStatus::RetainedShared;
            } else {
                let target = PathBuf::from(folder_path);
                let target_was_present = target.exists();
                let erase_ctx = ctx.clone();
                match tokio::task::spawn_blocking(move || {
                    crate::database::deletion::erase_recording_folder(
                        &target,
                        &allowed_recording_roots,
                        &erase_ctx,
                    )
                })
                .await
                .map_err(|error| anyhow::anyhow!("recording cleanup task failed: {error}"))?
                {
                    Ok(report) => {
                        artifacts = report;
                        recording_cleanup = if target_was_present {
                            RecordingCleanupStatus::Removed
                        } else {
                            RecordingCleanupStatus::Absent
                        };
                    }
                    Err(error) => {
                        // These failures occur during trust/ownership validation,
                        // before erase_recording_folder touches any artifact.
                        // Keep the folder and continue with the database delete.
                        let chain = format!("{error:#}").to_ascii_lowercase();
                        recording_cleanup = if chain.contains("different workspace")
                            || chain.contains("ownership marker")
                            || chain.contains("restricted to the local workspace")
                        {
                            RecordingCleanupStatus::RetainedOwnershipMismatch
                        } else if chain.contains("outside the managed")
                            || chain.contains("must be an absolute path")
                            || chain.contains("must be a real directory")
                            || chain.contains("read recording folder metadata")
                            || chain.contains("canonicalize recording folder")
                        {
                            RecordingCleanupStatus::RetainedUntrusted
                        } else {
                            return Err(error);
                        };
                        log::warn!(
                            "Meeting recording folder was retained because it could not be safely verified (status={})",
                            recording_cleanup.as_str()
                        );
                    }
                }
            }
        }

        let deleted =
            crate::database::deletion::delete_database_records(&self.pool, ctx, meeting_id).await?;
        if !deleted {
            return Ok(VerifiedMeetingDeletion {
                already_absent: true,
                artifacts,
                recording_cleanup,
                maintenance_pending,
            });
        }

        if crate::database::deletion::complete_privacy_maintenance(&self.pool)
            .await
            .is_err()
        {
            log::warn!(
                "Meeting rows were deleted, but local privacy maintenance remains pending for retry"
            );
            maintenance_pending = true;
        } else {
            maintenance_pending = false;
        }
        Ok(VerifiedMeetingDeletion {
            already_absent: false,
            artifacts,
            recording_cleanup,
            maintenance_pending,
        })
    }

    pub async fn with_transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await;

        match result {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback().await?;
                Err(err)
            }
        }
    }

    /// Cleanup database connection and checkpoint WAL
    /// This should be called on application shutdown to ensure:
    /// - All WAL changes are written to the main database file
    /// - The .wal and .shm files are deleted
    /// - Connection pool is gracefully closed
    pub async fn cleanup(&self) -> Result<()> {
        log::info!("Starting database cleanup...");

        // Force checkpoint of WAL to main database file and remove WAL file
        // TRUNCATE mode: checkpoints all pages AND deletes the WAL file
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("WAL checkpoint completed successfully"),
            Err(e) => log::warn!("WAL checkpoint failed (non-fatal): {}", e),
        }

        // Close the connection pool gracefully
        self.pool.close().await;
        log::info!("Database connection pool closed");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::migrate::Migrator;
    use sqlx::sqlite::SqlitePoolOptions;

    static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

    #[tokio::test]
    async fn pending_wal_maintenance_does_not_make_meetings_undeletable() {
        let temp = tempfile::tempdir().expect("temp directory");
        let database = temp.path().join("pending-maintenance.sqlite");
        let recording_root = temp.path().join("recordings");
        let meeting_folder = recording_root.join("meeting-folder");
        std::fs::create_dir_all(&meeting_folder).expect("recording folder");
        std::fs::write(
            meeting_folder.join("metadata.json"),
            br#"{"workspace_id":"local"}"#,
        )
        .expect("ownership marker");
        std::fs::write(meeting_folder.join("audio.mp4"), b"managed audio").expect("managed audio");

        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .expect("open database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        crate::database::deletion::complete_privacy_maintenance(&pool)
            .await
            .expect("finish initial maintenance");

        let now = "2026-09-11T00:00:00.000Z";
        sqlx::query(
            "INSERT INTO meetings \
             (id, workspace_id, title, created_at, updated_at, folder_path) \
             VALUES ('meeting-pending-maintenance', 'local', 'Pending maintenance', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(meeting_folder.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("seed meeting");
        sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&pool)
            .await
            .expect("checkpoint seed");

        // Hold a read snapshot while creating newer WAL frames. A TRUNCATE
        // checkpoint cannot finish until this reader leaves, reproducing the
        // persistent maintenance marker that previously blocked every delete.
        let mut reader = pool.begin().await.expect("begin reader");
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM meetings")
            .fetch_one(&mut *reader)
            .await
            .expect("establish read snapshot");
        sqlx::query(
            "UPDATE local_privacy_maintenance SET required = 1, completed_at = NULL \
             WHERE singleton = 1",
        )
        .execute(&pool)
        .await
        .expect("mark maintenance pending after reader snapshot");

        let manager = DatabaseManager {
            pool: pool.clone(),
            deletion_lock: Arc::new(Mutex::new(())),
            at_rest_encrypted: false,
        };
        let outcome = manager
            .delete_meeting_verified(
                &AuthContext::local(),
                "meeting-pending-maintenance",
                vec![recording_root],
            )
            .await
            .expect("logical deletion must not be blocked by pending maintenance");

        assert!(!outcome.already_absent);
        assert!(outcome.maintenance_pending);
        assert!(!meeting_folder.exists());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM meetings WHERE id = 'meeting-pending-maintenance'",
            )
            .fetch_one(&pool)
            .await
            .expect("meeting count"),
            0
        );
        assert!(
            crate::database::deletion::privacy_maintenance_required(&pool)
                .await
                .expect("pending marker")
        );

        reader.rollback().await.expect("release reader");
        crate::database::deletion::complete_privacy_maintenance(&pool)
            .await
            .expect("retry maintenance after reader leaves");
        assert!(
            !crate::database::deletion::privacy_maintenance_required(&pool)
                .await
                .expect("cleared marker")
        );
    }

    #[tokio::test]
    async fn untrusted_recording_folder_is_retained_without_blocking_meeting_deletion() {
        let temp = tempfile::tempdir().expect("temp directory");
        let database = temp.path().join("untrusted-recording.sqlite");
        let recording_root = temp.path().join("recordings");
        let outside_folder = temp.path().join("legacy-recording");
        std::fs::create_dir_all(&recording_root).expect("recording root");
        std::fs::create_dir_all(&outside_folder).expect("outside folder");
        std::fs::write(outside_folder.join("audio.mp4"), b"must survive").expect("audio");

        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .expect("open database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        crate::database::deletion::complete_privacy_maintenance(&pool)
            .await
            .expect("finish initial maintenance");

        let now = "2026-09-11T00:00:00.000Z";
        sqlx::query(
            "INSERT INTO meetings \
             (id, workspace_id, title, created_at, updated_at, folder_path) \
             VALUES ('meeting-untrusted-recording', 'local', 'Legacy path', ?, ?, ?)",
        )
        .bind(now)
        .bind(now)
        .bind(outside_folder.to_string_lossy().to_string())
        .execute(&pool)
        .await
        .expect("seed meeting");

        let manager = DatabaseManager {
            pool: pool.clone(),
            deletion_lock: Arc::new(Mutex::new(())),
            at_rest_encrypted: false,
        };
        let outcome = manager
            .delete_meeting_verified(
                &AuthContext::local(),
                "meeting-untrusted-recording",
                vec![recording_root],
            )
            .await
            .expect("untrusted folder must not block logical deletion");

        assert_eq!(
            outcome.recording_cleanup,
            RecordingCleanupStatus::RetainedUntrusted
        );
        assert!(outside_folder.join("audio.mp4").exists());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM meetings WHERE id = 'meeting-untrusted-recording'",
            )
            .fetch_one(&pool)
            .await
            .expect("meeting count"),
            0
        );
    }

    #[tokio::test]
    async fn shared_recording_folder_is_retained_without_blocking_target_deletion() {
        let temp = tempfile::tempdir().expect("temp directory");
        let database = temp.path().join("shared-recording.sqlite");
        let recording_root = temp.path().join("recordings");
        let shared_folder = recording_root.join("shared-folder");
        std::fs::create_dir_all(&shared_folder).expect("shared folder");
        std::fs::write(
            shared_folder.join("metadata.json"),
            br#"{"workspace_id":"local"}"#,
        )
        .expect("ownership marker");
        std::fs::write(shared_folder.join("audio.mp4"), b"shared audio").expect("audio");

        let options = SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .pragma("secure_delete", "ON");
        let pool = SqlitePoolOptions::new()
            .max_connections(3)
            .connect_with(options)
            .await
            .expect("open database");
        MIGRATOR.run(&pool).await.expect("apply migrations");
        crate::database::deletion::complete_privacy_maintenance(&pool)
            .await
            .expect("finish initial maintenance");

        let now = "2026-09-11T00:00:00.000Z";
        for (id, title) in [
            ("meeting-shared-target", "Shared target"),
            ("meeting-shared-owner", "Shared owner"),
        ] {
            sqlx::query(
                "INSERT INTO meetings \
                 (id, workspace_id, title, created_at, updated_at, folder_path) \
                 VALUES (?, 'local', ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(title)
            .bind(now)
            .bind(now)
            .bind(shared_folder.to_string_lossy().to_string())
            .execute(&pool)
            .await
            .expect("seed shared meeting");
        }

        let manager = DatabaseManager {
            pool: pool.clone(),
            deletion_lock: Arc::new(Mutex::new(())),
            at_rest_encrypted: false,
        };
        let outcome = manager
            .delete_meeting_verified(
                &AuthContext::local(),
                "meeting-shared-target",
                vec![recording_root],
            )
            .await
            .expect("shared folder must not block target deletion");

        assert_eq!(
            outcome.recording_cleanup,
            RecordingCleanupStatus::RetainedShared
        );
        assert!(shared_folder.join("audio.mp4").exists());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM meetings WHERE id = 'meeting-shared-target'",
            )
            .fetch_one(&pool)
            .await
            .expect("target count"),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM meetings WHERE id = 'meeting-shared-owner'",
            )
            .fetch_one(&pool)
            .await
            .expect("owner count"),
            1
        );
    }
}
