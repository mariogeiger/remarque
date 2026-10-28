use crate::relay::{StoredSession, StoredShare};
use remarque_page_log::{PageHistory, PageJournal, Participant, ShareId};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const STORED_SHARE_MAGIC: &[u8] = b"remarque-share-v2\0";

#[derive(Deserialize, Serialize)]
struct StoredShareFile {
    id: ShareId,
    expires_at_unix_seconds: u64,
    guest_secret_digest: [u8; 32],
    owner: Participant,
    participants: Vec<Participant>,
    sessions: Vec<StoredSession>,
    journal: PageJournal,
    history: PageHistory,
    revoked: bool,
}

#[derive(Deserialize, Serialize)]
struct PreviousStoredShareFile {
    id: ShareId,
    expires_at_unix_seconds: u64,
    guest_secret_digest: [u8; 32],
    owner: Participant,
    participants: Vec<Participant>,
    sessions: Vec<StoredSession>,
    journal: PageJournal,
    revoked: bool,
}

pub fn load_shares(directory: &Path) -> io::Result<Vec<StoredShare>> {
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let mut shares = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "tmp") {
            fs::remove_file(path)?;
            continue;
        }
        if path
            .extension()
            .is_some_and(|extension| extension == "share")
        {
            match fs::read(&path)
                .and_then(|bytes| decode_stored_share(&bytes).map_err(io::Error::other))
            {
                Ok(share) => shares.push(share),
                Err(error) => {
                    eprintln!(
                        "page_relay_share_load_failed path={} error={error}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(shares)
}

pub fn write_share(directory: &Path, share: &StoredShare) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let destination = share_path(directory, share.id);
    let temporary = directory.join(format!(".{}.{}.tmp", share.id, std::process::id()));
    let bytes = encode_stored_share(share).map_err(io::Error::other)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    use std::io::Write;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        if let Ok(directory_file) = fs::File::open(directory) {
            directory_file.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn encode_stored_share(share: &StoredShare) -> Result<Vec<u8>, postcard::Error> {
    let encoded = postcard::to_allocvec(&StoredShareFile {
        id: share.id,
        expires_at_unix_seconds: share.expires_at_unix_seconds,
        guest_secret_digest: share.guest_secret_digest,
        owner: share.owner,
        participants: share.participants.clone(),
        sessions: share.sessions.clone(),
        journal: share.journal.clone(),
        history: share.journal.history().clone(),
        revoked: share.revoked,
    })?;
    let mut bytes = Vec::with_capacity(STORED_SHARE_MAGIC.len() + encoded.len());
    bytes.extend_from_slice(STORED_SHARE_MAGIC);
    bytes.extend_from_slice(&encoded);
    Ok(bytes)
}

fn decode_stored_share(bytes: &[u8]) -> Result<StoredShare, postcard::Error> {
    if let Some(encoded) = bytes.strip_prefix(STORED_SHARE_MAGIC) {
        let file: StoredShareFile = postcard::from_bytes(encoded)?;
        let mut journal = file.journal;
        journal.replace_history(file.history);
        return Ok(StoredShare {
            id: file.id,
            expires_at_unix_seconds: file.expires_at_unix_seconds,
            guest_secret_digest: file.guest_secret_digest,
            owner: file.owner,
            participants: file.participants,
            sessions: file.sessions,
            journal,
            revoked: file.revoked,
        });
    }
    let file: PreviousStoredShareFile = postcard::from_bytes(bytes)?;
    Ok(StoredShare {
        id: file.id,
        expires_at_unix_seconds: file.expires_at_unix_seconds,
        guest_secret_digest: file.guest_secret_digest,
        owner: file.owner,
        participants: file.participants,
        sessions: file.sessions,
        journal: file.journal,
        revoked: file.revoked,
    })
}

pub fn write_asset(directory: &Path, digest: &[u8; 32], bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    let name = encode_digest(digest);
    let destination = directory.join(format!("{name}.bgra"));
    if destination.exists() {
        return Ok(());
    }
    let temporary = directory.join(format!(".{name}.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    use std::io::Write;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn read_asset(directory: &Path, digest: &[u8; 32]) -> io::Result<Vec<u8>> {
    fs::read(directory.join(format!("{}.bgra", encode_digest(digest))))
}

fn encode_digest(digest: &[u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write;
        write!(text, "{byte:02x}").expect("writing to a string cannot fail");
    }
    text
}

fn share_path(directory: &Path, share_id: ShareId) -> PathBuf {
    directory.join(format!("{share_id}.share"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use remarque_core::color::Color;
    use remarque_core::stroke::StrokePoint;
    use remarque_page_log::{
        CommandId, PageCommand, PageDimensions, PageIdentity, PageSnapshot, ParticipantId,
        ParticipantRole, StrokeId, SubmittedPageOperation,
    };

    #[test]
    fn current_storage_preserves_history_and_previous_storage_becomes_a_baseline() {
        let owner = Participant {
            id: ParticipantId::from_bytes([1; 16]),
            role: ParticipantRole::Owner,
            color: Color::Black,
        };
        let mut stored = StoredShare {
            id: ShareId::from_bytes([2; 16]),
            expires_at_unix_seconds: u64::MAX,
            guest_secret_digest: [3; 32],
            owner,
            participants: vec![owner],
            sessions: Vec::new(),
            journal: PageJournal::from_snapshot(PageSnapshot {
                identity: PageIdentity {
                    document_id: "notebook-1".to_owned(),
                    page_index: 0,
                },
                dimensions: PageDimensions {
                    width: 100,
                    height: 200,
                },
                background: None,
                strokes: Vec::new(),
                active_strokes: Vec::new(),
                revision: 0,
            })
            .unwrap(),
            revoked: false,
        };
        let stroke_id = StrokeId::from_bytes([4; 16]);
        for (byte, operation) in [
            SubmittedPageOperation::BeginStroke { stroke_id },
            SubmittedPageOperation::AppendStrokePoints {
                stroke_id,
                first_point: 0,
                points: vec![StrokePoint {
                    x: 12.0,
                    y: 34.0,
                    two_segment_distance_quarters: 0,
                    width_quarter_pixels: 8,
                    direction: 0,
                    pressure: 0,
                }],
            },
            SubmittedPageOperation::CommitStroke { stroke_id },
        ]
        .into_iter()
        .enumerate()
        {
            stored
                .journal
                .submit(
                    owner,
                    PageCommand {
                        id: CommandId::from_bytes([10 + byte as u8; 16]),
                        operation,
                    },
                )
                .unwrap();
        }

        let current_bytes = encode_stored_share(&stored).unwrap();
        let mut current = decode_stored_share(&current_bytes).unwrap();
        current.journal.revalidate().unwrap();
        assert!(current.journal.history_availability(owner.id).can_undo);

        let previous_bytes = postcard::to_allocvec(&PreviousStoredShareFile {
            id: stored.id,
            expires_at_unix_seconds: stored.expires_at_unix_seconds,
            guest_secret_digest: stored.guest_secret_digest,
            owner: stored.owner,
            participants: stored.participants.clone(),
            sessions: stored.sessions.clone(),
            journal: stored.journal,
            revoked: stored.revoked,
        })
        .unwrap();
        let mut previous = decode_stored_share(&previous_bytes).unwrap();
        previous.journal.revalidate().unwrap();
        assert!(!previous.journal.history_availability(owner.id).can_undo);
        assert_eq!(previous.journal.snapshot().strokes.len(), 1);
    }
}
