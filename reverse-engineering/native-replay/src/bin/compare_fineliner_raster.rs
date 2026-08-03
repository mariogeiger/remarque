use remarque_core::bgra_image::BgraImage;
use remarque_core::color::Color;
use remarque_core::render_fineliner::FinelinerRasterizer;
use remarque_core::stroke::StrokePoint;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Deserialize)]
struct RasterMetadata {
    rectangle: Rectangle,
}

#[derive(Clone, Copy, Deserialize)]
struct Rectangle {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Deserialize)]
struct CapturedRecord {
    view: CapturedPoint,
    render_point: Option<CapturedRenderPoint>,
    native_point: CapturedStrokePoint,
}

#[derive(Clone, Copy, Deserialize)]
struct CapturedPoint {
    x: f64,
    y: f64,
}

#[derive(Clone, Copy, Deserialize)]
struct CapturedRenderPoint {
    x: f32,
    y: f32,
    width: f32,
}

#[derive(Deserialize)]
struct CapturedStrokePoint {
    two_segment_distance_quarters: u16,
    width_quarters: u16,
    direction: u8,
    pressure: u8,
}

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .ok_or_else(|| "usage: compare_fineliner_raster CAPTURE [PREFIX_BYTES]".to_owned())?;
    let prefix_bytes = arguments
        .next()
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| format!("invalid prefix byte count: {error}"))?
        .unwrap_or(0);
    if prefix_bytes % 4 != 0 || arguments.next().is_some() {
        return Err("usage: compare_fineliner_raster CAPTURE [PREFIX_BYTES]".to_owned());
    }
    compare(Path::new(&directory), prefix_bytes / 4)
}

fn compare(directory: &Path, prefix_pixels: usize) -> Result<(), String> {
    let metadata: RasterMetadata = read_json(&directory.join("raster.json"))?;
    let records: Vec<CapturedRecord> = read_json_lines(&directory.join("points.jsonl"))?;
    let before = fs::read(directory.join("before.bgra"))
        .map_err(|error| format!("cannot read before.bgra: {error}"))?;
    let native = fs::read(directory.join("after.bgra"))
        .map_err(|error| format!("cannot read after.bgra: {error}"))?;
    let expected_bytes = metadata.rectangle.width * metadata.rectangle.height * 4;
    if before.len() != expected_bytes || native.len() != expected_bytes {
        return Err(format!(
            "raster byte count differs from the declared {expected_bytes} bytes"
        ));
    }

    let origin_x = metadata
        .rectangle
        .x
        .checked_sub(prefix_pixels)
        .ok_or_else(|| "framebuffer prefix exceeds the capture x coordinate".to_owned())?;
    for record in &records {
        if let Some(render_point) = record.render_point {
            let expected_width = 0.75 + f32::from(record.native_point.width_quarters) * 0.25;
            if (render_point.width - expected_width).abs() > 1e-5 {
                return Err(format!(
                    "native raster width {} differs from reconstructed width {expected_width}",
                    render_point.width
                ));
            }
        }
    }
    let points = records
        .iter()
        .map(|record| {
            let (x, y) = record.render_point.map_or_else(
                || (record.view.x as f32, record.view.y as f32),
                |point| (point.x, point.y),
            );
            StrokePoint {
                x: x - origin_x as f32,
                y: y - metadata.rectangle.y as f32,
                two_segment_distance_quarters: record.native_point.two_segment_distance_quarters,
                width_quarter_pixels: record.native_point.width_quarters,
                direction: record.native_point.direction,
                pressure: record.native_point.pressure,
            }
        })
        .collect::<Vec<_>>();
    let mut reconstructed = BgraImage::try_from_bgra(
        metadata.rectangle.width,
        metadata.rectangle.height,
        before.clone(),
    )
    .map_err(str::to_owned)?;
    let mut rasterizer = FinelinerRasterizer::new(Color::Black);
    for point in points {
        rasterizer.append_point(&mut reconstructed, point);
    }
    report_errors(
        &before,
        &native,
        reconstructed.pixels(),
        metadata.rectangle.width,
    );
    Ok(())
}

fn read_json<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

fn read_json_lines<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<Vec<T>, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|error| {
                format!(
                    "invalid JSON in {} line {}: {error}",
                    path.display(),
                    index + 1
                )
            })
        })
        .collect()
}

fn report_errors(before: &[u8], native: &[u8], reconstructed: &[u8], width: usize) {
    let native_support = changed_pixels(before, native);
    let reconstructed_support = changed_pixels(before, reconstructed);
    let union = native_support
        .union(&reconstructed_support)
        .copied()
        .collect::<Vec<_>>();
    let intersection = native_support.intersection(&reconstructed_support).count();
    let absolute_error = union
        .iter()
        .map(|pixel| {
            let offset = pixel * 4;
            (0..3)
                .map(|channel| native[offset + channel].abs_diff(reconstructed[offset + channel]))
                .max()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mean_absolute_error = absolute_error
        .iter()
        .map(|error| usize::from(*error))
        .sum::<usize>() as f64
        / absolute_error.len().max(1) as f64;
    println!("native_support={}", native_support.len());
    println!("reconstructed_support={}", reconstructed_support.len());
    println!("support_intersection={intersection}");
    println!("support_union={}", union.len());
    println!("native_extent={}", support_extent(&native_support, width));
    println!(
        "reconstructed_extent={}",
        support_extent(&reconstructed_support, width)
    );
    println!("mean_absolute_error={mean_absolute_error:.3}");
    println!(
        "maximum_absolute_error={}",
        absolute_error.iter().max().copied().unwrap_or(0)
    );
    let mut worst = union
        .iter()
        .map(|pixel| {
            let offset = pixel * 4;
            let error = (0..3)
                .map(|channel| native[offset + channel].abs_diff(reconstructed[offset + channel]))
                .max()
                .unwrap();
            (
                error,
                pixel % width,
                pixel / width,
                &native[offset..offset + 4],
                &reconstructed[offset..offset + 4],
            )
        })
        .collect::<Vec<_>>();
    worst.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    for (error, x, y, native, reconstructed) in worst.into_iter().take(8) {
        println!("difference={error}@{x},{y}:native={native:?}:reconstructed={reconstructed:?}");
    }
}

fn support_extent(support: &BTreeSet<usize>, width: usize) -> String {
    let mut points = support.iter().map(|pixel| (pixel % width, pixel / width));
    let Some(first) = points.next() else {
        return "empty".to_owned();
    };
    let (mut left, mut right, mut top, mut bottom) = (first.0, first.0, first.1, first.1);
    for (x, y) in points {
        left = left.min(x);
        right = right.max(x);
        top = top.min(y);
        bottom = bottom.max(y);
    }
    format!("{left},{top}..{right},{bottom}")
}

fn changed_pixels(before: &[u8], after: &[u8]) -> BTreeSet<usize> {
    before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
        .filter_map(|(index, (before, after))| (before != after).then_some(index))
        .collect()
}
