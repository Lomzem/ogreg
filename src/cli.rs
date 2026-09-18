use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Read registers and send device commands",
    after_help = "Example: ogreg --host 192.0.2.1 --slot 3 reg read 0x20\nPlace --host and --slot before the subcommand."
)]
pub struct Cli {
    /// Frame hostname or IP address
    #[arg(long)]
    pub host: String,
    /// Target card slot, from 1 through 20
    #[arg(long, value_parser = clap::value_parser!(u8).range(1..=20))]
    pub slot: u8,
    /// Binary protocol TCP port
    #[arg(long, default_value_t = 5253, global = true, value_parser = clap::value_parser!(u16).range(1..))]
    pub port: u16,
    /// Response timeout or command collection window, such as 500ms or 2s
    #[arg(long, default_value = "2s", global = true, value_parser = parse_duration)]
    pub timeout: Duration,
    /// Do not evict another connection when the frame is full
    #[arg(long, global = true)]
    pub no_force: bool,
    /// Print one JSON object per result or received text message
    #[arg(long, global = true)]
    pub json: bool,
    /// Print decimal values; addresses stay hexadecimal and input parsing is unchanged
    #[arg(long, global = true)]
    pub decimal_output: bool,
    #[command(subcommand)]
    pub operation: Operation,
}

#[derive(Debug, Subcommand)]
pub enum Operation {
    /// Read or write a register
    Reg {
        #[command(subcommand)]
        operation: RegisterOperation,
    },
    /// Send quoted command text unchanged and collect printed output
    Command {
        #[arg(value_name = "TEXT", value_parser = parse_command)]
        text: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum RegisterOperation {
    /// Read one register; inputs are decimal or hexadecimal with a 0x prefix
    Read {
        /// Register address, 0..16777215 or 0x0..0xffffff
        #[arg(value_parser = parse_address)]
        address: u32,
        /// Repeat timestamped reads at this interval, such as 500ms or 2s
        #[arg(long, value_parser = parse_duration)]
        poll: Option<Duration>,
    },
    /// Write one byte and require matching readback
    Write {
        /// Register address, 0..16777215 or 0x0..0xffffff
        #[arg(value_parser = parse_address)]
        address: u32,
        /// Byte value, 0..255 or 0x0..0xff
        #[arg(value_parser = parse_byte)]
        value: u8,
    },
}

fn parse_unsigned(input: &str) -> Result<u32, String> {
    let (digits, radix) = match input
        .strip_prefix("0x")
        .or_else(|| input.strip_prefix("0X"))
    {
        Some(digits) => (digits, 16),
        None => (input, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii() && c.is_digit(radix)) {
        return Err("use an unsigned decimal integer or hexadecimal with a 0x prefix".into());
    }
    u32::from_str_radix(digits, radix).map_err(|_| "integer exceeds the supported range".into())
}

fn parse_address(input: &str) -> Result<u32, String> {
    let value = parse_unsigned(input)?;
    if value > 0x00ff_ffff {
        return Err("address must be between 0 and 0xffffff".into());
    }
    Ok(value)
}

fn parse_byte(input: &str) -> Result<u8, String> {
    u8::try_from(parse_unsigned(input)?).map_err(|_| "value must be between 0 and 0xff".into())
}

fn parse_duration(input: &str) -> Result<Duration, String> {
    let value = humantime::parse_duration(input).map_err(|error| error.to_string())?;
    if value.is_zero() {
        return Err("duration must be greater than zero".into());
    }
    if Instant::now().checked_add(value).is_none() {
        return Err("duration exceeds the supported range".into());
    }
    Ok(value)
}

fn parse_command(input: &str) -> Result<String, String> {
    if input.trim().is_empty() || input.contains('\0') {
        return Err("command must contain text and cannot contain a NUL character".into());
    }
    if input.len() > 259 {
        return Err("command must fit in 259 UTF-8 bytes".into());
    }
    Ok(input.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(
            ["ogreg", "--host", "192.0.2.1", "--slot", "3"]
                .into_iter()
                .chain(args.iter().copied()),
        )
    }

    #[test]
    fn input_radix_is_independent_of_output_format() {
        for output in ["--json", "--decimal-output"] {
            let cli = parse(&["reg", "write", "20", "0x2a", output]).unwrap();
            assert!(matches!(
                cli.operation,
                Operation::Reg {
                    operation: RegisterOperation::Write {
                        address: 20,
                        value: 42
                    }
                }
            ));
            assert_eq!(cli.timeout, Duration::from_secs(2));
            assert!(!cli.no_force);
        }
        assert_eq!(parse_address("0x20").unwrap(), 32);
        assert_eq!(parse_address("020").unwrap(), 20);
        assert_eq!(parse_address("0XFF").unwrap(), 255);
    }

    #[test]
    fn rejects_values_that_firmware_would_truncate() {
        assert!(parse(&["reg", "read", "0x1000000"]).is_err());
        assert!(parse(&["reg", "write", "0", "256"]).is_err());
        assert!(parse(&["reg", "write", "0xffffff", "0xff"]).is_ok());
        for input in ["-1", "+1", "0x", "ff", "1.0", "1_000", "4294967296"] {
            assert!(parse_address(input).is_err(), "accepted {input}");
        }
    }

    #[test]
    fn polling_accepts_units_and_rejects_invalid_intervals() {
        let cli = parse(&[
            "reg",
            "read",
            "0x20",
            "--poll",
            "500ms",
            "--timeout",
            "1.5s",
        ])
        .unwrap();
        assert_eq!(cli.timeout, Duration::from_millis(1500));
        assert!(
            matches!(cli.operation, Operation::Reg { operation: RegisterOperation::Read { address: 32, poll: Some(interval) } } if interval == Duration::from_millis(500))
        );
        for interval in ["0s", "-1s", "500", "nonsense"] {
            assert!(parse(&["reg", "read", "0", "--poll", interval]).is_err());
        }
        assert!(parse(&["reg", "write", "0", "0", "--poll", "1s"]).is_err());
    }

    #[test]
    fn raw_command_does_not_rewrite_register_inputs() {
        for text in ["fpgarr 20", "fpgarw 0x20 0xff", "  custom command  "] {
            let cli = parse(&["command", text]).unwrap();
            assert!(matches!(cli.operation, Operation::Command { text: actual } if actual == text));
        }
        assert!(parse(&["command", ""]).is_err());
        assert!(parse(&["command", "read\0write"]).is_err());
        assert!(parse(&["command", &"x".repeat(259)]).is_ok());
        assert!(parse(&["command", &"x".repeat(260)]).is_err());
        assert!(parse(&["command", &"é".repeat(130)]).is_err());
    }

    #[test]
    fn target_is_required_and_slot_is_validated() {
        assert!(Cli::try_parse_from(["ogreg", "reg", "read", "0"]).is_err());
        for slot in ["0", "21", "255"] {
            assert!(
                Cli::try_parse_from([
                    "ogreg",
                    "--host",
                    "192.0.2.1",
                    "--slot",
                    slot,
                    "reg",
                    "read",
                    "0"
                ])
                .is_err()
            );
        }
    }
}
