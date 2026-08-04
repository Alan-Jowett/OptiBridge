# I2C Bridge Validation

## Required automated validation

| ID | Requirement coverage | Method | Expected result |
| --- | --- | --- | --- |
| VAL-BRIDGE-001 | REQ-BRIDGE-003 | `cargo test -p optibridge-protocol` | Parser accepts valid frames and rejects invalid magic/length. |
| VAL-BRIDGE-002 | REQ-BRIDGE-004 | Host-side bridge parser tests | A frame split at every byte boundary produces one response; three maximum-length frames in one 63-byte input produce three ordered responses. |
| VAL-BRIDGE-003 | REQ-BRIDGE-005 to REQ-BRIDGE-007 | Dispatcher tests | Valid write/read shapes, invalid address, flags, length errors, unsupported commands, and I2C failure status mappings match the requirements. |
| VAL-BRIDGE-004 | REQ-BRIDGE-001, REQ-BRIDGE-002, REQ-BRIDGE-009 | `cargo build --release -p i2c-bridge-firmware --target riscv32imc-unknown-none-elf --features firmware` | Target image compiles and links. |
| VAL-BRIDGE-005 | REQ-BRIDGE-010 | `cargo xtask size` | Both images remain at or below 32,768 bytes. |

## Required hardware validation

### USB CDC session

1. Flash the bridge release image.
2. Open COM9 at 115200 baud and assert DTR/RTS.
3. Confirm the bridge can accept a binary request.
4. Close COM9 while I2C is idle and while an I2C transaction is pending.
5. Wait for USB CDC re-enumeration and open the next session.

Expected result: each closed session causes the bridge MCU to reset and a new
session can perform requests without a manual board reset.

### SHT40 control transaction

Use the known SHT40 address `0x44`.

1. Send bridge write frame:

   ```text
   A5 10 02 01 00 44 FD
   ```

2. Expect:

   ```text
   A5 00 00 01 00
   ```

3. Wait at least 20 ms, then send bridge read frame:

   ```text
   A5 11 02 02 00 44 06
   ```

4. Expect a response beginning:

   ```text
   A5 00 06 02 00
   ```

The final six bytes are the SHT40 measurement and CRC bytes.

### OptiBridge Read Status transaction

With the target at I2C address `0x42`:

1. Send bridge write frame:

   ```text
   A5 10 06 01 00 42 A5 06 00 01 00
   ```

2. Expect:

   ```text
   A5 00 00 01 00
   ```

3. Send bridge read frame:

   ```text
   A5 11 02 02 00 42 0A
   ```

4. Expect:

   ```text
   A5 00 0A 02 00 A5 00 05 01 00 72 65 61 64 79
   ```

The embedded target response decodes to `ready`.

### I2C waveform capture

Use `.github/skills/picoscope-i2c/SKILL.md` when electrical confirmation is
needed. Capture SDA on PicoScope channel A and SCL on channel B. Trigger on
SDA falling while SCL is high, then decode bytes on SCL rising edges.

Interpretation:

- no START: USB request did not reach I2C;
- address NACK: check address, target power, wiring, and pull-ups;
- SDA/SCL held low: inspect electrical bus state;
- valid write with no later data: inspect target command timing/state.

## Existing evidence

- **KNOWN:** SHT40 write/read at `0x44` completed successfully through COM9.
- **KNOWN:** The prior OptiBridge `alive` write/read at `0x42` completed
  successfully through COM9; Read Status requires the updated hardware
  validation above.
- **KNOWN:** The PicoScope 2204A procedure is documented and verified with legacy
  `ps2000.dll`.

## Deferred validation

The protocol unit suite covers byte-stream fragmentation, coalescing, and
resynchronization. End-to-end CDC and I2C validation remains pending until the
new bridge image is flashed and the required hardware validation is performed.

## Traceability

| Requirement | Validation |
| --- | --- |
| REQ-BRIDGE-001 | VAL-BRIDGE-004 |
| REQ-BRIDGE-002 | VAL-BRIDGE-004 |
| REQ-BRIDGE-003 | VAL-BRIDGE-001, VAL-BRIDGE-002 |
| REQ-BRIDGE-004 | VAL-BRIDGE-002 |
| REQ-BRIDGE-005 | VAL-BRIDGE-003 |
| REQ-BRIDGE-006 | VAL-BRIDGE-003 |
| REQ-BRIDGE-007 | VAL-BRIDGE-003 |
| REQ-BRIDGE-008 | USB CDC session hardware validation |
| REQ-BRIDGE-009 | VAL-BRIDGE-004 |
| REQ-BRIDGE-010 | VAL-BRIDGE-005 |
