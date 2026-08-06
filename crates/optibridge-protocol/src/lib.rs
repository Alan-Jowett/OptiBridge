#![cfg_attr(not(feature = "std"), no_std)]

pub const MAGIC: u8 = 0xa5;
pub const HEADER_LEN: usize = 5;
pub const MAX_PAYLOAD: usize = 16;
pub const MAX_FRAME: usize = HEADER_LEN + MAX_PAYLOAD;

pub const CMD_RESET: u8 = 0x01;
pub const CMD_LOAD_BPF: u8 = 0x02;
pub const CMD_START_BPF: u8 = 0x03;
pub const CMD_READ_BPF_MAP: u8 = 0x04;
pub const CMD_WRITE_BPF_MAP: u8 = 0x05;
pub const CMD_READ_STATUS: u8 = 0x06;
pub const CMD_QUERY_BPF_CRC: u8 = 0x07;
pub const CMD_I2C_WRITE: u8 = 0x10;
pub const CMD_I2C_READ: u8 = 0x11;
pub const STATUS_OK: u8 = 0x00;
pub const STATUS_BAD_COMMAND: u8 = 0x01;
pub const STATUS_BAD_LENGTH: u8 = 0x02;
pub const STATUS_BAD_FLAGS: u8 = 0x03;
pub const STATUS_NOT_IMPLEMENTED: u8 = 0x04;
pub const STATUS_BUSY: u8 = 0x05;
pub const STATUS_BAD_STATE: u8 = 0x06;
pub const STATUS_BAD_CRC: u8 = 0x07;
pub const STATUS_FLASH_ERROR: u8 = 0x08;
pub const STATUS_NO_PROGRAM: u8 = 0x09;

pub const BPF_FLASH_OFFSET: u32 = 0x6000;
pub const BPF_FLASH_SIZE: usize = 8192;
pub const BPF_HEADER_SIZE: usize = 16;
pub const BPF_MAX_BYTECODE: usize = 7680;
pub const BPF_MAX_MAPS: usize = 8;
pub const BPF_MAP_DESCRIPTOR_SIZE: usize = 16;
pub const BPF_MAX_MAP_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadOperation {
    Begin {
        bytecode_len: u16,
        map_count: u8,
        expected_crc: u32,
    },
    Data {
        offset: u16,
        bytes: [u8; 12],
        len: u8,
    },
    Finalize,
}

