# OptiBridge Firmware Requirements

## Scope

This specification describes the CH32V203G6U6 OptiBridge firmware's bounded
I2C action surface. It implements Reset and Load BPF, exposes three remaining
action stubs, and exposes startup liveness through Read Status.

**KNOWN:** BPF execution, verification beyond the startup probe, map access,
helpers, optical I/O, calibration, interrupts, and a general status-buffer API
are not implemented and are out of scope. This specification adds only the
flash-resident BPF image loader and image-CRC query surface.

## Requirements

### REQ-OPT-FW-001: Platform and generated HAL

The firmware **MUST** target CH32V203G6U6 and consume the externally generated
HardwareAbstractionIR HAL. It **MUST NOT** vendor HAL source code.

The HAL generator input and source revision **MUST** be pinned to
`60e0de038210008018d0168f45854d113a5964cc` and regenerate the CH32V203G6U6
HAL from that pin.

### REQ-OPT-FW-002: Startup and I2C electrical configuration

Startup **MUST** configure the 48 MHz USB FS device clock and Embassy time
runtime, enable and release-reset GPIOB and I2C1 slave resources, and set
PB6/SCL and PB7/SDA high before configuring them as 50 MHz alternate-function
open-drain outputs on I2C1's default route.

The firmware **MUST** configure I2C1 as a slave with own seven-bit address
`0x42`.

### REQ-OPT-FW-003: Packet-scoped request parsing

The firmware **MUST** process each completed I2C receive capture independently.
It **MUST NOT** carry a partial frame from one capture to a later capture.

An incomplete capture **MUST** replace the response slot with an empty packet.

### REQ-OPT-FW-004: Multiple frames and malformed input

When one non-truncated I2C receive capture contains multiple complete valid
frames, the firmware **MUST** dispatch only the last complete frame.

On a parser error or a truncated capture, the firmware **MUST** abandon the
remaining bytes and replace the response slot with an empty packet.

### REQ-OPT-FW-005: Shared frame format

Requests and responses **MUST** use the shared frame format:

```text
byte 0: magic = 0xA5
byte 1: command (request) or status (response)
byte 2: payload length, 0 through 16
byte 3: sequence
byte 4: flags
bytes 5..: payload
```

Request flags are reserved and **MUST** be zero. Responses **MUST** set flags
to zero.

### REQ-OPT-FW-006: I2C error behavior

For every completed I2C receive capture, the firmware **MUST** replace the
single response slot with either one bounded protocol response or an empty
packet. A completed malformed, incomplete, or truncated capture **MUST** use
an empty packet.

Valid Reset is the exception: it **MUST** initiate reset before queuing a
response.

Low-level I2C bus-error paths that do not deliver a completed capture are
deferred; this change **MUST NOT** introduce bus recovery or a response-slot
clear API outside the generated HAL.

### REQ-OPT-FW-007: Sonde size probe

Startup **MUST** execute the fixed two-instruction Sonde BPF program that
returns `42`. If the probe fails or produces another value, firmware **MUST**
panic. This probe **MUST NOT** be represented as a BPF program loader or
runtime feature.

### REQ-OPT-FW-008: Resource constraints

The firmware **MUST** be `no_std` and allocation-free. Its release `text +
data` flash use **MUST NOT** exceed 32,768 bytes.

The firmware image **MUST** link entirely below flash offset `0x6000`, reserving
`0x6000..=0x7fff` exclusively for BPF image storage. All static RAM, runtime
stack, 1,024-byte map backing store, and Sonde interpreter execution stack
**MUST** fit in the 10,240-byte RAM region. The final link image **MUST**
reserve at least 4,096 bytes of stack for Sonde execution.

### REQ-OPT-FW-009: Repeated transaction availability

After startup, the I2C slave **MUST** accept consecutive master
write/read-response cycles at address `0x42` without an OptiBridge reset.
After a master terminates a response read with STOP or NACK, the slave **MUST**
remain available for the next write.

### REQ-OPT-FW-010: Bounded ISR dispatch

The firmware **MUST** register the generated I2C RX-packet ISR dispatcher with
a static receive buffer of `MAX_FRAME` bytes. Its callback **MUST NOT**
allocate, await, or use unbounded storage. Responses **MUST NOT** exceed
`MAX_FRAME`, which fits within the generated 32-byte single response slot.

### REQ-OPT-FW-011: Dispatch ordering

For a valid Read Status capture, the firmware **MUST** consume the stored
status before queuing the response. Invalid requests **MUST NOT** consume the
stored status.

### REQ-OPT-FW-012: Empty response behavior

An empty response slot is not a protocol response. A master read after an
empty packet **MUST NOT** be interpreted as a shared frame; the generated HAL
may transmit zero filler bytes.

