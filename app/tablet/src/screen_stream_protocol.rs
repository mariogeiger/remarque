const TILE_SIZE: usize = 64;
const PROTOCOL_MAGIC: &[u8; 4] = b"RMKS";
const PROTOCOL_VERSION: u8 = 3;
const MESSAGE_FULL_FRAME: u8 = 1;
const MESSAGE_DELTA_FRAME: u8 = 2;
const HEADER_SIZE: usize = 28;
const TILE_HEADER_SIZE: usize = 16;

pub fn encode_changed_pixels(
    previous: &[u8],
    current: &[u8],
    width: usize,
    height: usize,
) -> Option<Vec<u8>> {
    let (delta, tile_count) = build_delta(previous, current, width, height);
    if tile_count == 0 {
        return None;
    }
    if delta.len() > current.len() / 2 {
        Some(encode_full_frame(width, height, current))
    } else {
        Some(encode_message(
            MESSAGE_DELTA_FRAME,
            tile_count,
            width,
            height,
            width * 4,
            &delta,
        ))
    }
}

pub fn encode_full_frame(width: usize, height: usize, pixels: &[u8]) -> Vec<u8> {
    assert_eq!(pixels.len(), width * height * 4);
    encode_message(MESSAGE_FULL_FRAME, 0, width, height, width * 4, pixels)
}

fn encode_message(
    message_type: u8,
    tile_count: u32,
    width: usize,
    height: usize,
    stride: usize,
    payload: &[u8],
) -> Vec<u8> {
    let mut message = Vec::with_capacity(HEADER_SIZE + payload.len());
    message.extend_from_slice(PROTOCOL_MAGIC);
    message.push(PROTOCOL_VERSION);
    message.push(message_type);
    message.extend_from_slice(&[0, 0]);
    message.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    message.extend_from_slice(&tile_count.to_le_bytes());
    message.extend_from_slice(&(width as u32).to_le_bytes());
    message.extend_from_slice(&(height as u32).to_le_bytes());
    message.extend_from_slice(&(stride as u32).to_le_bytes());
    message.extend_from_slice(payload);
    message
}

fn build_delta(previous: &[u8], current: &[u8], width: usize, height: usize) -> (Vec<u8>, u32) {
    assert_eq!(previous.len(), width * height * 4);
    assert_eq!(current.len(), previous.len());
    let stride = width * 4;
    let mut payload = Vec::new();
    let mut tile_count = 0_u32;

    for y in (0..height).step_by(TILE_SIZE) {
        let tile_height = TILE_SIZE.min(height - y);
        for x in (0..width).step_by(TILE_SIZE) {
            let tile_width = TILE_SIZE.min(width - x);
            let row_bytes = tile_width * 4;
            let changed = (0..tile_height).any(|row| {
                let offset = (y + row) * stride + x * 4;
                previous[offset..offset + row_bytes] != current[offset..offset + row_bytes]
            });
            if !changed {
                continue;
            }

            payload.reserve(TILE_HEADER_SIZE + tile_width * tile_height * 4);
            payload.extend_from_slice(&(x as u32).to_le_bytes());
            payload.extend_from_slice(&(y as u32).to_le_bytes());
            payload.extend_from_slice(&(tile_width as u32).to_le_bytes());
            payload.extend_from_slice(&(tile_height as u32).to_le_bytes());
            for row in 0..tile_height {
                let offset = (y + row) * stride + x * 4;
                payload.extend_from_slice(&current[offset..offset + row_bytes]);
            }
            tile_count += 1;
        }
    }
    (payload, tile_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_frame_header_carries_image_geometry() {
        let message = encode_full_frame(3, 2, &[0; 24]);
        assert_eq!(&message[0..4], b"RMKS");
        assert_eq!(message[4], 3);
        assert_eq!(message[5], MESSAGE_FULL_FRAME);
        assert_eq!(u32::from_le_bytes(message[8..12].try_into().unwrap()), 24);
        assert_eq!(u32::from_le_bytes(message[16..20].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(message[20..24].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(message[24..28].try_into().unwrap()), 12);
    }

    #[test]
    fn delta_contains_only_changed_tiles() {
        let previous = vec![0_u8; 192 * 64 * 4];
        let mut current = previous.clone();
        current[(3 * 192 + 70) * 4] = 255;
        let (delta, tile_count) = build_delta(&previous, &current, 192, 64);
        assert_eq!(tile_count, 1);
        assert_eq!(
            &delta[0..16],
            &[64, 0, 0, 0, 0, 0, 0, 0, 64, 0, 0, 0, 64, 0, 0, 0]
        );
        assert_eq!(delta.len(), TILE_HEADER_SIZE + 64 * 64 * 4);
        let message = encode_changed_pixels(&previous, &current, 192, 64).unwrap();
        assert_eq!(message[5], MESSAGE_DELTA_FRAME);
        assert_eq!(u32::from_le_bytes(message[12..16].try_into().unwrap()), 1);
    }
}
