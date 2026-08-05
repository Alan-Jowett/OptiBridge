# OptiBridge Firmware Requirements

## Scope

This specification describes the CH32V203G6U6 OptiBridge firmware's bounded
I2C action surface. It implements Reset, exposes four remaining action stubs,
and exposes startup liveness through Read Status.

**KNOWN:** BPF loading, verification beyond the startup probe, maps, helpers,
optical I/O, calibration, interrupts, and a general status-buffer API are not
implemented and are out of scope.

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

`CMD_LOAD_BPF`, `CMD_START_BPF`, `CMD_READ_BPF_MAP`, and `CMD_WRITE_BPF_MAP`
**MUST** require zero flags and zero payload. Nonzero flags **MUST** return
`STATUS_BAD_FLAGS`; a nonempty payload **MUST** return `STATUS_BAD_LENGTH`.

### REQ-OPT-ACT-005: Remaining stub behavior

`CMD_LOAD_BPF`, `CMD_START_BPF`, `CMD_READ_BPF_MAP`, and `CMD_WRITE_BPF_MAP`
**MUST NOT** mutate runtime state, load or execute BPF, access maps, or access
optical hardware.

They **MUST** return status-only `STATUS_NOT_IMPLEMENTED` (`0x04`) and retain
the request sequence.

### REQ-OPT-ACT-006: Unknown commands

Commands outside the six defined action values **MUST** return status-only
`STATUS_BAD_COMMAND` and retain the request sequence.

### REQ-OPT-ACT-007: Deferred action semantics

BPF loading/execution, map semantics, optical behavior, additional status
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
