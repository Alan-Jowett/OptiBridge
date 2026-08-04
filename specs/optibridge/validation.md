# OptiBridge Firmware Validation

## Required automated validation

| ID | Requirement coverage | Method | Expected result |
| --- | --- | --- | --- |
| VAL-OPT-001 | REQ-OPT-ACT-001, REQ-OPT-ACT-004 to REQ-OPT-ACT-006 | `cargo test -p optibridge-protocol` | Each action command has its documented status-only response; invalid flags/payload and unknown commands preserve existing error behavior. |
| VAL-OPT-002 | REQ-OPT-ACT-002, REQ-OPT-ACT-003 | `cargo test -p optibridge-protocol` | Read Status returns `ready` with the request sequence, then consumes it; the next read returns `STATUS_OK` with an empty payload. |
| VAL-OPT-003 | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-006, REQ-OPT-FW-007 | `cargo build --release -p optibridge-firmware --target riscv32imc-unknown-none-elf --features firmware` | Firmware compiles and links with generated HAL, status storage, and Sonde probe. |
| VAL-OPT-004 | REQ-OPT-FW-008, REQ-OPT-ACT-007 | `cargo xtask size` | OptiBridge release image is at or below 32,768 bytes. |

## Required hardware validation

Use a seven-bit I2C master at address `0x42`.

### Read Status liveness

1. Write:

   ```text
   A5 06 00 01 00
   ```

2. Read ten bytes.
3. Expect:

   ```text
   A5 00 05 01 00 72 65 61 64 79
   ```

4. Repeat with a new sequence. Expect:

   ```text
   A5 00 00 02 00
   ```

### Recognized stubs

Write each request and read the five-byte response:

| Request | Expected response |
| --- | --- |
| `A5 01 00 02 00` | `A5 04 00 02 00` |
| `A5 02 00 03 00` | `A5 04 00 03 00` |
| `A5 03 00 04 00` | `A5 04 00 04 00` |
| `A5 04 00 05 00` | `A5 04 00 05 00` |
| `A5 05 00 06 00` | `A5 04 00 06 00` |

### Request errors

| Request | Expected response |
| --- | --- |
| `A5 06 00 07 01` | `A5 03 00 07 00` |
| `A5 06 01 08 00 00` | `A5 02 00 08 00` |
| `A5 07 00 09 00` | `A5 01 00 09 00` |

### Packet behavior and waveform capture

Retain the packet-boundary cases from the prior validation baseline. Use
`docs/picoscope-i2c-debugging.md` to confirm I2C START, address `0x42` ACK,
request bytes, and the expected response bytes.

## Existing evidence

- **KNOWN:** The Sonde-enabled release image measured 21,756 bytes of flash.

## Deferred validation

No automated firmware-level test currently controls I2C slave packet boundaries
or injects I2C receive/write errors. The packet-boundary cases remain required
hardware or future harness validation.

## Traceability

| Requirement | Validation |
| --- | --- |
| REQ-OPT-FW-001 | VAL-OPT-003 |
| REQ-OPT-FW-002 | VAL-OPT-003, Read Status hardware validation |
| REQ-OPT-FW-003 | Packet behavior hardware validation |
| REQ-OPT-FW-004 | Packet behavior hardware validation |
| REQ-OPT-FW-005 | VAL-OPT-001, VAL-OPT-002 |
| REQ-OPT-FW-006 | VAL-OPT-003, packet behavior hardware validation |
| REQ-OPT-FW-007 | VAL-OPT-003 |
| REQ-OPT-FW-008 | VAL-OPT-004 |
| REQ-OPT-ACT-001 | VAL-OPT-001 |
| REQ-OPT-ACT-002 | VAL-OPT-002 |
| REQ-OPT-ACT-003 | VAL-OPT-002, Read Status hardware validation |
| REQ-OPT-ACT-004 | VAL-OPT-001, request-error hardware validation |
| REQ-OPT-ACT-005 | VAL-OPT-001, recognized-stub hardware validation |
| REQ-OPT-ACT-006 | VAL-OPT-001, request-error hardware validation |
| REQ-OPT-ACT-007 | VAL-OPT-004 |