### REQ-OPT-FW-013: Single-slot response ordering

The I2C master **MUST** complete its response-read transaction before writing
the next OptiBridge command. The firmware **MUST** maintain at most one queued
response. If a later valid command arrives before the prior response is read,
the later response **MUST** replace the unread response. The firmware **MUST
NOT** introduce a multi-entry response queue.

### REQ-OPT-ACT-001: Action command identifiers

`CMD_ALIVE` **MUST NOT** be present. The shared protocol **MUST** define the
following action commands:

| Command | Value |
| --- | ---: |
| `CMD_RESET` | `0x01` |
| `CMD_LOAD_BPF` | `0x02` |
| `CMD_START_BPF` | `0x03` |
| `CMD_READ_BPF_MAP` | `0x04` |
| `CMD_WRITE_BPF_MAP` | `0x05` |
| `CMD_READ_STATUS` | `0x06` |
| `CMD_QUERY_BPF_CRC` | `0x07` |

### REQ-OPT-ACT-002: Status storage

Startup **MUST** initialize fixed-capacity, allocation-free status storage with
the ASCII message `ready`. Status storage **MUST** retain only its newest
message and its message **MUST NOT** exceed 16 bytes.

### REQ-OPT-ACT-003: Read Status

`CMD_READ_STATUS` **MUST** require zero flags and zero payload. It **MUST**
return `STATUS_OK`, retain the request sequence, and include the newest status
while removing it from status storage. When no status is available, it **MUST**
return `STATUS_OK` with an empty payload.

The initial response payload **MUST** be ASCII `ready`.

### REQ-OPT-ACT-004: Stub request validation

`CMD_START_BPF`, `CMD_READ_BPF_MAP`, and `CMD_WRITE_BPF_MAP` **MUST** require
zero flags and zero payload. Nonzero flags **MUST** return `STATUS_BAD_FLAGS`;
a nonempty payload **MUST** return `STATUS_BAD_LENGTH`.

### REQ-OPT-ACT-005: Remaining stub behavior

`CMD_START_BPF`, `CMD_READ_BPF_MAP`, and `CMD_WRITE_BPF_MAP` **MUST NOT**
mutate runtime state, load or execute BPF, access maps, or access optical
hardware.

They **MUST** return status-only `STATUS_NOT_IMPLEMENTED` (`0x04`) and retain
the request sequence.

### REQ-OPT-ACT-006: Unknown commands

Commands outside the six defined action values **MUST** return status-only
`STATUS_BAD_COMMAND` and retain the request sequence.

### REQ-OPT-ACT-007: Deferred action semantics

BPF execution, map access semantics, optical behavior, additional status
enqueue sources, and a general circular-status-buffer API **MUST NOT** be
introduced by this change.

### REQ-OPT-ACT-008: Immediate Reset

`CMD_RESET` **MUST** require zero flags and zero payload. A valid request
**MUST** request generated HAL `interrupt::system_reset()` after its I2C ISR
returns and **MUST NOT** queue a protocol response. A master **MUST NOT** read
a target response after a valid Reset request and **MUST** wait at least one
second before its next I2C request.

### REQ-OPT-ACT-009: Invalid Reset

Reset with nonzero flags **MUST** return `STATUS_BAD_FLAGS`. Reset with zero
flags and nonempty payload **MUST** return `STATUS_BAD_LENGTH`. Invalid Reset
requests **MUST NOT** reset the MCU or consume status.

### REQ-OPT-FW-014: Reset reinitialization

The firmware **MUST** use generated HAL `interrupt::system_reset()` rather
than direct PFIC MMIO. After restart, it **MUST** restore I2C address `0x42`
and the initial `ready` status. It **MUST NOT** add reset acknowledgments,
delay-based reset handling, allocation, or a reset-recovery subsystem.

### REQ-OPT-BPF-001: Reserved image storage

The firmware **MUST** reserve the two 4,096-byte flash pages at offsets
`0x6000..=0x7fff` for a single BPF image and **MUST NOT** link executable or
read-only firmware data into that range. A valid image **MUST** have a
16-byte header followed by canonical image bytes: bytecode first, then map
descriptors in map-index order.

The 16-byte header **MUST** encode `OBPF` magic at bytes `0..4`, format
version `1` at byte `4`, map count at byte `5`, bytecode length as a
little-endian `u16` at bytes `6..8`, CRC-32/ISO-HDLC as a little-endian `u32`
at bytes `8..12`, commit marker as a little-endian `u16` at bytes `12..14`,
and erased reserved bytes at `14..16`. CRC-32 **MUST** use polynomial
`0x04C11DB7`, reflected input/output, initial value `0xFFFFFFFF`, and final
XOR `0xFFFFFFFF`. The commit marker **MUST** be `0xFFFF` until all image bytes
and the CRC have been validated, then be programmed to `0x0000` last.
Firmware **MUST** treat an erased, malformed, or CRC-mismatched header as no
image.

