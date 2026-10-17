# Installing Aristotle

> [!WARNING]
> Changing a Kindle's software can brick it, and no recovery method has been verified for the Paperwhite 4. Try Aristotle only on a device you can afford to lose.

## What you need

- A Kindle Paperwhite 4 (2018, PQ94WIF) on firmware 5.13.x, jailbroken. Other models and firmware are not supported, and the scripts that change the system refuse to run on them (developers can override this with `ARISTOTLE_FORCE=1`).
- KUAL, the launcher that holds Aristotle's menu. MRPI installs it. [kindlemodding.org](https://kindlemodding.org) explains both.
- A computer and a USB cable.
- For the optional agent only: KOReader, whose SSH server it uses, and your SSH public key.

## Install

1. Download `aristotle-X.Y.Z.zip` from the latest [release](https://github.com/piotrkowski/aristotle/releases).
2. Connect the Kindle over USB. Unzip the file to the root of its drive, next to `documents/`, and eject the Kindle.
3. On the Kindle, open KUAL › Aristotle › Start Aristotle.

The zip adds two folders: `aristotle/` (the app, the licences of everything in it, and `SOURCE.txt`, which names the source it was built from) and `extensions/aristotle/` (the KUAL menu). To check the download, compare it with `SHA256SUMS` from the release, or run `gh attestation verify aristotle-X.Y.Z.zip --repo piotrkowski/aristotle`.

To quit, hold the "Library" title for a second, or open KUAL › Aristotle › Return to the Kindle interface. The stock interface comes back after its boot splash, in 30 to 60 seconds. A reboot also brings it back. To update, unzip a newer release over the old one.

Aristotle contains no exploit or DRM-removal code, and it opens only DRM-free books. It uses the e-ink waveforms and firmware already on the Kindle and never ships them or copies them off it.

## The agent (optional)

KUAL › Aristotle › Install agent adds a small service that starts with the Kindle. Before you install it, save your public key, such as `~/.ssh/id_ed25519.pub`, as `extensions/aristotle/authorized_keys` on the Kindle's drive. The agent:

- keeps SSH available over the USB cable, at `192.168.15.244` ([device/README.md](../device/README.md) describes the computer's side);
- lets Aristotle's settings switch the USB port to drive mode for copying books;
- keeps Amazon's update daemons from running, so a firmware update does not undo the setup. To update the firmware on purpose, remove the agent first, and keep the `update.bin.tmp.partial` folder on the drive.

The agent writes three files to the root filesystem (`/etc/upstart/aristotle-agent.conf`, `aristotle-ui.conf` and `aristotle-ota.conf`) and changes no stock file. Everything else goes to the drive and `/var/local`.

## Start Aristotle at boot

With the agent installed, KUAL › Aristotle › Start Aristotle at boot: on makes the Kindle start Aristotle instead of the stock interface, about 41 seconds after power-on. It copies the app to the Kindle's internal storage, so turn it on again after an update. Start Aristotle at boot: off undoes it.

The Kindle starts the stock interface instead when `aristotle-disable` exists on the drive, after three failed launches in a row (until you turn it on again), and for the rest of the boot after you quit Aristotle.

## Your own fonts

Copy TTF or OTF files into a folder named `fonts` at the root of the Kindle's drive, the folder the stock reader uses for its custom fonts, so fonts you added there already work. In a book, open the menu and tap Font: the list shows Literata and each family found in the folder, and a tap lays the book out in it. Put the regular, italic, bold and bold italic files of a family side by side. A family needs a regular face, and files over 20 MB are skipped. Letters a font lacks, such as Polish or Greek ones, are drawn in Literata.

## Uninstall

1. On the Kindle, open KUAL › Aristotle › Remove agent completely. This removes the three jobs, Aristotle at boot with its copy of the app, the update block and the SSH key. The agent's log is saved to the drive as `aristotle-agent.log`.
2. Delete `aristotle/`, `.aristotle/` (the index, page cache and snapshots) and `extensions/aristotle/` from the drive, and `extensions/aristotle-probe/` if you installed a developer build. Older installs have `extensions/aristotle-usbnet/` instead of `extensions/aristotle/`.

Your books, `My Clippings.txt` and the exported notes stay.

## If something goes wrong

Without SSH, open KUAL › Aristotle › USB drive mode for 10 minutes and connect the Kindle to a computer. Create an empty file named `aristotle-disable` at the root of the drive, then restart the Kindle. The agent and Aristotle at boot do not start, and the Kindle runs the stock system. If Aristotle starts at boot, quit it first (hold the "Library" title), so the stock interface offers USB drive mode.

Without KUAL or the agent, the Kindle still runs a script named `emergency.sh` from the root of the drive as root once the stock interface has started (this comes with the community's hotfix; we checked it on our Kindle). [device/README.md](../device/README.md#recovery-without-ssh) says what to put in it. Delete it afterwards.

## Build it yourself

Developers can build the same zip from source on Linux. [CONTRIBUTING.md](../.github/CONTRIBUTING.md#building) lists the tools and commands.
