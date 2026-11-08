//! Decode a small grayscale file into canonical linear 8-bit samples.
use multicad_core::model::{MAX_SURFACE_IMAGE_AXIS, SurfaceStructurePattern};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

const MAX_PNG_BYTES: usize = 1024 * 1024;
const MAX_SIDE: u32 = MAX_SURFACE_IMAGE_AXIS as u32;

pub(crate) fn read(path: &Path) -> Result<SurfaceStructurePattern, String> {
    let file = File::open(path).map_err(|error| format!("Cannot open the height map: {error}"))?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Choose a regular grayscale PNG file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_PNG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("Cannot read the height map: {error}"))?;
    decode(&bytes)
}

fn decode(bytes: &[u8]) -> Result<SurfaceStructurePattern, String> {
    if bytes.len() > MAX_PNG_BYTES {
        return Err("Choose a height-map PNG smaller than 1 MiB".into());
    }
    let failure = |error: png::DecodingError| format!("Cannot decode the grayscale PNG: {error}");
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: MAX_PNG_BYTES,
        },
    );
    let header = decoder.read_header_info().map_err(failure)?;
    if !(1..=MAX_SIDE).contains(&header.width) || !(1..=MAX_SIDE).contains(&header.height) {
        return Err("Choose a height map between 1 and 12 pixels on each axis".into());
    }
    if header.color_type != png::ColorType::Grayscale || header.bit_depth != png::BitDepth::Eight {
        return Err("Choose an 8-bit grayscale PNG with no alpha channel".into());
    }
    let (width, height) = (header.width as u8, header.height as u8);
    let mut reader = decoder.read_info().map_err(failure)?;
    if reader.info().trns.is_some() || reader.info().animation_control.is_some() {
        return Err("Choose a single grayscale PNG without transparency or animation".into());
    }
    let mut samples = vec![0; usize::from(width) * usize::from(height)];
    reader.next_frame(&mut samples).map_err(failure)?;
    reader.finish().map_err(failure)?;
    Ok(SurfaceStructurePattern::HeightMap {
        width,
        height,
        samples,
        repeat: [1, 1],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, color: png::ColorType, samples: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(samples)
            .unwrap();
        bytes
    }

    #[test]
    fn surface_height_map_keeps_top_row_first_and_refuses_out_of_domain_images() {
        let pixels = [0, 128, 255, 64];
        let mut image = png(2, 2, png::ColorType::Grayscale, &pixels);
        let SurfaceStructurePattern::HeightMap {
            width,
            height,
            samples,
            repeat,
        } = decode(&image).unwrap()
        else {
            panic!("height map")
        };
        assert_eq!((width, height, repeat), (2, 2, [1, 1]));
        assert_eq!(samples, pixels);
        image.truncate(image.len() - 16);
        assert!(decode(&image).is_err());
        assert!(decode(&png(13, 1, png::ColorType::Grayscale, &[0; 13])).is_err());
        assert!(decode(&png(1, 1, png::ColorType::GrayscaleAlpha, &[0; 2])).is_err());
    }
}
