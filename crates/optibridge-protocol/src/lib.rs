#![cfg_attr(not(feature = "std"), no_std)]

pub const MAGIC: u8 = 0xa5;
pub const HEADER_LEN: usize = 5;
pub const MAX_PAYLOAD: usize = 16;
pub const MAX_FRAME: usize = HEADER_LEN + MAX_PAYLOAD;

pub const CMD_ALIVE: u8 = 0x01;
pub const CMD_I2C_WRITE: u8 = 0x10;
pub const CMD_I2C_READ: u8 = 0x11;
pub const STATUS_OK: u8 = 0x00;
pub const STATUS_BAD_COMMAND: u8 = 0x01;
pub const STATUS_BAD_LENGTH: u8 = 0x02;
pub const STATUS_BAD_FLAGS: u8 = 0x03;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Request {
    pub command: u8,
    pub sequence: u8,
    pub flags: u8,
    pub payload: [u8; MAX_PAYLOAD],
    pub payload_len: u8,
}

impl Request {
    pub const fn empty() -> Self {
        Self {
            command: 0,
            sequence: 0,
            flags: 0,
            payload: [0; MAX_PAYLOAD],
            payload_len: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    BadMagic,
    FrameTooLong,
    BadLength,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Parser {
    frame: [u8; MAX_FRAME],
    length: usize,
}

impl Parser {
    pub const fn new() -> Self {
        Self {
            frame: [0; MAX_FRAME],
            length: 0,
        }
    }

    pub fn reset(&mut self) {
        self.length = 0;
    }

    pub fn push(&mut self, byte: u8) -> Result<Option<Request>, ParseError> {
        if self.length == 0 && byte != MAGIC {
            return Err(ParseError::BadMagic);
        }
        if self.length == MAX_FRAME {
            self.reset();
            return Err(ParseError::FrameTooLong);
        }

        self.frame[self.length] = byte;
        self.length += 1;

        if self.length == HEADER_LEN {
            let payload_len = self.frame[2] as usize;
            if payload_len > MAX_PAYLOAD {
                self.reset();
                return Err(ParseError::BadLength);
            }
        }

        if self.length >= HEADER_LEN && self.length == HEADER_LEN + self.frame[2] as usize {
            let mut payload = [0; MAX_PAYLOAD];
            let payload_len = self.frame[2] as usize;
            payload[..payload_len].copy_from_slice(&self.frame[HEADER_LEN..self.length]);
            let request = Request {
                command: self.frame[1],
                sequence: self.frame[3],
                flags: self.frame[4],
                payload,
                payload_len: payload_len as u8,
            };
            self.reset();
            return Ok(Some(request));
        }

        Ok(None)
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

pub fn encode_response(
    status: u8,
    sequence: u8,
    flags: u8,
    payload: &[u8],
    output: &mut [u8; MAX_FRAME],
) -> Result<usize, ParseError> {
    if payload.len() > MAX_PAYLOAD {
        return Err(ParseError::BadLength);
    }
    output[0] = MAGIC;
    output[1] = status;
    output[2] = payload.len() as u8;
    output[3] = sequence;
    output[4] = flags;
    output[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
    Ok(HEADER_LEN + payload.len())
}

pub fn validate_i2c_request(request: &Request) -> Result<(), u8> {
    if request.flags != 0 {
        return Err(STATUS_BAD_FLAGS);
    }
    if request.command != CMD_I2C_WRITE && request.command != CMD_I2C_READ {
        return Err(STATUS_BAD_COMMAND);
    }
    let payload_len = request.payload_len as usize;
    if payload_len == 0 || payload_len > MAX_PAYLOAD {
        return Err(STATUS_BAD_LENGTH);
    }
    if request.payload[0] & 0x80 != 0 {
        return Err(STATUS_BAD_COMMAND);
    }
    if request.command == CMD_I2C_READ
        && (payload_len != 2 || request.payload[1] as usize > MAX_PAYLOAD)
    {
        return Err(STATUS_BAD_LENGTH);
    }
    Ok(())
}

pub fn dispatch(request: &Request, output: &mut [u8; MAX_FRAME]) -> usize {
    if request.flags != 0 {
        return encode_response(STATUS_BAD_FLAGS, request.sequence, 0, &[], output)
            .unwrap_or(HEADER_LEN);
    }
    if request.command != CMD_ALIVE || request.payload_len != 0 {
        return encode_response(
            if request.command == CMD_ALIVE {
                STATUS_BAD_LENGTH
            } else {
                STATUS_BAD_COMMAND
            },
            request.sequence,
            0,
            &[],
            output,
        )
        .unwrap_or(HEADER_LEN);
    }

    encode_response(STATUS_OK, request.sequence, 0, b"alive", output).unwrap_or(HEADER_LEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_bytes(sequence: u8, payload: &[u8]) -> [u8; MAX_FRAME] {
        let mut bytes = [0; MAX_FRAME];
        bytes[0] = MAGIC;
        bytes[1] = CMD_ALIVE;
        bytes[2] = payload.len() as u8;
        bytes[3] = sequence;
        bytes[4] = 0;
        bytes[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
        bytes
    }

    #[test]
    fn parses_alive_request() {
        let bytes = request_bytes(7, &[]);
        let mut parser = Parser::new();
        let mut result = None;
        for byte in bytes.iter().take(HEADER_LEN) {
            result = parser.push(*byte).unwrap();
        }
        assert_eq!(
            result,
            Some(Request {
                command: CMD_ALIVE,
                sequence: 7,
                flags: 0,
                payload: [0; MAX_PAYLOAD],
                payload_len: 0,
            })
        );
    }

    #[test]
    fn rejects_bad_magic_and_recovers() {
        let mut parser = Parser::new();
        assert_eq!(parser.push(0), Err(ParseError::BadMagic));
        let bytes = request_bytes(1, &[]);
        for byte in bytes.iter().take(HEADER_LEN - 1) {
            assert_eq!(parser.push(*byte), Ok(None));
        }
        assert!(parser.push(bytes[HEADER_LEN - 1]).unwrap().is_some());
    }

    #[test]
    fn parses_frame_fragmented_at_each_byte_boundary() {
        let bytes = request_bytes(3, &[1, 2, 3]);
        let mut parser = Parser::new();

        for byte in bytes.iter().take(HEADER_LEN + 3 - 1) {
            assert_eq!(parser.push(*byte), Ok(None));
        }

        assert_eq!(
            parser.push(bytes[HEADER_LEN + 3 - 1]),
            Ok(Some(Request {
                command: CMD_ALIVE,
                sequence: 3,
                flags: 0,
                payload: [1, 2, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                payload_len: 3,
            }))
        );
    }

    #[test]
    fn parses_coalesced_maximum_length_frames_in_order() {
        let mut bytes = [0; MAX_FRAME * 3];
        for (index, frame) in bytes.chunks_exact_mut(MAX_FRAME).enumerate() {
            frame[0] = MAGIC;
            frame[1] = CMD_ALIVE;
            frame[2] = MAX_PAYLOAD as u8;
            frame[3] = index as u8 + 1;
            for payload in &mut frame[HEADER_LEN..] {
                *payload = index as u8;
            }
        }

        let mut parser = Parser::new();
        let mut sequences = [0; 3];
        let mut count = 0;
        for byte in bytes {
            if let Some(request) = parser.push(byte).unwrap() {
                sequences[count] = request.sequence;
                count += 1;
            }
        }

        assert_eq!(count, 3);
        assert_eq!(sequences, [1, 2, 3]);
    }

    #[test]
    fn recovers_from_invalid_length_within_same_stream() {
        let mut parser = Parser::new();
        let malformed = [MAGIC, CMD_ALIVE, MAX_PAYLOAD as u8 + 1, 9, 0];
        for byte in &malformed[..HEADER_LEN - 1] {
            assert_eq!(parser.push(*byte), Ok(None));
        }
        assert_eq!(
            parser.push(malformed[HEADER_LEN - 1]),
            Err(ParseError::BadLength)
        );

        let bytes = request_bytes(4, &[]);
        let mut result = None;
        for byte in bytes.iter().take(HEADER_LEN) {
            result = parser.push(*byte).unwrap();
        }
        assert_eq!(result.map(|request| request.sequence), Some(4));
    }

    #[test]
    fn validates_i2c_request_statuses() {
        let mut request = Request::empty();
        request.command = CMD_I2C_WRITE;
        request.payload_len = 1;
        request.payload[0] = 0x42;
        assert_eq!(validate_i2c_request(&request), Ok(()));

        request.flags = 1;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_FLAGS));
        request.flags = 0;

        request.command = CMD_ALIVE;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_COMMAND));
        request.command = CMD_I2C_WRITE;

        request.payload_len = 0;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_LENGTH));
        request.payload_len = 1;

        request.payload_len = MAX_PAYLOAD as u8 + 1;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_LENGTH));
        request.payload_len = 1;

        request.payload[0] = 0x80;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_COMMAND));
        request.payload[0] = 0x42;

