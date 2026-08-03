use remarque_document::DocumentExchange;
use remarque_tablet::display::QuillDisplay;
use remarque_tablet::document_requests::apply_oldest_document_request;
use remarque_tablet::input::{PenDevice, TouchDevice};
use remarque_tablet::notebook::Notebook;
use remarque_tablet::screen_stream::start_screen_stream;
use signal_hook::consts::{SIGINT, SIGTERM};
use std::io;
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn poll_inputs(pen: RawFd, touch: RawFd) -> io::Result<()> {
    let mut descriptors = [
        libc::pollfd {
            fd: pen,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: touch,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, 16) };
    if result < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGTERM, Arc::clone(&stop))?;
    signal_hook::flag::register(SIGINT, Arc::clone(&stop))?;

    let display = Arc::new(QuillDisplay::open()?);
    let exchange = DocumentExchange::new(
        std::env::var_os("REMARQUE_EXCHANGE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/home/root/remarque/data/exchange")),
    );
    exchange.prepare()?;
    let mut notebook = Notebook::new(Arc::clone(&display), exchange.library_state_path())?;
    let _screen_stream = match start_screen_stream(display) {
        Ok(thread) => Some(thread),
        Err(error) => {
            eprintln!("screen_stream_unavailable={error}");
            None
        }
    };
    let mut pen = PenDevice::open(notebook.width(), notebook.height())?;
    let mut touch = TouchDevice::open(notebook.width(), notebook.height())?;

    while !stop.load(Ordering::Relaxed) {
        apply_oldest_document_request(&mut notebook, &exchange)?;
        poll_inputs(pen.raw_fd(), touch.raw_fd())?;
        for frame in pen.drain()? {
            if notebook.apply_pen_frame(frame)? {
                return Ok(());
            }
        }
        for frame in touch.drain()? {
            if notebook.apply_touch_frame(frame)? {
                return Ok(());
            }
        }
    }
    Ok(())
}
