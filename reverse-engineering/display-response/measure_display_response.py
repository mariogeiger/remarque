#!/usr/bin/env python3
import argparse
import csv
import statistics
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class ClockAnchor:
    device_seconds: float
    device_minus_mac_seconds: float
    uncertainty_seconds: float


@dataclass(frozen=True)
class DeviceEvent:
    update: str
    direction: str
    size: int
    repetition: int
    before_submit: float
    after_submit: float
    after_drain: float
    accepted: int
    drained: int


def median(values):
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
    uncertainty = max(
        median(round_trips) / 2,
        (max(offsets) - min(offsets)) / 2,
    )
    return ClockAnchor(median(device_times), median(offsets), uncertainty)


def read_device_events(path):
    events = []
    header_seen = False
    for line in Path(path).read_text().splitlines():
        if line.startswith("update,"):
            header_seen = True
            continue
        fields = line.split(",")
        if not header_seen or len(fields) != 9 or not fields[4].isdigit():
            continue
        events.append(DeviceEvent(
            update=fields[0],
            direction=fields[1],
            size=int(fields[2].split("x")[0]),
            repetition=int(fields[3]),
            before_submit=int(fields[4]) / 1_000_000,
            after_submit=int(fields[5]) / 1_000_000,
            after_drain=int(fields[6]) / 1_000_000,
            accepted=int(fields[7]),
            drained=int(fields[8]),
        ))
    return events


def read_camera_samples(path):
    with open(path, newline="") as source:
        rows = list(csv.DictReader(source))
    return [
        (float(row["presentation_seconds"]), float(row["center"]))
        for row in rows
    ]


def interpolate_crossing(samples, progress, index, threshold):
    if index == 0:
        return samples[index][0]
    previous = progress[index - 1]
    current = progress[index]
    if current == previous:
        return samples[index][0]
    fraction = (threshold - previous) / (current - previous)
    fraction = min(1.0, max(0.0, fraction))
    return samples[index - 1][0] + fraction * (samples[index][0] - samples[index - 1][0])


def first_sustained_crossing(samples, progress, start, end, threshold):
    candidates = [
        index for index, (time, _) in enumerate(samples)
        if start <= time <= end
    ]
    for index in candidates:
        if progress[index] < threshold:
            continue
        following = progress[index : index + 2]
        if len(following) == 2 and min(following) >= threshold:
            return interpolate_crossing(samples, progress, index, threshold)
    raise ValueError(f"no sustained {threshold:.0%} crossing")


def settled_time(samples, progress, start, end, tolerance):
    indices = [
        index for index, (time, _) in enumerate(samples)
        if start <= time <= end
    ]
    if not indices:
        raise ValueError("empty settling window")
    last_outside = None
    for index in indices:
        if abs(progress[index] - 1) > tolerance:
            last_outside = index
    if last_outside is None:
        return samples[indices[0]][0]
    next_index = last_outside + 1
    if next_index > indices[-1]:
        raise ValueError("response did not settle")
    return samples[next_index][0]


