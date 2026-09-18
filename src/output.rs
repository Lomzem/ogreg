use std::io::{self, Write};

use time::{OffsetDateTime, UtcOffset, macros::format_description};

#[derive(Debug, Clone, Copy)]
pub struct OutputFormat {
    pub json: bool,
    pub decimal_output: bool,
}

impl OutputFormat {
    pub fn write_register(
        &self,
        writer: &mut impl Write,
        address: u32,
        value: u8,
    ) -> io::Result<()> {
        self.write_register_record(writer, address, value, None)
    }

    pub fn write_polled_register(
        &self,
        writer: &mut impl Write,
        address: u32,
        value: u8,
        timestamp: OffsetDateTime,
    ) -> io::Result<()> {
        let timestamp = timestamp
            .to_offset(UtcOffset::UTC)
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
            ))
            .map_err(io::Error::other)?;
        self.write_register_record(writer, address, value, Some(&timestamp))
    }

    pub fn write_text(&self, writer: &mut impl Write, text: &str) -> io::Result<()> {
        write_text(writer, text, self.json)
    }
    fn write_register_record(
        &self,
        writer: &mut impl Write,
        address: u32,
        value: u8,
        timestamp: Option<&str>,
    ) -> io::Result<()> {
        if self.json {
            let mut result = if self.decimal_output {
                serde_json::json!({ "address": format!("0x{address:x}"), "value": value })
            } else {
                serde_json::json!({ "address": format!("0x{address:x}"), "value": format!("0x{value:x}") })
            };
            if let Some(timestamp) = timestamp {
                result["timestamp"] = timestamp.into();
            }
            serde_json::to_writer(&mut *writer, &result)?;
            writeln!(writer)?;
        } else {
            if let Some(timestamp) = timestamp {
                write!(writer, "{timestamp} ")?;
            }
            if self.decimal_output {
                writeln!(writer, "Address 0x{address:x}={value}")?;
            } else {
                writeln!(writer, "Address 0x{address:x}=0x{value:x}")?;
            }
        }
        writer.flush()
    }
}

fn write_text(writer: &mut impl Write, text: &str, json: bool) -> io::Result<()> {
    if json {
        serde_json::to_writer(&mut *writer, &serde_json::json!({ "text": text }))?;
        writeln!(writer)?;
    } else {
        writer.write_all(text.as_bytes())?;
        if !text.ends_with('\n') {
            writeln!(writer)?;
        }
    }
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_formats_match_the_public_contract() {
        for (json, decimal, expected) in [
            (false, false, "Address 0x20=0x2a\n"),
            (false, true, "Address 0x20=42\n"),
            (true, false, "{\"address\":\"0x20\",\"value\":\"0x2a\"}\n"),
            (true, true, "{\"address\":\"0x20\",\"value\":42}\n"),
        ] {
            let mut bytes = Vec::new();
            OutputFormat {
                json,
                decimal_output: decimal,
            }
            .write_register(&mut bytes, 32, 42)
            .unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), expected);
        }
    }

    #[test]
    fn polling_timestamps_are_utc_with_fixed_milliseconds_in_every_format() {
        let timestamp = time::macros::datetime!(2026-09-17 19:30:00.123456789 -07:00);
        for decimal_output in [false, true] {
            for json in [false, true] {
                let mut bytes = Vec::new();
                OutputFormat {
                    json,
                    decimal_output,
                }
                .write_polled_register(&mut bytes, 32, 42, timestamp)
                .unwrap();
                assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 1);
                assert!(bytes.ends_with(b"\n"));
                if json {
                    let record: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(record["timestamp"], "2026-09-18T02:30:00.123Z");
                    assert_eq!(record["address"], "0x20");
                    assert_eq!(
                        record["value"],
                        if decimal_output {
                            serde_json::json!(42)
                        } else {
                            serde_json::json!("0x2a")
                        }
                    );
                } else {
                    let value = if decimal_output { "42" } else { "0x2a" };
                    assert_eq!(
                        String::from_utf8(bytes).unwrap(),
                        format!("2026-09-18T02:30:00.123Z Address 0x20={value}\n")
                    );
                }
            }
        }
    }

    #[test]
    fn polling_timestamp_keeps_zero_milliseconds() {
        let mut bytes = Vec::new();
        OutputFormat {
            json: false,
            decimal_output: false,
        }
        .write_polled_register(
            &mut bytes,
            32,
            42,
            time::macros::datetime!(2026-09-18 02:30 UTC),
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "2026-09-18T02:30:00.000Z Address 0x20=0x2a\n"
        );
    }

    #[test]
    fn raw_output_adds_only_a_missing_final_newline() {
        for text in [
            "Register 0x20 = 0x2a",
            "line one\nline two",
            "line\n",
            "line\r\n",
            "line\r",
            "",
        ] {
            let mut bytes = Vec::new();
            write_text(&mut bytes, text, false).unwrap();
            let expected = if text.ends_with('\n') {
                text.to_owned()
            } else {
                format!("{text}\n")
            };
            assert_eq!(bytes, expected.as_bytes());
        }
    }

    #[test]
    fn json_preserves_and_escapes_original_text() {
        let text = "Register 0x20 = 0x2a\r\n\"done\"";
        let mut bytes = Vec::new();
        write_text(&mut bytes, text, true).unwrap();
        assert_eq!(bytes.iter().filter(|&&byte| byte == b'\n').count(), 1);
        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(result["text"], text);
    }

    #[test]
    fn every_result_flushes_for_polling_consumers() {
        #[derive(Default)]
        struct Sink {
            flushes: usize,
        }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }
        let mut sink = Sink::default();
        for json in [false, true] {
            let format = OutputFormat {
                json,
                decimal_output: false,
            };
            format.write_register(&mut sink, 0, 0).unwrap();
            format
                .write_polled_register(
                    &mut sink,
                    0,
                    0,
                    time::macros::datetime!(2026-09-18 02:30 UTC),
                )
                .unwrap();
        }
        write_text(&mut sink, "partial", false).unwrap();
        assert_eq!(sink.flushes, 5);
    }
}
