use remarque_core::bgra_image::BgraImage;
use remarque_core::color::Color;
use remarque_core::render_fineliner::{FinelinerRasterPoint, FinelinerRasterizer};
use serde::Deserialize;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct TraceEvent {
    kind: String,
    sample: Option<Position>,
    render_point: Option<RasterPosition>,
}

#[derive(Clone, Copy, Deserialize)]
struct Position {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy, Deserialize)]
struct RasterPosition {
    x: f32,
    y: f32,
    width: f32,
}

#[derive(Deserialize)]
struct DrawingSurfaceMetadata {
    stride_pixels: usize,
    height: usize,
}

struct ActiveStroke {
    first_position: Position,
    rasterizer: FinelinerRasterizer,
    appended_first_position: bool,
}

fn main() {
    if let Err(error) = compare_trace() {
        eprintln!("comparison_error={error}");
        std::process::exit(1);
    }
}

fn compare_trace() -> Result<(), String> {
    let directory = parse_directory_argument()?;
    let metadata: DrawingSurfaceMetadata = read_json(&directory.join("drawing-surface.json"))?;
    let before = fs::read(directory.join("drawing-before.bgra"))
        .map_err(|error| format!("cannot read drawing-before.bgra: {error}"))?;
    let native_after = fs::read(directory.join("drawing-after.bgra"))
        .map_err(|error| format!("cannot read drawing-after.bgra: {error}"))?;
    let mut reconstructed =
        BgraImage::try_from_bgra(metadata.stride_pixels, metadata.height, before.clone())
            .map_err(str::to_owned)?;

    let events: Vec<TraceEvent> = read_json_lines(&directory.join("events.jsonl"))?;
    let mut active = None;
    let mut completed_strokes = 0;
    for event in events {
        match event.kind.as_str() {
            "begin_line" => {
                active = Some(ActiveStroke {
                    first_position: event
                        .sample
                        .ok_or_else(|| "begin_line has no sample".to_owned())?,
                    rasterizer: FinelinerRasterizer::new(Color::Black),
                    appended_first_position: false,
                });
            }
            "line_point" => {
                let point = event
                    .render_point
                    .ok_or_else(|| "line_point has no render_point".to_owned())?;
                let stroke = active
                    .as_mut()
                    .ok_or_else(|| "line_point occurred outside a line".to_owned())?;
                if !stroke.appended_first_position {
                    stroke.rasterizer.append_point(
                        &mut reconstructed,
                        FinelinerRasterPoint {
                            x: stroke.first_position.x,
                            y: stroke.first_position.y,
                            width: point.width,
                        },
                    );
                    stroke.appended_first_position = true;
                }
                stroke.rasterizer.append_point(
                    &mut reconstructed,
                    FinelinerRasterPoint {
                        x: point.x,
                        y: point.y,
                        width: point.width,
                    },
                );
            }
            "finish_line" => {
                let mut stroke = active
                    .take()
                    .ok_or_else(|| "finish_line occurred outside a line".to_owned())?;
                stroke.rasterizer.finish(&mut reconstructed);
                completed_strokes += 1;
            }
            _ => {}
        }
    }
    if active.is_some() {
        return Err("trace ends inside an unfinished line".to_owned());
    }
    compare_pixels(
        &before,
        &native_after,
        reconstructed.pixels(),
        completed_strokes,
        metadata.stride_pixels,
    )
}

fn parse_directory_argument() -> Result<PathBuf, String> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: compare-native-stroke-trace TRACE-DIRECTORY".to_owned())?;
    if arguments.next().is_some() {
        return Err("usage: compare-native-stroke-trace TRACE-DIRECTORY".to_owned());
    }
    Ok(directory)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))
}

fn read_json_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, String> {
    let file =
        fs::File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    BufReader::new(file)
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line = line.map_err(|error| error.to_string())?;
            serde_json::from_str(&line).map_err(|error| {
                format!("invalid JSON at {}:{}: {error}", path.display(), index + 1)
            })
        })
        .collect()
}

fn compare_pixels(
    before: &[u8],
    native_after: &[u8],
    reconstructed_after: &[u8],
    completed_strokes: usize,
    stride_pixels: usize,
) -> Result<(), String> {
    if before.len() != native_after.len() || before.len() != reconstructed_after.len() {
        return Err("drawing surface byte counts differ".to_owned());
    }
    let mut native_support = 0;
    let mut reconstructed_support = 0;
    let mut support_intersection = 0;
    let mut support_union = 0;
    let mut absolute_error = 0_u64;
    let mut maximum_error = 0_u8;
    let mut exact_pixels = 0;
    let mut differing_pixels = 0;
    let mut maximum_error_pixels = Vec::new();
    for (pixel_index, ((before, native), reconstructed)) in before
        .chunks_exact(4)
        .zip(native_after.chunks_exact(4))
        .zip(reconstructed_after.chunks_exact(4))
        .enumerate()
    {
        let native_changed = before != native;
        let reconstructed_changed = before != reconstructed;
        native_support += usize::from(native_changed);
        reconstructed_support += usize::from(reconstructed_changed);
        support_intersection += usize::from(native_changed && reconstructed_changed);
        support_union += usize::from(native_changed || reconstructed_changed);
        exact_pixels += usize::from(native == reconstructed);
        differing_pixels += usize::from(native != reconstructed);
        let pixel_maximum_error = (0..4)
            .map(|channel| native[channel].abs_diff(reconstructed[channel]))
            .max()
            .unwrap();
        if pixel_maximum_error > maximum_error {
            maximum_error_pixels.clear();
        }
        if pixel_maximum_error >= maximum_error && pixel_maximum_error != 0 {
            maximum_error_pixels.push((
                pixel_index % stride_pixels,
                pixel_index / stride_pixels,
                <[u8; 4]>::try_from(before).unwrap(),
                <[u8; 4]>::try_from(native).unwrap(),
                <[u8; 4]>::try_from(reconstructed).unwrap(),
            ));
        }
        for channel in 0..4 {
            let error = native[channel].abs_diff(reconstructed[channel]);
            absolute_error += u64::from(error);
            maximum_error = maximum_error.max(error);
        }
    }
    let channel_count = native_after.len();
    println!("completed_strokes={completed_strokes}");
    println!("native_support={native_support}");
    println!("reconstructed_support={reconstructed_support}");
    println!("support_intersection={support_intersection}");
    println!("support_union={support_union}");
    println!("exact_pixels={exact_pixels}");
    println!("differing_pixels={differing_pixels}");
    println!(
        "whole_surface_mean_absolute_error={:.6}",
        absolute_error as f64 / channel_count as f64
    );
    println!(
        "support_union_mean_absolute_error={:.6}",
        absolute_error as f64 / (support_union * 4) as f64
    );
    println!("maximum_absolute_error={maximum_error}");
    for (x, y, before, native, reconstructed) in maximum_error_pixels.into_iter().take(8) {
        println!(
            "maximum_error_pixel=({x},{y}) before={before:?} native={native:?} reconstructed={reconstructed:?}"
        );
    }
    Ok(())
}
