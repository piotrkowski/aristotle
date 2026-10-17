# Security policy

Report security issues privately with **Report a vulnerability** under
[Advisories](https://github.com/piotrkowski/aristotle/security/advisories/new)
([how it works](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability)).
Do not open a public issue. Include the steps to reproduce, the commit you
built, and the device and firmware.

## What runs on the device

The reader opens no ports. The attack surface is the device agent
(`device/extension/`).

- The agent turns the USB port into a network link (`usb0`, with the Kindle at
  `192.168.15.244`) and starts KOReader's `dropbear` as root, with public-key
  authentication only (`-s`). Anyone with a cable and the private key has root
  on the device. `install-extensions.sh` installs one dedicated key; do not
  reuse a key you use elsewhere.
- `dropbear` listens on `192.168.15.244:22` only, and the agent opens only
  `usb0` in the stock firewall. SSH is not reachable over Wi-Fi.
- The agent installs one file on the root filesystem,
  `/etc/upstart/aristotle-agent.conf`. Everything it runs lives in
  `/var/local/aristotle`. KUAL › Aristotle › Remove agent deletes both, and
  removes the agent's key from KOReader's `settings/SSH/authorized_keys` and
  from the extension. Only the log stays, as `aristotle-agent.log` on the USB
  drive.
- A file named `aristotle-disable` at the root of the USB drive keeps the agent
  from starting at the next boot, which leaves USB to the stock system. To
  create the file while the agent runs, use KUAL › Aristotle › USB drive mode
  for 10 minutes.
- The scripts that change the device outside the USB drive refuse to run on
  anything but a Kindle Paperwhite 4 (2018) on firmware 5.13.x, as read from
  `/proc/usid` and `/etc/prettyversion.txt`, unless `ARISTOTLE_FORCE=1` is
  set.
- In the default Wi-Fi mode, `off`, the agent adds no rules, and the stock
  firewall lets any process, Amazon's included, connect out. The optional
  `scoped` mode (`aristotlectl net scoped`) drops everything on `wlan0` except
  DHCP, DNS to the configured resolvers and the hosts in
  `/var/local/aristotle/net-allow`. It filters by IP address, so anything else
  hosted on an allowed address is reachable too. It does not inspect TLS SNI
  or filter DNS queries, and it applies from the agent's first tick after boot.
  [device/README.md](../device/README.md#network) describes these limits and how to check
  the rules on a device. A way past `scoped` that those limits do not cover is
  a security issue.

The device probe (`device/probe`) is read-only, but its output
identifies the device and its owner (serial number, MAC addresses, Wi-Fi
networks). Review it before sharing any of it, never attach it unedited to a
public issue, and never share the e-ink waveforms it can copy.

Books are parsed by Plato's engine (Rust) and MuPDF (C) from untrusted files.
A malformed book that crashes the reader, or worse, is a security issue.
