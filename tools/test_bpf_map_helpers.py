import argparse
import struct
import time
import zlib

import serial

from test_bpf_loader import (
    CMD_RESET,
    STATUS_OK,
    STATUS_NO_PROGRAM,
    frame,
    read_map,
    target_request,
    target_write,
    wait_ready,
    write_map,
)


CMD_LOAD_BPF = 0x02
CMD_START_BPF = 0x03
MAP_LOOKUP_HELPER_ID = 10
MAP_UPDATE_HELPER_ID = 11
EXPECTED_VALUE = 41
UPDATED_VALUE = 42

# eBPF instruction opcodes used by the smoke-test program.
LD_DW_IMM = 0x18
LD_W_REG = 0x61
ST_W_IMM = 0x62
ST_W_REG = 0x63
MOV64_REG = 0xbf
MOV64_IMM = 0xb7
ADD64_IMM = 0x07
CALL = 0x85
JEQ_IMM = 0x15
JEQ_IMM32 = 0x16
JNE_IMM = 0x55
JA = 0x05
EXIT = 0x95


def insn(code, dst=0, src=0, offset=0, immediate=0):
    return struct.pack("<BBhI", code, dst | (src << 4), offset, immediate & 0xFFFFFFFF)


def smoke_test_bytecode():
    # The program uses stack slots -4 and -8 for the key and updated value.
    # Failure paths return 1; success returns 0 after the update helper succeeds.
    instructions = [
        insn(LD_DW_IMM, dst=1, src=1, immediate=0),
        insn(0),
        insn(ST_W_IMM, dst=10, offset=-4, immediate=0),
        insn(MOV64_REG, dst=2, src=10),
        insn(ADD64_IMM, dst=2, immediate=-4),
        insn(CALL, immediate=MAP_LOOKUP_HELPER_ID),
        insn(JEQ_IMM, dst=0, offset=16, immediate=0),
        insn(LD_W_REG, dst=3, src=0),
        insn(JEQ_IMM32, dst=3, offset=1, immediate=EXPECTED_VALUE),
        insn(JA, offset=13),
        insn(ADD64_IMM, dst=3, immediate=1),
        insn(ST_W_REG, dst=10, src=3, offset=-8),
        insn(LD_DW_IMM, dst=1, src=1, immediate=0),
        insn(0),
        insn(MOV64_REG, dst=2, src=10),
        insn(ADD64_IMM, dst=2, immediate=-4),
        insn(MOV64_REG, dst=3, src=10),
        insn(ADD64_IMM, dst=3, immediate=-8),
        insn(MOV64_IMM, dst=4, immediate=0),
        insn(CALL, immediate=MAP_UPDATE_HELPER_ID),
        insn(JNE_IMM, dst=0, offset=2, immediate=0),
        insn(MOV64_IMM, dst=0, immediate=0),
        insn(EXIT),
        insn(MOV64_IMM, dst=0, immediate=1),
        insn(EXIT),
    ]
    bytecode = b"".join(instructions)
    if len(bytecode) % 8 != 0:
        raise RuntimeError("smoke-test bytecode is not eight-byte aligned")
    return bytecode


def submit_load_step(port, sequence, payload, expected_query_status):
    response, sequence = target_request(
        port,
        sequence,
        CMD_LOAD_BPF,
        payload,
        5,
    )
    if response[1:] != bytes((STATUS_OK, 0, (sequence - 2) & 0xFF, 0)):
        raise RuntimeError(f"load command failed: {response.hex(' ')}")

    response, sequence = wait_ready(port, sequence)
    if len(response) < 5 or response[1] != expected_query_status:
        raise RuntimeError(f"load completion failed: {response.hex(' ')}")
    return response, sequence


def load_image(port, sequence, bytecode):
    image = bytecode + struct.pack("<IIII", 1, 4, 4, 1)
    crc = zlib.crc32(image) & 0xFFFFFFFF

    begin = bytes((0,)) + struct.pack("<HB", len(bytecode), 1) + struct.pack("<I", crc)
    _, sequence = submit_load_step(port, sequence, begin, STATUS_NO_PROGRAM)

    # The bridge leaves six even data bytes after the target address and frame header.
    for offset in range(0, len(image), 6):
        fragment = image[offset : offset + 6]
        payload = bytes((1,)) + struct.pack("<H", offset) + fragment
        _, sequence = submit_load_step(port, sequence, payload, STATUS_NO_PROGRAM)

    finalize = bytes((2,))
    response, sequence = submit_load_step(port, sequence, finalize, STATUS_OK)
    if response[1] != STATUS_OK or response[2] != 4:
        raise RuntimeError(f"image finalize failed: {response.hex(' ')}")
    actual_crc = struct.unpack("<I", response[5:9])[0]
    if actual_crc != crc:
        raise RuntimeError(f"CRC mismatch: expected {crc:08X}, got {actual_crc:08X}")
    return sequence


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", default="COM9")
    args = parser.parse_args()

    bytecode = smoke_test_bytecode()
    with serial.Serial(args.port, 115200, timeout=3, dsrdtr=False, rtscts=False) as port:
        port.dtr = True
        port.rts = True
        time.sleep(1)
        port.reset_input_buffer()
        sequence = 1

        target_write(
            port,
            sequence,
            frame(CMD_RESET, sequence),
        )
        time.sleep(1)
        sequence = (sequence + 1) & 0xFF

        sequence = load_image(port, sequence, bytecode)

        initial, sequence = read_map(port, sequence, 0, 0, 4)
        if initial != b"\x00\x00\x00\x00":
            raise RuntimeError(f"initial map value mismatch: {initial.hex(' ')}")
        print("PASS: map starts at zero")

        sequence = write_map(port, sequence, 0, 0, struct.pack("<I", EXPECTED_VALUE))
        print(f"PASS: wrote map value {EXPECTED_VALUE}")

        response, sequence = target_request(port, sequence, CMD_START_BPF, b"", 5)
        if response[1:] != bytes((STATUS_OK, 0, (sequence - 2) & 0xFF, 0)):
            raise RuntimeError(f"BPF start failed: {response.hex(' ')}")
        print("PASS: BPF helper program executed")

        actual, _ = read_map(port, sequence, 0, 0, 4)
        expected = struct.pack("<I", UPDATED_VALUE)
        if actual != expected:
            raise RuntimeError(
                f"updated map value mismatch: expected {expected.hex(' ')}, "
                f"got {actual.hex(' ')}"
            )
        print(f"PASS: map value incremented to {UPDATED_VALUE}")


if __name__ == "__main__":
    main()
