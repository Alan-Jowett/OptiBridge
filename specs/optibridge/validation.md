# OptiBridge Firmware Validation

## Required automated validation

| ID | Requirement coverage | Method | Expected result |
| --- | --- | --- | --- |
| VAL-OPT-001 | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-011, REQ-OPT-ACT-001, REQ-OPT-ACT-004 to REQ-OPT-ACT-006 | `cargo test -p optibridge-protocol` | Each action command has its documented status-only response. Packet dispatch rejects malformed, incomplete, and truncated captures; it dispatches only the final complete frame and preserves status until a valid Read Status request. |
| VAL-OPT-002 | REQ-OPT-ACT-002, REQ-OPT-ACT-003 | `cargo test -p optibridge-protocol` | Read Status returns `ready` with the request sequence, then consumes it; the next read returns `STATUS_OK` with an empty payload. |
| VAL-OPT-003 | REQ-OPT-FW-001, REQ-OPT-FW-002, REQ-OPT-FW-006, REQ-OPT-FW-007 | `cargo build --release -p optibridge-firmware --target riscv32imc-unknown-none-elf --features firmware` | Firmware compiles and links with generated HAL, status storage, and Sonde probe. |
| VAL-OPT-004 | REQ-OPT-FW-008, REQ-OPT-ACT-007 | `cargo xtask size` | OptiBridge release image is at or below 32,768 bytes. |
| VAL-OPT-005 | REQ-OPT-FW-009 to REQ-OPT-FW-011 | Hardware through the CDC-I2C bridge | One target reset is followed by two Read Status requests and all five stub request/read pairs; every response completes without another target reset. |
| VAL-OPT-006 | REQ-OPT-FW-003, REQ-OPT-FW-004, REQ-OPT-FW-006 | Hardware through the CDC-I2C bridge | An incomplete or bad-length request queues no protocol response; the immediately following valid request succeeds. |
| VAL-OPT-007 | REQ-OPT-FW-013 | Hardware through the CDC-I2C bridge | Two valid writes without an intervening read leave only the later response in the generated slot. |
| VAL-OPT-008 | REQ-OPT-FW-006, REQ-OPT-FW-012 | Hardware through the CDC-I2C bridge | An unread valid response followed by a bad-length request yields zero filler, then a valid request succeeds. |

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

4. Without resetting the target, repeat with a new sequence. Expect:

   ```text
   A5 00 00 02 00
   ```

### Recognized stubs

After the two Read Status transactions, write each request and read the
five-byte response without resetting the target:

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

### Response-slot ordering

1. Write `A5 01 00 0A 00`; do not read.
2. Write `A5 02 00 0B 00`; do not read.
3. Read five bytes. Expect `A5 04 00 0B 00`.

### Empty response-slot replacement

1. Write `A5 01 00 0A 00`; do not read.
2. Write the bad-length header `A5 06 11 0B 00`.
3. Read five bytes. Expect non-protocol filler `00 00 00 00 00`.
4. Write `A5 02 00 0C 00`, read five bytes, and expect
   `A5 04 00 0C 00`.

### Packet behavior and waveform capture

Use `docs/picoscope-i2c-debugging.md` to confirm I2C START, address `0x42`
ACK, request bytes, and expected response bytes. An incomplete packet followed
by a valid request is required hardware validation. Multiple frames are tested
only when their combined bytes fit in the 21-byte RX capture bound.

## Existing evidence

- **KNOWN:** The Sonde-enabled release image measured 21,756 bytes of flash.

## Deferred validation

No automated firmware-level test controls I2C slave packet boundaries or
injects I2C errors. A direct I2C master must validate an oversized packet after
an unread response: the next read must yield zero filler and a later valid
request must succeed. The bridge cannot perform this test because it writes at
most 15 target bytes per transaction.

## Traceability

| Requirement | Validation |
| --- | --- |
| REQ-OPT-FW-001 | VAL-OPT-003 |
| REQ-OPT-FW-002 | VAL-OPT-003, Read Status hardware validation |
| REQ-OPT-FW-003 | VAL-OPT-001, VAL-OPT-006 |
| REQ-OPT-FW-004 | VAL-OPT-001, VAL-OPT-006 |
| REQ-OPT-FW-005 | VAL-OPT-001, VAL-OPT-002 |
| REQ-OPT-FW-006 | VAL-OPT-003, VAL-OPT-006, VAL-OPT-008 |
| REQ-OPT-FW-007 | VAL-OPT-003 |
| REQ-OPT-FW-008 | VAL-OPT-004 |
| REQ-OPT-FW-009 | VAL-OPT-005 |
| REQ-OPT-FW-010 | VAL-OPT-003, VAL-OPT-005 |
| REQ-OPT-FW-011 | VAL-OPT-001, VAL-OPT-005 |
| REQ-OPT-FW-012 | VAL-OPT-008 |
| REQ-OPT-FW-013 | VAL-OPT-007 |
| REQ-OPT-ACT-001 | VAL-OPT-001 |
| REQ-OPT-ACT-002 | VAL-OPT-002 |
| REQ-OPT-ACT-003 | VAL-OPT-002, Read Status hardware validation |
| REQ-OPT-ACT-004 | VAL-OPT-001, request-error hardware validation |
| REQ-OPT-ACT-005 | VAL-OPT-001, recognized-stub hardware validation |
| REQ-OPT-ACT-006 | VAL-OPT-001, request-error hardware validation |
| REQ-OPT-ACT-007 | VAL-OPT-004 |
