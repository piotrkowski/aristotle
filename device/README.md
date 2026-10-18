# Device-side code

Everything here but the tests runs on the Kindle, under its BusyBox shell, which provides compact versions of standard Linux commands.
The host-side tools that install and drive it are in [`tools/`](../tools/), and the app
itself is the `aristotle` crate.

For installation and daily use, start with the [user documentation](../docs/README.md).
This page describes device scripts, host access, and measured behavior.

| Path | What | Installed to |
|------|------|--------------|
| `extension/` | The [KUAL](../docs/glossary.md#kual) launcher extension: the menu, the manual USB network, the device guard | `/mnt/us/extensions/aristotle/` |
| `extension/agent/` | The agent: three [Upstart](../docs/glossary.md#upstart) service jobs (the agent, Aristotle at boot, the [OTA](../docs/glossary.md#ota) update block), their scripts, `aristotlectl` and the Wi-Fi policy | `/etc/upstart/aristotle-{agent,ui,ota}.conf` and `/var/local/aristotle/` |
| `app/` | `run-aristotle.sh` and `takeover.sh`, which launch the app through a [takeover](../docs/glossary.md#takeover) of the stock UI | `/mnt/us/aristotle/`, in the app bundle |
| `probe/` | A read-only probe, a KUAL extension | `/mnt/us/extensions/aristotle-probe/` |
| `tests/` | Host-side tests of the scripts above, against a fake device root | stays on the host |

`tools/package.sh` defines what goes on the USB drive: `aristotle/` (the app
bundle from `tools/build-render.sh device`) and `extensions/aristotle/` (the
extension with `takeover.sh`). The release zip holds exactly that.
`tools/host/install-extensions.sh` copies the same tree (the app only if it is
built), the probe and the dev-loop public key to a Kindle in [drive mode](../docs/glossary.md#drive-mode), then
ejects it. Earlier versions installed the extension
to `/mnt/us/extensions/aristotle-usbnet/`; the install script removes that
folder.

No recovery path is verified for the PW4. Writes to the [root filesystem](../docs/glossary.md#rootfs) are limited to installing or removing the three Upstart jobs. Everything else lives in `/var/local/aristotle`, on `/mnt/us`, or in runtime state that a reboot clears.

## Host setup

These steps use a Linux host with NetworkManager (`nmcli`) and OpenSSH 8.4
or later. The Kindle needs KOReader's `dropbear` for agent installation or
manual USB networking. Install the [release bundle](../docs/install.md) first.
No internet connection is needed for the USB link.

1. From the repository, create a dedicated key in the project cache:

   ```sh
   . tools/env.sh
   mkdir -p "$ARISTOTLE_CACHE/ssh"
   ssh-keygen -t ed25519 -f "$ARISTOTLE_CACHE/ssh/kindle_ed25519"
   ```

   Keep an existing dedicated key if you have one. Do not overwrite it or
   reuse a key for another machine.

2. Connect the Kindle in USB drive mode. Copy
   `$ARISTOTLE_CACHE/ssh/kindle_ed25519.pub` to
   `extensions/aristotle/authorized_keys` on the drive. Copy only the public
   key, not `kindle_ed25519`.

   The manual USB-network option replaces KOReader's
   `settings/SSH/authorized_keys` with this file. Save the existing file on
   the computer first if it contains other keys you need.

3. Eject the drive and choose **KUAL > Aristotle > Install agent** for
   persistent access, or **Start USB network once (manual)** for this boot.
4. Keep the cable connected. Configure the host profile:

   ```sh
   tools/host/setup-host-net.sh
   ```

5. Connect through the repository's SSH configuration:

   ```sh
   ssh -F tools/host/ssh_config kindle
   ```

The host uses `192.168.15.201/24`; the Kindle uses `192.168.15.244/24`.
The connection has no gateway or DHCP. `ssh_config` uses the dedicated key
and saves host keys in `$ARISTOTLE_CACHE/known_hosts`. Source `tools/env.sh`
again in each new host shell before using that configuration.

For a development build, `tools/host/install-extensions.sh` copies the same
public key, the built app bundle, and the probe to a mounted Kindle. It uses
`$KINDLE_MOUNT` if the drive is not mounted at its default location. Build the
bundle first as described in [Contributing](../.github/CONTRIBUTING.md#building).

For build, copy, run, and screen capture in one command, use
`tools/host/run.sh PACKAGE[:BINARY] [ARGS]`. For a connection failure, check
[USB and SSH troubleshooting](../docs/troubleshooting.md#usb-drive-mode-and-ssh)
and [address conflicts](#the-usb-link).

## The device guard

`extension/guard.sh` is sourced by every script that changes the device
outside `/mnt/us`: `agent/install.sh`, `bin/start.sh` and `takeover.sh start`.
They refuse to run, and change nothing, unless the device is a Kindle
Paperwhite 4 (2018) on firmware 5.13.x. The model comes from the device code
in `/proc/usid`,[^4] the firmware from `/etc/prettyversion.txt`. On a mismatch
the guard says why on stderr and on screen. `ARISTOTLE_FORCE=1` skips the check, for developers
trying another model or firmware on purpose, at their own risk; the emulator
may need it.

`takeover.sh` looks for the guard beside itself (the app bundle and the
installed extension carry a copy), in `../extension/` when it runs from the
repository, and in `/mnt/us/extensions/aristotle/` when it comes on stdin
(`sh -s`).

## The KUAL extension

*KUAL › Aristotle* offers:

| Item | Runs |
|------|------|
| *Start Aristotle* | `bin/aristotle-start.sh` |
| *Return to the Kindle interface* | `bin/aristotle-stop.sh` |
| *Start Aristotle at boot: on (agent installed)* | `agent/autostart.sh on` |
| *Start Aristotle at boot: off* | `agent/autostart.sh off` |
| *Restore previous boot app (reading data unchanged)* | `agent/autostart.sh rollback` |
| *Install agent (SSH on every boot, no Amazon updates)* | `agent/install.sh` |
| *USB drive mode for 10 minutes (agent installed)* | `aristotlectl mode storage 10` |
| *Remove agent completely (jobs, state, SSH key)* | `agent/uninstall.sh` |
| *Start USB network once (manual)* | `bin/start.sh` |
| *Stop USB network (manual)* | `bin/stop.sh` |

`bin/aristotle-start.sh` runs `/mnt/us/aristotle/run-aristotle.sh` and
returns at once. KUAL runs its actions as children of the Java framework,
which the takeover stops, and upstart signals the framework's whole process
group, so the launcher starts in a session of its own (`setsid`). The app's
log goes to `/mnt/us/.aristotle/app.log`. `bin/aristotle-stop.sh` stops
Aristotle however it was started (`SIGTERM`, or `aristotlectl app stop` for
the boot job; it saves first) and brings back what the takeover stopped. On
the device:[^3] an app started this way kept running after
the stand-in for KUAL's process group was killed with `SIGKILL`, and *Return*
stopped it (`=== exit 0`) and brought the stock GUI back. Started from
KUAL's own menu by tapping, it has not been tried yet.[^5] From inside Aristotle,
**Settings > Return to Kindle** also returns. **Open KUAL**, beside it on every settings page, saves and returns to stock before opening the installed booklet. Holding the *Library* title for a second remains available.

The two launchers advertise exit-action support only when `open-kual.sh` is present. Exit code 20 requests KUAL after a normal save and shutdown. It does not count as a failed boot launch. The helper waits for a running framework and a nonzero application-manager active PID, then sends `start app://com.mobileread.ixtab.kindlelauncher`. That handler and the installed jar path were read from the tested PW4's application registry. They also match [KUAL Booklet's registration](https://github.com/coplate/KUAL_Booklet/blob/master/src/com/mobileread/ixtab/kindlelauncher/resources/booklet/install/appreg.install.sql). No KUAL or Amazon bytecode is included.

The helper uses a 90-second monotonic wait budget. Each device command runs through BusyBox `timeout` for at most three seconds, capped by the remaining budget. An unavailable booklet or a failed handoff leaves stock running. The handoff is host-tested, not device-verified. Update the app bundle for manual launching and reinstall the agent scripts for boot launching to enable it.

`bin/start.sh` is the dev loop without the agent: it swaps the USB mass-storage [gadget](../docs/glossary.md#usb-gadget) for [`g_ether`](../docs/glossary.md#g_ether), gives `usb0` the address `192.168.15.244`, opens
`usb0` in the stock firewall and starts KOReader's [`dropbear`](../docs/glossary.md#dropbear), key-only, on
that address. It logs to `usbnet.log` beside itself. `bin/stop.sh`, or a
reboot, undoes it. On the host, `tools/host/setup-host-net.sh` (once) and
`ssh -F tools/host/ssh_config kindle` connect.

## The agent

`agent/install.sh` copies the scripts, the key, `takeover.sh` and a copy of
KOReader's `dropbear` (with its host key) to `/var/local/aristotle` and
installs three Upstart jobs in one read-write window, opened with [`mntroot rw`](../docs/glossary.md#mntroot) and closed with `mntroot ro`:
`aristotle-agent.conf`, `aristotle-ota.conf` and `aristotle-ui.conf`. That is
its only root-filesystem write, skipped when the job files are unchanged. No
stock file is changed. The agent job starts on `started lab126` and runs
`/var/local/aristotle/boot.sh`, which:

- brings up the USB network and key-only SSH whenever the USB mode is `net`,
  the default;
- switches to USB drive mode for a limited time on request, then back to `net`;
- runs a watchdog that restarts the stock GUI if it stays dead for 5 minutes, and reboots at most
  once an hour if that does not help. It stays out of the way while
  `/var/run/aristotle-takeover` exists (`takeover.sh start` creates it on the
  `/var` tmpfs, so a reboot clears it);
- applies the [Wi-Fi policy](#network).

State and logs live in `/var/local/aristotle` (persistent ext3). Deleting
`boot.sh` there disables the job without touching the root filesystem. SSH
runs the agent's copy of `dropbear` from there; if it does not start, the
agent falls back to KOReader's.

If `/mnt/us` is still unmounted after the boot wait, the agent stays idle. It leaves USB to the stock system because it cannot check `aristotle-disable`. This fail-closed path has host model tests only.

`aristotlectl` is the remote control, run over SSH as root:

```sh
aristotlectl status            # agent, USB mode, network, stock GUI, last log lines
aristotlectl mode net          # USB network + SSH (default)
aristotlectl mode storage [N]  # USB drive mode for N minutes (default 10); SSH drops meanwhile
aristotlectl net off|scoped    # Wi-Fi policy
aristotlectl net status        # mode, allowed addresses, rule counters
aristotlectl net allow add|remove HOST:PORT FEATURE   # a tagged net-allow line
aristotlectl takeover on|off   # tell the watchdog the stock GUI is down on purpose
aristotlectl gui-restart       # restart the stock GUI now
aristotlectl app status        # Aristotle at boot: on/off, running, failed launches
aristotlectl app start         # start Aristotle as at boot (needs autostart's copy)
aristotlectl app stop [--keep-takeover]  # stop it, not counted as a crash
aristotlectl log [N]           # last N log lines (default 40)
aristotlectl reboot            # reboot; the agent brings SSH back on boot
```

The app's settings screen switches the USB mode through the same script
(`aristotle_hal::system::set_usb_mode`).

### Aristotle at boot

`aristotle-ui.conf` starts on `started lab126_gui`, once per boot, and does
nothing unless `/var/local/aristotle/autostart` exists. *KUAL › Aristotle ›
Start Aristotle at boot: on* (`agent/autostart.sh on`) copies the app from
`/mnt/us/aristotle` to `/var/local/aristotle/app`, where USB drive mode cannot
take it away, and creates that file. Updates retain the prior internal app in
`app.previous`. Off removes the autostart marker but keeps the copies.
See [rollback](../docs/maintenance.md#roll-back-the-boot-app) for the KUAL action and its limits. The job runs
`ui-boot.sh`, which stops the stock GUI as `takeover.sh start` does (the
stock boot is unchanged up to `lab126_gui`), unloads `blanket`'s boot splash,
sends [`powerd`](../docs/glossary.md#powerd) the `bootSplashCleanup` that the framework would have sent, and
runs the app. It falls back to the stock GUI for the rest of the boot when:

- `/mnt/us/aristotle-disable` exists;
- `/mnt/us` does not mount in time, so the kill switch cannot be checked;
- the USB app's `SOURCE.txt` differs from the one recorded at the last explicit boot selection, or the boot copy has no such record;
- three launches in a row did not stay up for two minutes. The count
  survives reboots, so three forced restarts also count, and later boots
  stay stock until autostart is turned on again;
- Aristotle quits normally;
- `aristotlectl app stop` asks for it (not counted as a failure).
  `run-aristotle.sh` stops a boot-started Aristotle this way first, with
  `--keep-takeover`, so developers can run their own build.

`aristotlectl app status` reports the internal, previous, and USB release identities. Refresh autostart after a USB app update. An explicit rollback records the current USB identity without changing the selected older binary. These checks compare release metadata, not a signature or the binary's contents. The missing-mount and changed-source fallbacks have host tests only.

The job logs to `ui.log`, and the app logs to `app.log`. Measured on our PW4:[^3]
the job started at 37.2 s of uptime, the stock GUI was down at
39.6 s, the app started at 39.8 s and submitted the library's first frame
1.0 s later, so about 41 s from power-on to the library, without Java
(`framework`, `pillow` and `lab126_gui` stopped, `x` and `powerd` running).
382 MB of 490 MB were available. The crash fallback was checked on the
device too, with a copy of the app that exits `1`: launches 1 to 3 at 39.8,
42.0 and 44.1 s of uptime, then "3 failed launches in a row" and the stock
GUI; after the real binary was back and autostart was turned on again, the
next boot reached the library 41 s after power-on. In the emulator
`app start` and a boot that stays stock after three failures were also
checked.

### Amazon's updates

`aristotle-ota.conf` handles `starting otaupd` and `starting otav3`: it
bind-mounts `/var/local/aristotle/ota-stub`, which only sleeps, read-only
over `/usr/bin/otaupd` and `/usr/bin/otav3` before upstart runs them. The real
daemons never run and `com.lab126.ota` never appears. No stock file changes;
the mounts end at the next reboot, and without the job they do not come
back. The `update.bin.tmp.partial` folder on the USB drive stays as a second
layer: our firmware deletes it if it is not a directory, but `otav3` also has
code to delete it.[^1] On the device:[^2] both daemons ran as
`sleep 2147483647` after install, with the two read-only bind mounts in
`/proc/mounts`; in the emulator the real `otav3` never started at boot (no
`OTAController` lines). To update the firmware on purpose: remove the agent,
reboot, update. The [kill switch](../docs/glossary.md#kill-switch) does not turn this off.

### Security

The stock `framework` job runs `chgrp -R javausers` and `chmod -R g=u` on
`/var/local` at every start,[^1] and `/var/local` itself belongs to the
`framework` user.[^2] A directory there that root's scripts run from would
be writable by the framework (Pillow, the web content it shows) after the
first stock boot. So:

- `install.sh` makes `/var/local/aristotle` root's with mode `000`. With no
  bits for the owner, `g=u` gives the group none; root needs none. After a
  stock boot the directory shows as `d-----S---` with group `javausers`,[^2]
  still closed to everyone else.
- Each job checks, before running anything, that the directory is not a
  link, belongs to root and gives nothing to group or others (a setgid bit
  is fine). Otherwise it does not start, logs to syslog and to
  `/mnt/us/aristotle-refused.log`, and the stock system boots as usual.
  `aristotlectl` checks the same.
- The jobs enter the checked directory and the scripts reach their files
  through `/proc/PID/cwd`, so a directory renamed or replaced later in the
  boot cannot change what runs.
- Nothing on the USB drive runs as root at boot: SSH uses the agent's copy
  of `dropbear` and its own `authorized_keys`, and the boot job runs its own
  copy of the app.
- `install.sh` over a directory others could write, as in earlier versions, keeps
  only the log and the settings as plain files and replaces the rest.

Stock behavior remains outside these protections. Every process can write to the USB drive, and KUAL runs its extensions from it as root. The jailbreak runs `/mnt/us/emergency.sh` as root. The stock system also runs `/var/local/system/fixup`, whose directory the framework's group can write.
`takeover.sh` and the KUAL launcher still log to `/var/tmp`, which any user
can write.

### Recovery without SSH

Verified on our PW4:[^3] the jailbreak's `bridge.conf` runs
`/mnt/us/emergency.sh` as root once the stock framework is up. A script that
only appended a line to a log on the USB drive ran as uid `0` about 110 s
after the reboot, and the jailbreak skipped its own repairs on that boot, as
its job says. Unplug the cable before rebooting. With the agent unreachable:

1. Put a script there that undoes what is needed, for example `sh /mnt/us/extensions/aristotle/agent/uninstall.sh`.
2. Eject and reboot.
3. Delete the script afterwards.

It runs only on a boot that reaches the stock framework, so with Aristotle at boot it needs the kill
switch, a fallback or *Return* first. It also needs `/mnt/us` mounted at that
moment: with the cable in and the agent idle (the kill switch), the Kindle
showed up on the computer as a USB drive about a minute after the reboot,[^2]
before the framework was ready, so a script dropped there could not
run while the drive was exported.

### Kill switch

A file named `aristotle-disable` at the root of the USB drive
(`/mnt/us/aristotle-disable`) stops the agent before it does anything on that
boot: no USB network, no SSH, no watchdog, no Wi-Fi policy, and USB stays with
`volumd`. It is the way out without SSH. While the agent runs, the USB port is a network link:

1. Switch to drive mode first (*KUAL › Aristotle › USB drive mode for 10 minutes*, or the app's settings).
2. Create the file.
3. Eject and reboot.

Delete the file and reboot to have the agent back. The file also
keeps Aristotle at boot from starting (`ui-boot.sh` checks it first; host
test and emulator). On the device the reboot with the file present came up
as the stock system in USB drive mode on the computer, as expected with the
agent idle; the rest of that test (the next boot without the file) was not
finished.

### Uninstall

*KUAL › Aristotle › Remove agent completely*, or `agent/uninstall.sh` over SSH
(the session drops at the end), removes:

- the three upstart jobs, in one `mntroot` read-write window;
- Aristotle started at boot, if it runs (it saves; the stock GUI comes back);
- the OTA block: the stubs are unmounted and daemons that were running are
  started again for real;
- `/var/local/aristotle`: the scripts, the state (USB mode, Wi-Fi policy and
  allowlist, watchdog flags, autostart and its copy of the app), the agent's
  copies of the key and of `dropbear`, and the logs;
- `/var/local/aristotle-migrate`, backups an earlier version may have left;
- the agent's keys from KOReader's `settings/SSH/authorized_keys` (other keys
  there stay), and the extension's `authorized_keys`, also under the old
  `aristotle-usbnet` folder;
- the runtime state: `dropbear`, `usb0` and `g_ether`, the `usb0` firewall
  rules and the Wi-Fi policy chains.

Checked on the device[^3] by running it, then `install.sh`,
from a detached script, as KUAL would: everything above was gone, the real
`otav3` and `otaupd` ran again, the reinstall brought back the three jobs,
the mode-`000` directory, the OTA stubs and SSH within seconds. The first run
found a bug (the OTA job bound the stubs again when uninstall restarted the
daemons); the stub file now goes first. It appends the logs to `/mnt/us/aristotle-agent.log` first, for diagnosis. It
is not guarded, and running it twice is harmless. The app, the extensions and
`/mnt/us/.aristotle` stay on the USB drive until you delete them.

## Launching the app

The app bundle unpacks to `/mnt/us/aristotle/` with `run-aristotle.sh`,
`takeover.sh` and a copy of `guard.sh`. `run-aristotle.sh` runs
`takeover.sh start`, which stops the stock GUI and the cloud and telemetry
jobs, then the app, then `takeover.sh stop`, which brings the stock GUI back
(30 to 60 s of boot splash). The app follows `powerd` for sleep only with
`ARISTOTLE_POWERD=1` for now ([`docs/reader.md`](../docs/reader.md)). It logs to `/var/tmp/aristotle.log` and ignores
hangups, so the app outlives a dropped SSH session:

```sh
ssh -F tools/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh
```

`takeover.sh pause` and `resume` freeze the stock GUI with `SIGSTOP` instead.
They are experimental: on our PW4 the frozen framework was killed, most likely
by Amazon's watchdog, and the GUI came back only after a reboot. Details are
in [`docs/pw4-reference.md`](../docs/pw4-reference.md#taking-over-the-screen).

## The probe

`probe/bin/probe.sh` (*KUAL › Aristotle › Run read-only probe*) collects kernel,
hardware, process, Upstart, and [LIPC](../docs/glossary.md#lipc) state into
`/mnt/us/aristotle-probe/<UTC stamp>/` and writes nothing else. `probe.sh wfm`
also copies the e-ink [waveforms](../docs/glossary.md#waveform), which are Amazon's files and stay private.
The output identifies the device and its owner; review it before sharing any
of it. Extend it for runtime questions, keeping it read-only, and record
lasting findings in [`docs/pw4-reference.md`](../docs/pw4-reference.md).

## Tests

```sh
sh device/tests/agent.sh   # the guard, install, uninstall, the kill switch, the takeover flag
sh device/tests/net.sh     # the Wi-Fi policy generator against net/expected-*
```

Both build a fake device root under `$TMPDIR` with stub device commands, run
the real scripts against it (`AGENT_TEST_ROOT`) and touch nothing outside it.
CI runs them on every pull request. `UPDATE=1 sh device/tests/net.sh` rewrites
the expected files.

## Network

The reader listens on no port. Its outbound requests serve the configured
progress-sync server, OPDS catalogs, and their acquisition links.
[Network setup](../docs/network.md) covers account imports and a first
connection. [The USB link](#the-usb-link) is separate, for development and SSH.
Wi-Fi stays with the stock system unless you switch on the `scoped` policy,
which restricts the device to an allowlist of destinations.

### The USB link

The device agent (`agent/boot.sh`) and *KUAL › Aristotle › Start USB network
once* (`bin/start.sh`) give `usb0` the address `192.168.15.244/24`, and
`dropbear` listens there only. On the computer,
`tools/host/setup-host-net.sh` creates a NetworkManager network profile for the
gadget's fixed MAC with `192.168.15.201/24` and no default route. There is no
gateway or DHCP.

If your own network also uses `192.168.15.0/24`, the two routes clash while
the Kindle is plugged in. Move the USB link to a private `/24` that nothing
else on your computer uses (see `ip route`), for example `10.73.77.0/24`. The
address appears in these files:

| File | Side | What to change |
|------|------|----------------|
| `device/extension/agent/boot.sh` | device, agent | `DEV_IP`; the `netmask` in `net_up` if the prefix is not `/24` |
| `device/extension/bin/start.sh` | device, KUAL | `DEV_IP`; the `netmask` of its `ifconfig` line likewise |
| `tools/host/setup-host-net.sh` | computer | the address in `ipv4.addresses` |
| `tools/host/ssh_config` | computer | `HostName` |

Put both sides in the same subnet, then apply the change:

1. With the device in drive mode (*Settings › USB port › USB drive*, or
   `aristotlectl mode storage 10` over SSH), run
   `tools/host/install-extensions.sh`.
2. If the agent is installed, run *KUAL › Aristotle › Install agent* again. The
   agent runs its own copy of `boot.sh` from `/var/local/aristotle`, and only
   the install replaces it. This writes to `/var/local`, so on someone else's
   device it needs their go-ahead.
3. Run `tools/host/setup-host-net.sh` again on the computer.
4. Reconnect. SSH adds a new host key entry for the new address
   (`accept-new`).

`.github/SECURITY.md` and `AGENTS.md` also mention the default address.

### Modes

The policy is kept in `/var/local/aristotle/net-mode` and applied by
`agent/net.sh`. Run these commands in a device SSH session. The installed
control script is not added to the shell's `PATH`:

```sh
sh /var/local/aristotle/aristotlectl net status   # mode, allowed addresses, counters
sh /var/local/aristotle/aristotlectl net scoped   # apply the allowlist
sh /var/local/aristotle/aristotlectl net off      # restore the stock firewall
```

In `off` mode, the default, Aristotle adds no rules. The stock firewall drops
inbound traffic except replies and the USB link, and allows all outbound
traffic,[^1] so Amazon's services reach their servers whenever Wi-Fi is on.
To stay offline, leave Wi-Fi off.

In `scoped` mode the device may open connections on `wlan0` only to the hosts
in `/var/local/aristotle/net-allow`, plus DHCP and DNS to the resolvers in
`/etc/resolv.conf`. All other packets on `wlan0` are dropped in both
directions, whichever process sends them.

Neither mode touches `usb0`. A future settings entry will write the same two
files and call `aristotlectl net`.

### The allowlist

One entry per line, `host[:port][/udp]`. The port defaults to 443 and the
protocol to TCP. `#` starts a comment.

```
# /var/local/aristotle/net-allow
dash.example.org            # HTTPS
192.168.1.10:8080           # a dashboard server on the local network
time.example.net:123/udp
```

Host names may contain letters, digits, dots and hyphens. Lines that don't
parse are skipped and logged to `/var/local/aristotle/log`.

The app's settings manage their own lines. Saving a sync server or a
catalog runs `aristotlectl net allow add HOST:PORT sync|opds`, which appends
`HOST:PORT   # aristotle:sync` (or `opds`) unless that tagged line is there,
and deleting it runs `remove`, which deletes only lines with that entry and
that tag. In `scoped` mode the change is applied at once; in `off` mode the
line waits for `scoped`. The entry is checked with the same parser as the
rest of the file. The app also checks each request and redirect against the
list in `scoped` mode, so a missing host fails with a clear error instead
of a timeout.

### How it works

iptables rules are grouped into chains. `net.sh` adds two chains and jumps to them from the top of `INPUT` and
`OUTPUT` for `wlan0` only. `ARISTOTLE_OUT` accepts DHCP (`udp 68 → 67`), DNS
(`udp` and `tcp` 53) to each resolver and each allowed address, port and
protocol, then drops the rest. `ARISTOTLE_IN` accepts `ESTABLISHED,RELATED`
and DHCP replies, then drops the rest.

An update builds the new chains next to the old ones and switches the jumps
over before deleting the old chains, so `wlan0` is never left without rules.
`off` removes them, leaving the stock rules untouched. The rules live only in
memory; after a reboot the stock firewall applies until the agent's first
tick.

The generated commands are kept in `/var/local/aristotle/net-rules.sh` and
run with `sh -e`. CI checks them with `device/tests/net.sh`, against
expected files and a model of `iptables`.

`iptables` matches addresses, so `net.sh` resolves each host (glibc's
`getent ahostsv4`[^1] or `busybox nslookup`) when the policy is applied,
every 15 minutes after that, and whenever `wlan0`'s address, the resolvers or
`net-allow` change. The agent checks for changes every 15 s, and allows DNS to
a new resolver before resolving through it. A host that fails to resolve
keeps its previous addresses.

### Limits

- Rules are per IP address. Any process can reach anything else served from
  an allowed address, which on a shared content delivery network (CDN) can be many unrelated sites.
- A host whose addresses rotate faster than the 15-minute refresh can fail in
  between. Prefer stable addresses, ideally a server on your local network.
- TLS Server Name Indication (SNI) and the HTTP `Host` header are not checked.
- Domain Name System (DNS) queries to the configured resolvers are unrestricted, so data could leak out in DNS
  queries.
- Until the agent's first tick after `started lab126`, the stock firewall
  applies. Wi-Fi may already be up then, since `wifid` starts on the same
  event.[^1]
- IPv4 only. The 5.13.6 kernel has `CONFIG_IPV6` unset and the firmware has no
  `ip6tables`.[^1] On a kernel with IPv6, `scoped` sets `disable_ipv6` on
  `wlan0`.
- The stock Network Time Protocol (NTP) clock sync uses `ntp-g7g.amazon.com` (`/etc/sysconfig/ntp`[^1])
  and is blocked in `scoped`. Allowing another NTP server does not reconfigure
  the stock client, so the clock drifts until Aristotle sets it itself.
- The rules have been tested on the host only. Confirm them on the device with
  the checks below.

### Stock processes that use the network

From the firmware[^1] and KOReader, not yet checked on a device:

- The Java framework (`cvm`: store, sync, cloud library).
- `wifid`, which tests a new connection by contacting Amazon.
- The OTA updaters (`otaupd`, `otav3`).
- Telemetry (`fastmetrics`, `clickstream_logging`).
- `tmd`, `todo`, `dmld`, `rcm`, `archive` and the stock NTP client.

`scoped` drops their traffic, and the takeover (`takeover.sh`) stops most of them.

`wifid` may report the network as unusable when its test fails (not
verified). With `/mnt/us/WIFI_NO_NET_PROBE` present, its upstart job skips the
test (`-n`).[^1]

Read-only checks to run with Wi-Fi on, first in `off` mode to see who
connects, then in `scoped` mode:

```sh
netstat -tunp                                # sockets with their processes
cat /proc/net/tcp /proc/net/udp              # the same, raw; match inodes with
ls -l /proc/[0-9]*/fd 2> /dev/null | grep socket:
iptables -L ARISTOTLE_OUT -n -v              # DROP counter: packets stopped
iptables -L INPUT -n -v; iptables -L OUTPUT -n -v
cat /var/run/resolv.conf                     # the resolvers DHCP gave
cat /tmp/wifid.err; grep -i wifid /var/log/messages | tail
lipc-get-prop com.lab126.wifid cmState       # wifid's state, as KOReader reads it
```

A temporary `LOG` rule before the `DROP` would show the blocked
destinations. Once Aristotle runs as its own user, the `owner` match could
limit the allowlist to its processes. The kernel has both.[^1]

[^1]: Read from firmware.
[^2]: Observed on the device.
[^3]: Observed on our PW4, 2026-10-17.
[^4]: [KOReader](../docs/glossary.md#koreader)'s list of PW4 codes, with `0PP` verified on ours.
[^5]: Taps injected with `aristotle-observe tap` reached the touchscreen node (`aristotle-observe raw` showed them) but KUAL did not react to them.
