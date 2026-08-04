# Power validation

Remarque requests deep suspend followed by shutdown-mode hibernation after four
hours. The repository deploys that policy from
`app/deploy/60-remarque-sleep.conf`.

## Established observations

- The Paper Pro exposes `mem` and `disk`; `deep` is the selected memory sleep.
- The encrypted swap device and kernel resume target are configured.
- Short Remarque cycles increment the kernel successful-suspend counter while
  preserving the process, open document, and view.
- No Remarque cycle has yet remained asleep beyond the four-hour boundary.

## Overnight experiment

The competing outcomes are:

1. The RTC wakes deep suspend after four hours and systemd hibernates. The
   journal reports an attempted hibernation, a later power-button wake restores
   the same Remarque process, and one measurement spans the whole night.
2. The RTC transition fails and the tablet remains only suspended. The journal
   has no hibernation attempt even though the measurement spans more than four
   hours.
3. Resume fails and the tablet cold-boots. The in-memory measurement cannot be
   completed, and Remarque starts as a new process.

For a controlled test, disconnect charging power, record the Remarque process
ID, press the power button once, leave the tablet untouched for at least eight
hours, then wake it once. Inspect the newest JSON line at
`/home/root/remarque/data/exchange/sleep-cycle-measurements.jsonl`, the process
ID, and the `systemd-suspend-then-hibernate.service` journal. A valid consumption
sample has discharging battery states before and after, an elapsed duration of
at least eight hours, and an increased successful-suspend counter. Hibernation
is established only if the journal also records the hibernation transition and
the original process resumes.
