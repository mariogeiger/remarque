use remarque_core::bgra_image::BgraImage;
use remarque_core::color::Color;
use remarque_core::render_fineliner::{AntialiasedCoverageVertex, render_antialiased_triangle};
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
struct TriangleRecord {
    vertices: [[f32; 2]; 3],
    coverage_coordinates: [[f32; 2]; 3],
}

fn main() -> Result<(), String> {
    let mut arguments = std::env::args().skip(1);
    let directory = arguments
        .next()
        .ok_or_else(|| "usage: compare_native_triangles CAPTURE TRIANGLES_JSONL".to_owned())?;
    let triangles = arguments
        .next()
        .ok_or_else(|| "usage: compare_native_triangles CAPTURE TRIANGLES_JSONL".to_owned())?;
    if arguments.next().is_some() {
        return Err("usage: compare_native_triangles CAPTURE TRIANGLES_JSONL".to_owned());
    }
    compare(Path::new(&directory), Path::new(&triangles))
}

fn compare(directory: &Path, triangles_path: &Path) -> Result<(), String> {
    let metadata: RasterMetadata = read_json(&directory.join("raster.json"))?;
    let triangles: Vec<TriangleRecord> = read_json_lines(triangles_path)?;
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

    let mut reconstructed = BgraImage::try_from_bgra(
        metadata.rectangle.width,
        metadata.rectangle.height,
        before.clone(),
    )
    .map_err(str::to_owned)?;
    for triangle in triangles {
        let vertices = std::array::from_fn(|index| AntialiasedCoverageVertex {
            x: triangle.vertices[index][0] - metadata.rectangle.x as f32,
            y: triangle.vertices[index][1] - metadata.rectangle.y as f32,
            half_width: triangle.coverage_coordinates[index][0],
            signed_distance: triangle.coverage_coordinates[index][1],
        });
        render_antialiased_triangle(&mut reconstructed, vertices, Color::Black);
    }
    report_errors(&before, &native, reconstructed.pixels());
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

fn report_errors(before: &[u8], native: &[u8], reconstructed: &[u8]) {
    let native_support = changed_pixels(before, native);
    let reconstructed_support = changed_pixels(before, reconstructed);
    let union = native_support
        .union(&reconstructed_support)
        .copied()
        .collect::<Vec<_>>();
    let absolute_error = union
        .iter()
        .map(|pixel| {
            let offset = pixel * 4;
            (0..4)
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
    println!(
        "support_intersection={}",
        native_support.intersection(&reconstructed_support).count()
    );
    println!("support_union={}", union.len());
    println!("mean_absolute_error={mean_absolute_error:.3}");
    println!(
        "maximum_absolute_error={}",
        absolute_error.iter().max().copied().unwrap_or(0)
    );
}

fn changed_pixels(before: &[u8], after: &[u8]) -> BTreeSet<usize> {
    before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .enumerate()
        .filter_map(|(index, (before, after))| (before != after).then_some(index))
        .collect()
}
