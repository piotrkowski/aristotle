# Device-side code

Everything here but the tests runs on the Kindle, under its busybox shell.
The host-side tools that install and drive it are in [`tools/`](../tools/), and the app
itself is the `aristotle` crate.

| Path | What | Installed to |
|------|------|--------------|
| `extension/` | The KUAL extension: the menu, the manual USB network, the device guard | `/mnt/us/extensions/aristotle/` |
| `extension/agent/` | The agent: an upstart job, its boot script, `aristotlectl` and the Wi-Fi policy | `/etc/upstart/aristotle-agent.conf` and `/var/local/aristotle/` |
| `app/` | `run-aristotle.sh` and `takeover.sh`, which launch the app in place of the stock UI | `/mnt/us/aristotle/`, in the app bundle |
| `probe/` | A read-only probe, a KUAL extension | `/mnt/us/extensions/aristotle-probe/` |
| `tests/` | Host-side tests of the scripts above, against a fake device root | stays on the host |

`tools/host/install-extensions.sh` copies both extensions, the app bundle from
`tools/build-render.sh device` if it is built, `takeover.sh` and the dev-loop public key to a
Kindle in drive mode, then ejects it. Earlier versions installed the extension
to `/mnt/us/extensions/aristotle-usbnet/`; the install script removes that
folder.

No recovery path is verified for the PW4, so these scripts write to the root
filesystem once, for the upstart job, and keep everything else in
`/var/local/aristotle`, on `/mnt/us` or in runtime state that a reboot clears.

## The device guard

