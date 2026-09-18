use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const MAGIC: [u8; 4] = [0xba, 0xd2, 0xac, 0xe5];

struct Frame {
    source: u8,
    destination: u8,
    kind: u8,
    payload: Vec<u8>,
}

fn read_frame(stream: &mut TcpStream) -> Frame {
    let mut header = [0; 9];
    stream.read_exact(&mut header).unwrap();
    assert_eq!(header[..4], MAGIC);
    let mut payload = vec![0; u16::from_be_bytes([header[7], header[8]]) as usize];
    stream.read_exact(&mut payload).unwrap();
    Frame {
        source: header[4],
        destination: header[5],
        kind: header[6],
        payload,
    }
}

fn send(stream: &mut TcpStream, source: u8, kind: u8, payload: &[u8]) {
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&[source, if kind == 0 { 1 } else { 0 }, kind]);
    bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    bytes.extend_from_slice(payload);
    stream.write_all(&bytes).unwrap();
}

fn handshake(stream: &mut TcpStream, force: bool) {
    let frame = read_frame(stream);
    assert_eq!(
        (frame.source, frame.destination, frame.kind),
        (0, 0x10, 0x4a)
    );
    assert_eq!(frame.payload, [0, 0xff, 3, 2, 0, u8::from(force)]);
    send(stream, 0x10, 0xca, &[0, 0xff, 3, 2, 0, 1]);
}

fn expect_command(stream: &mut TcpStream, expected: &str) {
    let frame = read_frame(stream);
    assert_eq!(
        (frame.source, frame.destination, frame.kind),
        (0, 0x13, 0x44)
    );
    assert_eq!(frame.payload, format!("{expected}\0").as_bytes());
}

fn expect_closed_without_another_command(stream: &mut TcpStream) {
    let mut byte = [0];
    match stream.read(&mut byte) {
        Ok(0) => {}
        Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => {}
        other => panic!("expected client to close without another command, got {other:?}"),
    }
}

fn run(args: &[&str], server: impl FnOnce(&mut TcpStream) + Send + 'static) -> Output {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    listener.set_nonblocking(true).unwrap();
    let peer = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "CLI never connected");
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        server(&mut stream);
    });
    let output = Command::new(env!("CARGO_BIN_EXE_register-cli"))
        .args(["--host", "127.0.0.1", "--port", &port, "--slot", "3"])
        .args(args)
        .output()
        .unwrap();
    peer.join().unwrap();
    output
}

fn assert_success(output: &Output, expected: &str) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
}

#[test]
fn read_routes_to_slot_filters_other_reports_and_retains_print_before_ack() {
    let output = run(&["reg", "read", "32"], |stream| {
        handshake(stream, true);
        expect_command(stream, "fpgarr 0x20");
        send(stream, 0x14, 0, b"Register 0x20 = 0xff\0");
        send(stream, 0x13, 0, b"Register 0x21 = 0xff\0");
        send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
        send(stream, 0x13, 0xc4, &[0]);
        expect_closed_without_another_command(stream);
    });
    assert_success(&output, "Address 0x20=0x2a\n");
}

#[test]
fn no_force_and_output_formats_work_through_the_binary() {
    for (flags, expected) in [
        (
            vec!["--json"],
            "{\"address\":\"0x20\",\"value\":\"0x2a\"}\n",
        ),
        (
            vec!["--json", "--decimal-output"],
            "{\"address\":32,\"value\":42}\n",
        ),
        (vec!["--decimal-output"], "Address 32=42\n"),
    ] {
        let mut args = vec!["--no-force", "reg", "read", "0x20"];
        args.extend(flags);
        let output = run(&args, |stream| {
            handshake(stream, false);
            expect_command(stream, "fpgarr 0x20");
            send(stream, 0x13, 0xc4, &[0]);
            send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
            expect_closed_without_another_command(stream);
        });
        assert_success(&output, expected);
    }
}

