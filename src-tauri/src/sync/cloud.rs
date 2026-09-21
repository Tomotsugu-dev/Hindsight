//! The cloud storage sync runs on. Files live in one flat folder and are
//! addressed by name; the operations of [`CloudBackend`] are all the engine
//! needs, and every backend must give them the same meaning.
//!
//! `InMemory` keeps the files in a map for the end-to-end tests, with its own
//! clock so that modification times only ever move forward.

#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use crate::error::Result;
use crate::storage::DbPool;
#[cfg(test)]
use crate::sync::drive::InMemoryDriveStore;
use crate::sync::drive::{self, DriveClient};

/// Cloud file metadata (id + name + modified time), without the file content.
#[derive(Debug, Clone)]
pub struct FileMeta {
    /// The backend's own handle for the file, assigned when the file is created
    /// and opaque to us. Download, overwrite and delete address the file by it.
    pub id: String,
    /// The name Hindsight gave the file, `device.<device id>.<kind>.json`. It is
    /// what pull reads to tell what the file holds and which device wrote it.
    pub name: String,
    /// Time in RFC3339 format.
    pub modified_time: String,
    /// File size in bytes; reserved for future diagnostics / "cloud usage" display.
    #[allow(dead_code)]
    pub size: Option<u64>,
}

/// The cloud behind sync. The sync engine holds one, and push, pull and the
/// cloud-clearing commands reach the cloud only through its methods.
/// `InMemory` stands in for a real cloud in tests and is the reference for
/// what each method must do.
pub enum CloudBackend {
    Drive(DriveClient),
    /// The tests' stand-in; not compiled into the shipped binary.
    #[cfg(test)]
    InMemory(Arc<InMemoryDriveStore>),
}

impl CloudBackend {
    /// The backend the app runs on: Google Drive, with the credential taken
    /// from `pool`'s `auth_state` table.
    pub fn drive(pool: DbPool) -> Self {
        CloudBackend::Drive(DriveClient::new(pool))
    }

    /// How much pull should rewind its cursor to avoid missing files that share
    /// the same timestamp.
    ///
    /// Drive uses zero because its timestamps are precise enough, and pull already
    /// stops before failed files instead of advancing past them.
    pub fn time_precision(&self) -> Duration {
        match self {
            CloudBackend::Drive(_) => Duration::ZERO,
            #[cfg(test)]
            CloudBackend::InMemory(_) => Duration::ZERO,
        }
    }

    /// How long the background loop sleeps between ticks. Every tick pushes;
    /// a tick also pulls once `pull_interval` has passed since the last pull.
    /// A backend that counts requests (WebDAV, ADR-0007 §3) sets minutes.
    pub fn push_interval(&self) -> Duration {
        match self {
            CloudBackend::Drive(_) => drive::DRIVE_PUSH_INTERVAL,
            #[cfg(test)]
            CloudBackend::InMemory(_) => drive::DRIVE_PUSH_INTERVAL,
        }
    }

    /// The least time between two pulls; see [`Self::push_interval`].
    pub fn pull_interval(&self) -> Duration {
        match self {
            CloudBackend::Drive(_) => drive::DRIVE_PULL_INTERVAL,
            #[cfg(test)]
            CloudBackend::InMemory(_) => drive::DRIVE_PULL_INTERVAL,
        }
    }

    /// Readies the credential this round needs, reading the local credential
    /// table and refreshing when that is due.
    ///
    /// `Ok(false)` means the user is not signed in to this backend: push and
    /// pull skip the round and it does not count as a failure. `Err` means a
    /// credential is there but no usable one came of it, a refused refresh for
    /// instance; the round fails and the Devices page shows the error.
    pub async fn ensure_credential(&self) -> Result<bool> {
        match self {
            CloudBackend::Drive(c) => c.ensure_credential().await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => Ok(store.signed_in()),
        }
    }

    /// Lists the files modified strictly after `modified_after`, oldest first;
    /// an empty string lists everything. Pull passes its cursor here, so
    /// "strictly after" and the ordering are part of the contract.
    pub async fn list(&self, modified_after: &str) -> Result<Vec<FileMeta>> {
        match self {
            CloudBackend::Drive(c) => c.list(modified_after).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.list_appdata_files(modified_after).await,
        }
    }

    /// Downloads a file's whole content into memory.
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>> {
        match self {
            CloudBackend::Drive(c) => c.download(file_id).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.download(file_id).await,
        }
    }

    /// Writes a file by name: replaces the content when the name exists,
    /// creates the file otherwise. Either way the modification time moves to
    /// now. Returns the file's id.
    pub async fn upsert_by_name(&self, name: &str, content: &[u8]) -> Result<String> {
        match self {
            CloudBackend::Drive(c) => c.upsert_by_name(name, content).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.upsert_by_name(name, content).await,
        }
    }

    /// Deletes a file for good; there is no trash to recover it from. Deleting
    /// a file that is already gone counts as success.
    pub async fn delete(&self, file_id: &str) -> Result<()> {
        match self {
            CloudBackend::Drive(c) => c.delete(file_id).await,
            #[cfg(test)]
            CloudBackend::InMemory(store) => store.delete(file_id).await,
        }
    }
}
