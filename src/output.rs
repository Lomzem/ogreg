use std::io::{self, Write};

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
        write_register(writer, address, value, self.json, self.decimal_output)
    }

    pub fn write_text(&self, writer: &mut impl Write, text: &str) -> io::Result<()> {
        write_text(writer, text, self.json)
    }
}

fn write_register(
    writer: &mut impl Write,
    address: u32,
    value: u8,
    json: bool,
    decimal: bool,
) -> io::Result<()> {
    if json {
        let result = if decimal {
            serde_json::json!({ "address": address, "value": value })
        } else {
            serde_json::json!({ "address": format!("0x{address:x}"), "value": format!("0x{value:x}") })
        };
        serde_json::to_writer(&mut *writer, &result)?;
        writeln!(writer)?;
    } else if decimal {
        writeln!(writer, "Address {address}={value}")?;
    } else {
        writeln!(writer, "Address 0x{address:x}=0x{value:x}")?;
    }
    writer.flush()
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
            (false, true, "Address 32=42\n"),
            (true, false, "{\"address\":\"0x20\",\"value\":\"0x2a\"}\n"),
            (true, true, "{\"address\":32,\"value\":42}\n"),
        ] {
            let mut bytes = Vec::new();
            write_register(&mut bytes, 32, 42, json, decimal).unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), expected);
        }
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
        write_register(&mut sink, 0, 0, true, false).unwrap();
        write_register(&mut sink, 0, 0, false, false).unwrap();
        write_text(&mut sink, "partial", false).unwrap();
        assert_eq!(sink.flushes, 3);
    }
}
