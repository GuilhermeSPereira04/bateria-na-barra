# Zaopin receiver battery protocol

Receiver: `VID 3554 / PID F527` (product string "Z4D"), Compx-based. Official
software: "Zaopin Wireless Mouse" (Qt app + `HIDUsb.dll`), installed at
`D:\Zaopin Wireless Mouse`.

## How it was found (2026-11-24)

Static reading of the official driver instead of a USB capture (no extra
driver needed). The app (`Zaopin Wireless Mouse.exe`) imports
`UsbServer_ReadBatteryLevel` and `GetDeviceBatteryStatus` from `HIDUsb.dll`.

- `UsbServer_ReadBatteryLevel` / `CS_UsbFinder_GetBatteryLevel` build a 17-byte
  buffer: `[0]=0x08`, `[1]=0x04`, rest zero, `[16]=checksum(bytes 0..16)`.
  Byte 5 gets `|= 0x80` only in keyboard mode (`*_SetKeyboardMode`); the mouse
  uses mouse mode, so byte 5 stays 0.
- Checksum (`HIDUsb.dll` RVA `0x114c0`): `0x55 - sum(bytes)` (wrapping u8).
- Transport (RVA `0xdcd0`): overlapped `ReadFile` of 17 bytes posted, then
  `WriteFile` of the 17-byte request, `WaitForSingleObject` 100 ms, verify the
  reply checksum and that reply byte 1 equals the command; up to 3 tries.
- `CS_GetDeviceBatteryStatus` only unpacks the reply (`level, status, mV`); the
  app shows `level` as-is and treats `status > 0` as charging.

Collection: interface 1, usage page `0xFF02` — its descriptor declares report
`0x08` with 16 bytes in and 16 bytes out (see `zaopin-probe.txt`).

## Request / reply

```
request  08 04 00 00 00 00 00 00 00 00 00 00 00 00 00 00 49
reply    08 04 00 00 00 02 19 00 0E 3D 00 00 00 00 00 00 E3
               |        |  |  |  |                       '- checksum
               |        |  |  '--'- millivolts, big endian (0x0E3D = 3645)
               |        |  '- status: > 0 = charging
               |        '- battery percent (0x19 = 25)
               '- echoes the command
```

Observed: 25 % / 3637–3645 mV, not charging. The official app showed 23 % at
another moment; the percentage is computed by the mouse from its voltage and
moves a little with load.

On the USB cable the mouse shows up as `3554:F526` ("2.4G Dual Mode Mouse")
with the same `0xFF02` collection and answers the same query:

```
reply    08 04 00 00 00 02 28 01 0E BF 00 00 00 00 00 00 51   (40 %, charging, 3775 mV)
```

Not yet observed: the reply while the mouse is asleep (the app then shows the
held value, then `--%`).

## What the official app shows (smoothing)

The app does not show byte 6 directly. `HIDUsb.dll` (RVA `0x63f0`, enabled by
the app through `UsbServer_SetBatteryOptimizeEnable(1)`) keeps a displayed
value and moves it 1 % per tick toward the mouse's value:

- discharging: only down, 1 % per 10 s, never below the mouse value; at
  <= 15 % it snaps to 15; mouse at 0 → 0;
- charging: only up, 1 % per 10 s while more than 10 below the mouse value;
  with the mouse >= 95 and the display >= 85, 1 % per 5 min up to 99; 100 after
  10 ticks with the mouse at 100.

Seen: mouse 25 % → app 23 %; mouse 45 % charging → app 31 % (still climbing).
battery-bar applies the same walk (`zaopin::Smoother`). It starts from the
mouse value (or 10 below it while charging) because it does not have the
app's saved history.
