# Network

The reader opens no connections and listens on no port. Aristotle's only
network is [the USB link](#the-usb-link) for development. Wi-Fi stays with the
stock system unless you switch on the `scoped` policy, which lets the device
reach a short list of hosts, such as a dashboard server.

## The USB link

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

## Modes

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

## The allowlist

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

## How it works

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

## Limits

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

## Stock processes that use the network

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
