use remarque_core::{
    bgra_image::BgraImage,
    color::Color,
    fineliner::{FinelinerStrokeBuilder, FinelinerThickness},
    render_fineliner::FinelinerRasterizer,
    stroke::{PenSample, StrokePoint},
    view_transform::{Point, two_finger_scale},
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum NativeScenario {
    TwoFingerScale {
        id: String,
        evidence: NativeEvidence,
        initial: [RecordedPoint; 2],
        current: [RecordedPoint; 2],
        native_output: f64,
        absolute_tolerance: f64,
    },
    FinelinerWidth {
        id: String,
        evidence: NativeEvidence,
        thickness: RecordedFinelinerThickness,
        native_output: u16,
    },
    FinelinerSamples {
        id: String,
        evidence: NativeEvidence,
        thickness: RecordedFinelinerThickness,
        scene_to_view_scale: f32,
        records: Vec<RecordedFinelinerRecord>,
    },
    FinelinerRaster {
        id: String,
        evidence: NativeEvidence,
        image_width: usize,
        image_height: usize,
        background_bgra: [u8; 4],
        points: Vec<RecordedStrokePoint>,
        native_changed_pixels: Vec<RecordedRasterPixel>,
        maximum_mean_absolute_error: f64,
        maximum_absolute_error: u8,
    },
}

#[derive(Clone, Debug, Deserialize)]
pub struct NativeEvidence {
    pub firmware: String,
    pub binary_sha256: String,
    pub capture: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RecordedPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RecordedPenSample {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RecordedFinelinerRecord {
    pub sample: RecordedPenSample,
    pub native_output: RecordedStrokePoint,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct RecordedStrokePoint {
    pub x: f32,
    pub y: f32,
    pub two_segment_distance_quarters: u16,
    pub width_quarters: u16,
    pub direction: u8,
    pub pressure: u8,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct RecordedRasterPixel {
    pub x: usize,
    pub y: usize,
    pub bgra: [u8; 4],
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedFinelinerThickness {
    Thin,
    Medium,
    Thick,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ComparisonFailure {
    pub scenario_id: String,
    pub field: String,
    pub native_output: f64,
    pub reconstructed_output: Option<f64>,
    pub absolute_tolerance: f64,
}

pub fn load_scenarios(directory: &Path) -> Result<Vec<NativeScenario>, String> {
    let mut paths = fs::read_dir(directory)
        .map_err(|error| format!("cannot read {}: {error}", directory.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, _>>()
        .map_err(|error| format!("cannot list {}: {error}", directory.display()))?;
    paths.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "json")
    });
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let bytes = fs::read(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid fixture {}: {error}", path.display()))
        })
        .collect()
}

pub fn compare_scenario(scenario: &NativeScenario) -> Result<(), ComparisonFailure> {
    match scenario {
        NativeScenario::TwoFingerScale {
            id,
            initial,
            current,
            native_output,
            absolute_tolerance,
            ..
        } => {
            let reconstructed_output = two_finger_scale(
                [initial[0].into(), initial[1].into()],
                [current[0].into(), current[1].into()],
            );
            if reconstructed_output
                .is_some_and(|output| (output - native_output).abs() <= *absolute_tolerance)
            {
                Ok(())
            } else {
                Err(ComparisonFailure {
                    scenario_id: id.clone(),
                    field: "scale".to_owned(),
                    native_output: *native_output,
                    reconstructed_output,
                    absolute_tolerance: *absolute_tolerance,
                })
            }
        }
        NativeScenario::FinelinerWidth {
            id,
            thickness,
            native_output,
            ..
        } => {
            let reconstructed_output = FinelinerThickness::from(*thickness).quarter_pixels();
            if reconstructed_output == *native_output {
                Ok(())
            } else {
                Err(ComparisonFailure {
                    scenario_id: id.clone(),
                    field: "width_quarters".to_owned(),
                    native_output: f64::from(*native_output),
                    reconstructed_output: Some(f64::from(reconstructed_output)),
                    absolute_tolerance: 0.0,
                })
            }
        }
        NativeScenario::FinelinerSamples {
            id,
            thickness,
            scene_to_view_scale,
            records,
            ..
        } => {
            let mut builder = FinelinerStrokeBuilder::new((*thickness).into());
            let reconstructed = records
                .iter()
                .map(|record| {
                    builder.append_sample(
                        PenSample {
                            x: record.sample.x,
                            y: record.sample.y,
                            pressure: record.sample.pressure,
                        },
                        *scene_to_view_scale,
                    )
                })
                .collect::<Vec<_>>();
            let native_output = records
                .iter()
                .map(|record| record.native_output)
                .collect::<Vec<_>>();
            compare_stroke_points(id, &native_output, &reconstructed)
        }
        NativeScenario::FinelinerRaster {
            id,
            image_width,
            image_height,
            background_bgra,
            points,
            native_changed_pixels,
            maximum_mean_absolute_error,
            maximum_absolute_error,
            ..
        } => compare_fineliner_raster(
            id,
            *image_width,
            *image_height,
            *background_bgra,
            points,
            native_changed_pixels,
            *maximum_mean_absolute_error,
            *maximum_absolute_error,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_fineliner_raster(
    scenario_id: &str,
    width: usize,
    height: usize,
    background_bgra: [u8; 4],
    points: &[RecordedStrokePoint],
    native_changed_pixels: &[RecordedRasterPixel],
    maximum_mean_absolute_error: f64,
    maximum_absolute_error: u8,
) -> Result<(), ComparisonFailure> {
    let mut reconstructed = BgraImage::filled(
        width,
        height,
        [background_bgra[2], background_bgra[1], background_bgra[0]],
    );
    let points = points
        .iter()
        .map(|point| StrokePoint {
            x: point.x,
            y: point.y,
            two_segment_distance_quarters: point.two_segment_distance_quarters,
            width_quarter_pixels: point.width_quarters,
            direction: point.direction,
            pressure: point.pressure,
        })
        .collect::<Vec<_>>();
    let mut rasterizer = FinelinerRasterizer::new(Color::Black);
    for point in points {
        rasterizer.append_point(&mut reconstructed, point);
    }

    let native = native_changed_pixels
        .iter()
        .map(|pixel| (pixel.y * width + pixel.x, pixel.bgra))
        .collect::<BTreeMap<_, _>>();
    let reconstructed_support = reconstructed
        .pixels()
        .chunks_exact(4)
        .enumerate()
        .filter_map(|(index, pixel)| (pixel != background_bgra).then_some(index))
        .collect::<BTreeSet<_>>();
    let native_support = native.keys().copied().collect::<BTreeSet<_>>();
    if reconstructed_support != native_support {
        let native_first_difference = native_support
            .symmetric_difference(&reconstructed_support)
            .next()
            .copied()
            .unwrap();
        return Err(ComparisonFailure {
            scenario_id: scenario_id.to_owned(),
            field: "pixel_support".to_owned(),
            native_output: native_first_difference as f64,
            reconstructed_output: None,
            absolute_tolerance: 0.0,
        });
    }

    let errors = native
        .iter()
        .map(|(index, native)| {
            let offset = index * 4;
            native
                .iter()
                .zip(&reconstructed.pixels()[offset..offset + 4])
                .map(|(native, reconstructed)| native.abs_diff(*reconstructed))
                .max()
                .unwrap()
        })
        .collect::<Vec<_>>();
    let mean_absolute_error =
        errors.iter().map(|error| f64::from(*error)).sum::<f64>() / errors.len().max(1) as f64;
    if mean_absolute_error > maximum_mean_absolute_error {
        return Err(ComparisonFailure {
            scenario_id: scenario_id.to_owned(),
            field: "mean_absolute_pixel_error".to_owned(),
            native_output: 0.0,
            reconstructed_output: Some(mean_absolute_error),
            absolute_tolerance: maximum_mean_absolute_error,
        });
    }
    let largest_error = errors.iter().copied().max().unwrap_or(0);
    if largest_error > maximum_absolute_error {
        return Err(ComparisonFailure {
            scenario_id: scenario_id.to_owned(),
            field: "maximum_absolute_pixel_error".to_owned(),
            native_output: 0.0,
            reconstructed_output: Some(f64::from(largest_error)),
            absolute_tolerance: f64::from(maximum_absolute_error),
        });
    }
    Ok(())
}

fn compare_stroke_points(
    scenario_id: &str,
    native: &[RecordedStrokePoint],
    reconstructed: &[StrokePoint],
) -> Result<(), ComparisonFailure> {
    if native.len() != reconstructed.len() {
        return Err(ComparisonFailure {
            scenario_id: scenario_id.to_owned(),
            field: "point_count".to_owned(),
            native_output: native.len() as f64,
            reconstructed_output: Some(reconstructed.len() as f64),
            absolute_tolerance: 0.0,
        });
    }
    for (index, (native, reconstructed)) in native.iter().zip(reconstructed).enumerate() {
        let fields = [
            ("x", f64::from(native.x), f64::from(reconstructed.x)),
            ("y", f64::from(native.y), f64::from(reconstructed.y)),
            (
                "two_segment_distance_quarters",
                f64::from(native.two_segment_distance_quarters),
                f64::from(reconstructed.two_segment_distance_quarters),
            ),
            (
                "width_quarters",
                f64::from(native.width_quarters),
                f64::from(reconstructed.width_quarter_pixels),
            ),
            (
                "direction",
                f64::from(native.direction),
                f64::from(reconstructed.direction),
            ),
            (
                "pressure",
                f64::from(native.pressure),
                f64::from(reconstructed.pressure),
            ),
        ];
        if let Some((field, native_output, reconstructed_output)) = fields
            .into_iter()
            .find(|(_, native, reconstructed)| native != reconstructed)
        {
            return Err(ComparisonFailure {
                scenario_id: scenario_id.to_owned(),
                field: format!("points[{index}].{field}"),
                native_output,
                reconstructed_output: Some(reconstructed_output),
                absolute_tolerance: 0.0,
            });
        }
    }
    Ok(())
}

impl From<RecordedPoint> for Point {
    fn from(point: RecordedPoint) -> Self {
        Self {
            x: point.x,
            y: point.y,
        }
    }
}

impl From<RecordedFinelinerThickness> for FinelinerThickness {
    fn from(thickness: RecordedFinelinerThickness) -> Self {
        match thickness {
            RecordedFinelinerThickness::Thin => Self::Thin,
            RecordedFinelinerThickness::Medium => Self::Medium,
            RecordedFinelinerThickness::Thick => Self::Thick,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_recorded_native_scenarios_match() {
        let scenarios = load_scenarios(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .as_path(),
        )
        .unwrap();
        assert!(!scenarios.is_empty());
        let failures = scenarios
            .iter()
            .filter_map(|scenario| compare_scenario(scenario).err())
            .collect::<Vec<_>>();
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
