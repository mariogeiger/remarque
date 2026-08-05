use crate::relay::StoredShare;
use remarque_page_log::ShareId;
use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

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
                .and_then(|bytes| postcard::from_bytes(&bytes).map_err(io::Error::other))
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
    let bytes = postcard::to_allocvec(share).map_err(io::Error::other)?;
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
