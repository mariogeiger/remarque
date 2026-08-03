use drm::control::{Device as ControlDevice, ResourceHandle};
use drm::{ClientCapability, Device};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, BorrowedFd};

struct Card(File);

impl Card {
    fn open() -> io::Result<Self> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/dri/card0")
            .map(Self)
    }
}

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl Device for Card {}
impl ControlDevice for Card {}

fn object_properties<T>(card: &Card, object: T) -> io::Result<BTreeMap<String, String>>
where
    T: ResourceHandle,
{
    let mut values = BTreeMap::new();
    for (handle, raw_value) in card.get_properties(object)? {
        let info = card.get_property(handle)?;
        let name = info.name().to_string_lossy().into_owned();
        let value = format!("{:?}", info.value_type().convert_value(raw_value));
        values.insert(name, value);
    }
    Ok(values)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let card = Card::open()?;
    card.set_client_capability(ClientCapability::UniversalPlanes, true)?;
    card.set_client_capability(ClientCapability::Atomic, true)?;

    let resources = card.resource_handles()?;
    println!("connectors={:?}", resources.connectors());
    println!("crtcs={:?}", resources.crtcs());
    println!("framebuffers={:?}", resources.framebuffers());
    println!("planes={:?}", card.plane_handles()?);

    for &handle in resources.connectors() {
        let info = card.get_connector(handle, false)?;
        println!(
            "connector={handle:?} interface={:?} state={:?} modes={:?} encoder={:?}",
            info.interface(),
            info.state(),
            info.modes(),
            info.current_encoder()
        );
        println!(
            "connector_properties={:?}",
            object_properties(&card, handle)?
        );
    }

    for &handle in resources.crtcs() {
        let info = card.get_crtc(handle)?;
        println!(
            "crtc={handle:?} position={:?} mode={:?} framebuffer={:?}",
            info.position(),
            info.mode(),
            info.framebuffer()
        );
        println!("crtc_properties={:?}", object_properties(&card, handle)?);
    }

    for handle in card.plane_handles()? {
        let info = card.get_plane(handle)?;
        println!(
            "plane={handle:?} crtc={:?} framebuffer={:?} formats={:?}",
            info.crtc(),
            info.framebuffer(),
            info.formats()
        );
        println!("plane_properties={:?}", object_properties(&card, handle)?);
    }

    for &handle in resources.framebuffers() {
        let info = card.get_planar_framebuffer(handle)?;
        println!(
            "framebuffer={handle:?} size={:?} format={:?} modifier={:?} pitches={:?} offsets={:?}",
            info.size(),
            info.pixel_format(),
            info.modifier(),
            info.pitches(),
            info.offsets()
        );
    }

    Ok(())
}
