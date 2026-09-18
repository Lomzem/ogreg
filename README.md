# Register command client

A command-line client for reading and writing device registers through a frame's TCP connection. It runs on Linux and Windows.

## Build

With a current stable Rust toolchain installed, run:

```sh
cargo build --release --locked
```

The executable is `target/release/register-cli` on Linux and `target/release/register-cli.exe` on Windows. Run `cargo test --locked` to exercise the parser, protocol, and mock TCP frame tests without hardware.

### Build both platforms with Docker

With Docker running in Linux-container mode and Buildx installed, run:

```sh
docker buildx build --platform linux/amd64 --output type=local,dest=dist .
```

This exports both x86-64 release binaries to the host:

- `dist/linux-x86_64/register-cli`, linked statically with musl.
- `dist/windows-x86_64/register-cli.exe`, built with MinGW.

No host Rust toolchain is needed. The Docker build runs the Linux tests and checks the Windows executable for unexpected compiler runtime DLL dependencies. It does not run the Windows executable. The final Docker stage contains build artifacts, not a runnable container.

The build defaults to Rust 1.98.1. Override it with `--build-arg RUST_VERSION=<version>` when updating the toolchain. Keep `--platform linux/amd64` on ARM hosts too; the builder must support x86-64 emulation. The build context includes only manifests, source, tests, and Docker configuration.

## Usage

Specify the frame host and card slot before the subcommand. Slots range from 1 through 20. The TCP port defaults to 5253; override it with `--port`.

```sh
register-cli --host 192.0.2.1 --slot 3 reg read 0x20
register-cli --host 192.0.2.1 --slot 3 reg write 0x20 0x2a
register-cli --host 192.0.2.1 --slot 3 reg read 0x20 --poll 500ms
register-cli --host 192.0.2.1 --slot 3 command "fpgarr 0x20"
register-cli --host 192.0.2.1 --slot 3 command "fpgarw 0x20 0x2a"
```

In PowerShell, use `register-cli.exe`, or `./register-cli.exe` when running it from the current directory. The arguments and quoted commands are the same.

Unprefixed register inputs are decimal. Use `0x` for hexadecimal. For example, `20` is decimal twenty and `0x20` is decimal thirty-two. Dedicated register commands accept addresses up to `0xffffff` and byte values up to `0xff`. They reject invalid inputs rather than letting the firmware truncate them. Generic `command` text is sent unchanged and follows the device's own parsing rules.

Connection takeover is enabled by default. Add `--no-force` to refuse connection takeover when the frame has no available connection capacity. This option does not make access exclusive. Authentication-protected connections report a refusal; password authentication is not implemented.

## Output for scripts

Each register result occupies one line and is flushed immediately. Diagnostics go to stderr.

| Options | Output |
| --- | --- |
| Default | `Address 0x20=0x2a` |
| `--decimal-output` | `Address 32=42` |
| `--json` | `{"address":"0x20","value":"0x2a"}` |
| `--json --decimal-output` | `{"address":32,"value":42}` |

`--decimal-output` changes output only. Polling emits the same format repeatedly, including one complete JSON object per line with `--json`.

Generic commands print the device's text, adding a newline when a message lacks one. With `--json`, each received text message becomes an object such as `{"text":"Register 0x20 = 0x2a"}`. Generic output is not parsed as a register result, and `--decimal-output` does not rewrite it. Commands must fit in 259 UTF-8 bytes.

## Timing and failures

`--timeout` defaults to `2s` and accepts units, such as `500ms` or `10s`. Connection setup has its own timeout. Register operations require both a successful command acknowledgement and a result for the requested card and register within the operation timeout. Writes require the reported readback to match the requested value. A mismatch fails without retrying the write.

Generic commands wait up to the timeout for acknowledgement, then collect text for another timeout window. Text received before acknowledgement is also printed. Expiry of the collection window is normal. It means collection ended, not that the device completed or successfully executed the command. A missing or rejected acknowledgement, malformed message, or disconnected socket is an error.

Polling keeps one connection open and permits only one outstanding read. The poll interval is the minimum time between request starts. Slow reads do not trigger catch-up bursts. A timeout or disconnect stops polling. Press Ctrl+C to stop it yourself.

Exit status is 0 for a completed register operation or generic collection window, 1 for an operation error, 2 for invalid CLI arguments, and 130 for Ctrl+C handled during an operation.

The existing firmware does not report hardware access failures reliably. A failed read can appear as zero. Printed output also has no request identifier, so overlapping clients can cause stale results. The client checks the card and register address but cannot remove these firmware limitations. The default timeout still needs validation against physical hardware.
