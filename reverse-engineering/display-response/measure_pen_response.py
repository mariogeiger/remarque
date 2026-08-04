#!/usr/bin/env python3
import argparse
import csv
import json
import statistics
import subprocess
from contextlib import contextmanager
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ClockAnchor:
    device_seconds: float
    device_minus_host_seconds: float
    uncertainty_seconds: float


@dataclass(frozen=True)
class InputSample:
    device_seconds: float
    x: int
    y: int


@contextmanager
def open_text(path):
    path = Path(path)
    if path.suffix != ".zst":
        with path.open() as source:
            yield source
        return
    process = subprocess.Popen(
        ["zstdcat", "--", str(path)],
        stdout=subprocess.PIPE,
        text=True,
    )
    try:
        yield process.stdout
    finally:
        process.stdout.close()
        return_code = process.wait()
        if return_code:
            raise subprocess.CalledProcessError(return_code, process.args)


def median(values):
    values = list(values)
    if not values:
        raise ValueError("median of empty sequence")
    return statistics.median(values)


def read_clock_anchor(path):
    with open(path, newline="") as source:
        rows = list(csv.DictReader(source))
    rows.sort(key=lambda row: float(row["network_rtt_s"]))
    selected = rows[: max(3, min(10, len(rows)))]
    offsets = [float(row["device_minus_mac_s"]) for row in selected]
    device_times = [
        (float(row["device_receive_s"]) + float(row["device_send_s"])) / 2
        for row in selected
    ]
    round_trips = [float(row["network_rtt_s"]) for row in selected]
    return ClockAnchor(
        median(device_times),
        median(offsets),
        max(median(round_trips) / 2, (max(offsets) - min(offsets)) / 2),
    )


def make_device_to_host(clock_before, clock_after):
    slope = (
        clock_after.device_minus_host_seconds
        - clock_before.device_minus_host_seconds
    ) / (clock_after.device_seconds - clock_before.device_seconds)

    def device_to_host(device_seconds):
        offset = clock_before.device_minus_host_seconds + slope * (
            device_seconds - clock_before.device_seconds
        )
        return device_seconds - offset

    return device_to_host, slope


def read_longest_contact(path):
    x = None
    y = None
    touching = False
    contacts = []
    current = []
    with open_text(path) as source:
        for line in source:
            event = json.loads(line)
            if event["kind"] != "event" or event["source_event"] is None:
                continue
            if event["type"] == 3 and event["code"] == 0:
                x = event["value"]
            elif event["type"] == 3 and event["code"] == 1:
                y = event["value"]
            elif event["type"] == 1 and event["code"] == 330:
                touching = event["value"] == 1
                if not touching and current:
                    contacts.append(current)
                    current = []
            elif event["type"] == 0 and event["code"] == 0 and touching:
                if x is None or y is None:
                    raise ValueError("contact precedes absolute position")
                current.append(InputSample(event["before_write_ns"] / 1e9, x, y))
    if current:
        contacts.append(current)
    if not contacts:
        raise ValueError("injection log contains no complete contact")
    return max(contacts, key=lambda contact: contact[-1].device_seconds - contact[0].device_seconds)


def interpolate_input_time(samples, axis, coordinate):
    values = [(getattr(sample, axis), sample.device_seconds) for sample in samples]
    increasing = values[-1][0] >= values[0][0]
    for (before_value, before_time), (after_value, after_time) in zip(values, values[1:]):
        crosses = (
            before_value <= coordinate <= after_value
            if increasing
            else after_value <= coordinate <= before_value
        )
        if not crosses:
            continue
        if after_value == before_value:
            return before_time
        fraction = (coordinate - before_value) / (after_value - before_value)
        return before_time + fraction * (after_time - before_time)
    raise ValueError(f"input coordinate {coordinate:.3f} is outside the contact")


def read_camera_columns(path, names):
    samples = {name: [] for name in names}
    with open_text(path) as source:
        reader = csv.DictReader(source)
        missing = names.difference(reader.fieldnames)
        if missing:
            raise ValueError(f"camera columns not found: {sorted(missing)}")
        for row in reader:
            time = float(row["presentation_seconds"])
            for name in names:
                samples[name].append((time, float(row[name])))
    return samples


