# OptiBridge Firmware Requirements

## Scope

This specification describes the currently implemented CH32V203G6U6 OptiBridge
firmware only. It is an I2C slave that responds to the shared `alive` request
and runs a fixed Sonde BPF size probe at startup.

**KNOWN:** BPF loading, verification beyond the probe, maps, helpers, optical
I/O, calibration, interrupts, and status buffers are not implemented and are
out of scope.

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

### REQ-OPT-FW-006: Alive command

A request with command `0x01`, zero payload, and zero flags **MUST** return
`STATUS_OK` (`0x00`) with the same sequence and ASCII payload `alive`.

### REQ-OPT-FW-007: Request errors

Nonzero flags **MUST** return `STATUS_BAD_FLAGS` (`0x03`). An unsupported
command **MUST** return `STATUS_BAD_COMMAND` (`0x01`). An `alive` request with
a nonempty payload **MUST** return `STATUS_BAD_LENGTH` (`0x02`). Each such
response **MUST** be status-only and retain the request sequence.

### REQ-OPT-FW-008: I2C error behavior

An I2C receive error **MUST** clear parser state and return to the receive
loop. An I2C response-write error **MUST** be ignored; the implementation
continues to the next receive operation.

### REQ-OPT-FW-009: Sonde size probe

Startup **MUST** execute the fixed two-instruction Sonde BPF program that
returns `42`. If the probe fails or produces another value, firmware **MUST**
panic. This probe **MUST NOT** be represented as a BPF program loader or
runtime feature.

### REQ-OPT-FW-010: Resource constraints

The firmware **MUST** be `no_std` and allocation-free. Its release `text +
data` flash use **MUST NOT** exceed 32,768 bytes.

## Deferred functionality

The README describes a future programmable optical runtime. It is outside this
specification and has no current firmware requirements.
