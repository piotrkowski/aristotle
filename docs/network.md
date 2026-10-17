# Network

Aristotle is offline first. The reader opens no connections and listens on no
port. The only network it sets up is the USB link to a computer for
development (`usb0`, see [The USB link](#the-usb-link)). Wi-Fi stays under the
stock system's control unless you switch on the **scoped** policy described
here. Features that need a server, such as a dashboard that fetches from one
host, are meant to run with it.

## The USB link

The device agent (`agent/boot.sh`) and KUAL › Aristotle › Start USB network
once (`bin/start.sh`) load `g_ether` and give `usb0` the address
`192.168.15.244/24`; `dropbear` listens on that address only. On the
computer, `devloop/host/setup-host-net.sh` creates a NetworkManager profile,
matched by the gadget's fixed MAC, with the address `192.168.15.201/24` and
no default route, and `devloop/host/ssh_config` names the Kindle by its
address. There is no gateway or DHCP on the link: both ends have fixed
addresses in one `/24`.

If your own network uses `192.168.15.0/24` too, the computer has two routes
to the same subnet while the Kindle is plugged in, and one of them wins:
either SSH to the Kindle or the hosts of your network in that range stop
answering. Move the USB link to a private `/24` that nothing else on your
computer uses (`ip route` lists the ones in use), for example
`10.73.77.0/24`. The address appears in these files:

| File | Side | What to change |
|------|------|----------------|
| `devloop/kindle/aristotle-usbnet/agent/boot.sh` | device, agent | `DEV_IP`; the `netmask` in `net_up` if the prefix is not `/24` |
| `devloop/kindle/aristotle-usbnet/bin/start.sh` | device, KUAL | `DEV_IP`; the `netmask` of its `ifconfig` line likewise |
| `devloop/kindle/aristotle-usbnet/agent/migrate.sh` | device | `DEV_IP` (what the migration checks before it commits) |
| `devloop/host/setup-host-net.sh` | computer | the address in `ipv4.addresses` |
| `devloop/host/ssh_config` | computer | `HostName` |
| `devloop/kindle/tests/migrate.sh` | test | the address the fake `ifconfig` prints |

Change both sides together; the device and the computer must agree on the
subnet, or SSH has nothing to connect to. Then apply the change on each
side:

1. Copy the extension to the device with
   `devloop/host/install-extensions.sh` while it is in drive mode (Settings ›
   USB port › USB drive in Aristotle, or `aristotlectl mode storage 10` over
   SSH). The KUAL start item uses the new address from then on.
2. If the agent is installed, run KUAL › Aristotle › Install agent once more.
   The agent runs its own copy of `boot.sh` in `/var/local/aristotle`, so a
   new extension alone does not change it; the install copies it and
   restarts the agent, which moves `usb0` and `dropbear` to the new address.
   It writes `/var/local`, so on someone else's device it needs their
   go-ahead, like the first install.
3. On the computer, run `devloop/host/setup-host-net.sh` again. It replaces
   the profile bound to the gadget's MAC.
4. Reconnect. SSH keeps known host keys by address, so the first connection
   to the new address adds a new entry (`accept-new`).

`.github/SECURITY.md` and `AGENTS.md` quote the default address in prose.

## Modes

The device agent keeps the policy in `/var/local/aristotle/net-mode` and
applies it with `agent/net.sh`. Change it over SSH:

```sh
aristotlectl net status          # mode, allowed addresses, rule counters
aristotlectl net scoped          # apply the allowlist now
aristotlectl net off             # back to the stock firewall
```

- **off** (the default). Aristotle adds no rules. The stock firewall applies
  as the stock `network` job loads it from `/etc/sysconfig/iptables`: inbound
  traffic is dropped except for replies and the USB link, and **outbound
  traffic is allowed** [fw]. With Wi-Fi on, Amazon's services reach their
  servers as usual. To keep the device fully offline, leave Wi-Fi off
  (airplane mode); `off` does not switch the radio.
- **scoped**. On `wlan0`, the device may only open connections to the hosts
  listed in `/var/local/aristotle/net-allow`. DNS to the resolvers in
  `/etc/resolv.conf` and DHCP are allowed too. Every other packet on `wlan0`
  is dropped in both directions, whichever process sends it. Inbound, only
  replies to allowed connections get through.

`usb0` is never touched: SSH over the cable works in both modes.

A future settings entry will write the same two files and call
`aristotlectl net`.

## The allowlist

One entry per line, `host[:port][/udp]`. The port defaults to 443 and the
protocol to TCP. `#` starts a comment.

```
# /var/local/aristotle/net-allow
dash.example.org            # HTTPS
192.168.1.10:8080           # a dashboard server on the local network
time.example.net:123/udp
```

Host names may contain letters, digits, dots and hyphens; lines that don't
parse are skipped and logged to `/var/local/aristotle/log`.

## How it works

`net.sh` builds two chains of its own, `ARISTOTLE_IN` and `ARISTOTLE_OUT`, and
jumps to them from the top of `INPUT` and `OUTPUT` for `wlan0` only:

- `ARISTOTLE_OUT`: DHCP (`udp 68 → 67`), DNS (`udp` and `tcp` 53) to each
  resolver, one `ACCEPT` per allowed address, port and protocol, then `DROP`.
- `ARISTOTLE_IN`: `ESTABLISHED,RELATED` (replies, and ICMP errors about them),
  DHCP replies, then `DROP`.

An update builds the new chains beside the old ones, inserts their jumps,
then removes the old chains and renames the new ones, so a complete rule set
covers `wlan0` at every moment. `off` removes both chains and their jumps;
the stock rules were never changed, so they are what remains. The rules are
runtime state: a reboot loads the stock firewall again, and the agent
re-applies `scoped` on its first tick.

The generated commands go to `/var/local/aristotle/net-rules.sh` and run
with `sh -e`. The host-side test `devloop/kindle/tests/net.sh` checks the
parser, resolution and the generated commands against expected files, and
runs `apply` against a model of iptables that fails where iptables fails
(deleting a referenced or non-empty chain, a missing rule). CI runs it.

### Names become addresses

iptables matches addresses, not names. `net.sh` resolves each host (glibc's
`getent ahostsv4`, which the firmware ships [fw], or busybox `nslookup`)
when the policy is applied, then:

- again every 15 minutes, to follow DNS changes;
- whenever `wlan0`'s address, the resolvers in `/etc/resolv.conf` or
  `net-allow` change. The agent checks every 15 s, so a DHCP renewal on a new
  network is picked up within one tick. DNS to a new resolver is allowed
  before the names are resolved through it.

A host that fails to resolve during a refresh keeps the addresses it had, so a
DNS hiccup does not cut off a working server. A host removed from `net-allow`
loses its rules at the next apply.

## Limits

Be clear about what this is: an IP allowlist on one interface, not a proxy.

- **Rules are per IP address.** Anything else served from an allowed address
  is reachable too. For a host on a shared CDN, that can be many unrelated
  sites, and any process on the device, Amazon's included, can reach them.
- **CDN address churn.** A host whose addresses rotate faster than the
  refresh, or that hands each client different addresses, can fail between
  refreshes. Prefer servers with stable addresses, ideally your own on the
  local network.
- **HTTPS is not inspected.** The TLS SNI and the HTTP `Host` are not
  checked; a connection to an allowed address and port gets through,
  whatever name it asks for.
- **DNS is open** to the configured resolvers. Any process can query any
  name, which also means data could leak out in DNS queries.
- **A short window at boot.** Until the agent's first tick after
  `started lab126`, the stock firewall applies; Wi-Fi may already be up by
  then, as `wifid` starts on the same event [fw].
- **IPv4 only.** The PW4 kernel has no IPv6 (`CONFIG_IPV6` is unset in the
  5.13.6 kernel config [fw]), and the firmware has no `ip6tables` [fw].
  Should a kernel have IPv6, `scoped` sets `disable_ipv6` on `wlan0`.
- **Time.** The stock clock sync uses `ntp-g7g.amazon.com`
  (`/etc/sysconfig/ntp` [fw]) and is blocked in `scoped`; allowing another NTP
  server does not change the stock client. The clock drifts until Aristotle
  sets it itself.
- **Not tried on the device yet.** The rule set is tested on the host against
  a model of iptables. The first device run should confirm it, with the
  checks below.

## Stock processes that use the network

What follows comes from the firmware [fw] and from KOReader's practice; none
of it is measured on a device yet. Processes that reach out on Wi-Fi include
the Java framework (`cvm`: store, sync, the cloud library), `wifid` (it
validates a new connection by contacting Amazon), the OTA updaters (`otaupd`,
`otav3`), telemetry (`fastmetrics`, `clickstream_logging`), `tmd`, `todo`,
`dmld`, `rcm`, `archive` and the stock NTP client. In `scoped` mode all of
them hit the `DROP` rule; the takeover (`takeover.sh`) also stops most of them.

`wifid`'s validation failing may make it report the network as unusable or
retry; that is not verified. Its upstart job passes `-n` (no validation) when
`/mnt/us/WIFI_NO_NET_PROBE` exists [fw], which is the switch to try first.

Read-only checks to run on the device, Wi-Fi on, first in `off` mode to see
who connects and then in `scoped` mode to see it stopped:

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

The kernel has the `LOG` target and the `owner` match [fw]. A temporary
`LOG` rule before the `DROP` would name the blocked destinations (a change,
not a read-only check). `owner` could later restrict the allowlist to
Aristotle's own user, so stock processes could not use the allowed addresses
at all; that needs Aristotle to run as a user of its own instead of root.