        request.command = CMD_I2C_READ;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_LENGTH));
        request.payload_len = 2;
        request.payload[1] = MAX_PAYLOAD as u8 + 1;
        assert_eq!(validate_i2c_request(&request), Err(STATUS_BAD_LENGTH));
        request.payload[1] = MAX_PAYLOAD as u8;
        assert_eq!(validate_i2c_request(&request), Ok(()));
    }

    #[test]
    fn dispatches_alive_response() {
        let request = Request {
            command: CMD_ALIVE,
            sequence: 9,
            flags: 0,
            payload: [0; MAX_PAYLOAD],
            payload_len: 0,
        };
        let mut output = [0; MAX_FRAME];
        let length = dispatch(&request, &mut output);
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_OK, 5, 9, 0, b'a', b'l', b'i', b'v', b'e']
        );
    }

    #[test]
    fn dispatches_alive_request_errors() {
        let mut request = Request::empty();
        request.command = CMD_ALIVE;
        request.sequence = 1;

        request.flags = 1;
        let mut output = [0; MAX_FRAME];
        let length = dispatch(&request, &mut output);
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_FLAGS, 0, request.sequence, 0]
        );

        request.flags = 0;
        request.command = 0x02;
        let length = dispatch(&request, &mut output);
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_COMMAND, 0, request.sequence, 0]
        );

        request.command = CMD_ALIVE;
        request.payload_len = 1;
        let length = dispatch(&request, &mut output);
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_LENGTH, 0, request.sequence, 0]
        );
    }
}
