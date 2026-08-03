use flate2::Compression;
use flate2::write::ZlibEncoder;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

pub struct RasterPdfPage<'a> {
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub size_points: [f64; 2],
    pub bgra: &'a [u8],
}

pub struct OwnedRasterPdfPage {
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub size_points: [f64; 2],
    pub bgra: Vec<u8>,
}

pub fn write_bgra_page_as_pdf(
    path: &Path,
    pixel_width: usize,
    pixel_height: usize,
    size_points: [f64; 2],
    bgra: &[u8],
) -> io::Result<()> {
    write_bgra_pages_as_pdf(
        path,
        &[RasterPdfPage {
            pixel_width,
            pixel_height,
            size_points,
            bgra,
        }],
    )
}

pub fn write_bgra_pages_as_pdf(path: &Path, pages: &[RasterPdfPage<'_>]) -> io::Result<()> {
    if pages.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a PDF must contain at least one page",
        ));
    }
    for page in pages {
        validate_page(page)?;
    }
    write_pdf_atomically(path, |file| {
        write_pdf(file, pages.len(), |index| encode_page(&pages[index]))
    })
}

pub fn write_generated_bgra_pages_as_pdf(
    path: &Path,
    page_count: usize,
    mut generate_page: impl FnMut(usize) -> io::Result<OwnedRasterPdfPage>,
) -> io::Result<()> {
    if page_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a PDF must contain at least one page",
        ));
    }
    write_pdf_atomically(path, |file| {
        write_pdf(file, page_count, |index| {
            let page = generate_page(index)?;
            encode_page(&RasterPdfPage {
                pixel_width: page.pixel_width,
                pixel_height: page.pixel_height,
                size_points: page.size_points,
                bgra: &page.bgra,
            })
        })
    })
}

fn write_pdf(
    file: File,
    page_count: usize,
    mut encode_page: impl FnMut(usize) -> io::Result<EncodedPdfPage>,
) -> io::Result<()> {
    let mut pdf = CountingWriter::new(file);
    pdf.write_all(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n")?;
    let mut offsets = Vec::with_capacity(2 + page_count * 3);
    append_object(
        &mut pdf,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>",
    )?;
    let kids = (0..page_count)
        .map(|index| format!("{} 0 R", page_object_number(index)))
        .collect::<Vec<_>>()
        .join(" ");
    append_object(
        &mut pdf,
        &mut offsets,
        2,
        format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>").as_bytes(),
    )?;

    for index in 0..page_count {
        append_page(&mut pdf, &mut offsets, index, encode_page(index)?)?;
    }

    let xref = pdf.position();
    let object_count = 2 + page_count * 3;
    writeln!(&mut pdf, "xref\n0 {}", object_count + 1)?;
    pdf.write_all(b"0000000000 65535 f \n")?;
    for offset in offsets {
        writeln!(&mut pdf, "{offset:010} 00000 n ")?;
    }
    write!(
        &mut pdf,
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        object_count + 1
    )?;
    pdf.finish()
}

fn append_page(
    pdf: &mut CountingWriter<File>,
    offsets: &mut Vec<u64>,
    index: usize,
    page: EncodedPdfPage,
) -> io::Result<()> {
    let page_object = page_object_number(index);
    let image_object = page_object + 1;
    let content_object = page_object + 2;
    let page_width = pdf_number(page.size_points[0]);
    let page_height = pdf_number(page.size_points[1]);
    append_object(
        pdf,
        offsets,
        page_object,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_width} {page_height}] /Resources << /XObject << /Im0 {image_object} 0 R >> >> /Contents {content_object} 0 R >>"
        )
        .as_bytes(),
    )?;

    offsets.push(pdf.position());
    writeln!(pdf, "{image_object} 0 obj")?;
    writeln!(
        pdf,
        "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream",
        page.pixel_width,
        page.pixel_height,
        page.compressed_rgb.len()
    )?;
    pdf.write_all(&page.compressed_rgb)?;
    pdf.write_all(b"\nendstream\nendobj\n")?;

    let content = format!("q\n{page_width} 0 0 {page_height} 0 0 cm\n/Im0 Do\nQ\n");
    append_object(
        pdf,
        offsets,
        content_object,
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            content.len(),
            content
        )
        .as_bytes(),
    )
}

