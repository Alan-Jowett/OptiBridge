# OptiBridge Firmware Validation

## Required automated validation

| ID | Requirement coverage | Method | Expected result |
| --- | --- | --- | --- |
| VAL-OPT-001 | REQ-OPT-FW-005, REQ-OPT-FW-006 | `cargo test -p optibridge-protocol` | Valid frames, framing failures, and the `alive` response pass. |
| VAL-OPT-002 | REQ-OPT-FW-007 | `cargo test -p optibridge-protocol` | Flags, unsupported command, and nonempty `alive` payload return the required status-only response. |
| VAL-OPT-003 | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-009 | `cargo build --release -p optibridge-firmware --target riscv32imc-unknown-none-elf --features firmware` | Firmware compiles and links with generated HAL and Sonde probe. |
| VAL-OPT-004 | REQ-OPT-FW-010 | `cargo xtask size` | OptiBridge release image is at or below 32,768 bytes. |

## Required hardware validation

### Alive request/response

Use a seven-bit I2C master at address `0x42`.

1. Write:

   ```text
   A5 01 00 01 00
   ```

2. Read ten bytes.
3. Expect:

   ```text
   A5 00 05 01 00 61 6C 69 76 65
   ```

### Error response

Write each complete request below and read the five-byte response:

| Request | Expected response |
| --- | --- |
| `A5 01 00 02 01` | `A5 03 00 02 00` |
| `A5 02 00 03 00` | `A5 01 00 03 00` |
| `A5 01 01 04 00 00` | `A5 02 00 04 00` |

### Packet behavior

The following cases require either an I2C master test harness with
packet-boundary control or an equivalent firmware-level test:

| Case | Expected current behavior |
| --- | --- |
| Split one valid frame across two receive packets | No response to either packet. |
| Two valid frames in one receive packet | Only the response for the second frame. |
| Parser error followed by a valid frame in the same packet | No response. |
| Receive error followed by a valid packet | The valid packet receives the normal response. |
| Response-write error | No recovery response; the slave continues waiting for the next receive packet. |

### Waveform capture

Use `docs/picoscope-i2c-debugging.md` for PicoScope 2204A setup. Confirm an
I2C START, address `0x42` ACK, request bytes, and read response bytes.

## Existing evidence

- **KNOWN:** An OptiBridge alive request/response completed through the COM9
  bridge at address `0x42`.
- **KNOWN:** The Sonde-enabled release image measured 21,756 bytes of flash.

## Deferred validation

No automated firmware-level test currently controls the I2C slave packet
boundaries or injects I2C receive/write errors. The packet-behavior cases are
therefore required hardware or future harness validation.

## Traceability

| Requirement | Validation |
| --- | --- |
| REQ-OPT-FW-001 | VAL-OPT-003 |
| REQ-OPT-FW-002 | VAL-OPT-003, alive request/response hardware validation |
| REQ-OPT-FW-003 | Packet behavior validation |
| REQ-OPT-FW-004 | Packet behavior validation |
| REQ-OPT-FW-005 | VAL-OPT-001 |
| REQ-OPT-FW-006 | VAL-OPT-001, alive request/response hardware validation |
| REQ-OPT-FW-007 | VAL-OPT-002, error response hardware validation |
| REQ-OPT-FW-008 | Packet behavior validation |
| REQ-OPT-FW-009 | VAL-OPT-003 |
| REQ-OPT-FW-010 | VAL-OPT-004 |
