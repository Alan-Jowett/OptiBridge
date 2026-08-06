import argparse
import struct
import time
import zlib

import serial


MAGIC = 0xA5
TARGET_ADDRESS = 0x42
STATUS_BUSY = 0x05
STATUS_OK = 0x00
CMD_RESET = 0x01
CMD_READ_BPF_MAP = 0x04
CMD_WRITE_BPF_MAP = 0x05


def frame(command, sequence, payload=b""):
    return bytes((MAGIC, command, len(payload), sequence, 0)) + payload


def read_frame(port):
    header = port.read(5)
    if len(header) != 5:
        raise RuntimeError(f"short bridge header: {header.hex(' ')}")
    payload = port.read(header[2])
    if len(payload) != header[2]:
        raise RuntimeError(f"short bridge payload: {header.hex(' ')} {payload.hex(' ')}")
    print(f"< {header.hex(' ')} {payload.hex(' ')}")
    return header + payload


def bridge(port, command, sequence, payload):
    request = frame(command, sequence, payload)
    print(f"> {request.hex(' ')}")
    port.write(request)
    response = read_frame(port)
    if response[:2] != bytes((MAGIC, 0)) or response[3] != sequence:
        raise RuntimeError(f"bridge error: {response.hex(' ')}")
    return response[5:]


def target_write(port, sequence, request):
    bridge(port, 0x10, sequence, bytes((TARGET_ADDRESS,)) + request)


def target_request(port, sequence, command, payload, response_length):
    target_write(port, sequence, frame(command, sequence, payload))
    response = bridge(
        port,
        0x11,
        (sequence + 1) & 0xFF,
        bytes((TARGET_ADDRESS, response_length)),
    )
    if len(response) != response_length:
        raise RuntimeError(f"short target response: {response.hex(' ')}")
    if response[:1] != bytes((MAGIC,)) or response[3] != sequence or response[4] != 0:
        raise RuntimeError(f"invalid target response: {response.hex(' ')}")
    if len(response) != 5 + response[2]:
        raise RuntimeError(f"invalid target response length: {response.hex(' ')}")
    return response, (sequence + 2) & 0xFF


def query(port, sequence):
    target_write(port, sequence, frame(0x07, sequence))
    return bridge(port, 0x11, (sequence + 1) & 0xFF, bytes((TARGET_ADDRESS, 9)))


def wait_ready(port, sequence):
    for _ in range(100):
        response = query(port, sequence)
        sequence = (sequence + 2) & 0xFF
        if len(response) >= 5 and response[1] != STATUS_BUSY:
            return response, sequence
        time.sleep(0.02)
    raise RuntimeError("target remained busy")


def write_map(port, sequence, map_id, byte_offset, data):
    if not 1 <= len(data) <= 8:
        raise ValueError("bridge map-write pages must contain one through eight bytes")
    map_location = (map_id << 10) | byte_offset
    response, sequence = target_request(
        port,
        sequence,
        CMD_WRITE_BPF_MAP,
        struct.pack("<H", map_location) + data,
        5,
    )
    if response[1:] != bytes((STATUS_OK, 0, (sequence - 2) & 0xFF, 0)):
        raise RuntimeError(f"map write failed: {response.hex(' ')}")
    return sequence


def read_map(port, sequence, map_id, byte_offset, byte_length):
    if not 1 <= byte_length <= 11:
        raise ValueError("bridge map-read pages must contain one through 11 bytes")
    response, sequence = target_request(
        port,
        sequence,
        CMD_READ_BPF_MAP,
        bytes((map_id,)) + struct.pack("<HB", byte_offset, byte_length),
        5 + byte_length,
    )
    if response[1] != STATUS_OK or response[2] != byte_length:
        raise RuntimeError(f"map read failed: {response.hex(' ')}")
    return response[5:], sequence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", default="COM9")
    args = parser.parse_args()

    bytecode = bytes.fromhex("b70000002a000000") * 63 + bytes.fromhex("9500000000000000")
    map_descriptor = struct.pack("<IIII", 1, 4, 4, 16)
    image = bytecode + map_descriptor
    crc = zlib.crc32(image) & 0xFFFFFFFF
    with serial.Serial(args.port, 115200, timeout=3, dsrdtr=False, rtscts=False) as port:
        port.dtr = True
        port.rts = True
        time.sleep(1)
        port.reset_input_buffer()
        sequence = 1

        begin = bytes((0,)) + struct.pack("<HB", len(bytecode), 1) + struct.pack("<I", crc)
        target_write(port, sequence, frame(0x02, sequence, begin))
        _, sequence = wait_ready(port, sequence + 1)

        # The CDC bridge payload includes its target address and the five-byte
        # target frame header, leaving six even data bytes from its 16-byte cap.
        for offset in range(0, len(image), 6):
            fragment = image[offset : offset + 6]
            payload = bytes((1,)) + struct.pack("<H", offset) + fragment
            target_write(port, sequence, frame(0x02, sequence, payload))
            _, sequence = wait_ready(port, sequence + 1)

        target_write(port, sequence, frame(0x02, sequence, bytes((2,))))
        response, sequence = wait_ready(port, sequence + 1)
        if response[1] != 0 or response[2] != 4:
            raise RuntimeError(f"final query failed: {response.hex(' ')}")
        actual_crc = struct.unpack("<I", response[5:9])[0]
        if actual_crc != crc:
            raise RuntimeError(f"CRC mismatch: expected {crc:08X}, got {actual_crc:08X}")
        print(f"PASS: CRC-32/ISO-HDLC {actual_crc:08X}")

        expected_map = bytes(range(64))
        # The bridge can carry eight map bytes after its target address and
        # target-frame header; this still exercises target-side paging.
        for offset in range(0, len(expected_map), 8):
            sequence = write_map(port, sequence, 0, offset, expected_map[offset : offset + 8])

        actual_map = bytearray()
        for offset in range(0, len(expected_map), 8):
            page, sequence = read_map(port, sequence, 0, offset, 8)
            actual_map.extend(page)
        if actual_map != expected_map:
            raise RuntimeError(
                f"map readback mismatch: expected {expected_map.hex(' ')}, "
                f"got {actual_map.hex(' ')}"
            )
        print("PASS: paged map write/readback")

        target_write(port, sequence, frame(CMD_RESET, sequence))
        time.sleep(1)
        sequence = (sequence + 1) & 0xFF
        reset_map = bytearray()
        for offset in range(0, len(expected_map), 8):
            page, sequence = read_map(port, sequence, 0, offset, 8)
            reset_map.extend(page)
        if reset_map != bytes(len(expected_map)):
            raise RuntimeError(f"map backing was not reset: {reset_map.hex(' ')}")
        print("PASS: reset clears map backing")


if __name__ == "__main__":
    main()
