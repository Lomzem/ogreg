# Provide register operations and text commands

The CLI provides dedicated register read and write operations so scripts do not have to parse device output. A generic `command` mode also accepts the underlying register read and write commands and returns their printed text without extracting register values. This preserves access to other device commands and the original register output.

Users select the target with `--host` and `--slot`.

A register read performs one operation by default. An optional user-specified interval enables repeated reads and accepts duration units, such as `500ms` and `2s`.

Dedicated register operations interpret unprefixed addresses and values as decimal, and inputs with a `0x` prefix as hexadecimal. They reject malformed and out-of-range inputs instead of relying on firmware truncation. This parsing rule does not rewrite text supplied to generic `command` mode.

Default output uses hexadecimal in the form `Address 0x20=0x2a`. Addresses always remain hexadecimal. The `--decimal-output` option changes only the value to decimal, as in `Address 0x20=42`, without changing input parsing. A `--json` option provides structured output, with hexadecimal strings by default, such as `{"address":"0x20","value":"0x2a"}`. With `--json --decimal-output`, the address remains a hexadecimal string and the value becomes a JSON number, as in `{"address":"0x20","value":42}`.

Register writes require the reported readback to equal the requested value and fail on a mismatch. A matching readback still cannot prove hardware access success because the firmware does not report those failures.

Polling keeps one connection open and allows one outstanding read at a time. The interval sets the minimum spacing between request starts, without catch-up bursts. A timeout or disconnect stops polling with a nonzero exit status.

The `--timeout` option defaults to `2s` and accepts duration units. Register operations finish when their expected result arrives or fail on timeout. Generic text commands collect output for the configured window after acknowledgement, then exit. Output collection starts before sending the request so early output is retained. Missing or rejected acknowledgement is an error. Finishing the collection window does not establish that an arbitrary command succeeded.

For polling, the timeout applies to each operation, not to the total polling duration. The two-second default is an initial setting to validate against hardware.
