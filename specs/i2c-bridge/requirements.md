# I2C Bridge Requirements

## Purpose and scope

The I2C bridge is a CH32V203G6U6 test-harness firmware image that exposes an
I2C1 master through USB CDC ACM. It is not an OptiBridge production runtime,
an I2C slave, or a general-purpose host API.

This specification governs USB CDC session behavior, binary framing, I2C
master transactions, recovery, and validation for the bridge.

## Requirements

### REQ-BRIDGE-001: Platform and role

The firmware **MUST** run on CH32V203G6U6 and provide a USB CDC ACM to I2C1
master bridge. It **MUST NOT** implement concurrent or multi-master I2C
arbitration policy.

### REQ-BRIDGE-002: I2C electrical configuration

The firmware **MUST** enable and release-reset GPIOB, configure PB6/SCL and
PB7/SDA for I2C1's default route as 50 MHz alternate-function open-drain
outputs, and release both lines high before enabling I2C1 master mode.

I2C1 **MUST** run at 100 kHz.

### REQ-BRIDGE-003: Frame format

USB CDC transport data **MUST** use the shared binary frame layout:

```text
byte 0: magic = 0xA5
byte 1: command (request) or status (response)
byte 2: payload length, 0 through 16
byte 3: sequence
byte 4: flags
bytes 5..: payload
```

The maximum request or response payload is 16 bytes. Request flags are
reserved and **MUST** be zero. Responses **MUST** set flags to zero.

### REQ-BRIDGE-004: CDC byte-stream handling

The bridge **MUST** treat CDC OUT data as a byte stream, rather than requiring
USB packet boundaries to coincide with frames.

It **MUST**:

- retain an incomplete frame across CDC packets;
- process each complete frame, in order, when one CDC packet contains multiple
  frames;
- accept all bytes from one configured 64-byte CDC packet, including up to
  three maximum-length frames;
- discard bytes before a magic byte without generating a response;
- reset only the affected parser state when an invalid declared payload length
  is encountered, allowing a later magic byte to re-synchronize parsing.

The bridge processes one I2C transaction at a time. It **MUST NOT** begin a
later complete frame's I2C operation before responding to its predecessor.

### REQ-BRIDGE-005: I2C write command

Request command `0x10` **MUST** perform a 7-bit I2C write. Its payload is:

```text
[address, write-byte-0, ...]
```

`address` is a seven-bit address in the range `0x00..0x7F`. An address-only
payload is valid and represents a zero-byte write. A successful transaction
returns a status-only response with `STATUS_OK` and the request sequence.
An address with bit 7 set is invalid and **MUST** return `STATUS_BAD_COMMAND`.

### REQ-BRIDGE-006: I2C read command

Request command `0x11` **MUST** perform a 7-bit I2C read. Its payload is:

```text
[address, count]
```

`count` is 0 through 16. A successful response contains exactly `count` I2C
bytes and preserves the request sequence. A failed read **MUST** return a
status-only response and **MUST NOT** expose read-buffer contents.

### REQ-BRIDGE-007: Status and malformed requests

The bridge **MUST** use these response status values:

| Status | Value | Meaning |
| --- | ---: | --- |
| `STATUS_OK` | `0x00` | Transaction or request succeeded. |
| `STATUS_BAD_COMMAND` | `0x01` | Unsupported command or I2C operation failure. |
| `STATUS_BAD_LENGTH` | `0x02` | Unsupported command payload shape or declared payload longer than 16 bytes. |
| `STATUS_BAD_FLAGS` | `0x03` | Nonzero request flags. |

An incomplete frame produces no response until it completes. A complete request
with invalid flags, unsupported command, or invalid command payload produces
one status-only response with its sequence. A malformed frame detected before
a reliable sequence is available may be discarded without a response.

### REQ-BRIDGE-008: Session lifecycle and recovery

After USB CDC endpoints are enabled, the bridge **MUST** send the diagnostic
line `READY\r\n`. It **MUST** wait for host DTR before accepting requests.

The bridge **MUST** poll DTR at an interval no greater than 50 ms while idle
and while an I2C operation is pending. If DTR drops, or any CDC endpoint read
or write fails, including the diagnostic write, the firmware **MUST** request
an MCU software reset through the CH32 PFIC system-control register.

The reset **MUST** return all bridge session state to startup state. The host
is responsible for waiting until USB CDC re-enumerates before opening the next
session.

### REQ-BRIDGE-009: Asynchronous I2C execution

The bridge **MUST** use the generated asynchronous I2C1 master operations for
read and write transactions. It **MUST NOT** use a blocking I2C operation in
the CDC request path.

No independent I2C timeout is defined. DTR drop and the resulting MCU reset
are the required recovery mechanism for a transfer that remains pending.

### REQ-BRIDGE-010: Resource limit

The release image's flash consumption (`text + data`) **MUST NOT** exceed
32,768 bytes. The existing `cargo xtask size` enforcement is normative.

## Non-goals

- I2C slave behavior or OptiBridge Read Status semantics.
- USB CDC line coding behavior.
- I2C bus recovery without resetting the MCU.
- Host-side SDK, CLI, retry, or device-discovery policy.
- Production-grade multiplexing or concurrent transaction scheduling.
