# Security policy

Report security issues privately with **Report a vulnerability** under
[Advisories](https://github.com/piotrkowski/aristotle/security/advisories/new)
([how it works](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability)),
not in a public issue. Include the steps to reproduce, the commit you built and
the device and firmware.

## What runs on the device

Aristotle itself has no network code: the reader is offline and opens no
ports. The attack surface is the device agent for development and
installation (`devloop/kindle/aristotle-usbnet/`):

- **SSH as root over the USB cable.** The agent turns the USB port into a
  network link (`usb0`, the Kindle at `192.168.15.244`) and starts KOReader's
  `dropbear` with public-key authentication only (`-s`). Anyone holding the
  private key has root on the device, so keep it to one dedicated key
  (`install-extensions.sh` installs exactly one) and do not reuse a key you use
  elsewhere.
- **Bound to the USB address.** `dropbear` listens on `192.168.15.244:22` only
  (`-p ADDRESS:PORT`, which KOReader's build, dropbear 2026.92, supports), and
  the agent opens only `usb0` in the stock firewall. SSH is not reachable over
  Wi-Fi.
- **Physical access.** Whoever plugs a cable into the Kindle reaches that port.
  Without the key they get no further, but the device is not protected against
  someone who has both the device and the key.
- **One file on the root filesystem.** The agent installs
  `/etc/upstart/aristotle.conf`; everything it runs lives in
  `/var/local/aristotle`. KUAL > Aristotle > Remove agent takes it out again.

Books are parsed by Plato's engine (Rust) and MuPDF (C), which read
untrusted files. A malformed book that crashes the reader or worse is a
security issue too.
