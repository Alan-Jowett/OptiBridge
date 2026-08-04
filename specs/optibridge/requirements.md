# OptiBridge Firmware Requirements

## Scope

This specification describes the CH32V203G6U6 OptiBridge firmware's bounded
I2C action surface. It exposes six README-defined actions as no-side-effect
stubs and exposes startup liveness through Read Status.

**KNOWN:** BPF loading, verification beyond the startup probe, maps, helpers,
optical I/O, calibration, interrupts, and a general status-buffer API are not
implemented and are out of scope.

## Requirements

### REQ-OPT-FW-001: Platform and generated HAL

The firmware **MUST** target CH32V203G6U6 and consume the externally generated
HardwareAbstractionIR HAL. It **MUST NOT** vendor HAL source code.

### REQ-OPT-FW-002: Startup and I2C electrical configuration

Startup **MUST** configure the 48 MHz USB FS device clock and Embassy time
runtime, enable and release-reset GPIOB and I2C1 slave resources, and set
PB6/SCL and PB7/SDA high before configuring them as 50 MHz alternate-function
open-drain outputs on I2C1's default route.

The firmware **MUST** configure I2C1 as a slave with own seven-bit address
`0x42`.

### REQ-OPT-FW-003: Packet-scoped request parsing

The firmware **MUST** reset shared-frame parser state before processing each
successful I2C receive packet and after an I2C receive error. It **MUST NOT**
carry a partial frame from one received I2C packet to a later packet.

An incomplete frame produces no response.

### REQ-OPT-FW-004: Multiple frames and malformed input

When one I2C receive packet contains multiple complete valid frames, the
firmware **MUST** dispatch only the last complete frame.

On a parser error, the firmware **MUST** reset parser state, abandon the
remaining bytes of that I2C receive packet, and produce no response for that
packet.

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

An I2C receive error **MUST** clear parser state and return to the receive
loop. An I2C response-write error **MUST** be ignored; the implementation
continues to the next receive operation.

### REQ-OPT-FW-007: Sonde size probe

Startup **MUST** execute the fixed two-instruction Sonde BPF program that
returns `42`. If the probe fails or produces another value, firmware **MUST**
panic. This probe **MUST NOT** be represented as a BPF program loader or
runtime feature.

### REQ-OPT-FW-008: Resource constraints

The firmware **MUST** be `no_std` and allocation-free. Its release `text +
data` flash use **MUST NOT** exceed 32,768 bytes.

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
without consuming it.

The initial response payload **MUST** be ASCII `ready`.

### REQ-OPT-ACT-004: Stub request validation

`CMD_RESET`, `CMD_LOAD_BPF`, `CMD_START_BPF`, `CMD_READ_BPF_MAP`, and
`CMD_WRITE_BPF_MAP` **MUST** require zero flags and zero payload. Nonzero flags
**MUST** return `STATUS_BAD_FLAGS`; a nonempty payload **MUST** return
`STATUS_BAD_LENGTH`.

### REQ-OPT-ACT-005: Stub behavior

The five non-status commands **MUST NOT** reset the MCU, mutate runtime state,
load or execute BPF, access maps, or access optical hardware.

They **MUST** return status-only `STATUS_NOT_IMPLEMENTED` (`0x04`) and retain
the request sequence.

### REQ-OPT-ACT-006: Unknown commands

Commands outside the six defined action values **MUST** return status-only
`STATUS_BAD_COMMAND` and retain the request sequence.

### REQ-OPT-ACT-007: Deferred action semantics

Actual reset behavior, BPF loading/execution, map semantics, optical behavior,
additional status enqueue sources, and a general circular-status-buffer API
**MUST NOT** be introduced by this change.
