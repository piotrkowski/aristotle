# Installing Aristotle

Aristotle is experimental. A mistake can brick the Kindle, and no recovery method is verified for the Paperwhite 4. Use only a device you can afford to lose.

The default screensaver does not suspend the CPU, and the power button has no effect while Aristotle runs. Battery use is unverified. Quit to the stock interface when you finish reading. The alternative `ARISTOTLE_POWERD=1` path has host tests only, not verified device sleep or wake behavior.

The [glossary](glossary.md) explains device and book-format terms.

## What you need

- A jailbroken Kindle Paperwhite 4, 2018 model PQ94WIF, on firmware 5.13.x. Other models and firmware are unsupported.
- [KUAL](glossary.md#kual), the launcher, installed and working. [kindlemodding.org](https://kindlemodding.org) explains the setup.
- A computer and USB cable.
- For the optional agent, [KOReader](glossary.md#koreader) and your SSH public key.

Aristotle contains no exploit or DRM-removal code. It opens only DRM-free books. It uses the Kindle's existing waveforms and firmware, never ships them, and never copies them off the device.

## Install

1. Download `aristotle-X.Y.Z.zip` and `SHA256SUMS` from the same [release](https://github.com/piotrkowski/aristotle/releases).
2. Verify the zip before extracting or running it. On Linux, run this in the download folder:

   ```sh
   sha256sum --check --ignore-missing SHA256SUMS
   ```

   The zip's line must report `OK`. On other systems, calculate the zip's SHA-256 with a local checksum tool and compare it with the entry in `SHA256SUMS`. Stop if the values differ.

   For releases with build attestations, you can also verify provenance with the GitHub CLI. Replace `X.Y.Z` with the release version:

   ```sh
   gh attestation verify aristotle-X.Y.Z.zip --repo piotrkowski/aristotle
   ```

3. Connect the Kindle over USB and open its drive on the computer.
4. Unzip the archive to the drive root, next to `documents/`.
5. Eject the Kindle.
6. Open **KUAL > Aristotle > Start Aristotle**.

The zip contains:

- `aristotle/`: the app and licenses.
- `extensions/aristotle/`: the KUAL menu.

`aristotle/SOURCE.txt` identifies the source build.

In **Settings**, select **Return to Kindle** to save and quit. **Open KUAL** returns to the stock interface, then opens the installed KUAL booklet. Both buttons stay at the top of every settings page and do not need the agent. Holding **Library** for one second in the library still quits.

The boot splash before the stock interface returns lasted 30 to 60 seconds on the tested PW4. **Return to Kindle** was verified from Settings on that device. The revised KUAL launch confirmation has host tests only, pending a cold-start device retest. A reboot restores stock unless you have enabled Aristotle at boot.

Continue with [Your first book](first-book.md). For later releases, follow [Back up and update](maintenance.md#update-the-app), including the separate internal copy used by autostart.

## The agent (optional)

The agent provides SSH over USB at `192.168.15.244` without an internet connection. It also provides [drive-mode](glossary.md#drive-mode) switching from settings and an update block. The USB link provides no gateway. See [Host setup](../device/README.md#host-setup) for a dedicated SSH key and host network configuration.

1. Copy your SSH public key to `extensions/aristotle/authorized_keys` on the Kindle drive.
2. Open *KUAL > Aristotle > Install agent*.

The agent adds three [Upstart](glossary.md#upstart) service jobs on the [root filesystem](glossary.md#rootfs):

- `/etc/upstart/aristotle-agent.conf`
- `/etc/upstart/aristotle-ui.conf`
- `/etc/upstart/aristotle-ota.conf`

It changes no stock file. Other files go to the drive and `/var/local`.

Before a deliberate firmware update, remove the agent. Keep the `update.bin.tmp.partial` folder on the drive.

## Start Aristotle at boot

With the agent installed, select *Start Aristotle at boot: on*. This copies the app to internal storage and starts it about 41 seconds after power-on.[^1] Select it again after updating the app. *Start Aristotle at boot: off* disables it.

The stock interface starts instead if the [`aristotle-disable` kill switch](glossary.md#kill-switch) exists on the drive or after three failed launches. Quitting Aristotle keeps the stock interface for the rest of that boot.

Updating the internal app retains its previous copy for [rollback](maintenance.md#roll-back-the-boot-app). Quit the boot app before refreshing its copy. Turning autostart off keeps both copies but disables boot startup.

## Your own fonts

1. Quit Aristotle and connect the Kindle in USB drive mode.
2. Copy TTF or OTF files directly into `fonts/` at the drive root. Put a family's regular, italic, bold, and bold italic faces together.
3. Eject the drive and start Aristotle.
4. Open an EPUB, or a MOBI book read through EPUB conversion, and tap the top of the page.
5. Tap **Font**, then the family name. The current page is laid out in that font.

Aristotle uses the stock reader's custom-font folder, so fonts already there can be reused. Files over 20 MiB are skipped. A multi-file family needs a regular face. A single file can be used on its own. Missing Latin and Greek glyphs fall back to Literata. PDF and MuPDF reflow do not use the selected custom font ([Format support](formats.md)).

## Uninstall

If you want to preserve progress, annotations, preferences, or network credentials, [back up first](maintenance.md#back-up-reading-data). Deleting `.aristotle/` removes reading state, not just caches. Agent removal deletes saved network credentials.

1. Quit Aristotle to return to the stock interface.
2. If the agent is installed, select **KUAL > Aristotle > Remove agent completely**. This removes its jobs, internal app copy, update block, and SSH key. It saves `aristotle-agent.log` to the drive.
3. Connect the Kindle in USB drive mode.
4. Delete `aristotle/`, `.aristotle/`, and `extensions/aristotle/` from the drive.
5. If present, delete `extensions/aristotle-probe/` or the older `extensions/aristotle-usbnet/`.
6. Eject the drive.

Books, [`My Clippings.txt`](glossary.md#my-clippingstxt), and exported notes remain.

## If something goes wrong

If Aristotle starts at boot, quit it first so the stock interface can offer drive mode.

Without SSH:

1. Select *KUAL > Aristotle > USB drive mode for 10 minutes*.
2. Connect the Kindle.
3. Create an empty `aristotle-disable` file at the drive root.
4. Restart. The stock system starts without the agent or Aristotle at boot.

Without KUAL or the agent, the community hotfix can run `emergency.sh` from the drive root after the stock interface starts.[^1] Follow [Recovery without SSH](../device/README.md#recovery-without-ssh), then delete the script.

For startup, USB, book, and network symptoms, see [Troubleshooting](troubleshooting.md).

## Build it yourself

See [Building](../.github/CONTRIBUTING.md#building) for Linux tools and commands.

[^1]: Observed on the device.
