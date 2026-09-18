use crate::cli::{Cli, Operation, RegisterOperation};
use crate::output::OutputFormat;
use crate::protocol::{Client, Message};
use std::error::Error;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const RECEIVE_SLICE: Duration = Duration::from_millis(100);

pub fn run(args: &Cli, interrupted: &AtomicBool, output: &mut impl Write) -> Result<()> {
    let mut client = Client::connect(&args.host, args.port, args.timeout, !args.no_force)?;
    let format = OutputFormat {
        json: args.json,
        decimal_output: args.decimal_output,
    };
    match &args.operation {
        Operation::Reg { operation } => match operation {
            RegisterOperation::Read { address, poll } => loop {
                check_interrupt(interrupted)?;
                let started = Instant::now();
                let value = register_operation(
                    &mut client,
                    args,
                    &format!("fpgarr 0x{address:x}"),
                    *address,
                    interrupted,
                )?;
                if poll.is_some() {
                    let received_at = time::OffsetDateTime::now_utc();
                    format.write_polled_register(output, *address, value, received_at)?;
                } else {
                    format.write_register(output, *address, value)?;
                }
                let Some(interval) = poll else { break };
                wait_until(deadline(started, *interval)?, interrupted)?;
            },
            RegisterOperation::Write { address, value } => {
                let observed = register_operation(
                    &mut client,
                    args,
                    &format!("fpgarw 0x{address:x} 0x{value:x}"),
                    *address,
                    interrupted,
                )?;
                if observed != *value {
                    return Err(format!(
                        "readback mismatch at address 0x{address:x}: requested 0x{value:02x}, read 0x{observed:02x}"
                    ).into());
                }
                format.write_register(output, *address, observed)?;
            }
        },
        Operation::Command { text } => {
            raw_command(&mut client, args, text, &format, interrupted, output)?;
        }
    }
    Ok(())
}

fn deadline(start: Instant, duration: Duration) -> Result<Instant> {
    start
        .checked_add(duration)
        .ok_or_else(|| "duration is too large".into())
}

fn check_interrupt(interrupted: &AtomicBool) -> Result<()> {
    if interrupted.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "interrupted").into());
    }
    Ok(())
}

fn wait_until(until: Instant, interrupted: &AtomicBool) -> Result<()> {
    loop {
        check_interrupt(interrupted)?;
        let remaining = until.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        thread::sleep(remaining.min(RECEIVE_SLICE));
    }
}

fn receive(
    client: &mut Client,
    until: Instant,
    interrupted: &AtomicBool,
) -> Result<Option<Message>> {
    loop {
        check_interrupt(interrupted)?;
        let now = Instant::now();
        if now >= until {
            return Ok(None);
        }
        if let Some(message) = client.receive(until.min(now + RECEIVE_SLICE))? {
            return Ok(Some(message));
        }
    }
}

fn acknowledgement(payload: &[u8]) -> Result<()> {
    match payload {
        [0] => Ok(()),
        [1] => Err("device does not support the command interface".into()),
        [code] => Err(format!("device rejected command request with code {code}").into()),
        _ => Err("malformed command acknowledgement".into()),
    }
}

fn printed_text(payload: &[u8]) -> Result<&str> {
    let Some(text) = payload.strip_suffix(&[0]) else {
        return Err("printed output is missing its terminating NUL".into());
    };
    if text.contains(&0) {
        return Err("printed output contains an embedded NUL".into());
    }
    Ok(std::str::from_utf8(text)?)
}

fn register_operation(
    client: &mut Client,
    args: &Cli,
    command: &str,
    address: u32,
    interrupted: &AtomicBool,
) -> Result<u8> {
    check_interrupt(interrupted)?;
    let until = deadline(Instant::now(), args.timeout)?;
    client.send_command(args.slot, command, until)?;
    let mut acknowledged = false;
    let mut value = None;
    let mut last_text = None;
    while let Some(message) = receive(client, until, interrupted)? {
        if message.source != 0x10 + args.slot {
            continue;
        }
        match message.kind {
            0xc4 => {
                acknowledgement(&message.payload)?;
                acknowledged = true;
            }
            0x00 => {
                let text = printed_text(&message.payload)?;
                if let Some((reported_address, reported_value)) = parse_register(text)?
                    && reported_address == address
                    && value.is_none()
                {
                    value = Some(reported_value);
                }
                last_text = Some(text.to_owned());
            }
            _ => {}
        }
        if acknowledged && let Some(value) = value {
            return Ok(value);
        }
    }
    if !acknowledged {
        return Err("timed out waiting for command acknowledgement".into());
    }
    let detail = last_text
        .map(|text| format!("; last device output: {text:?}"))
        .unwrap_or_default();
    Err(format!("timed out waiting for register 0x{address:x}{detail}").into())
}

fn parse_register(text: &str) -> Result<Option<(u32, u8)>> {
    let mut words = text.split_whitespace();
    if words.next() != Some("Register") {
        return Ok(None);
    }
    let malformed = || format!("malformed register output: {text:?}");
    let address = words
        .next()
        .and_then(|v| v.strip_prefix("0x"))
        .ok_or_else(malformed)?;
    if words.next() != Some("=") {
        return Err(malformed().into());
    }
    let value = words
        .next()
        .and_then(|v| v.strip_prefix("0x"))
        .ok_or_else(malformed)?;
    if words.next().is_some() {
        return Err(malformed().into());
    }
    let address = u32::from_str_radix(address, 16).map_err(|_| malformed())?;
    let value = u8::from_str_radix(value, 16).map_err(|_| malformed())?;
    Ok(Some((address, value)))
}

fn raw_command(
    client: &mut Client,
    args: &Cli,
    command: &str,
    format: &OutputFormat,
    interrupted: &AtomicBool,
    output: &mut impl Write,
) -> Result<()> {
    check_interrupt(interrupted)?;
    let mut until = deadline(Instant::now(), args.timeout)?;
    client.send_command(args.slot, command, until)?;
    let mut acknowledged = false;
    while let Some(message) = receive(client, until, interrupted)? {
        if message.source != 0x10 + args.slot {
            continue;
        }
        match message.kind {
            0xc4 => {
                acknowledgement(&message.payload)?;
                if !acknowledged {
                    until = deadline(Instant::now(), args.timeout)?;
                    acknowledged = true;
                }
            }
            0x00 => format.write_text(output, printed_text(&message.payload)?)?,
            _ => {}
        }
    }
    if !acknowledged {
        return Err("timed out waiting for command acknowledgement".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_format_accepts_large_addresses_and_leading_zeroes() {
        assert_eq!(
            parse_register("Register 0x123456 = 0x0a\r\n").unwrap(),
            Some((0x123456, 10))
        );
        assert_eq!(
            parse_register("Register 0x00 = 0x00").unwrap(),
            Some((0, 0))
        );
        assert_eq!(parse_register("other diagnostic").unwrap(), None);
    }

    #[test]
    fn malformed_register_values_are_not_silently_converted() {
        for value in [
            "Register 0x20 = 0x100",
            "Register 0x20 = 0xzz",
            "Register 0x20 = 0x01 extra",
            "Register 0x20 0x01",
        ] {
            assert!(parse_register(value).is_err(), "{value}");
        }
    }

    #[test]
    fn print_payload_must_be_terminated_utf8() {
        assert_eq!(printed_text(b"hello\0").unwrap(), "hello");
        assert!(printed_text(b"hello").is_err());
        assert!(printed_text(b"a\0b\0").is_err());
        assert!(printed_text(&[0xff, 0]).is_err());
    }
}
