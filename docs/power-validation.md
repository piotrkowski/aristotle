# Power validation

## PW4 guard probe, 2026-10-18

Measured on the device with a standalone static ARM binary using the current
`aristotle-hal` power APIs. The connection used USB networking. The probe did
not run the reader, press the power button, change the frontlight, or request
suspend.

The existing reader was paused for the probe and resumed during cleanup. Its
library and preferences were not read or changed. The binary ran from
`/var/tmp` and was removed after the test.

The probe started `PowerEvents`, acquired `Screensaver` with powerd enabled,
read the power state, stopped the event listener, and restored the guard:

```text
before: state=active prevent=true
managed: state=active prevent=false
restored: state=active prevent=true
```

Blanket's loaded modules before the probe were `screensaver langpicker
blankwindow`. After restoration they were `langpicker blankwindow
screensaver`. No powerd event listener remained. The original reader resumed
in process state `S`; the probe directory was absent after cleanup. `/var`
returned to 316 KiB used, with 63.7 MiB available.

This verifies the runtime property round trip, module restoration, event-child
cleanup, and the static binary's compatibility with this device. It does not
verify sleep or wake in the reader.

## Runtime framework-readiness probe, 2026-10-18

Measured on the PW4 with a separate temporary Lua helper after stopping the stock GUI. KAF was absent after takeover. The helper used the stock Lua `liblipclua` module to hold `com.lab126.kaf` and expose read-only `frameworkStarted=1`. A second initialization of that name was rejected without disturbing the existing helper.

With the helper and the powerd-enabled reader running, the owner disconnected USB, waited 90 seconds, physically woke the device and reconnected it. Kernel suspend counters changed from success0/fail0 to success1/fail0. The app log recorded readyToSuspend, Suspending, WakeupFromSuspend, outOfScreenSaver1 and a powerd wake.

This verifies a suspend and physical wake using the temporary external helper. It does not validate the new Rust-owned helper's startup, cleanup or default launch on the device. Those paths have host tests and remain pending device validation.

## Remaining checks

Managed sleep is the default; `ARISTOTLE_POWERD=0` selects the non-suspending fallback. These checks remain outstanding:

- Normal startup and physical sleep/wake using the app-owned helper.
- Cover sleep and wake with the reader running.
- Idle-timer sleep and wake, including the reader's sleep-delay settings.
- Progress persistence and frontlight restoration across actual suspend.
- Repeated suspend cycles and recovery after a lost event subscription.
- Battery discharge while unplugged, compared with the stock reader under
  the same conditions.

Neither probe establishes lower battery use or completes these checks. See [the reader's power policy](reader.md#the-app) for fallback behavior.
