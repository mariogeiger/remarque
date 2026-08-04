use crate::bgra_image::{BgraImage, PixelRectangle};
use crate::render_page_view::OUTSIDE_PAGE_RGB;
use std::ffi::{CString, c_char, c_int, c_void};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Once;

type PdfDocumentPointer = *mut c_void;
type PdfPagePointer = *mut c_void;
type PdfBitmapPointer = *mut c_void;

unsafe extern "C" {
    fn FPDF_InitLibrary();
    fn FPDF_LoadDocument(path: *const c_char, password: *const c_char) -> PdfDocumentPointer;
    fn FPDF_GetPageCount(document: PdfDocumentPointer) -> c_int;
    fn FPDF_LoadPage(document: PdfDocumentPointer, index: c_int) -> PdfPagePointer;
    fn FPDF_GetPageWidthF(page: PdfPagePointer) -> f32;
    fn FPDF_GetPageHeightF(page: PdfPagePointer) -> f32;
    fn FPDFBitmap_Create(width: c_int, height: c_int, alpha: c_int) -> PdfBitmapPointer;
    fn FPDFBitmap_FillRect(
        bitmap: PdfBitmapPointer,
        left: c_int,
        top: c_int,
        width: c_int,
        height: c_int,
        color: u32,
    ) -> c_int;
    fn FPDFBitmap_GetBuffer(bitmap: PdfBitmapPointer) -> *mut c_void;
    fn FPDFBitmap_GetStride(bitmap: PdfBitmapPointer) -> c_int;
    fn FPDF_RenderPageBitmap(
        bitmap: PdfBitmapPointer,
        page: PdfPagePointer,
        start_x: c_int,
        start_y: c_int,
        size_x: c_int,
        size_y: c_int,
        rotate: c_int,
        flags: c_int,
    );
    fn FPDFBitmap_Destroy(bitmap: PdfBitmapPointer);
    fn FPDF_ClosePage(page: PdfPagePointer);
    fn FPDF_CloseDocument(document: PdfDocumentPointer);
}

const FPDF_ANNOT: c_int = 0x01;
const MAXIMUM_PAGE_RASTER_HEIGHT: usize = 8192;
static INITIALIZE_PDFIUM: Once = Once::new();

pub(crate) struct RenderedPdfPage {
    pub background: BgraImage,
    pub page_rectangle: PixelRectangle,
    pub page_size_points: [f64; 2],
}

pub(crate) fn read_pdf_page_sizes(path: &Path) -> io::Result<Vec<[f64; 2]>> {
    INITIALIZE_PDFIUM.call_once(|| unsafe { FPDF_InitLibrary() });
    let encoded_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PDF path contains NUL"))?;
    let document = PdfDocument::open(&encoded_path)?;
    let page_count = document.page_count()?;
    (0..page_count)
        .map(|index| {
            let page = document.page(index)?;
            validated_page_size(&page)
        })
        .collect()
}

pub(crate) fn render_pdf_page(
    path: &Path,
    page_index: u32,
    canvas_width: usize,
    canvas_height: usize,
    content_top: usize,
) -> io::Result<RenderedPdfPage> {
    INITIALIZE_PDFIUM.call_once(|| unsafe { FPDF_InitLibrary() });
    let encoded_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "PDF path contains NUL"))?;
    let document = PdfDocument::open(&encoded_path)?;
    let page_count = document.page_count()?;
    if page_index >= page_count {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PDF page number is out of range",
        ));
    }
    let page = document.page(page_index)?;
    let [page_width, page_height] = validated_page_size(&page)?;
    let render_width = canvas_width;
    let render_height = (page_height * canvas_width as f64 / page_width).round() as usize;
    if render_height == 0 || render_height > MAXIMUM_PAGE_RASTER_HEIGHT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF page aspect ratio exceeds the supported range",
        ));
    }
    let bitmap = PdfBitmap::new(render_width, render_height)?;
    unsafe {
        FPDFBitmap_FillRect(
            bitmap.0.as_ptr(),
            0,
            0,
            render_width as c_int,
            render_height as c_int,
            0xffff_ffff,
        );
        FPDF_RenderPageBitmap(
            bitmap.0.as_ptr(),
            page.0.as_ptr(),
            0,
            0,
            render_width as c_int,
            render_height as c_int,
            0,
            FPDF_ANNOT,
        );
    }
    let stride = usize::try_from(unsafe { FPDFBitmap_GetStride(bitmap.0.as_ptr()) })
        .map_err(|_| io::Error::other("PDFium returned a negative bitmap stride"))?;
    let buffer = NonNull::new(unsafe { FPDFBitmap_GetBuffer(bitmap.0.as_ptr()) }.cast::<u8>())
        .ok_or_else(|| io::Error::other("PDFium returned a null bitmap buffer"))?;
    let source = unsafe { std::slice::from_raw_parts(buffer.as_ptr(), stride * render_height) };
    let background_height = canvas_height.max(content_top + render_height);
    let mut background = BgraImage::filled(canvas_width, background_height, OUTSIDE_PAGE_RGB);
    background
        .copy_bgra_rectangle(0, content_top, render_width, render_height, stride, source)
        .map_err(io::Error::other)?;
    Ok(RenderedPdfPage {
        background,
        page_rectangle: PixelRectangle {
            x: 0,
            y: content_top,
            width: render_width,
            height: render_height,
        },
        page_size_points: [page_width, page_height],
    })
}

fn validated_page_size(page: &PdfPage) -> io::Result<[f64; 2]> {
    let width = unsafe { FPDF_GetPageWidthF(page.0.as_ptr()) } as f64;
    let height = unsafe { FPDF_GetPageHeightF(page.0.as_ptr()) } as f64;
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "PDF page has invalid dimensions",
        ));
    }
    Ok([width, height])
}

struct PdfDocument(NonNull<c_void>);

impl PdfDocument {
    fn open(path: &CString) -> io::Result<Self> {
        NonNull::new(unsafe { FPDF_LoadDocument(path.as_ptr(), std::ptr::null()) })
            .map(Self)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PDFium could not open PDF"))
    }

    fn page_count(&self) -> io::Result<u32> {
        u32::try_from(unsafe { FPDF_GetPageCount(self.0.as_ptr()) })
            .ok()
            .filter(|count| *count > 0)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PDF has no pages"))
    }

    fn page(&self, index: u32) -> io::Result<PdfPage> {
        NonNull::new(unsafe { FPDF_LoadPage(self.0.as_ptr(), index as c_int) })
            .map(PdfPage)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "PDFium could not load page"))
    }
}

impl Drop for PdfDocument {
    fn drop(&mut self) {
        unsafe { FPDF_CloseDocument(self.0.as_ptr()) }
    }
}

struct PdfPage(NonNull<c_void>);

impl Drop for PdfPage {
    fn drop(&mut self) {
        unsafe { FPDF_ClosePage(self.0.as_ptr()) }
    }
}

struct PdfBitmap(NonNull<c_void>);

impl PdfBitmap {
    fn new(width: usize, height: usize) -> io::Result<Self> {
        NonNull::new(unsafe { FPDFBitmap_Create(width as c_int, height as c_int, 0) })
            .map(Self)
            .ok_or_else(|| io::Error::other("PDFium could not allocate bitmap"))
    }
}

impl Drop for PdfBitmap {
    fn drop(&mut self) {
        unsafe { FPDFBitmap_Destroy(self.0.as_ptr()) }
    }
}
