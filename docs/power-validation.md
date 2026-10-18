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

## Unverified behavior

`ARISTOTLE_POWERD=1` remains opt-in. These device checks are still outstanding:

- Power-button and cover sleep and wake with the reader running.
- Idle-timer sleep and wake, including the reader's sleep-delay settings.
- Progress persistence and frontlight restoration across actual suspend.
- Repeated suspend cycles and recovery after a lost event subscription.
- Battery discharge while unplugged, compared with the stock reader under
  the same conditions.

The guard probe is not evidence for any of these checks or for lower battery
use. See [the reader's power policy](reader.md#the-app) for the default behavior.
