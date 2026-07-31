# reMarkable Paper Pro field guide

This is a compact record of the device behavior used by Remarque. It combines
community research with observations verified on the tablet used during
development.

The interfaces described here are undocumented and firmware-sensitive. Treat
addresses and allocation layouts as runtime discoveries, never constants.

## Tested device

| Property | Value |
| --- | --- |
| Device | reMarkable Paper Pro |
| Firmware | `3.27.3.0` |
| Architecture | `aarch64` |
| Kernel | Linux `6.12.49` |
| Display | `1620x2160`, portrait |
| Pixel format | 32-bit BGRA |
| Bytes per row | `6528` |
| Visible frame size | `14,100,480` bytes |

The panel is physically color e-ink. Screen contents change far less often than
an LCD, which makes tile-based updates particularly effective.

## Developer access

Developer Mode is required for SSH and access to `xochitl` process memory.
Enabling it performs a factory reset, weakens the device security model, and may
affect support or warranty coverage. Back up and sync the tablet first.

Enable it on the tablet:

```text
Settings > General > Paper Tablet > Software > Advanced > Developer Mode
```

After the reset, find the generated `root` password under:

```text
Settings > General > Help > About > Copyrights and Licenses
```

USB SSH uses a fixed address:

```sh
ssh root@10.11.99.1
```

Install a public key, then enable SSH over Wi-Fi from the tablet:

```sh
rm-ssh-over-wlan on
```

The Wi-Fi address is assigned by the local router and may change.

## Filesystem rules

- Keep custom binaries and data under `/home/root`; this partition persists
  across normal OS updates.
- The root filesystem is read-only by default.
- Avoid changing `/`, `/etc`, or system services until recovery and backup paths
  have been tested.
- Firmware updates can invalidate framebuffer discovery even when the binary
  itself still runs.

Remarque runs manually from `/home/root` and does not install a boot service.

## Why `/dev/fb0` does not work

Earlier reMarkable generations exposed either a real framebuffer or a useful
buffer adjacent to a `/dev/fb0` mapping. The Paper Pro uses a DRM-based graphics
stack instead. On the tested firmware:

- `/dev/fb0` is absent;
- `/dev/dri/card0` exists;
- `xochitl` maps `/dev/dri/card0` multiple times;
- the active image is stored in a following anonymous allocation.

This is why older `armv7` binaries and framebuffer recipes for reMarkable 1 or 2
are not applicable.

## Framebuffer discovery

Remarque follows the approach demonstrated by `goMarkableStream`:

1. Find the current `xochitl` PID by process name.
2. Parse `/proc/<pid>/maps` and take the end address of the last
   `/dev/dri/card0` mapping.
3. Open `/proc/<pid>/mem` read-only.
4. Follow the allocator headers after the DRM mapping until a block large enough
   to hold the Paper Pro frame is found.
5. Read `6528 * 2160` bytes from that address.

On the tested session, the discovered allocation was `14,102,530` bytes. The
visible frame consumes `14,100,480` bytes. These values validate the candidate,
but the virtual address and process ID change and are always rediscovered.

The current binary refuses to run on firmware other than `3.27.3.0`. Reading a
plausible but incorrect process address is a worse failure mode than stopping
with a clear compatibility error.

## Streaming protocol

The tablet serves an embedded HTML viewer and a `/ws/2` WebSocket on port
`7432`. Versioning the path rejects stale viewers before starting a capture.

Each binary WebSocket message starts with a 16-byte little-endian header. The
display geometry is fixed by protocol version 2 rather than repeated in every
message:

| Offset | Size | Field |
| --- | ---: | --- |
| `0` | 4 | Magic: `RMKS` |
| `4` | 1 | Protocol version |
| `5` | 1 | Message type |
| `8` | 4 | Payload length |
| `12` | 4 | Tile count |

Message types are:

- `1`: complete BGRA frame;
- `2`: changed tiles.

A changed `64x64` tile contains an 8-byte `x, y, width, height` descriptor
followed by tightly packed BGRA pixels. The browser converts only those tiles to
RGBA and updates the corresponding canvas regions. A complete frame is sent on
connect or when a delta would exceed half a frame.

Pen pressure and touch events wake a 10 Hz capture loop. It remains active for
800 ms after the last input, then falls back to one scan every five seconds. This
keeps idle CPU usage low while still catching non-input screen changes. Only one
viewer is allowed, and the one-message queue provides strict backpressure.

## Input devices

The tested firmware reports:

| Device name | Typical node |
| --- | --- |
| Power key | `/dev/input/event0` |
| Hall sensors | `/dev/input/event1` |
| Elan marker input | `/dev/input/event2` |
| Elan touch input | `/dev/input/event3` |

These node numbers are observations, not an API. Any future input support should
enumerate devices and match their names and capabilities at runtime.

## Current limitations

- Firmware support is pinned to `3.27.3.0`.
- The server is plain HTTP/WebSocket with no authentication.
- A `xochitl` restart requires restarting the agent.
- Pixel tiles are not compressed. A measured drawing session averaged roughly
  `192 KB/s`; compression would spend more of the tablet's limited CPU to save
  bandwidth that a local Wi-Fi connection does not need.
- Screen content is private data. Anyone who can reach the server can view it.

## Primary references

- [Official Developer Mode documentation](https://developer.remarkable.com/documentation/developer-mode)
- [XOVI](https://github.com/asivery/xovi)
- [XOVI framebuffer-spy](https://github.com/asivery/rm-xovi-extensions/tree/master/framebuffer-spy)
- [goMarkableStream](https://github.com/owulveryck/goMarkableStream)
- [awesome-reMarkable](https://github.com/reHackable/awesome-reMarkable)