pub trait BpfFlash {
    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), ()>;
    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()>;
    fn erase(&mut self, from: u32, to: u32) -> Result<(), ()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoaderState {
    Empty,
    Receiving,
    Committed(u32),
    Failed(u8),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MapLayout {
    offset: u16,
    len: u16,
}

impl MapLayout {
    const EMPTY: Self = Self { offset: 0, len: 0 };
}

/// Fixed-capacity BPF image loader state. Flash operations are performed only
/// by `execute_pending`, allowing the I2C callback to remain bounded.
pub struct BpfLoader {
    state: LoaderState,
    pending: Option<LoadOperation>,
    bytecode_len: u16,
    map_count: u8,
    image_len: u16,
    next_offset: u16,
    expected_crc: u32,
    crc: u32,
    map_layouts: [MapLayout; BPF_MAX_MAPS],
    committed_map_count: u8,
}

impl BpfLoader {
    pub const fn new() -> Self {
        Self {
            state: LoaderState::Empty,
            pending: None,
            bytecode_len: 0,
            map_count: 0,
            image_len: 0,
            next_offset: 0,
            expected_crc: 0,
            crc: 0xffff_ffff,
            map_layouts: [MapLayout::EMPTY; BPF_MAX_MAPS],
            committed_map_count: 0,
        }
    }

    pub fn accepts_load(&mut self, request: &Request) -> Result<(), u8> {
        if self.pending.is_some() {
            return Err(STATUS_BUSY);
        }
        let payload = &request.payload[..request.payload_len as usize];
        let operation = match payload.first().copied() {
            Some(0) if payload.len() == 8 => {
                let bytecode_len = u16::from_le_bytes([payload[1], payload[2]]);
                let map_count = payload[3];
                let expected_crc =
                    u32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                let image_len = image_len(bytecode_len, map_count).ok_or(STATUS_BAD_LENGTH)?;
                if bytecode_len == 0
                    || bytecode_len as usize > BPF_MAX_BYTECODE
                    || bytecode_len as usize % 8 != 0
                    || map_count as usize > BPF_MAX_MAPS
                    || image_len + BPF_HEADER_SIZE > BPF_FLASH_SIZE
                {
                    return Err(STATUS_BAD_LENGTH);
                }
                LoadOperation::Begin {
                    bytecode_len,
                    map_count,
                    expected_crc,
                }
            }
            Some(1)
                if (5..=15).contains(&payload.len()) && (payload.len() - 3).is_multiple_of(2) =>
            {
                if !matches!(self.state, LoaderState::Receiving) {
                    return Err(STATUS_BAD_STATE);
                }
                let offset = u16::from_le_bytes([payload[1], payload[2]]);
                if offset != self.next_offset {
                    return Err(STATUS_BAD_STATE);
                }
                let len = payload.len() - 3;
                if offset as usize + len > self.image_len as usize {
                    return Err(STATUS_BAD_LENGTH);
                }
                let mut bytes = [0; 12];
                bytes[..len].copy_from_slice(&payload[3..]);
                LoadOperation::Data {
                    offset,
                    bytes,
                    len: len as u8,
                }
            }
            Some(2) if payload.len() == 1 => {
                if !matches!(self.state, LoaderState::Receiving) {
                    return Err(STATUS_BAD_STATE);
                }
                if self.next_offset != self.image_len {
                    return Err(STATUS_BAD_STATE);
                }
                LoadOperation::Finalize
            }
            Some(0..=2) => return Err(STATUS_BAD_LENGTH),
            _ => return Err(STATUS_BAD_COMMAND),
        };
        self.pending = Some(operation);
        Ok(())
    }

    pub fn query(&self) -> Result<u32, u8> {
        if self.pending.is_some() {
            return Err(STATUS_BUSY);
        }
        match self.state {
            LoaderState::Committed(crc) => Ok(crc),
            LoaderState::Failed(status) => Err(status),
            LoaderState::Empty | LoaderState::Receiving => Err(STATUS_NO_PROGRAM),
        }
    }

    pub const fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    pub fn read_map<'a>(
        &self,
        map_id: u8,
        byte_offset: u16,
        byte_length: u8,
        backing: &'a [u8; BPF_MAX_MAP_BYTES],
    ) -> Result<&'a [u8], u8> {
        if byte_length == 0 || byte_length as usize > MAX_PAYLOAD {
            return Err(STATUS_BAD_LENGTH);
        }
        if !matches!(self.state, LoaderState::Committed(_)) {
            return Err(STATUS_NO_PROGRAM);
        }
        let Some(layout) = self.map_layouts.get(map_id as usize) else {
            return Err(STATUS_BAD_COMMAND);
        };
        if map_id >= self.committed_map_count {
            return Err(STATUS_BAD_COMMAND);
        }
        let Some(map_end) = byte_offset.checked_add(byte_length as u16) else {
            return Err(STATUS_BAD_LENGTH);
        };
        if map_end > layout.len {
            return Err(STATUS_BAD_LENGTH);
        }
        let start = layout.offset as usize + byte_offset as usize;
        let end = start + byte_length as usize;
        backing.get(start..end).ok_or(STATUS_BAD_LENGTH)
    }

    pub fn execute_pending<F: BpfFlash>(&mut self, flash: &mut F) {
        let Some(operation) = self.pending.take() else {
            return;
        };
        match operation {
            LoadOperation::Begin {
                bytecode_len,
                map_count,
                expected_crc,
            } => self.begin(flash, bytecode_len, map_count, expected_crc),
            LoadOperation::Data { offset, bytes, len } => {
                if flash
                    .write(
                        BPF_FLASH_OFFSET + BPF_HEADER_SIZE as u32 + offset as u32,
                        &bytes[..len as usize],
                    )
                    .is_err()
                {
                    self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
                    return;
                }
                self.crc = crc32_update(self.crc, &bytes[..len as usize]);
                self.next_offset += len as u16;
            }
            LoadOperation::Finalize => self.finalize(flash),
        }
    }

    pub fn validate_committed<F: BpfFlash>(&mut self, flash: &mut F) {
        self.clear_map_layouts();
        let mut header = [0; BPF_HEADER_SIZE];
        if flash.read(BPF_FLASH_OFFSET, &mut header).is_err() {
            self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
            return;
        }
        if header[..4] != *b"OBPF"
            || header[4] != 1
            || u16::from_le_bytes([header[12], header[13]]) != 0
            || header[14..] != [0xff, 0xff]
        {
            self.state = LoaderState::Empty;
            return;
        }
        let bytecode_len = u16::from_le_bytes([header[6], header[7]]);
        let map_count = header[5];
        let Some(image_len) = image_len(bytecode_len, map_count) else {
            self.state = LoaderState::Empty;
            return;
        };
        if bytecode_len == 0
            || bytecode_len as usize > BPF_MAX_BYTECODE
            || bytecode_len as usize % 8 != 0
            || map_count as usize > BPF_MAX_MAPS
            || image_len + BPF_HEADER_SIZE > BPF_FLASH_SIZE
        {
            self.state = LoaderState::Empty;
            return;
        }
        let expected_crc = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        let mut crc = 0xffff_ffff;
        let mut offset = 0usize;
        let mut chunk = [0; MAX_PAYLOAD];
        while offset < image_len {
            let length = core::cmp::min(chunk.len(), image_len - offset);
            if flash
                .read(
                    BPF_FLASH_OFFSET + BPF_HEADER_SIZE as u32 + offset as u32,
                    &mut chunk[..length],
                )
                .is_err()
            {
                self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
                return;
            }
            crc = crc32_update(crc, &chunk[..length]);
            offset += length;
        }
        let Some((map_layouts, committed_map_count)) = map_layouts(flash, bytecode_len, map_count)
        else {
            self.state = LoaderState::Empty;
            return;
        };
        if crc32_finish(crc) != expected_crc {
            self.state = LoaderState::Empty;
            return;
        }
        self.state = LoaderState::Committed(expected_crc);
        self.map_layouts = map_layouts;
        self.committed_map_count = committed_map_count;
    }

    fn begin<F: BpfFlash>(
        &mut self,
        flash: &mut F,
        bytecode_len: u16,
        map_count: u8,
        expected_crc: u32,
    ) {
        self.clear_map_layouts();
        let Some(image_len) = image_len(bytecode_len, map_count) else {
            self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
            return;
        };
        let pages = (BPF_HEADER_SIZE + image_len).div_ceil(4096);
        if flash
            .erase(BPF_FLASH_OFFSET, BPF_FLASH_OFFSET + (pages * 4096) as u32)
            .is_err()
        {
            self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
            return;
        }
        let mut header = [0xff; BPF_HEADER_SIZE];
        header[..4].copy_from_slice(b"OBPF");
        header[4] = 1;
        header[5] = map_count;
        header[6..8].copy_from_slice(&bytecode_len.to_le_bytes());
        header[8..12].copy_from_slice(&expected_crc.to_le_bytes());
        if flash.write(BPF_FLASH_OFFSET, &header).is_err() {
            self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
            return;
        }
        self.state = LoaderState::Receiving;
        self.bytecode_len = bytecode_len;
        self.map_count = map_count;
        self.image_len = image_len as u16;
        self.next_offset = 0;
        self.expected_crc = expected_crc;
        self.crc = 0xffff_ffff;
    }

    fn finalize<F: BpfFlash>(&mut self, flash: &mut F) {
        if crc32_finish(self.crc) != self.expected_crc {
            self.state = LoaderState::Failed(STATUS_BAD_CRC);
            return;
        }
        let Some((map_layouts, committed_map_count)) =
            map_layouts(flash, self.bytecode_len, self.map_count)
        else {
            self.state = LoaderState::Failed(STATUS_BAD_STATE);
            return;
        };
        if flash.write(BPF_FLASH_OFFSET + 12, &[0, 0]).is_err() {
            self.state = LoaderState::Failed(STATUS_FLASH_ERROR);
            return;
        }
        self.state = LoaderState::Committed(self.expected_crc);
        self.map_layouts = map_layouts;
        self.committed_map_count = committed_map_count;
    }

    fn clear_map_layouts(&mut self) {
        self.map_layouts = [MapLayout::EMPTY; BPF_MAX_MAPS];
        self.committed_map_count = 0;
    }
}

