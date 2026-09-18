# ogreg

Read and write device registers on Linux or Windows.

## Build

Start Docker with Linux containers. Run this command to build both x86-64 executables:

```sh
docker buildx build --platform linux/amd64 --output type=local,dest=dist .
```

Find the executables in these directories:

- Linux: `dist/linux-x86_64/ogreg`
- Windows: `dist/windows-x86_64/ogreg.exe`

To build for your current system with Rust, run `cargo build --release --locked`.
Find the executable in `target/release/`.

## Use

Copy the executable to a directory in `PATH`. On Windows, use `ogreg.exe`.
Replace the example IP address and slot number with your target values.
Put `--host` and `--slot` before the command. Use a slot from 1 through 20.

```sh
# Read a register.
ogreg --host 192.0.2.1 --slot 3 reg read 0x20

# Write a register and check the returned value.
ogreg --host 192.0.2.1 --slot 3 reg write 0x20 0x2a

# Read a register every 500 milliseconds.
ogreg --host 192.0.2.1 --slot 3 reg read 0x20 --poll 500ms

# Send command text to the device.
ogreg --host 192.0.2.1 --slot 3 command "fpgarr 0x20"
ogreg --host 192.0.2.1 --slot 3 command "fpgarw 0x20 0x2a"
```

Use `0x` for hexadecimal input. Without a prefix, register commands use decimal input.
The maximum address is `0xffffff`. The maximum value is `0xff`.

## Options

Add these options to a register command:

| Option | Example output |
| --- | --- |
| None | `Address 0x20=0x2a` |
| `--decimal-output` | `Address 32=42` |
| `--json` | `{"address":"0x20","value":"0x2a"}` |
| `--json --decimal-output` | `{"address":32,"value":42}` |

Use `--timeout 10s` to change the default timeout of two seconds.
For `command`, this timeout also sets the output collection period after acknowledgement.
Use `--port` to change TCP port 5253.
Use `--no-force` to keep other clients connected when all connections are in use.

Press Ctrl+C to stop repeated reads. The tool stops on an error and returns a nonzero exit code.
A failed device read can report zero. Other clients can cause the tool to show an earlier result.

Run `ogreg --help` for more options.