`extension/guard.sh` is sourced by every script that changes the device
outside `/mnt/us`: `agent/install.sh`, `bin/start.sh` and `takeover.sh start`.
They refuse to run, and change nothing, unless the device is a Kindle
Paperwhite 4 (2018) on firmware 5.13.x. The model comes from the device code
in `/proc/usid` (KOReader's list of PW4 codes; `0PP` is verified on ours), the
firmware from `/etc/prettyversion.txt`. On a mismatch the guard says why on
stderr and on screen. `ARISTOTLE_FORCE=1` skips the check, for developers
trying another model or firmware on purpose, at their own risk; the emulator
may need it.

`takeover.sh` looks for the guard beside itself (the app bundle and the
installed extension carry a copy), in `../extension/` when it runs from the
repository, and in `/mnt/us/extensions/aristotle/` when it comes on stdin
(`sh -s`).

## The KUAL extension

KUAL › Aristotle offers:

| Item | Runs |
|------|------|
| Install agent (SSH on every boot) | `agent/install.sh` |
| USB drive mode for 10 minutes (agent installed) | `aristotlectl mode storage 10` |
| Remove agent completely (job, state, SSH key) | `agent/uninstall.sh` |
| Start USB network once (manual) | `bin/start.sh` |
| Stop USB network (manual) | `bin/stop.sh` |

`bin/start.sh` is the dev loop without the agent: it swaps the USB mass
storage gadget for `g_ether`, gives `usb0` the address `192.168.15.244`, opens
`usb0` in the stock firewall and starts KOReader's `dropbear`, key-only, on
that address. It logs to `usbnet.log` beside itself. `bin/stop.sh`, or a
reboot, undoes it. On the host, `tools/host/setup-host-net.sh` (once) and
`ssh -F tools/host/ssh_config kindle` connect.

## The agent

`agent/install.sh` copies `boot.sh`, `aristotlectl`, `net.sh` and the key to
`/var/local/aristotle` and installs one upstart job,
`/etc/upstart/aristotle-agent.conf`, with `mntroot rw` and `mntroot ro`. That
is its only root-filesystem write, and it is skipped when the job file is
unchanged. The job starts on `started lab126` and runs
`/var/local/aristotle/boot.sh`, which:

- brings up the USB network and key-only SSH whenever the USB mode is `net`,
  the default;
- switches to USB drive mode for a limited time on request, then back to `net`;
- restarts the stock GUI if it stays dead for 5 minutes, and reboots at most
  once an hour if that does not help. It stays out of the way while
  `/var/run/aristotle-takeover` exists (`takeover.sh start` creates it on the
  `/var` tmpfs, so a reboot clears it);
- applies the [Wi-Fi policy](#network).

State and logs live in `/var/local/aristotle` (persistent ext3). Deleting
`boot.sh` there disables the job without touching the root filesystem.

`aristotlectl` is the remote control, run over SSH as root:

```sh
aristotlectl status            # agent, USB mode, network, stock GUI, last log lines
aristotlectl mode net          # USB network + SSH (default)
aristotlectl mode storage [N]  # USB drive mode for N minutes (default 10); SSH drops meanwhile
aristotlectl net off|scoped    # Wi-Fi policy
aristotlectl net status        # mode, allowed addresses, rule counters
aristotlectl takeover on|off   # tell the watchdog the stock GUI is down on purpose
aristotlectl gui-restart       # restart the stock GUI now
aristotlectl log [N]           # last N log lines (default 40)
aristotlectl reboot            # reboot; the agent brings SSH back on boot
```

The app's settings screen switches the USB mode through the same script
(`aristotle_hal::system::set_usb_mode`).

### Kill switch

A file named `aristotle-disable` at the root of the USB drive
(`/mnt/us/aristotle-disable`) stops the agent before it does anything on that
boot: no USB network, no SSH, no watchdog, no Wi-Fi policy, and USB stays with
`volumd`. It is the way out without SSH. While the agent runs, the USB port is
a network link, so switch to drive mode first (KUAL › Aristotle › USB drive
mode for 10 minutes, or the app's settings), create the file, eject and
reboot. Delete the file and reboot to have the agent back.

### Uninstall

KUAL › Aristotle › Remove agent completely, or `agent/uninstall.sh` over SSH
(the session drops at the end), removes:

- the upstart job;
- `/var/local/aristotle`: the scripts, the state (USB mode, Wi-Fi policy and
  allowlist, watchdog flags), the agent's copy of the key and the log;
- `/var/local/aristotle-migrate`, backups an earlier version may have left;
- the agent's keys from KOReader's `settings/SSH/authorized_keys` (other keys
  there stay), and the extension's `authorized_keys`, also under the old
  `aristotle-usbnet` folder;
- the runtime state: `dropbear`, `usb0` and `g_ether`, the `usb0` firewall
  rules and the Wi-Fi policy chains.

It appends the log to `/mnt/us/aristotle-agent.log` first, for diagnosis. It
is not guarded, and running it twice is harmless. The app, the extensions and
`/mnt/us/.aristotle` stay on the USB drive until you delete them.

## Launching the app

The app bundle unpacks to `/mnt/us/aristotle/` with `run-aristotle.sh`,
`takeover.sh` and a copy of `guard.sh`. `run-aristotle.sh` runs
`takeover.sh start`, which stops the stock GUI and the cloud and telemetry
jobs, then the app, then `takeover.sh stop`, which brings the stock GUI back
(30 to 60 s of boot splash). It logs to `/var/tmp/aristotle.log` and ignores
hangups, so the app outlives a dropped SSH session:

```sh
ssh -F tools/host/ssh_config kindle sh /mnt/us/aristotle/run-aristotle.sh
```

`takeover.sh pause` and `resume` freeze the stock GUI with SIGSTOP instead.
They are experimental: on our PW4 the frozen framework was killed, most likely
by Amazon's watchdog, and the GUI came back only after a reboot. Details are
in [docs/pw4-reference.md](../docs/pw4-reference.md#taking-over-the-screen).

## The probe

`probe/bin/probe.sh` (KUAL › Aristotle › Run read-only probe) collects kernel,
hardware, process, upstart and LIPC state into
`/mnt/us/aristotle-probe/<UTC stamp>/` and writes nothing else. `probe.sh wfm`
also copies the e-ink waveforms, which are Amazon's files and stay private.
The output identifies the device and its owner; review it before sharing any
of it. Extend it for runtime questions, keeping it read-only, and record
lasting findings in [docs/pw4-reference.md](../docs/pw4-reference.md).

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

The reader opens no connections and listens on no port. Aristotle's only
network is [the USB link](#the-usb-link) for development. Wi-Fi stays with the
stock system unless you switch on the `scoped` policy, which lets the device
reach a short list of hosts, such as a dashboard server.

### The USB link

The device agent (`agent/boot.sh`) and KUAL › Aristotle › Start USB network
once (`bin/start.sh`) give `usb0` the address `192.168.15.244/24`, and
`dropbear` listens there only. On the computer,
`tools/host/setup-host-net.sh` creates a NetworkManager profile for the
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

1. With the device in drive mode (Settings › USB port › USB drive, or
   `aristotlectl mode storage 10` over SSH), run
   `tools/host/install-extensions.sh`.
2. If the agent is installed, run KUAL › Aristotle › Install agent again. The
   agent runs its own copy of `boot.sh` from `/var/local/aristotle`, and only
   the install replaces it. This writes to `/var/local`, so on someone else's
   device it needs their go-ahead.
3. Run `tools/host/setup-host-net.sh` again on the computer.
4. Reconnect. SSH adds a new host key entry for the new address
   (`accept-new`).

`.github/SECURITY.md` and `AGENTS.md` also mention the default address.

### Modes

The policy is kept in `/var/local/aristotle/net-mode` and applied by
`agent/net.sh`. Change it over SSH:

```sh
aristotlectl net status          # mode, allowed addresses, rule counters
aristotlectl net scoped          # apply the allowlist now
aristotlectl net off             # back to the stock firewall
```

In `off` mode, the default, Aristotle adds no rules. The stock firewall drops
inbound traffic except replies and the USB link, and allows all outbound
traffic [fw], so Amazon's services reach their servers whenever Wi-Fi is on.
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

### How it works

`net.sh` adds two chains and jumps to them from the top of `INPUT` and
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
expected files and a model of iptables.

iptables matches addresses, so `net.sh` resolves each host (glibc's
`getent ahostsv4` [fw] or busybox `nslookup`) when the policy is applied,
every 15 minutes after that, and whenever `wlan0`'s address, the resolvers or
`net-allow` change. The agent checks for changes every 15 s, and allows DNS to
a new resolver before resolving through it. A host that fails to resolve
keeps its previous addresses.

### Limits

- Rules are per IP address. Any process can reach anything else served from
  an allowed address, which on a shared CDN can be many unrelated sites.
- A host whose addresses rotate faster than the 15-minute refresh can fail in
  between. Prefer stable addresses, ideally a server on your local network.
- TLS SNI and the HTTP `Host` header are not checked.
- DNS to the configured resolvers is open, so data could leak out in DNS
  queries.
- Until the agent's first tick after `started lab126`, the stock firewall
  applies. Wi-Fi may already be up then, since `wifid` starts on the same
  event [fw].
- IPv4 only. The 5.13.6 kernel has `CONFIG_IPV6` unset and the firmware has no
  `ip6tables` [fw]. On a kernel with IPv6, `scoped` sets `disable_ipv6` on
  `wlan0`.
- The stock clock sync uses `ntp-g7g.amazon.com` (`/etc/sysconfig/ntp` [fw])
  and is blocked in `scoped`. Allowing another NTP server does not reconfigure
  the stock client, so the clock drifts until Aristotle sets it itself.
- The rules have been tested on the host only. Confirm them on the device with
  the checks below.

### Stock processes that use the network

From the firmware [fw] and KOReader, not yet checked on a device: the Java
framework (`cvm`: store, sync, cloud library), `wifid` (it tests a new
connection by contacting Amazon), the OTA updaters (`otaupd`, `otav3`),
telemetry (`fastmetrics`, `clickstream_logging`), `tmd`, `todo`, `dmld`,
`rcm`, `archive` and the stock NTP client. `scoped` drops their traffic, and
the takeover (`takeover.sh`) stops most of them.

`wifid` may report the network as unusable when its test fails (not
verified). With `/mnt/us/WIFI_NO_NET_PROBE` present, its upstart job skips the
test (`-n`) [fw].

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
limit the allowlist to its processes. The kernel has both [fw].
