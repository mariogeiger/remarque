# Remarque soul

This file defines how Remarque should feel and how design decisions should be
made. It stays shorter and more stable than technical documentation.

## Mission

Build an independent, customizable reMarkable Paper Pro application in Rust
that preserves the native application's best behaviors through verified
reconstruction and grows beyond them with purpose-built features.

## Design rules

1. **Choose the simplest sufficient design.** Model each behavior directly and
   add a layer only when it creates an independent contract.
2. **Name things by what they do.** Names describe operations and data, never a
   current caller or implementation accident.
3. **Keep evidence, behavior, and hardware separate.** Decompiled output is an
   oracle aid, `app/core` is device-independent behavior, and `app/tablet`
   adapts Linux input and display devices.
4. **Own every transition.** Tablet takeover, input grabs, temporary drawing,
   display updates, and return to Xochitl have explicit lifecycles.
5. **Fail explicitly and actionably.** Unsupported firmware, malformed native
   fixtures, and hardware failures must identify the violated boundary.
6. **Treat native behavior as a foundation, not a ceiling.** Preserve what it
   does best, discard accidental constraints, and design original features
   around the experience Remarque intends to provide.
7. **Earn claims with tests.** Native-parity claims become immutable fixtures;
   original behavior receives explicit contracts and property-focused tests.
8. **Protect captured data.** Runtime captures may contain private handwriting
   and stay outside version control until deliberately distilled.
9. **Keep each file bounded.** Give each file one coherent mission and split it
   before 1000 lines.