impl Default for BpfLoader {
    fn default() -> Self {
        Self::new()
    }
}

pub const fn crc32_iso_hdlc(bytes: &[u8]) -> u32 {
    crc32_finish(crc32_update(0xffff_ffff, bytes))
}

const fn crc32_update(mut crc: u32, bytes: &[u8]) -> u32 {
    let mut index = 0;
    while index < bytes.len() {
        crc ^= bytes[index] as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        index += 1;
    }
    crc
}

const fn crc32_finish(crc: u32) -> u32 {
    crc ^ 0xffff_ffff
}

fn image_len(bytecode_len: u16, map_count: u8) -> Option<usize> {
    (bytecode_len as usize).checked_add((map_count as usize).checked_mul(BPF_MAP_DESCRIPTOR_SIZE)?)
}

fn map_layouts<F: BpfFlash>(
    flash: &mut F,
    bytecode_len: u16,
    map_count: u8,
) -> Option<([MapLayout; BPF_MAX_MAPS], u8)> {
    let mut total = 0u32;
    let mut layouts = [MapLayout::EMPTY; BPF_MAX_MAPS];
    let mut descriptor = [0; BPF_MAP_DESCRIPTOR_SIZE];
    for index in 0..map_count as usize {
        let offset = BPF_FLASH_OFFSET
            + BPF_HEADER_SIZE as u32
            + bytecode_len as u32
            + (index * BPF_MAP_DESCRIPTOR_SIZE) as u32;
        if flash.read(offset, &mut descriptor).is_err() {
            return None;
        }
        let map_type =
            u32::from_le_bytes([descriptor[0], descriptor[1], descriptor[2], descriptor[3]]);
        let key_size =
            u32::from_le_bytes([descriptor[4], descriptor[5], descriptor[6], descriptor[7]]);
        let value_size =
            u32::from_le_bytes([descriptor[8], descriptor[9], descriptor[10], descriptor[11]]);
        let max_entries = u32::from_le_bytes([
            descriptor[12],
            descriptor[13],
            descriptor[14],
            descriptor[15],
        ]);
        let Some(bytes) = value_size.checked_mul(max_entries) else {
            return None;
        };
        let Some(next_total) = total.checked_add(bytes) else {
            return None;
        };
        if map_type != 1
            || key_size != 4
            || value_size == 0
            || max_entries == 0
            || next_total > BPF_MAX_MAP_BYTES as u32
        {
            return None;
        }
        layouts[index] = MapLayout {
            offset: total as u16,
            len: bytes as u16,
        };
        total = next_total;
    }
    Some((layouts, map_count))
}

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
/// A fixed-capacity, single-entry status queue.
pub struct StatusQueue {
    newest: [u8; MAX_PAYLOAD],
    newest_len: u8,
}

