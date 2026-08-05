import argparse
import struct
import time
import zlib

import serial


MAGIC = 0xA5
TARGET_ADDRESS = 0x42
STATUS_BUSY = 0x05


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
        response, _ = wait_ready(port, sequence + 1)
        if response[1] != 0 or response[2] != 4:
            raise RuntimeError(f"final query failed: {response.hex(' ')}")
        actual_crc = struct.unpack("<I", response[5:9])[0]
        if actual_crc != crc:
            raise RuntimeError(f"CRC mismatch: expected {crc:08X}, got {actual_crc:08X}")
        print(f"PASS: CRC-32/ISO-HDLC {actual_crc:08X}")


if __name__ == "__main__":
    main()
