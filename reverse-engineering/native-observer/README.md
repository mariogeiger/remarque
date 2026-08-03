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