fn validate_page(page: &RasterPdfPage<'_>) -> io::Result<()> {
    let expected_bytes = page
        .pixel_width
        .checked_mul(page.pixel_height)
        .and_then(|pixels| pixels.checked_mul(4));
    if page.pixel_width == 0
        || page.pixel_height == 0
        || expected_bytes != Some(page.bgra.len())
        || page
            .size_points
            .iter()
            .any(|size| !size.is_finite() || *size <= 0.0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "pixels and PDF page dimensions must be finite, positive, and consistent",
        ));
    }
    Ok(())
}

fn compress_bgra_as_rgb(page: &RasterPdfPage<'_>) -> io::Result<Vec<u8>> {
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::default());
    let mut rgb_row = Vec::with_capacity(page.pixel_width * 3);
    for bgra_row in page.bgra.chunks_exact(page.pixel_width * 4) {
        rgb_row.clear();
        for pixel in bgra_row.chunks_exact(4) {
            rgb_row.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
        }
        compressor.write_all(&rgb_row)?;
    }
    compressor.finish()
}

struct EncodedPdfPage {
    pixel_width: usize,
    pixel_height: usize,
    size_points: [f64; 2],
    compressed_rgb: Vec<u8>,
}

fn encode_page(page: &RasterPdfPage<'_>) -> io::Result<EncodedPdfPage> {
    validate_page(page)?;
    Ok(EncodedPdfPage {
        pixel_width: page.pixel_width,
        pixel_height: page.pixel_height,
        size_points: page.size_points,
        compressed_rgb: compress_bgra_as_rgb(page)?,
    })
}

fn page_object_number(index: usize) -> usize {
    3 + index * 3
}

fn pdf_number(value: f64) -> String {
    let formatted = format!("{value:.3}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}

fn append_object(
    pdf: &mut CountingWriter<File>,
    offsets: &mut Vec<u64>,
    number: usize,
    body: &[u8],
) -> io::Result<()> {
    offsets.push(pdf.position());
    writeln!(pdf, "{number} 0 obj")?;
    pdf.write_all(body)?;
    pdf.write_all(b"\nendobj\n")
}

struct CountingWriter<W> {
    inner: W,
    position: u64,
}

impl<W> CountingWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner, position: 0 }
    }

    fn position(&self) -> u64 {
        self.position
    }
}

impl CountingWriter<File> {
    fn finish(self) -> io::Result<()> {
        self.inner.sync_all()
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(buffer)?;
        self.position += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn write_pdf_atomically(
    path: &Path,
    write_pdf: impl FnOnce(File) -> io::Result<()>,
) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("PDF path has no parent directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&temporary)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    write_pdf(file)?;
    fs::rename(&temporary, path)?;
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "remarque-{name}-{}-{:?}.pdf",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn writes_multiple_pages_with_distinct_sizes() {
        let path = test_path("multipage");
        let first = [0, 0, 255, 255, 0, 255, 0, 255];
        let second = [255, 0, 0, 255];
        write_bgra_pages_as_pdf(
            &path,
            &[
                RasterPdfPage {
                    pixel_width: 2,
                    pixel_height: 1,
                    size_points: [612.0, 792.0],
                    bgra: &first,
                },
                RasterPdfPage {
                    pixel_width: 1,
                    pixel_height: 1,
                    size_points: [300.0, 400.0],
                    bgra: &second,
                },
            ],
        )
        .unwrap();
        let pdf = fs::read(&path).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.windows(8).any(|window| window == b"/Count 2"));
        assert!(pdf.ends_with(b"%%EOF\n"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn rejects_empty_and_inconsistent_pages() {
        assert!(write_bgra_pages_as_pdf(Path::new("unused.pdf"), &[]).is_err());
        assert!(
            write_bgra_page_as_pdf(Path::new("unused.pdf"), 2, 2, [2.0, 2.0], &[0; 15]).is_err()
        );
    }
}