### REQ-OPT-BPF-002: Image bounds and map definitions

Bytecode **MUST** be nonempty, eight-byte aligned, and no more than 7,680
bytes (960 BPF instructions). An image **MUST** define no more than eight
maps. Each canonical map descriptor **MUST** be 16 little-endian bytes:
`map_type`, `key_size`, `value_size`, and `max_entries`, each a `u32`.

Only Sonde `BPF_MAP_TYPE_ARRAY` (`map_type = 1`) with `key_size = 4` **MUST**
be accepted. Every definition **MUST** have nonzero `value_size` and
`max_entries`; the checked sum of `value_size * max_entries` across all maps
**MUST NOT** exceed 1,024 bytes. Definitions that overflow, exceed image
storage, or violate these constraints **MUST** be rejected.

### REQ-OPT-BPF-003: Load BPF command format

`CMD_LOAD_BPF` **MUST** require zero request flags. Its first payload byte
**MUST** select one of these operations:

| Operation | Value | Payload after operation byte |
| --- | ---: | --- |
| Begin | `0x00` | bytecode length (`u16` little-endian), map count (`u8`), expected CRC-32 (`u32` little-endian) |
| Data | `0x01` | canonical-image offset (`u16` little-endian), 2 through 12 data bytes |
| Finalize | `0x02` | Empty |

Data-byte counts **MUST** be even. Begin **MUST** validate declared bounds
before accepting a transfer. Data offsets **MUST** equal the next expected
canonical-image offset; duplicate, skipped, or overlapping data **MUST** be
rejected. Finalize **MUST** require exactly all declared image bytes.

### REQ-OPT-BPF-004: Flash write discipline

The loader **MUST** erase each required 4,096-byte reserved flash page at
most once per load attempt, before programming its first image byte. It
**MUST** program image data only in ascending, two-byte-aligned order and
**MUST NOT** erase a page per I2C fragment or rewrite programmed bytes.

The loader **MUST NOT** require a page-sized RAM buffer. Erase, program,
header validation, and CRC calculation **MUST NOT** run in the I2C callback.

### REQ-OPT-BPF-005: Deferred command completion

The I2C callback **MUST** copy at most one valid Load BPF request into
fixed-capacity pending state and return `STATUS_OK` only to acknowledge
acceptance. The main loop **MUST** perform the associated flash operation.
While an operation is pending, further Load BPF requests **MUST** return
`STATUS_BUSY`; such requests **MUST NOT** change loader state. A flash
failure **MUST** be retained until the next Begin or reset and exposed by
`CMD_QUERY_BPF_CRC`.

### REQ-OPT-BPF-006: Load state and reset

Before BPF execution begins, Begin **MAY** replace a previously committed
image. A failed or incomplete attempt **MUST NOT** be marked committed or
executed. Reset **MUST** preserve a committed flash image and its CRC while
clearing volatile loader and running state.

When Start BPF is later implemented and successfully transitions an image to
running, Load BPF **MUST** return `STATUS_BAD_STATE` until reset. The current
Start BPF stub **MUST NOT** transition this state.

### REQ-OPT-BPF-007: Image CRC query

`CMD_QUERY_BPF_CRC` **MUST** require zero flags and an empty payload. If a
flash operation is pending, it **MUST** return `STATUS_BUSY`. If no committed
valid image exists, it **MUST** return `STATUS_NO_PROGRAM`. If the last
operation failed, it **MUST** return `STATUS_FLASH_ERROR` or
`STATUS_BAD_CRC`, as applicable. Otherwise it **MUST** return `STATUS_OK`
with the committed four-byte CRC-32 in little-endian order.

The CRC query **MUST NOT** mutate flash, map backing storage, status storage,
or loader/running state. Responses **MUST** retain the request sequence.

### REQ-OPT-BPF-008: Sonde compatibility boundary

The loader **MUST** preserve Sonde interpreter prerequisites for later
execution: eight-byte instruction alignment and non-aliasing, live map-region
bounds derived from the committed array-map descriptors. It **MUST NOT**
implement Sonde CBOR encoding, helper registration, verification, map initial
data, read-only maps, or execution in this change.

### REQ-OPT-BPF-009: Loader statuses

The shared protocol **MUST** define these additional response statuses:

| Status | Value |
| --- | ---: |
| `STATUS_BUSY` | `0x05` |
| `STATUS_BAD_STATE` | `0x06` |
| `STATUS_BAD_CRC` | `0x07` |
| `STATUS_FLASH_ERROR` | `0x08` |
| `STATUS_NO_PROGRAM` | `0x09` |