#[test]
fn writes_require_matching_readback_and_never_retry() {
    for (reported, succeeds) in [(42, true), (41, false)] {
        let output = run(&["reg", "write", "0x20", "42"], move |stream| {
            handshake(stream, true);
            expect_command(stream, "fpgarw 0x20 0x2a");
            send(stream, 0x13, 0xc4, &[0]);
            send(
                stream,
                0x13,
                0,
                format!("Register 0x20 = 0x{reported:x}\0").as_bytes(),
            );
            expect_closed_without_another_command(stream);
        });
        if succeeds {
            assert_success(&output, "Address 0x20=0x2a\n");
        } else {
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
            let error = String::from_utf8(output.stderr).unwrap();
            assert!(error.contains("readback mismatch"));
            for value in ["0x20", "0x2a", "0x29"] {
                assert!(error.contains(value));
            }
        }
    }
}

#[test]
fn generic_register_commands_preserve_text_and_collect_for_the_window() {
    for text in ["fpgarr 0x20", "fpgarw 0x20 0xff"] {
        let output = run(&["command", text, "--timeout", "250ms"], move |stream| {
            handshake(stream, true);
            expect_command(stream, text);
            send(stream, 0x14, 0, b"unrelated\0");
            send(stream, 0x13, 0, b"before acknowledgement\0");
            send(stream, 0x13, 0xc4, &[0]);
            let acknowledged = Instant::now();
            thread::sleep(Duration::from_millis(100));
            send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
            send(stream, 0x13, 0, b"already terminated\n\0");
            expect_closed_without_another_command(stream);
            assert!(acknowledged.elapsed() >= Duration::from_millis(230));
        });
        assert_success(
            &output,
            "before acknowledgement\nRegister 0x20 = 0x2a\nalready terminated\n",
        );
    }
}

#[test]
fn missing_acknowledgement_and_missing_register_are_errors() {
    for acknowledge in [false, true] {
        let output = run(
            &["reg", "read", "0x20", "--timeout", "150ms"],
            move |stream| {
                handshake(stream, true);
                expect_command(stream, "fpgarr 0x20");
                if acknowledge {
                    send(stream, 0x13, 0xc4, &[0]);
                } else {
                    send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
                }
                expect_closed_without_another_command(stream);
            },
        );
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains(if acknowledge {
            "waiting for register"
        } else {
            "waiting for command acknowledgement"
        }));
    }
}

#[test]
fn polling_reuses_connection_spaces_requests_and_stops_on_disconnect() {
    let output = run(
        &["reg", "read", "0x20", "--poll", "150ms", "--timeout", "1s"],
        |stream| {
            handshake(stream, true);
            let mut previous = None;
            for index in 0..3 {
                expect_command(stream, "fpgarr 0x20");
                let now = Instant::now();
                if let Some(previous) = previous {
                    assert!(now.duration_since(previous) >= Duration::from_millis(130));
                }
                previous = Some(now);
                if index == 2 {
                    break;
                }
                send(stream, 0x13, 0xc4, &[0]);
                send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
            }
        },
    );
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"Address 0x20=0x2a\nAddress 0x20=0x2a\n");
    assert!(String::from_utf8_lossy(&output.stderr).contains("connection"));
}

#[test]
fn refusal_and_command_rejection_are_nonzero() {
    let refused = run(&["--no-force", "reg", "read", "0"], |stream| {
        let frame = read_frame(stream);
        assert_eq!(frame.payload, [0, 0xff, 3, 2, 0, 0]);
        send(stream, 0x10, 0xca, &[0, 0xff, 3, 2, 0, 0]);
        expect_closed_without_another_command(stream);
    });
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("refused"));
    let rejected = run(&["reg", "read", "0"], |stream| {
        handshake(stream, true);
        expect_command(stream, "fpgarr 0x0");
        send(stream, 0x13, 0xc4, &[1]);
        expect_closed_without_another_command(stream);
    });
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("does not support"));
}

#[test]
fn generic_command_without_acknowledgement_fails_after_printing_received_text() {
    let output = run(
        &["command", "fpgarr 0x20", "--timeout", "150ms", "--json"],
        |stream| {
            handshake(stream, true);
            expect_command(stream, "fpgarr 0x20");
            send(stream, 0x13, 0, b"Register 0x20 = 0x2a\0");
            expect_closed_without_another_command(stream);
        },
    );
    assert!(!output.status.success());
    assert_eq!(output.stdout, b"{\"text\":\"Register 0x20 = 0x2a\"}\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("waiting for command acknowledgement")
    );
}
