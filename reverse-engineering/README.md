# Native reverse engineering

This directory owns every tool and artifact used to understand Xochitl. None of
its crates are linked into the Remarque application.

- `native-observer/` contains on-device runtime capture probes.
- `display-response/` aligns device, camera, and input clocks to measure the
  physical panel rather than infer responsiveness from queue completion.
- `native-replay/` converts distilled fixtures into differential tests against
  the clean application core.
- `ghidra/` assigns evidence-backed names and types to the stripped binary.
- `scripts/` reproducibly exports and checks local readable decompilation.
- `xochitl.md` records measured behavior and bounded hypotheses.

Its output is the evidence layer for the clean Rust implementation documented
in `../app/core/README.md`. Decompiled functions are never copied into the
Rust crate. Each behavior is reduced first to equations, invariants, layouts,
and native test vectors.

`ghidra/recovered-symbols.tsv` is the source of truth. Every row names an
operation, records its subsystem and states the evidence supporting the name.
`ApplyRecoveredNames.java` applies those function and parameter names to the
Ghidra project. `recovered-signatures.tsv` records only signatures justified by
the calling convention and control flow. `ApplyRecoveredTypes.java` installs
those signatures plus the recovered binary layouts.
`ExportRecoveredFunctions.java` writes one C-like file per subsystem under
`readable/3.27.3.0/`. `ExportRecoveredCallGraph.java` records their direct call
edges so unnamed dependencies can be prioritized by evidence rather than by
address proximity.

The `readable/` tree is a local analysis artifact and is ignored by Git. Public
history contains the reproducible tooling, evidence-backed symbol map, and
clean-room behavioral descriptions, not generated proprietary decompiler
output.

Run:

```sh
reverse-engineering/scripts/export-readable-xochitl
reverse-engineering/scripts/check-readable-xochitl
```

The command is intentionally memory-capped. It expects the analyzed Ghidra
project created during the 3.27.3.0 investigation. Environment variables in
the script can point at another compatible Ghidra installation or project.
The export refuses to run against a binary whose SHA-256 does not match this
map. The check command validates both manifests and verifies that every named
operation appears in its subsystem export.

For a focused investigation, `DecompileFunctionsInRange.java` accepts an
inclusive start and exclusive end address and prints every function in that
range. Pass it to Ghidra headless with `-postScript`, for example
`-postScript DecompileFunctionsInRange.java 00795000 00799000`.

Only promote a symbol into the manifest when static control flow, recovered
metadata, or a controlled runtime test establishes what the function does.
Use a behavior name, not a call-site or feature-context name.
