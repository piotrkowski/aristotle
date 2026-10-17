# Installing Aristotle

Aristotle is experimental. A mistake can brick the Kindle, and no recovery method is verified for the Paperwhite 4. Use only a device you can afford to lose.

## What you need

- A jailbroken Kindle Paperwhite 4, 2018 model PQ94WIF, on firmware 5.13.x. Other models and firmware are unsupported.
- KUAL and MRPI. [kindlemodding.org](https://kindlemodding.org) explains their setup.
- A computer and USB cable.
- For the optional agent, KOReader and your SSH public key.

Aristotle contains no exploit or DRM-removal code. It opens only DRM-free books. It uses the Kindle's existing waveforms and firmware, never ships them, and never copies them off the device.

## Install

1. Download `aristotle-X.Y.Z.zip` from [Releases](https://github.com/piotrkowski/aristotle/releases).
2. Connect the Kindle over USB.
3. Unzip the archive to the drive root, next to `documents/`.
4. Eject the Kindle.
5. Open **KUAL > Aristotle > Start Aristotle**.

The zip contains `aristotle/`, the app and licenses, and `extensions/aristotle/`, the menu. `SOURCE.txt` identifies the source build. Verify the download with `SHA256SUMS` or:

```sh
gh attestation verify aristotle-X.Y.Z.zip --repo piotrkowski/aristotle
```

Hold **Library** for one second to quit. The stock interface returns after 30 to 60 seconds of boot splash. A reboot also restores it. To update, unpack a newer release over the old one.

## The agent (optional)

Yes, you can use SSH over USB, without an internet connection. It is quite useful.

The agent provides USB SSH at `192.168.15.244`, drive-mode switching from settings, and an update block. See [Host setup](../device/README.md).

1. Copy your SSH public key to `extensions/aristotle/authorized_keys` on the Kindle drive.
2. Open **KUAL > Aristotle > Install agent**.

The agent adds three root-filesystem jobs: `/etc/upstart/aristotle-agent.conf`, `/etc/upstart/aristotle-ui.conf`, and `/etc/upstart/aristotle-ota.conf`. It changes no stock file. Other files go to the drive and `/var/local`.

Before a deliberate firmware update, remove the agent. Keep the `update.bin.tmp.partial` folder on the drive.

## Start Aristotle at boot

With the agent installed, select **Start Aristotle at boot: on**. This copies the app to internal storage and starts it about 41 seconds after power-on [dev]. Select it again after updating the app. **Start Aristotle at boot: off** disables it.

The stock interface starts instead if `aristotle-disable` exists on the drive or after three failed launches. Quitting Aristotle keeps the stock interface for the rest of that boot.

## Uninstall

1. Select **KUAL > Aristotle > Remove agent completely**. This removes its jobs, internal app copy, update block, and SSH key. It saves `aristotle-agent.log` to the drive.
2. Delete `aristotle/`, `.aristotle/`, and `extensions/aristotle/` from the drive.
3. If present, delete `extensions/aristotle-probe/` or the older `extensions/aristotle-usbnet/`.

Books, `My Clippings.txt`, and exported notes remain.

## If something goes wrong

Without SSH, select **KUAL > Aristotle > USB drive mode for 10 minutes**. Connect the Kindle, create an empty `aristotle-disable` file at the drive root, and restart. The stock system starts without the agent or Aristotle at boot.

If Aristotle starts at boot, quit it first so the stock interface can offer drive mode.

Without KUAL or the agent, the community hotfix can run `emergency.sh` from the drive root after the stock interface starts [dev]. Follow [Recovery without SSH](../device/README.md#recovery-without-ssh), then delete the script.

## Build it yourself

See [Building](../.github/CONTRIBUTING.md#building) for Linux tools and commands.
