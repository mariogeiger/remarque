use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub fn write_bgra_page_as_pdf(
    path: &Path,
    pixel_width: usize,
    pixel_height: usize,
    page_size_points: [f64; 2],
    bgra: &[u8],
) -> io::Result<()> {
    let expected_bytes = pixel_width
        .checked_mul(pixel_height)
        .and_then(|pixels| pixels.checked_mul(4));
    if pixel_width == 0
        || pixel_height == 0
        || expected_bytes != Some(bgra.len())
        || page_size_points
            .iter()
            .any(|size| !size.is_finite() || *size <= 0.0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixels and PDF page dimensions must be finite, positive, and consistent",
        ));
    }
    let mut rgb = Vec::with_capacity(pixel_width * pixel_height * 3);
    for pixel in bgra.chunks_exact(4) {
        rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::default());
    compressor.write_all(&rgb)?;
    let compressed = compressor.finish()?;

    let page_width = pdf_number(page_size_points[0]);
    let page_height = pdf_number(page_size_points[1]);
    let content = format!("q\n{page_width} 0 0 {page_height} 0 0 cm\n/Im0 Do\nQ\n");
    let mut pdf = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(5);
    append_object(
        &mut pdf,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    );
    append_object(
        &mut pdf,
        &mut offsets,
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    append_object(
        &mut pdf,
        &mut offsets,
        3,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width} {page_height}] /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>"
        )
        .as_bytes(),
    );
    let image_header = format!(
        "<< /Type /XObject /Subtype /Image /Width {pixel_width} /Height {pixel_height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
        compressed.len()
    );
    offsets.push(pdf.len());
    write!(&mut pdf, "4 0 obj\n{image_header}")?;
    pdf.extend_from_slice(&compressed);
    pdf.extend_from_slice(b"\nendstream\nendobj\n");
    let content_object = format!(
        "<< /Length {} >>\nstream\n{}endstream",
        content.len(),
        content
    );
    append_object(&mut pdf, &mut offsets, 5, content_object.as_bytes());

    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        writeln!(&mut pdf, "{offset:010} 00000 n ")?;
    }
    write!(
        &mut pdf,
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
    )?;
    write_bytes_atomically(path, &pdf)
}

fn pdf_number(value: f64) -> String {
    let formatted = format!("{value:.3}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn append_object(pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, number: usize, body: &[u8]) {
    offsets.push(pdf.len());
    writeln!(pdf, "{number} 0 obj").unwrap();
    pdf.extend_from_slice(body);
    pdf.extend_from_slice(b"\nendobj\n");
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("PDF path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_single_page_pdf_with_rgb_pixels() {
        let path =
            std::env::temp_dir().join(format!("remarque-pdf-test-{}.pdf", std::process::id()));
        write_bgra_page_as_pdf(
            &path,
            2,
            1,
            [612.0, 792.0],
            &[0, 0, 255, 255, 0, 255, 0, 255],
        )
        .unwrap();
        let pdf = fs::read(&path).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.windows(8).any(|window| window == b"/Width 2"));
        let media_box = b"/MediaBox [0 0 612 792]";
        assert!(
            pdf.windows(media_box.len())
                .any(|window| window == media_box)
        );
        assert!(pdf.ends_with(b"%%EOF\n"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_mismatched_pixel_count() {
        assert!(
            write_bgra_page_as_pdf(Path::new("unused.pdf"), 2, 2, [2.0, 2.0], &[0; 15]).is_err()
        );
    }
}
