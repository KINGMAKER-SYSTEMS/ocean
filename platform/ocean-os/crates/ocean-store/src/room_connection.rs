//! Initial team-service selection for a daemon that has no federation history.
//!
//! The daemon validates the origin and requires operator authority. This store
//! owns the atomic first-write/history fence, not URL policy or network access.

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{Result, RoomStoreError, SqliteRoomStore};

pub(crate) const ROOM_CONNECTION_DDL: &str = "
    CREATE TABLE IF NOT EXISTS room_connection (
        singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
        origin TEXT NOT NULL
    );
";

/// Configuration only: neither value establishes connectivity or membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomConnectionState {
    pub origin: Option<String>,
    pub can_initialize: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialRoomConnection {
    Created,
    Unchanged,
    AlreadyConfigured,
    ExistingFederation,
}

fn origin_on(conn: &Connection) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT origin FROM room_connection WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .optional()?)
}

fn has_federation_on(conn: &Connection) -> Result<bool> {
    // Retain the fence after credentials are revoked: an old access projection
    // still identifies a host with federation history. Pending invites already
    // carry private tokens even before a Room exists.
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM room_access)
             OR EXISTS(SELECT 1 FROM room_federation)
             OR EXISTS(SELECT 1 FROM pending_redemptions)",
        [],
        |row| row.get(0),
    )?)
}

impl SqliteRoomStore {
    pub fn room_connection(&self) -> Result<RoomConnectionState> {
        let origin = origin_on(&self.conn)?;
        let can_initialize = origin.is_none() && !has_federation_on(&self.conn)?;
        Ok(RoomConnectionState {
            origin,
            can_initialize,
        })
    }

    /// Save a daemon-validated canonical origin exactly once. There is no
    /// replacement or clear operation; moving existing federation authority
    /// requires a separate explicit migration. Exact retries preserve the row.
    pub fn initialize_room_connection(&mut self, origin: &str) -> Result<InitialRoomConnection> {
        if origin.is_empty()
            || origin.len() > 2048
            || origin.trim() != origin
            || origin.chars().any(char::is_control)
        {
            return Err(RoomStoreError::Encode(
                "invalid room connection origin".into(),
            ));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(existing) = origin_on(&tx)? {
            return Ok(if existing == origin {
                InitialRoomConnection::Unchanged
            } else {
                InitialRoomConnection::AlreadyConfigured
            });
        }
        if has_federation_on(&tx)? {
            return Ok(InitialRoomConnection::ExistingFederation);
        }
        tx.execute(
            "INSERT INTO room_connection (singleton, origin) VALUES (1, ?1)",
            params![origin],
        )?;
        tx.commit()?;
        Ok(InitialRoomConnection::Created)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RoomStore;
    use chrono::Utc;
    use ocean_core::RoomKey;

    const FIRST: &str = "https://team.example/";
    const OTHER: &str = "https://other.example/";

    #[test]
    fn room_connection_survives_reopen_and_cannot_be_redirected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rooms.db");
        let mut store = SqliteRoomStore::open(&path).unwrap();
        assert!(store.room_connection().unwrap().can_initialize);
        let owner = store.owner_identity("Local owner").unwrap();
        store
            .create(RoomKey::new("local"), "Local", None, Utc::now())
            .unwrap();
        assert_eq!(
            store.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::Created
        );
        drop(store);
        let mut reopened = SqliteRoomStore::open(&path).unwrap();
        assert_eq!(reopened.owner_identity("ignored").unwrap(), owner);
        assert!(reopened.room_exists(&RoomKey::new("local")).unwrap());
        assert_eq!(
            reopened.room_connection().unwrap(),
            RoomConnectionState {
                origin: Some(FIRST.into()),
                can_initialize: false
            }
        );
        assert_eq!(
            reopened.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::Unchanged
        );
        assert_eq!(
            reopened.initialize_room_connection(OTHER).unwrap(),
            InitialRoomConnection::AlreadyConfigured
        );
        assert_eq!(
            reopened.room_connection().unwrap().origin.as_deref(),
            Some(FIRST)
        );
    }

    #[test]
    fn room_connection_refuses_preexisting_invite_authority() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteRoomStore::open(temp.path().join("rooms.db")).unwrap();
        store
            .get_or_insert_pending_redemption(
                "fixture-code",
                "redemption",
                "fixture-token",
                Utc::now(),
            )
            .unwrap();
        assert!(!store.room_connection().unwrap().can_initialize);
        assert_eq!(
            store.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::ExistingFederation
        );
        assert_eq!(store.room_connection().unwrap().origin, None);
        assert_eq!(store.list_pending_redemptions().unwrap().len(), 1);
    }

