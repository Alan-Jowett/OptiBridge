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
)


CMD_LOAD_BPF = 0x02
CMD_START_BPF = 0x03
MAP_LOOKUP_HELPER_ID = 10
MAP_UPDATE_HELPER_ID = 11
TIMER_SCHEDULE_HELPER_ID = 12
TIMER_DELAY_MS = 1000
TIMER_COOKIE = 0x54494D45
MIN_TIMER_INCREMENTS = 3
OBSERVATION_SECONDS = 4.2

# eBPF instruction opcodes used by the timer smoke-test program.
LD_DW_IMM = 0x18
LD_W_REG = 0x61
ST_W_IMM = 0x62
ST_W_REG = 0x63
MOV64_REG = 0xBF
MOV64_IMM = 0xB7
ADD64_IMM = 0x07
CALL = 0x85
JEQ_IMM = 0x15
JNE_IMM = 0x55
EXIT = 0x95


def insn(code, dst=0, src=0, offset=0, immediate=0):
    return struct.pack(
        "<BBhI", code, dst | (src << 4), offset, immediate & 0xFFFFFFFF
    )


def timer_test_bytecode():
    # The program increments map entry zero, then schedules the next invocation.
    # Failure paths return 1; success returns 0.
    instructions = [
        insn(LD_DW_IMM, dst=1, src=1, immediate=0),
        insn(0),
        insn(ST_W_IMM, dst=10, offset=-4, immediate=0),
        insn(MOV64_REG, dst=2, src=10),
        insn(ADD64_IMM, dst=2, immediate=-4),
        insn(CALL, immediate=MAP_LOOKUP_HELPER_ID),
        insn(JEQ_IMM, dst=0, offset=18, immediate=0),
        insn(LD_W_REG, dst=3, src=0),
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
        insn(JNE_IMM, dst=0, offset=6, immediate=0),
        insn(MOV64_IMM, dst=1, immediate=TIMER_DELAY_MS),
        insn(MOV64_IMM, dst=2, immediate=TIMER_COOKIE),
        insn(CALL, immediate=TIMER_SCHEDULE_HELPER_ID),
        insn(JNE_IMM, dst=0, offset=2, immediate=0),
        insn(MOV64_IMM, dst=0, immediate=0),
        insn(EXIT),
        insn(MOV64_IMM, dst=0, immediate=1),
        insn(EXIT),
    ]
    bytecode = b"".join(instructions)
    if len(bytecode) % 8 != 0:
        raise RuntimeError("timer-test bytecode is not eight-byte aligned")
    return bytecode


def submit_load_step(port, sequence, payload, expected_query_status):
    response, sequence = target_request(port, sequence, CMD_LOAD_BPF, payload, 5)
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

    for offset in range(0, len(image), 6):
        fragment = image[offset : offset + 6]
        payload = bytes((1,)) + struct.pack("<H", offset) + fragment
        _, sequence = submit_load_step(port, sequence, payload, STATUS_NO_PROGRAM)

    response, sequence = submit_load_step(port, sequence, bytes((2,)), STATUS_OK)
    if response[1] != STATUS_OK or response[2] != 4:
        raise RuntimeError(f"image finalize failed: {response.hex(' ')}")
    actual_crc = struct.unpack("<I", response[5:9])[0]
    if actual_crc != crc:
        raise RuntimeError(f"CRC mismatch: expected {crc:08X}, got {actual_crc:08X}")
    return sequence


def reset_target(port, sequence):
    target_write(port, sequence, frame(CMD_RESET, sequence))
    time.sleep(1)
    return (sequence + 1) & 0xFF


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", default="COM9")
    args = parser.parse_args()

    bytecode = timer_test_bytecode()
    with serial.Serial(args.port, 115200, timeout=3, dsrdtr=False, rtscts=False) as port:
        port.dtr = True
        port.rts = True
        time.sleep(1)
        port.reset_input_buffer()
        sequence = reset_target(port, 1)

        try:
            sequence = load_image(port, sequence, bytecode)
            initial, sequence = read_map(port, sequence, 0, 0, 4)
            if initial != b"\x00\x00\x00\x00":
                raise RuntimeError(f"initial map value mismatch: {initial.hex(' ')}")

            response, sequence = target_request(port, sequence, CMD_START_BPF, b"", 5)
            if response[1:] != bytes((STATUS_OK, 0, (sequence - 2) & 0xFF, 0)):
                raise RuntimeError(f"BPF start failed: {response.hex(' ')}")

            value, sequence = read_map(port, sequence, 0, 0, 4)
            last_value = struct.unpack("<I", value)[0]
            print(f"PASS: initial timer invocation incremented map to {last_value}")
            deadline = time.monotonic() + OBSERVATION_SECONDS
            while time.monotonic() < deadline:
                time.sleep(0.25)
                value, sequence = read_map(port, sequence, 0, 0, 4)
                current_value = struct.unpack("<I", value)[0]
                if current_value < last_value:
                    raise RuntimeError(
                        f"map value decreased: was {last_value}, got {current_value}"
                    )
                if current_value != last_value:
                    print(f"PASS: timer invocation incremented map to {current_value}")
                    last_value = current_value

            increments = last_value
            if increments < MIN_TIMER_INCREMENTS + 1:
                raise RuntimeError(
                    f"timer progress too slow: expected at least "
                    f"{MIN_TIMER_INCREMENTS + 1}, got {increments}"
                )
            print(f"PASS: observed {increments} total timer-driven increments")
        finally:
            reset_target(port, sequence)


if __name__ == "__main__":
    main()
