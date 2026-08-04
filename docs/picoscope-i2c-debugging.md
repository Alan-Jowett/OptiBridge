# PicoScope I2C Debugging

This document records the working procedure for capturing OptiBridge I2C
traffic with a PicoScope 2204A on Windows.

## SDK and API selection

The installed SDK is under:

```text
C:\Program Files\Pico Technology\SDK
```

Although the scope model is named `2204A`, this environment exposes it through
the legacy `ps2000.dll` API. The newer `ps2000a.dll` API returns
`PICO_NOT_FOUND (0x03)` for this device.

The legacy open function has a different signature from the newer API:

```c
int16_t ps2000_open_unit(void);
```

It takes no arguments and returns the device handle directly. A positive return
value is a valid handle. Do not pass a handle pointer or interpret the return
value as a PicoSDK status code.

Example using Python `ctypes`:

```python
import ctypes

api = ctypes.WinDLL(
    r"C:\Program Files\Pico Technology\SDK\lib\ps2000.dll"
)
api.ps2000_open_unit.argtypes = []
api.ps2000_open_unit.restype = ctypes.c_int16

handle = api.ps2000_open_unit()
if handle <= 0:
    raise RuntimeError(f"ps2000_open_unit failed: {handle}")
```

The device can be identified with `ps2000_get_unit_info`. The working device
reports model `2204A`.

The `picosdk` Python package may be installed for the newer APIs, but the
legacy capture path should use direct `ctypes` bindings to `ps2000.dll`.

## Probe wiring

Connect the scope as follows:

| PicoScope | Signal |
| --- | --- |
| Channel A | I2C SDA |
| Channel B | I2C SCL |
| Ground | Shared ground between both MCUs |

For the CH32V203G6U6 setup, I2C1 uses:

- PB7: SDA
- PB6: SCL

Use DC coupling and a voltage range that includes the 3.3 V logic level, such
as the 5 V range. The decoder threshold should be approximately 1.65 V.

## Capture procedure

1. Connect the PicoScope and probes.
2. Open the device with `ps2000_open_unit()` and retain the positive handle.
3. Enable channels A and B.
4. Configure a block capture with a trigger on SDA falling while SCL is high.
   This corresponds to the I2C START condition.
5. Arm the capture before triggering the bridge.
6. Reset the bridge if its previous blocking I2C transaction left it wedged.
7. Assert DTR on COM9 and send the bridge I2C request.
8. Wait for the capture to complete, then decode SDA on SCL rising edges.

The 2204A may report a relatively small maximum sample count at the selected
timebase when both channels are enabled. Respect the reported limit instead of
assuming a large buffer.

## Expected bridge transaction

The bridge protocol uses compact binary frames with magic byte `0xA5`.
To check OptiBridge liveness, the bridge must perform two I2C operations:

1. Write the Read Status request to address `0x42`:

   ```text
   A5 06 00 01 00
   ```

2. Read the response from address `0x42`. The expected response is:

   ```text
   A5 00 05 01 00 72 65 61 64 79
   ```

When sent through the USB bridge, the host sends a bridge-write frame first,
then a bridge-read frame. The bridge uses asynchronous HAL I2C operations and
resets when DTR drops, so each COM9 session starts from a clean MCU state.

## Interpreting a stalled capture

If the target does not acknowledge address `0x42`, or if SDA/SCL is held low,
the bridge keeps the I2C operation pending until its host session closes. Drop
DTR or close COM9 to reset the bridge before starting another capture.

Use the PicoScope trace to distinguish:

- no START: bridge did not reach the I2C operation;
- START and address followed by NACK: target address, wiring, pull-up, or
  target firmware issue;
- SDA/SCL held low: bus electrical or peripheral state issue;
- complete write followed by no read: target slave state-machine or repeated
  transaction issue.
