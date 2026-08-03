use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

pub fn pdf_content_id(path: &Path) -> io::Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut id = String::with_capacity(36);
    id.push_str("pdf-");
    for byte in &digest[..16] {
        write!(&mut id, "{byte:02x}").unwrap();
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn content_id_is_stable_and_callback_safe() {
        let path = std::env::temp_dir().join(format!(
            "remarque-content-id-{}-{:?}.pdf",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&path, b"%PDF-example").unwrap();
        let first = pdf_content_id(&path).unwrap();
        assert_eq!(first, pdf_content_id(&path).unwrap());
        assert_eq!(first.len(), 36);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
        let _ = fs::remove_file(path);
    }
}
