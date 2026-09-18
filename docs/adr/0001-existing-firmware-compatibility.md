# Support existing firmware

The CLI must work with unchanged firmware. For now, it assumes that the register read and write commands behave consistently across supported devices. This allows scripts to use deployed devices without requiring firmware updates.

The expected use is one operator per frame. Stale register results caused by overlapping clients are an accepted limitation. The CLI cannot infer hardware access success from a command acknowledgement or printed register value because the existing handlers do not report hardware access failures.

Connection requests permit takeover by default. The user can disable takeover with `--no-force`. This controls admission when connection capacity is exhausted; it does not establish exclusive access.