impl StatusQueue {
    pub const fn ready() -> Self {
        Self {
            newest: [
                b'r', b'e', b'a', b'd', b'y', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            newest_len: 5,
        }
    }

    pub fn pop(&mut self) -> &[u8] {
        let length = self.newest_len as usize;
        self.newest_len = 0;
        &self.newest[..length]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketOutcome {
    Response(usize),
    Empty,
    Reset,
}

pub fn dispatch(
    request: &Request,
    status_queue: &mut StatusQueue,
    output: &mut [u8; MAX_FRAME],
) -> PacketOutcome {
    if request.flags != 0 {
        return response(STATUS_BAD_FLAGS, request.sequence, &[], output);
    }
    match request.command {
        CMD_RESET if request.payload_len == 0 => PacketOutcome::Reset,
        CMD_RESET => response(STATUS_BAD_LENGTH, request.sequence, &[], output),
        CMD_READ_STATUS if request.payload_len == 0 => {
            response(STATUS_OK, request.sequence, status_queue.pop(), output)
        }
        CMD_READ_STATUS => response(STATUS_BAD_LENGTH, request.sequence, &[], output),
        CMD_LOAD_BPF | CMD_START_BPF | CMD_READ_BPF_MAP | CMD_WRITE_BPF_MAP => response(
            if request.payload_len == 0 {
                STATUS_NOT_IMPLEMENTED
            } else {
                STATUS_BAD_LENGTH
            },
            request.sequence,
            &[],
            output,
        ),
        _ => response(STATUS_BAD_COMMAND, request.sequence, &[], output),
    }
}

pub fn dispatch_with_bpf(
    request: &Request,
    status_queue: &mut StatusQueue,
    loader: &mut BpfLoader,
    map_backing: &[u8; BPF_MAX_MAP_BYTES],
    output: &mut [u8; MAX_FRAME],
) -> PacketOutcome {
    if request.flags != 0 {
        return response(STATUS_BAD_FLAGS, request.sequence, &[], output);
    }
    match request.command {
        CMD_LOAD_BPF => response(
            match loader.accepts_load(request) {
                Ok(()) => STATUS_OK,
                Err(status) => status,
            },
            request.sequence,
            &[],
            output,
        ),
        CMD_READ_BPF_MAP if request.payload_len != 4 => {
            response(STATUS_BAD_LENGTH, request.sequence, &[], output)
        }
        CMD_READ_BPF_MAP => {
            let byte_length = request.payload[3];
            let byte_offset = u16::from_le_bytes([request.payload[1], request.payload[2]]);
            match loader.read_map(request.payload[0], byte_offset, byte_length, map_backing) {
                Ok(bytes) => response(STATUS_OK, request.sequence, bytes, output),
                Err(status) => response(status, request.sequence, &[], output),
            }
        }
        CMD_QUERY_BPF_CRC if request.payload_len != 0 => {
            response(STATUS_BAD_LENGTH, request.sequence, &[], output)
        }
        CMD_QUERY_BPF_CRC => match loader.query() {
            Ok(crc) => response(STATUS_OK, request.sequence, &crc.to_le_bytes(), output),
            Err(status) => response(status, request.sequence, &[], output),
        },
        _ => dispatch(request, status_queue, output),
    }
}

fn response(
    status: u8,
    sequence: u8,
    payload: &[u8],
    output: &mut [u8; MAX_FRAME],
) -> PacketOutcome {
    PacketOutcome::Response(
        encode_response(status, sequence, 0, payload, output).unwrap_or(HEADER_LEN),
    )
}

pub fn dispatch_packet(
    packet: &[u8],
    truncated: bool,
    status_queue: &mut StatusQueue,
    output: &mut [u8; MAX_FRAME],
) -> PacketOutcome {
    match parse_packet(packet, truncated) {
        Some(request) => dispatch(&request, status_queue, output),
        None => PacketOutcome::Empty,
    }
}

pub fn dispatch_packet_with_bpf(
    packet: &[u8],
    truncated: bool,
    status_queue: &mut StatusQueue,
    loader: &mut BpfLoader,
    map_backing: &[u8; BPF_MAX_MAP_BYTES],
    output: &mut [u8; MAX_FRAME],
) -> PacketOutcome {
    match parse_packet(packet, truncated) {
        Some(request) => dispatch_with_bpf(&request, status_queue, loader, map_backing, output),
        None => PacketOutcome::Empty,
    }
}

/// Parses one complete I2C receive capture.
///
/// A capture is valid only if it is not truncated, has no malformed frames or
/// trailing partial frame, and contains at least one complete frame. For
/// captures containing multiple frames, returns the final complete frame.
pub fn parse_packet(packet: &[u8], truncated: bool) -> Option<Request> {
    if truncated {
        return None;
    }
    let mut parser = Parser::new();
    let mut request = None;
    for byte in packet {
        match parser.push(*byte) {
            Ok(Some(value)) => request = Some(value),
            Ok(None) => {}
            Err(_) => return None,
        }
    }
    if parser.length != 0 { None } else { request }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_bytes(sequence: u8, payload: &[u8]) -> [u8; MAX_FRAME] {
        let mut bytes = [0; MAX_FRAME];
        bytes[0] = MAGIC;
        bytes[1] = CMD_READ_STATUS;
        bytes[2] = payload.len() as u8;
        bytes[3] = sequence;
        bytes[4] = 0;
        bytes[HEADER_LEN..HEADER_LEN + payload.len()].copy_from_slice(payload);
        bytes
    }

    fn response_length(outcome: PacketOutcome) -> usize {
        match outcome {
            PacketOutcome::Response(length) => length,
            PacketOutcome::Empty | PacketOutcome::Reset => panic!("expected response"),
        }
    }

    #[test]
    fn parses_read_status_request() {
        let bytes = request_bytes(7, &[]);
        let mut parser = Parser::new();
        let mut result = None;
        for byte in bytes.iter().take(HEADER_LEN) {
            result = parser.push(*byte).unwrap();
        }
        assert_eq!(
            result,
            Some(Request {
                command: CMD_READ_STATUS,
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
                command: CMD_READ_STATUS,
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
            frame[1] = CMD_READ_STATUS;
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
        let malformed = [MAGIC, CMD_READ_STATUS, MAX_PAYLOAD as u8 + 1, 9, 0];
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

        request.command = CMD_READ_STATUS;
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
    fn dispatches_read_status_response() {
        let request = Request {
            command: CMD_READ_STATUS,
            sequence: 9,
            flags: 0,
            payload: [0; MAX_PAYLOAD],
            payload_len: 0,
        };
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_OK, 5, 9, 0, b'r', b'e', b'a', b'd', b'y']
        );
    }

    #[test]
    fn dispatches_action_request_errors() {
        let mut request = Request::empty();
        request.command = CMD_READ_STATUS;
        request.sequence = 1;
        let mut status_queue = StatusQueue::ready();

        request.flags = 1;
        let mut output = [0; MAX_FRAME];
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_FLAGS, 0, request.sequence, 0]
        );

        request.flags = 0;
        request.command = 0x07;
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_COMMAND, 0, request.sequence, 0]
        );

        request.command = CMD_READ_STATUS;
        request.payload_len = 1;
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_LENGTH, 0, request.sequence, 0]
        );