def first_sustained_crossing(samples, baseline, amplitude, start, end, threshold):
    for index, (time, value) in enumerate(samples[:-1]):
        if not start <= time <= end:
            continue
        progress = (value - baseline) / amplitude
        next_progress = (samples[index + 1][1] - baseline) / amplitude
        if progress >= threshold and next_progress >= threshold:
            return time
    raise ValueError("no sustained optical crossing")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--camera", required=True)
    parser.add_argument("--injection", required=True)
    parser.add_argument("--clock-before", required=True)
    parser.add_argument("--clock-after", required=True)
    parser.add_argument("--input-axis", choices=["x", "y"], required=True)
    parser.add_argument("--raw-start", type=float, required=True)
    parser.add_argument("--raw-end", type=float, required=True)
    parser.add_argument("--camera-start", type=float, required=True)
    parser.add_argument("--camera-end", type=float, required=True)
    parser.add_argument("--camera-fixed-axis", choices=["x", "y"], required=True)
    parser.add_argument("--camera-fixed-coordinate", type=int, required=True)
    parser.add_argument("--region-radius", type=float, required=True)
    parser.add_argument("--stroke-radius", type=float, required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--summary", required=True)
    arguments = parser.parse_args()
    if arguments.region_radius < 0 or arguments.stroke_radius < 0:
        parser.error("region and stroke radii must be nonnegative")

    clock_before = read_clock_anchor(arguments.clock_before)
    clock_after = read_clock_anchor(arguments.clock_after)
    device_to_host, clock_slope = make_device_to_host(clock_before, clock_after)
    contact = read_longest_contact(arguments.injection)
    contact_start = device_to_host(contact[0].device_seconds)
    contact_end = device_to_host(contact[-1].device_seconds)

    camera_direction = 1 if arguments.camera_end > arguments.camera_start else -1
    first_center = int(arguments.camera_start) + camera_direction * 10
    last_center = int(arguments.camera_end)
    centers = list(range(first_center, last_center + camera_direction, camera_direction * 10))
    variable_axis = "y" if arguments.camera_fixed_axis == "x" else "x"
    names = {
        f"p_{arguments.camera_fixed_coordinate}_{center}"
        if variable_axis == "y"
        else f"p_{center}_{arguments.camera_fixed_coordinate}"
        for center in centers
    }
    camera = read_camera_columns(arguments.camera, names)
    frame_interval = median(
        samples[index + 1][0] - samples[index][0]
        for samples in camera.values()
        for index in range(len(samples) - 1)
    )

    output_rows = []
    causal_offset = camera_direction * (arguments.region_radius + arguments.stroke_radius)
    for center in centers:
        causal_camera_coordinate = center - causal_offset
        calibration_fraction = (
            (causal_camera_coordinate - arguments.camera_start)
            / (arguments.camera_end - arguments.camera_start)
        )
        causal_raw_coordinate = arguments.raw_start + calibration_fraction * (
            arguments.raw_end - arguments.raw_start
        )
        input_device_time = interpolate_input_time(
            contact, arguments.input_axis, causal_raw_coordinate
        )
        input_host_time = device_to_host(input_device_time)
        name = (
            f"p_{arguments.camera_fixed_coordinate}_{center}"
            if variable_axis == "y"
            else f"p_{center}_{arguments.camera_fixed_coordinate}"
        )
        samples = camera[name]
        baseline = median(
            value
            for time, value in samples
            if contact_start - 0.30 <= time <= contact_start - 0.05
        )
        final = median(
            value
            for time, value in samples
            if contact_end + 0.10 <= time <= contact_end + 0.50
        )
        amplitude = final - baseline
        if abs(amplitude) < 5:
            continue
        onset = first_sustained_crossing(
            samples,
            baseline,
            amplitude,
            input_host_time - 0.10,
            min(input_host_time + 0.40, contact_end + 0.50),
            0.20,
        )
        output_rows.append({
            "region": name,
            "camera_center": center,
            "causal_camera_coordinate": causal_camera_coordinate,
            "causal_raw_coordinate": causal_raw_coordinate,
            "input_host_seconds": input_host_time,
            "visible_onset_seconds": onset,
            "visible_onset_ms": (onset - input_host_time) * 1000,
            "baseline_luma": baseline,
            "final_luma": final,
            "luma_amplitude": amplitude,
        })

    if not output_rows:
        raise ValueError("no camera region had a measurable optical response")
    with open(arguments.output, "w", newline="") as destination:
        writer = csv.DictWriter(
            destination, fieldnames=list(output_rows[0]), lineterminator="\n"
        )
        writer.writeheader()
        writer.writerows(output_rows)

    latencies = [row["visible_onset_ms"] for row in output_rows]
    latency_median = median(latencies)
    summary = {
        "schema": 1,
        "input_axis": arguments.input_axis,
        "raw_start": arguments.raw_start,
        "raw_end": arguments.raw_end,
        "camera_start": arguments.camera_start,
        "camera_end": arguments.camera_end,
        "camera_fixed_axis": arguments.camera_fixed_axis,
        "camera_fixed_coordinate": arguments.camera_fixed_coordinate,
        "region_radius": arguments.region_radius,
        "stroke_radius": arguments.stroke_radius,
        "optical_threshold": 0.20,
        "sample_count": len(latencies),
        "median_visible_onset_ms": latency_median,
        "median_absolute_deviation_ms": median(
            abs(latency - latency_median) for latency in latencies
        ),
        "minimum_visible_onset_ms": min(latencies),
        "maximum_visible_onset_ms": max(latencies),
        "camera_interval_ms": frame_interval * 1000,
        "clock_slope_ppm": clock_slope * 1_000_000,
        "clock_uncertainty_ms": max(
            clock_before.uncertainty_seconds,
            clock_after.uncertainty_seconds,
        ) * 1000,
        "camera_quantization_uncertainty_ms": frame_interval * 500,
        "contact_start_host_seconds": contact_start,
        "contact_end_host_seconds": contact_end,
    }
    Path(arguments.summary).write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
