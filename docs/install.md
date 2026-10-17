# Installing Aristotle

You download one zip file and unzip it onto the Kindle over the USB cable.

> [!WARNING]
> This can brick your Kindle. No recovery method has been verified for the Paperwhite 4, which is secure-boot locked. Try Aristotle only on a device you can afford to lose.

## Before you start

The Kindle needs a jailbreak and KUAL, the launcher that holds Aristotle's menu. MRPI installs KUAL and other packages. KOReader is needed only for the optional agent, which uses its SSH server. [kindlemodding.org](https://kindlemodding.org) explains how to install them.

Aristotle contains no jailbreak, exploit or DRM-removal code, and it opens only DRM-free books. It uses the e-ink waveforms and firmware already on the Kindle. Aristotle does not include them, and they never leave the device.

The only tested device is the Paperwhite 4 (2018, PQ94WIF) on firmware 5.13.x. On any other device, the scripts that change the system refuse to run. Developers can override this with `ARISTOTLE_FORCE=1`.

The stock system stays installed. The agent writes one file to the root filesystem, `/etc/upstart/aristotle-agent.conf`. Everything else goes to the user storage and `/var/local`.

## Download

Download `aristotle-X.Y.Z.zip` from the latest [release](https://github.com/piotrkowski/aristotle/releases). The release lists the SHA-256 sum of each file in `SHA256SUMS`. With the GitHub CLI you can also check that a file was built by the project's release workflow:

```sh
sha256sum -c --ignore-missing SHA256SUMS
gh attestation verify aristotle-X.Y.Z.zip --repo piotrkowski/aristotle
```

## Copy it to the Kindle

Connect the Kindle to a computer over USB. It appears as a drive with `documents/` at its root. Unzip the file to the root of that drive, so that two folders appear next to `documents/`:

- `aristotle/`: the app with its fonts and data, the licences of everything in it, and `SOURCE.txt`, which names the source it was built from.
- `extensions/aristotle/`: Aristotle's menu in KUAL.

Then eject the Kindle. To update, unzip a newer release over the old one.

## Run

On the Kindle, open KUAL › Aristotle › Start Aristotle. Aristotle stops the stock interface while it runs. To quit, hold the "Library" title for a second. The stock interface returns after its boot splash, in 30 to 60 seconds. A reboot also brings it back.

## The agent (optional)

KUAL › Aristotle › Install agent installs a small service that starts with the Kindle. It keeps SSH over the USB cable available, lets Aristotle's settings switch the USB port to drive mode for copying books, and restarts the stock interface if it stays down. It runs KOReader's SSH server, so install KOReader first.

The agent accepts one SSH key. Before you install it, save your public key, such as `~/.ssh/id_ed25519.pub`, as `extensions/aristotle/authorized_keys` on the Kindle's drive. The Kindle is then at `192.168.15.244` whenever the cable is connected. [device/README.md](../device/README.md) describes the computer's side of the link.

## Build it yourself

Developers can build the same zip from source on Linux. [CONTRIBUTING.md](../.github/CONTRIBUTING.md#building) lists the tools and commands.

## Uninstall

On the Kindle, open KUAL › Aristotle › Remove agent completely. This removes everything the agent put on the device, its SSH key included. The agent's log is saved to the Kindle's storage as `aristotle-agent.log`.

Then delete `aristotle/`, `.aristotle/` (the index, page cache and snapshots) and `extensions/aristotle/` from the Kindle's storage. If you installed a developer build, delete `extensions/aristotle-probe/` too. Older installs have `extensions/aristotle-usbnet/` instead of `extensions/aristotle/`.

Your books, `My Clippings.txt` and the exported notes stay.

## If something goes wrong

If you cannot reach the Kindle over SSH, open KUAL › Aristotle › USB drive mode for 10 minutes and connect the Kindle to a computer. Create an empty file named `aristotle-disable` at the root of the drive, then restart the Kindle. The agent does not start, and the Kindle runs the stock system.
