# I2C Bridge Design

## Components

| Component | Responsibility |
| --- | --- |
| USB device task | Runs Embassy USB device polling. |
| CDC session loop | Emits `READY`, gates requests on DTR, reads the CDC byte stream, writes responses, and resets on session failure. |
| Frame parser | Reassembles binary frames across CDC packets and emits complete requests in arrival order. |
| Request dispatcher | Validates request flags/payload shape and dispatches I2C read/write commands. |
| I2C transaction layer | Uses asynchronous generated I2C1 master operations and races each one against DTR-drop reset. |
| Reset path | Writes `SYSRESET` to CH32 PFIC `SCTLR` at `0xE000_ED10`, bit 31. |

## Initialization sequence

1. Configure RCC for 48 MHz USB FS device clock.
2. Initialize Embassy time runtime.
3. Enable and release-reset GPIOB.
4. Select I2C1's default pin route and configure PB6/PB7 as released,
   alternate-function open-drain outputs.
5. Enable and release-reset I2C1, then configure 100 kHz master mode.
6. Build USB descriptors and CDC ACM class with VID:PID `CAFE:4004`.
7. Spawn the Embassy USB device task.
8. Start the CDC session loop.

## Transport state machine

```text
endpoint disabled
  -> endpoints enabled
  -> send READY
  -> wait for DTR
  -> receive-and-dispatch
  -> DTR low / endpoint failure
  -> MCU reset
```

`READY\r\n` is a diagnostic line, not a binary bridge response. The bridge
does not process binary requests until DTR is high.

## Parser design

The parser state persists for the entire DTR-high session. For every byte from
each CDC packet:

1. Feed the byte to the shared protocol parser.
2. Ignore leading bytes that are not magic.
3. Retain partial state when the parser requires more bytes.
4. On a complete request, synchronously await its request dispatcher before
   consuming the next complete request.
5. On parser error, reset the parser state and continue consuming subsequent
   bytes from the same packet.

This design intentionally serializes I2C requests and their responses. It
avoids response reordering and requires no dynamic allocation or queue. The
CDC receive buffer is 64 bytes, matching the configured CDC packet size; the
parser's separate frame buffer remains 21 bytes.

## Request dispatch

The dispatcher first rejects nonzero flags with `STATUS_BAD_FLAGS`. It then
requires a nonempty payload, rejects addresses with bit 7 set using
`STATUS_BAD_COMMAND`, and applies command-specific length rules.

| Command | Valid payload | I2C operation | Response |
| --- | --- | --- | --- |
| `0x10` | `[address, bytes...]` | `write_async_7bit` | status only |
| `0x11` | `[address, count]`, `count <= 16` | `read_async_7bit` | status plus `count` bytes |
| other | any | none | `STATUS_BAD_COMMAND` |

I2C operation failure maps to `STATUS_BAD_COMMAND`, preserving existing wire
compatibility. A failed read encodes no read-buffer bytes.

## Pending-I2C recovery

Each asynchronous I2C future is raced against a DTR watcher. The watcher polls
at most every 50 ms; DTR drop does not wait for the I2C future to complete.
Instead, it resets the MCU, which clears USB, I2C, parser, and request state.

## Memory and concurrency

The bridge uses fixed descriptor buffers, a fixed 64-byte CDC receive buffer,
a fixed 21-byte frame buffer, and a 16-byte I2C read buffer. It performs no
heap allocation. One USB task and one main executor task share the generated
USB driver as intended by Embassy.

No transaction timeout, retry policy, or error-specific I2C status is
implemented. Those are deliberate non-goals for this test harness.

## Current implementation delta

**KNOWN:** `firmware/i2c-bridge/src/main.rs` resets the parser before every
CDC packet, keeps only the last complete request in a packet, sends
`STATUS_BAD_LENGTH` for an incomplete packet, includes a read buffer after a
failed read, and continues rather than resets if the initial `READY` write
fails. Its CDC receive buffer is only 21 bytes and it does not reject an
address with bit 7 set. Phase 5 must replace those paths to realize
REQ-BRIDGE-004, REQ-BRIDGE-005, REQ-BRIDGE-006, and REQ-BRIDGE-008.

## Traceability

| Design element | Requirements |
| --- | --- |
| Initialization and pins | REQ-BRIDGE-001, REQ-BRIDGE-002 |
| Frame parser and ordering | REQ-BRIDGE-003, REQ-BRIDGE-004 |
| Command dispatch | REQ-BRIDGE-005, REQ-BRIDGE-006, REQ-BRIDGE-007 |
| CDC/DTR/reset lifecycle | REQ-BRIDGE-008, REQ-BRIDGE-009 |
| Flash enforcement | REQ-BRIDGE-010 |
