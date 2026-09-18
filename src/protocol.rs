use std::fmt;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const SYNC: [u8; 4] = [0xba, 0xd2, 0xac, 0xe5];
const MAX_PAYLOAD: usize = 8192;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Timeout(&'static str),
    Invalid(String),
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "network error: {error}"),
            Self::Timeout(operation) => write!(f, "{operation} timed out"),
            Self::Invalid(message) | Self::Refused(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Message {
    pub source: u8,
    pub kind: u8,
    pub payload: Vec<u8>,
}

pub struct Client {
    stream: TcpStream,
    buffered: Vec<u8>,
}

impl Client {
    pub fn connect(host: &str, port: u16, timeout: Duration, force: bool) -> Result<Self, Error> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Error::Invalid("connection timeout is too large".into()))?;
        let host = host.to_owned();
        let (sender, receiver) = mpsc::sync_channel(1);
        // Name resolution has no standard-library timeout. Bound the caller's wait.
        std::thread::spawn(move || {
            let addresses = (host.as_str(), port)
                .to_socket_addrs()
                .map(|addresses| addresses.collect::<Vec<_>>());
            let _ = sender.send(addresses);
        });
        let addresses = receiver
            .recv_timeout(remaining(deadline, "connection")?)
            .map_err(|_| Error::Timeout("name resolution"))??;
        let mut last_error = None;
        let mut stream = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, remaining(deadline, "connection")?) {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(error) => last_error = Some(error),
            }
        }
        let stream = stream.ok_or_else(|| {
            last_error.map(Error::Io).unwrap_or_else(|| {
                Error::Invalid("host did not resolve to a network address".into())
            })
        })?;
        stream.set_nodelay(true)?;
        let mut client = Self {
            stream,
            buffered: Vec::new(),
        };
        client.send(
            0x10,
            0x4a,
            &[0, 0xff, 0x03, 2, 0, u8::from(force)],
            deadline,
        )?;
        loop {
            let message = client
                .receive(deadline)?
                .ok_or(Error::Timeout("connection handshake"))?;
            if message.source != 0x10 || message.kind != 0xca {
                continue;
            }
            if message.payload.get(1..3) != Some(&[0xff, 0x03]) {
                continue;
            }
            validate_handshake(&message.payload)?;
            return Ok(client);
        }
    }

    pub fn send_command(
        &mut self,
        slot: u8,
        command: &str,
        deadline: Instant,
    ) -> Result<(), Error> {
        if slot > 45 {
            return Err(Error::Invalid("slot must be between 0 and 45".into()));
        }
        if command.is_empty() || command.contains('\0') {
            return Err(Error::Invalid(
                "command must be nonempty and contain no NUL bytes".into(),
            ));
        }
        if command.len() > 259 {
            return Err(Error::Invalid(
                "command exceeds the 259-byte text limit".into(),
            ));
        }
        let mut payload = command.as_bytes().to_vec();
        payload.push(0);
        self.send(0x10 + slot, 0x44, &payload, deadline)
    }

    fn send(
        &mut self,
        destination: u8,
        kind: u8,
        payload: &[u8],
        deadline: Instant,
    ) -> Result<(), Error> {
        let frame = encode(destination, kind, payload)?;
        let mut offset = 0;
        while offset < frame.len() {
            self.stream
                .set_write_timeout(Some(remaining(deadline, "send")?))?;
            match self.stream.write(&frame[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "connection stopped accepting data",
                    )
                    .into());
                }
                Ok(count) => offset += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if is_timeout(&error) => return Err(Error::Timeout("send")),
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    pub fn receive(&mut self, deadline: Instant) -> Result<Option<Message>, Error> {
        loop {
            let Some(time_left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
            else {
                return Ok(None);
            };
            if let Some(message) = decode(&mut self.buffered)? {
                if message.source == 0x10
                    && message.kind == 0xca
                    && message.payload.get(1..3) == Some(&[0xff, 0x03])
                    && let Err(error) = validate_handshake(&message.payload)
                {
                    let _ = self.stream.shutdown(std::net::Shutdown::Both);
                    return Err(error);
                }
                return Ok(Some(message));
            }
            self.stream.set_read_timeout(Some(time_left))?;
            let mut chunk = [0; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "connection closed before the next complete message",
                    )
                    .into());
                }
                Ok(count) => self.buffered.extend_from_slice(&chunk[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if is_timeout(&error) => return Ok(None),
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn remaining(deadline: Instant, operation: &'static str) -> Result<Duration, Error> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or(Error::Timeout(operation))
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    )
}

fn encode(destination: u8, kind: u8, payload: &[u8]) -> Result<Vec<u8>, Error> {
    if payload.len() > MAX_PAYLOAD {
        return Err(Error::Invalid(
            "message exceeds the 8192-byte payload limit".into(),
        ));
    }
    let mut frame = Vec::with_capacity(9 + payload.len());
    frame.extend_from_slice(&SYNC);
    frame.extend_from_slice(&[0, destination, kind]);
    frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn decode(buffer: &mut Vec<u8>) -> Result<Option<Message>, Error> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    if buffer[..4] != SYNC {
        return Err(Error::Invalid(
            "invalid message synchronization bytes".into(),
        ));
    }
    if buffer.len() < 9 {
        return Ok(None);
    }
    let length = u16::from_be_bytes([buffer[7], buffer[8]]) as usize;
    if length > MAX_PAYLOAD {
        return Err(Error::Invalid("received payload exceeds 8192 bytes".into()));
    }
    if buffer.len() < 9 + length {
        return Ok(None);
    }
    let message = Message {
        source: buffer[4],
        kind: buffer[6],
        payload: buffer[9..9 + length].to_vec(),
    };
    buffer.drain(..9 + length);
    Ok(Some(message))
}

fn validate_handshake(payload: &[u8]) -> Result<(), Error> {
    if payload.len() < 6 || payload[3] as usize != payload.len() - 4 {
        return Err(Error::Invalid(
            "malformed connection handshake response".into(),
        ));
    }
    if payload[0] != 0 {
        return Err(Error::Refused(format!(
            "connection handshake returned code {}",
            payload[0]
        )));
    }
    if u16::from_be_bytes([payload[4], payload[5]]) != 0 {
        return Ok(());
    }
    let reason = if payload.len() >= 10 {
        match u16::from_be_bytes([payload[8], payload[9]]) {
            0 => "no connection available",
            1 => "trusted authentication required",
            2 => "password rejected",
            _ => "unspecified reason",
        }
    } else {
        "unspecified reason"
    };
    Err(Error::Refused(format!("connection refused: {reason}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn peer_frame(source: u8, kind: u8, payload: &[u8]) -> Vec<u8> {
        let mut bytes = encode(1, kind, payload).unwrap();
        bytes[4] = source;
        bytes
    }

    fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut header = [0; 9];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(&header[..4], &SYNC);
        let length = u16::from_be_bytes([header[7], header[8]]) as usize;
        let mut result = header.to_vec();
        result.resize(9 + length, 0);
        stream.read_exact(&mut result[9..]).unwrap();
        result
    }

    #[test]
    fn fragmented_and_joined_frames_preserve_boundaries() {
        let first = peer_frame(0x12, 0, b"value\0");
        let second = peer_frame(0x12, 0xc4, &[0]);
        for split in 0..first.len() {
            let mut buffer = first[..split].to_vec();
            assert_eq!(decode(&mut buffer).unwrap(), None);
            buffer.extend_from_slice(&first[split..]);
            buffer.extend_from_slice(&second);
            assert_eq!(decode(&mut buffer).unwrap().unwrap().payload, b"value\0");
            assert_eq!(decode(&mut buffer).unwrap().unwrap().kind, 0xc4);
            assert!(buffer.is_empty());
        }
    }

    #[test]
    fn rejects_corrupt_sync_and_oversized_length() {
        assert!(decode(&mut vec![0, 0, 0, 0]).is_err());
        let mut frame = peer_frame(0x12, 0, &[]);
        frame[7..9].copy_from_slice(&8193_u16.to_be_bytes());
        assert!(decode(&mut frame).is_err());
    }

    #[test]
    fn handshake_and_command_use_expected_wire_bytes() {
        for force in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_frame(&mut stream);
                assert_eq!(
                    &request[4..],
                    &[0, 0x10, 0x4a, 0, 6, 0, 0xff, 3, 2, 0, u8::from(force)]
                );
                stream
                    .write_all(&peer_frame(0x10, 0xca, &[0, 0xff, 3, 2, 0, 1]))
                    .unwrap();
                let request = read_frame(&mut stream);
                assert_eq!(&request[4..7], &[0, 0x12, 0x44]);
                assert_eq!(&request[9..], b"sample 0x20\0");
                let mut response = peer_frame(0x12, 0, b"reply\0");
                response.extend_from_slice(&peer_frame(0x12, 0xc4, &[0]));
                stream.write_all(&response).unwrap();
            });
            let mut client =
                Client::connect("127.0.0.1", port, Duration::from_secs(2), force).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            client.send_command(2, "sample 0x20", deadline).unwrap();
            assert_eq!(client.receive(deadline).unwrap().unwrap().kind, 0);
            assert_eq!(client.receive(deadline).unwrap().unwrap().kind, 0xc4);
            server.join().unwrap();
        }
    }

    #[test]
    fn timeout_retains_partial_frame() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let mut client = Client {
            stream,
            buffered: Vec::new(),
        };
        let frame = peer_frame(0x12, 0, b"reply\0");
        peer.write_all(&frame[..6]).unwrap();
        assert!(
            client
                .receive(Instant::now() + Duration::from_millis(20))
                .unwrap()
                .is_none()
        );
        peer.write_all(&frame[6..]).unwrap();
        assert_eq!(
            client
                .receive(Instant::now() + Duration::from_secs(1))
                .unwrap()
                .unwrap()
                .payload,
            b"reply\0"
        );
    }

    #[test]
    fn refuses_takeover_and_reports_capacity_reason() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let mut client = Client {
            stream,
            buffered: Vec::new(),
        };
        peer.write_all(&peer_frame(0x10, 0xca, &[0, 0xff, 3, 6, 0, 0, 0, 0, 0, 0]))
            .unwrap();
        let error = client
            .receive(Instant::now() + Duration::from_secs(1))
            .unwrap_err();
        assert!(matches!(error, Error::Refused(_)));
        assert!(error.to_string().contains("no connection available"));
    }

    #[test]
    fn invalid_commands_are_rejected_before_sending() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        let mut client = Client {
            stream,
            buffered: Vec::new(),
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        assert!(client.send_command(46, "sample", deadline).is_err());
        assert!(client.send_command(2, "sample\0extra", deadline).is_err());
        assert!(client.send_command(2, &"x".repeat(260), deadline).is_err());
    }
}