    #[test]
    fn room_connection_refuses_existing_room_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteRoomStore::open(temp.path().join("rooms.db")).unwrap();
        let room = RoomKey::new("shared");
        store
            .create(room.clone(), "Shared", None, Utc::now())
            .unwrap();
        store
            .install_room_credential(&room, "fixture-token", "member")
            .unwrap();
        assert_eq!(
            store.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::ExistingFederation
        );
        assert_eq!(store.room_connection().unwrap().origin, None);
        assert_eq!(store.list_credentialed_rooms().unwrap().len(), 1);
    }

    #[test]
    fn room_connection_refuses_retained_federation_history_without_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteRoomStore::open(temp.path().join("rooms.db")).unwrap();
        let room = RoomKey::new("shared");
        store
            .create(room.clone(), "Shared", None, Utc::now())
            .unwrap();
        let mut projection = store.room_access(&room).unwrap();
        projection.state = ocean_core::RoomAccessState::Revoked;
        store.replace_room_access(&room, &projection).unwrap();
        assert!(store.list_credentialed_rooms().unwrap().is_empty());
        assert_eq!(
            store.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::ExistingFederation
        );
        assert_eq!(store.room_connection().unwrap().origin, None);
        assert_eq!(
            store.room_access(&room).unwrap().state,
            ocean_core::RoomAccessState::Revoked
        );
    }

    #[test]
    fn room_connection_migrates_previous_schema_without_replacing_local_state() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rooms.db");
        let mut store = SqliteRoomStore::open(&path).unwrap();
        let owner = store.owner_identity("Existing owner").unwrap();
        store
            .create(RoomKey::new("existing"), "Existing", None, Utc::now())
            .unwrap();
        // The immediately preceding schema contains every table except this one.
        store
            .conn
            .execute_batch("DROP TABLE room_connection")
            .unwrap();
        drop(store);
        let mut migrated = SqliteRoomStore::open(&path).unwrap();
        assert_eq!(migrated.owner_identity("ignored").unwrap(), owner);
        assert!(migrated.room_exists(&RoomKey::new("existing")).unwrap());
        assert_eq!(
            migrated.initialize_room_connection(FIRST).unwrap(),
            InitialRoomConnection::Created
        );
    }

    #[test]
    fn room_connection_invalid_input_writes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let mut store = SqliteRoomStore::open(temp.path().join("rooms.db")).unwrap();
        for invalid in [
            String::new(),
            " https://team.example/".into(),
            "https://team.example/\n".into(),
            "x".repeat(2049),
        ] {
            assert!(store.initialize_room_connection(&invalid).is_err());
            assert!(store.room_connection().unwrap().can_initialize);
            assert_eq!(store.room_connection().unwrap().origin, None);
        }
    }

    #[test]
    fn room_connection_competing_origins_have_one_winner() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rooms.db");
        let first = SqliteRoomStore::open(&path).unwrap();
        let second = SqliteRoomStore::open(&path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = [(first, FIRST), (second, OTHER)]
            .into_iter()
            .map(|(mut store, origin)| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.initialize_room_connection(origin).unwrap()
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|r| **r == InitialRoomConnection::Created)
                .count(),
            1
        );
        assert_eq!(
            results
                .iter()
                .filter(|r| **r == InitialRoomConnection::AlreadyConfigured)
                .count(),
            1
        );
    }
}