        request.command = CMD_READ_STATUS;
        request.payload_len = 0;
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[
                MAGIC,
                STATUS_OK,
                5,
                request.sequence,
                0,
                b'r',
                b'e',
                b'a',
                b'd',
                b'y'
            ]
        );
    }

    #[test]
    fn dispatches_action_stubs_as_not_implemented() {
        let mut status_queue = StatusQueue::ready();
        let mut request = Request::empty();
        request.sequence = 7;
        let mut output = [0; MAX_FRAME];

        for command in [
            CMD_LOAD_BPF,
            CMD_START_BPF,
            CMD_READ_BPF_MAP,
            CMD_WRITE_BPF_MAP,
        ] {
            request.command = command;
            let length = response_length(dispatch(&request, &mut status_queue, &mut output));
            assert_eq!(
                &output[..length],
                &[MAGIC, STATUS_NOT_IMPLEMENTED, 0, request.sequence, 0]
            );

            request.payload_len = 1;
            let length = response_length(dispatch(&request, &mut status_queue, &mut output));
            assert_eq!(
                &output[..length],
                &[MAGIC, STATUS_BAD_LENGTH, 0, request.sequence, 0]
            );

            request.payload_len = 0;
            request.flags = 1;
            let length = response_length(dispatch(&request, &mut status_queue, &mut output));
            assert_eq!(
                &output[..length],
                &[MAGIC, STATUS_BAD_FLAGS, 0, request.sequence, 0]
            );
            request.flags = 0;
        }
    }

    #[test]
    fn dispatches_reset_outcomes() {
        let mut request = Request::empty();
        request.command = CMD_RESET;
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch(&request, &mut status_queue, &mut output),
            PacketOutcome::Reset
        );

        request.flags = 1;
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_FLAGS, 0, request.sequence, 0]
        );

        request.flags = 0;
        request.payload_len = 1;
        let length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_BAD_LENGTH, 0, request.sequence, 0]
        );
    }

    #[test]
    fn read_status_pops_newest_message() {
        let mut status_queue = StatusQueue::ready();
        let mut request = Request::empty();
        request.command = CMD_READ_STATUS;
        let mut output = [0; MAX_FRAME];

        request.sequence = 1;
        let first_length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(
            &output[..first_length],
            &[MAGIC, STATUS_OK, 5, 1, 0, b'r', b'e', b'a', b'd', b'y']
        );

        request.sequence = 2;
        let second_length = response_length(dispatch(&request, &mut status_queue, &mut output));
        assert_eq!(&output[..second_length], &[MAGIC, STATUS_OK, 0, 2, 0]);
    }

    #[test]
    fn dispatch_packet_uses_only_the_final_complete_frame() {
        let mut packet = [0; HEADER_LEN * 2];
        packet[..HEADER_LEN].copy_from_slice(&request_bytes(1, &[])[..HEADER_LEN]);
        packet[HEADER_LEN..].copy_from_slice(&request_bytes(2, &[])[..HEADER_LEN]);
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        let outcome = dispatch_packet(&packet, false, &mut status_queue, &mut output);

        assert_eq!(outcome, PacketOutcome::Response(10));
        assert_eq!(
            &output[..10],
            &[MAGIC, STATUS_OK, 5, 2, 0, b'r', b'e', b'a', b'd', b'y']
        );
    }

    #[test]
    fn dispatch_packet_rejects_incomplete_and_malformed_packets() {
        let incomplete = [MAGIC, CMD_READ_STATUS];
        let malformed = [MAGIC, CMD_READ_STATUS, MAX_PAYLOAD as u8 + 1, 3, 0];
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch_packet(&incomplete, false, &mut status_queue, &mut output),
            PacketOutcome::Empty
        );
        assert_eq!(
            dispatch_packet(&malformed, false, &mut status_queue, &mut output),
            PacketOutcome::Empty
        );
        assert_eq!(
            dispatch_packet(
                &request_bytes(4, &[])[..HEADER_LEN],
                false,
                &mut status_queue,
                &mut output,
            ),
            PacketOutcome::Response(10)
        );
    }

    #[test]
    fn dispatch_packet_rejects_trailing_partial_frames() {
        let complete = request_bytes(5, &[]);
        let packet = [
            complete[0],
            complete[1],
            complete[2],
            complete[3],
            complete[4],
            MAGIC,
            CMD_READ_STATUS,
        ];
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch_packet(&packet, false, &mut status_queue, &mut output),
            PacketOutcome::Empty
        );
        assert_eq!(
            dispatch_packet(
                &request_bytes(6, &[])[..HEADER_LEN],
                false,
                &mut status_queue,
                &mut output,
            ),
            PacketOutcome::Response(10)
        );
    }

    #[test]
    fn dispatch_packet_returns_reset_for_valid_reset() {
        let request = [MAGIC, CMD_RESET, 0, 8, 0];
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch_packet(&request, false, &mut status_queue, &mut output),
            PacketOutcome::Reset
        );
    }

    #[test]
    fn dispatch_packet_rejects_truncated_valid_prefixes() {
        let request = request_bytes(5, &[]);
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch_packet(&request[..HEADER_LEN], true, &mut status_queue, &mut output,),
            PacketOutcome::Empty
        );
        assert_eq!(
            dispatch_packet(
                &request[..HEADER_LEN],
                false,
                &mut status_queue,
                &mut output,
            ),
            PacketOutcome::Response(10)
        );
    }

    #[test]
    fn dispatch_packet_does_not_pop_status_for_invalid_requests() {
        let invalid = [MAGIC, CMD_READ_STATUS, 0, 6, 1];
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];

        assert_eq!(
            dispatch_packet(&invalid, false, &mut status_queue, &mut output),
            PacketOutcome::Response(5)
        );
        assert_eq!(&output[..5], &[MAGIC, STATUS_BAD_FLAGS, 0, 6, 0]);
        assert_eq!(
            dispatch_packet(
                &request_bytes(7, &[])[..HEADER_LEN],
                false,
                &mut status_queue,
                &mut output,
            ),
            PacketOutcome::Response(10)
        );
        assert_eq!(
            &output[..10],
            &[MAGIC, STATUS_OK, 5, 7, 0, b'r', b'e', b'a', b'd', b'y']
        );
    }

    struct FakeFlash {
        bytes: [u8; 32768],
        fail_write: bool,
    }

    impl FakeFlash {
        fn new() -> Self {
            Self {
                bytes: [0xff; 32768],
                fail_write: false,
            }
        }
    }

    impl BpfFlash for FakeFlash {
        fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), ()> {
            let offset = offset as usize;
            let end = offset.checked_add(bytes.len()).ok_or(())?;
            bytes.copy_from_slice(self.bytes.get(offset..end).ok_or(())?);
            Ok(())
        }

        fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), ()> {
            if self.fail_write || offset as usize % 2 != 0 || bytes.len() % 2 != 0 {
                return Err(());
            }
            let offset = offset as usize;
            let end = offset.checked_add(bytes.len()).ok_or(())?;
            let target = self.bytes.get_mut(offset..end).ok_or(())?;
            for (target, source) in target.iter_mut().zip(bytes) {
                *target &= *source;
            }
            Ok(())
        }

        fn erase(&mut self, from: u32, to: u32) -> Result<(), ()> {
            if from as usize % 4096 != 0 || to as usize % 4096 != 0 {
                return Err(());
            }
            self.bytes
                .get_mut(from as usize..to as usize)
                .ok_or(())?
                .fill(0xff);
            Ok(())
        }
    }

    fn load_request(payload: &[u8]) -> Request {
        let mut request = Request::empty();
        request.command = CMD_LOAD_BPF;
        request.payload_len = payload.len() as u8;
        request.payload[..payload.len()].copy_from_slice(payload);
        request
    }

    fn execute(loader: &mut BpfLoader, flash: &mut FakeFlash) {
        loader.execute_pending(flash);
    }

    #[test]
    fn crc32_iso_hdlc_matches_known_vector() {
        assert_eq!(crc32_iso_hdlc(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn loader_commits_valid_image_and_reports_crc() {
        let bytecode = [0u8; 8];
        let descriptor = [
            1, 0, 0, 0, // array
            4, 0, 0, 0, // u32 key
            1, 0, 0, 0, // one-byte value
            1, 0, 0, 0, // one entry
        ];
        let mut image = [0u8; 24];
        image[..8].copy_from_slice(&bytecode);
        image[8..].copy_from_slice(&descriptor);
        let crc = crc32_iso_hdlc(&image);
        let mut loader = BpfLoader::new();
        let mut flash = FakeFlash::new();

        let mut begin = [0; 8];
        begin[0] = 0;
        begin[1..3].copy_from_slice(&(8u16).to_le_bytes());
        begin[3] = 1;
        begin[4..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        assert_eq!(loader.query(), Err(STATUS_BUSY));
        execute(&mut loader, &mut flash);

        for (offset, data) in [(0u16, &image[..12]), (12u16, &image[12..])] {
            let mut payload = [0; 15];
            payload[0] = 1;
            payload[1..3].copy_from_slice(&offset.to_le_bytes());
            payload[3..3 + data.len()].copy_from_slice(data);
            assert_eq!(
                loader.accepts_load(&load_request(&payload[..3 + data.len()])),
                Ok(())
            );
            execute(&mut loader, &mut flash);
        }
        assert_eq!(loader.accepts_load(&load_request(&[2])), Ok(()));
        execute(&mut loader, &mut flash);
        assert_eq!(loader.query(), Ok(crc));

        let mut rebooted = BpfLoader::new();
        rebooted.validate_committed(&mut flash);
        assert_eq!(rebooted.query(), Ok(crc));
    }

    #[test]
    fn read_bpf_map_returns_committed_backing_ranges_and_errors() {
        let mut image = [0u8; 40];
        image[8..12].copy_from_slice(&1u32.to_le_bytes());
        image[12..16].copy_from_slice(&4u32.to_le_bytes());
        image[16..20].copy_from_slice(&2u32.to_le_bytes());
        image[20..24].copy_from_slice(&4u32.to_le_bytes());
        image[24..28].copy_from_slice(&1u32.to_le_bytes());
        image[28..32].copy_from_slice(&4u32.to_le_bytes());
        image[32..36].copy_from_slice(&4u32.to_le_bytes());
        image[36..40].copy_from_slice(&8u32.to_le_bytes());
        let crc = crc32_iso_hdlc(&image);
        let mut loader = BpfLoader::new();
        let mut flash = FakeFlash::new();
        let mut begin = [0; 8];
        begin[0] = 0;
        begin[1..3].copy_from_slice(&8u16.to_le_bytes());
        begin[3] = 2;
        begin[4..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        execute(&mut loader, &mut flash);
        for (index, data) in image.chunks(12).enumerate() {
            let mut payload = [0; 15];
            payload[0] = 1;
            payload[1..3].copy_from_slice(&((index * 12) as u16).to_le_bytes());
            payload[3..3 + data.len()].copy_from_slice(data);
            assert_eq!(
                loader.accepts_load(&load_request(&payload[..3 + data.len()])),
                Ok(())
            );
            execute(&mut loader, &mut flash);
        }
        assert_eq!(loader.accepts_load(&load_request(&[2])), Ok(()));
        execute(&mut loader, &mut flash);

        let mut loader = BpfLoader::new();
        loader.validate_committed(&mut flash);
        assert_eq!(loader.query(), Ok(crc));

        let mut map_backing = [0; BPF_MAX_MAP_BYTES];
        for (index, byte) in map_backing[..40].iter_mut().enumerate() {
            *byte = index as u8;
        }
        let original_backing = map_backing;
        assert_eq!(
            loader.read_map(1, 0, 0, &map_backing),
            Err(STATUS_BAD_LENGTH)
        );
        assert_eq!(
            loader.read_map(1, 0, MAX_PAYLOAD as u8 + 1, &map_backing),
            Err(STATUS_BAD_LENGTH)
        );
        let mut status_queue = StatusQueue::ready();
        let mut output = [0; MAX_FRAME];
        let mut request = Request::empty();
        request.command = CMD_READ_BPF_MAP;
        request.sequence = 9;
        request.payload_len = 4;
        request.payload[..4].copy_from_slice(&[1, 12, 0, 16]);

        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(
            &output[..length],
            &[
                MAGIC, STATUS_OK, 16, 9, 0, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33,
                34, 35
            ]
        );
        assert_eq!(map_backing, original_backing);

        request.flags = 1;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_FLAGS, 0, 9, 0]);
        request.flags = 0;

        request.payload_len = 3;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_LENGTH, 0, 9, 0]);
        request.payload_len = 4;

        request.payload[3] = 0;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_LENGTH, 0, 9, 0]);
        request.payload[3] = 16;

        request.payload[3] = 17;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_LENGTH, 0, 9, 0]);
        request.payload[3] = 16;

        request.payload[0] = 2;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_COMMAND, 0, 9, 0]);
        request.payload[0] = 1;
        request.payload[1..4].copy_from_slice(&[31, 0, 2]);
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_LENGTH, 0, 9, 0]);

        request.payload[1..4].copy_from_slice(&[0xff, 0xff, 1]);
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BAD_LENGTH, 0, 9, 0]);

        let mut no_program = BpfLoader::new();
        request.payload[1..4].copy_from_slice(&[0, 0, 1]);
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut no_program,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_NO_PROGRAM, 0, 9, 0]);
        request.command = CMD_READ_STATUS;
        request.payload_len = 0;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(
            &output[..length],
            &[MAGIC, STATUS_OK, 5, 9, 0, b'r', b'e', b'a', b'd', b'y']
        );
    }

    #[test]
    fn loader_rejects_invalid_transfer_and_never_commits_partial_image() {
        let mut loader = BpfLoader::new();
        let mut flash = FakeFlash::new();
        assert_eq!(
            loader.accepts_load(&load_request(&[0, 7, 0, 0, 0, 0, 0, 0])),
            Err(STATUS_BAD_LENGTH)
        );
        assert_eq!(
            loader.accepts_load(&load_request(&[1, 0, 0, 1, 2])),
            Err(STATUS_BAD_STATE)
        );

        let begin = [0, 8, 0, 0, 0, 0, 0, 0];
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        assert_eq!(loader.accepts_load(&load_request(&begin)), Err(STATUS_BUSY));
        execute(&mut loader, &mut flash);
        assert_eq!(
            loader.accepts_load(&load_request(&[1, 2, 0, 1, 2])),
            Err(STATUS_BAD_STATE)
        );
        assert_eq!(
            loader.accepts_load(&load_request(&[2])),
            Err(STATUS_BAD_STATE)
        );
        assert_eq!(loader.query(), Err(STATUS_NO_PROGRAM));
    }

    #[test]
    fn loader_retains_flash_and_crc_failures_for_query() {
        let mut loader = BpfLoader::new();
        let mut flash = FakeFlash::new();
        let begin = [0, 8, 0, 0, 0, 0, 0, 0];
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        flash.fail_write = true;
        execute(&mut loader, &mut flash);
        assert_eq!(loader.query(), Err(STATUS_FLASH_ERROR));

        flash.fail_write = false;
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        execute(&mut loader, &mut flash);
        assert_eq!(
            loader.accepts_load(&load_request(&[1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])),
            Ok(())
        );
        execute(&mut loader, &mut flash);
        assert_eq!(loader.accepts_load(&load_request(&[2])), Ok(()));
        execute(&mut loader, &mut flash);
        assert_eq!(loader.query(), Err(STATUS_BAD_CRC));
    }

    #[test]
    fn loader_rejects_invalid_map_definitions_at_finalize() {
        let mut image = [0u8; 24];
        image[8..12].copy_from_slice(&2u32.to_le_bytes());
        image[12..16].copy_from_slice(&4u32.to_le_bytes());
        image[16..20].copy_from_slice(&1u32.to_le_bytes());
        image[20..24].copy_from_slice(&1u32.to_le_bytes());
        let crc = crc32_iso_hdlc(&image);
        let mut loader = BpfLoader::new();
        let mut flash = FakeFlash::new();
        let mut begin = [0; 8];
        begin[0] = 0;
        begin[1..3].copy_from_slice(&8u16.to_le_bytes());
        begin[3] = 1;
        begin[4..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(loader.accepts_load(&load_request(&begin)), Ok(()));
        execute(&mut loader, &mut flash);

        for (offset, data) in [(0u16, &image[..12]), (12u16, &image[12..])] {
            let mut payload = [0; 15];
            payload[0] = 1;
            payload[1..3].copy_from_slice(&offset.to_le_bytes());
            payload[3..3 + data.len()].copy_from_slice(data);
            assert_eq!(
                loader.accepts_load(&load_request(&payload[..3 + data.len()])),
                Ok(())
            );
            execute(&mut loader, &mut flash);
        }
        assert_eq!(loader.accepts_load(&load_request(&[2])), Ok(()));
        execute(&mut loader, &mut flash);
        assert_eq!(loader.query(), Err(STATUS_BAD_STATE));
    }

    #[test]
    fn dispatches_bpf_operations_and_query_responses() {
        let mut status_queue = StatusQueue::ready();
        let mut loader = BpfLoader::new();
        let map_backing = [0; BPF_MAX_MAP_BYTES];
        let mut output = [0; MAX_FRAME];
        let mut request = load_request(&[0, 8, 0, 0, 0, 0, 0, 0]);
        request.sequence = 3;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_OK, 0, 3, 0]);

        request.command = CMD_QUERY_BPF_CRC;
        request.payload_len = 0;
        request.sequence = 4;
        let length = response_length(dispatch_with_bpf(
            &request,
            &mut status_queue,
            &mut loader,
            &map_backing,
            &mut output,
        ));
        assert_eq!(&output[..length], &[MAGIC, STATUS_BUSY, 0, 4, 0]);
    }
}