def classify(update):
    if update.startswith("mono-fast-"):
        return "mono-fast", update.removeprefix("mono-fast-")
    if update.startswith("mono-quality-"):
        return "mono-quality", update.removeprefix("mono-quality-")
    if update.startswith("color3-"):
        return "color3", update.removeprefix("color3-")
    if update.startswith("native-live-"):
        return "native-live", update.removeprefix("native-live-")
    if update.startswith("native-mode14-"):
        return "native-mode14", update.removeprefix("native-mode14-")
    if update.startswith("color-"):
        target = update.removeprefix("color-")
        return "color", target
    return "marker", update.removeprefix("marker-")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--camera", required=True)
    parser.add_argument("--device", required=True)
    parser.add_argument("--clock-before", required=True)
    parser.add_argument("--clock-after", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--summary")
    command_line = parser.parse_args()

    clock_before = read_clock_anchor(command_line.clock_before)
    clock_after = read_clock_anchor(command_line.clock_after)
    events = read_device_events(command_line.device)
    camera = read_camera_samples(command_line.camera)
    if not events or not camera:
        raise ValueError("missing device events or camera samples")

    offset_slope = (
        clock_after.device_minus_mac_seconds - clock_before.device_minus_mac_seconds
    ) / (clock_after.device_seconds - clock_before.device_seconds)

    def device_to_mac(device_seconds):
        offset = clock_before.device_minus_mac_seconds + offset_slope * (
            device_seconds - clock_before.device_seconds
        )
        return device_seconds - offset

    mac_events = []
    for event in events:
        mac_events.append((
            event,
            device_to_mac(event.before_submit),
            device_to_mac(event.after_submit),
            device_to_mac(event.after_drain),
        ))

    frame_interval = median([
        camera[index + 1][0] - camera[index][0]
        for index in range(len(camera) - 1)
    ])
    output_rows = []
    for event_index, (event, before, after, drain) in enumerate(mac_events):
        next_before = (
            mac_events[event_index + 1][1]
            if event_index + 1 < len(mac_events)
            else camera[-1][0]
        )
        baseline_values = [
            value for time, value in camera
            if before - 0.15 <= time <= before + 0.04
        ]
        final_end = min(drain + 0.30, next_before - 0.15)
        final_values = [
            value for time, value in camera
            if drain + 0.05 <= time <= final_end
        ]
        baseline = median(baseline_values)
        final = median(final_values)
        amplitude = final - baseline
        if abs(amplitude) < 5:
            raise ValueError(f"optical amplitude too small for event {event_index}")
        progress = [(value - baseline) / amplitude for _, value in camera]
        search_end = final_end
        onset = first_sustained_crossing(camera, progress, before, search_end, 0.10)
        first_90 = first_sustained_crossing(camera, progress, onset, search_end, 0.90)
        settle = settled_time(camera, progress, onset, search_end, 0.10)
        family, target = classify(event.update)
        output_rows.append({
            "event": event_index,
            "family": family,
            "target": target,
            "direction": event.direction,
            "size": event.size,
            "repetition": event.repetition,
            "software_submit_ms": (after - before) * 1000,
            "queue_wait_ms": (drain - after) * 1000,
            "queue_total_ms": (drain - before) * 1000,
            "visible_onset_ms": (onset - before) * 1000,
            "first_90_ms": (first_90 - before) * 1000,
            "settled_ms": (settle - before) * 1000,
            "visible_motion_ms": (settle - onset) * 1000,
            "settled_minus_drain_ms": (settle - drain) * 1000,
            "baseline_luma": baseline,
            "final_luma": final,
            "luma_amplitude": amplitude,
            "accepted": event.accepted,
            "drained": event.drained,
        })

    fieldnames = list(output_rows[0])
    with open(command_line.output, "w", newline="") as destination:
        writer = csv.DictWriter(
            destination, fieldnames=fieldnames, lineterminator="\n"
        )
        writer.writeheader()
        writer.writerows(output_rows)

    if command_line.summary:
        metric_names = [
            "software_submit_ms",
            "queue_wait_ms",
            "queue_total_ms",
            "visible_onset_ms",
            "first_90_ms",
            "settled_ms",
            "visible_motion_ms",
            "settled_minus_drain_ms",
            "baseline_luma",
            "final_luma",
            "luma_amplitude",
        ]
        groups = {}
        for row in output_rows:
            key = (row["family"], row["target"], row["size"])
            groups.setdefault(key, []).append(row)
        summary_rows = []
        for (family, target, size), rows in groups.items():
            summary_rows.append({
                "family": family,
                "target": target,
                "size": size,
                "sample_count": len(rows),
                **{
                    f"median_{metric}": median([row[metric] for row in rows])
                    for metric in metric_names
                },
            })
        summary_fieldnames = list(summary_rows[0])
        with open(command_line.summary, "w", newline="") as destination:
            writer = csv.DictWriter(
                destination, fieldnames=summary_fieldnames, lineterminator="\n"
            )
            writer.writeheader()
            writer.writerows(summary_rows)

    print(f"events,{len(output_rows)}")
    print(f"camera_frames,{len(camera)}")
    print(f"camera_interval_ms,{frame_interval * 1000:.6f}")
    print(f"clock_offset_slope_ppm,{offset_slope * 1_000_000:.3f}")
    print(
        "clock_uncertainty_ms,"
        f"{max(clock_before.uncertainty_seconds, clock_after.uncertainty_seconds) * 1000:.3f}"
    )


if __name__ == "__main__":
    main()
