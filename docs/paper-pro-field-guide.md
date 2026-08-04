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

The deployment units under `app/deploy/` keep Remarque and Xochitl mutually
exclusive and return to Xochitl when Remarque exits.

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

## Native framebuffer observation

The probes under `reverse-engineering/native-observer/` follow the approach
demonstrated by `goMarkableStream`:

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

These probes are pinned to firmware `3.27.3.0`. Reading a plausible but
incorrect process address is a worse failure mode than stopping with a clear
compatibility error. The Remarque application does not read Xochitl memory.

## Streaming protocol

The Remarque process serves an embedded HTML viewer and a `/ws/3` WebSocket on
port `7432`. This is part of the tablet process, not a second program.

Each binary WebSocket message starts with a 28-byte little-endian header:

| Offset | Size | Field |
| --- | ---: | --- |
| `0` | 4 | Magic: `RMKS` |
| `4` | 1 | Protocol version |
| `5` | 1 | Message type |
| `8` | 4 | Payload length |
| `12` | 4 | Tile count |
| `16` | 4 | Image width |
| `20` | 4 | Image height |
| `24` | 4 | Full-frame row stride |

Message types are:

- `1`: complete BGRA frame;
- `2`: changed tiles.

A changed `64x64` tile contains a 16-byte `x, y, width, height` descriptor
followed by tightly packed BGRA pixels. The browser converts only those tiles to
RGBA and updates the corresponding canvas regions. A complete frame is sent on
connect or when a delta would exceed half a frame.

Every display copy that changes at least one pixel increments a generation
counter. The most recently loaded viewer page replaces its predecessor;
automatic reconnects from an older page cannot take the stream back. The stream
snapshots at most 10 times per second and only after the display generation
changes. The snapshot reads the same synchronized Quill buffer shown on the
tablet, so idle streaming performs no framebuffer copies.

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
- The stream is plain HTTP/WebSocket with no authentication. Run it only on a
  trusted LAN or behind a private network transport.
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
