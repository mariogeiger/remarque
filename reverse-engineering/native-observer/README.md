# Native observer

This crate contains firmware-specific probes that observe Xochitl and turn its
runtime behavior into versioned evidence. It is not an application dependency
and contains no Remarque screen-streaming code.

`capture-native-stroke-trace` records one synchronized trace containing:

- raw marker `evdev` events on `CLOCK_MONOTONIC`;
- native line begin, packed-point, and line-finish events;
- ribbon finalization and every antialiased triangle call;
- requests entering the e-paper update queue;
- primary framebuffer and active drawing-surface snapshots before and after.

Start it while Xochitl is active, draw naturally, then send `SIGINT` or
`SIGTERM`. The tracer restores every patched instruction before detaching.

```sh
capture-native-stroke-trace /home/root/remarque/captures/my-trace
kill -INT CAPTURE_PID
```

Analyze a copied trace from the workspace with:

```sh
cargo run -p remarque-native-replay --bin summarize-native-stroke-trace -- \
  TRACE-DIRECTORY TRACE-DIRECTORY/summary.json
```

The narrower `capture-native-line-points` and `capture-native-triangles` probes
remain useful when instrumentation overhead must be isolated to one boundary.

Open the target notebook page before starting `capture-native-stroke-trace`.
The current ptrace probe attaches the threads that already exist and installs
process-global software breakpoints. Starting it before Xochitl creates a page
worker can leave that new thread untraced at a breakpoint. This is a bounded
observer limitation, not a product-runtime limitation.

`replay-marker-input` validates and replays a selected range of a raw marker
trace through a temporary `uinput` device. It preserves source indices and
tablet monotonic write bounds in JSONL and always emits a final all-buttons-up
frame. Xochitl 3.27.3.0 opens `/dev/input/event2` by path rather than discovering
the virtual marker. For a controlled replay, start the virtual device with a
long delay, bind its event node over `event2` while Xochitl is stopped, start
Xochitl, then lazily unmount the path. Xochitl's open descriptor remains on the
virtual device while the physical marker path is restored.

```sh
replay-marker-input input.raw FIRST END 12000 injection.jsonl &
# Identify the node whose sysfs name is "Elan marker input replay".
systemctl stop xochitl
mount --bind /dev/input/eventN /dev/input/event2
systemctl start xochitl
umount -l /dev/input/event2
```

Use this only on a disposable test page. The replay is real application input.
For synchronized instrumentation, pass a nonexistent path as the optional last
argument. The virtual device becomes ready first, then replay waits until that
path exists. This separates device binding and camera setup from input timing.
Pass a second nonexistent path to keep the virtual device alive after replay;
stop its consumer cleanly, then create that path to release the device.
