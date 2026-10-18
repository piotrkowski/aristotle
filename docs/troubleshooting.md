# Troubleshooting

Start with the symptom below. Keep reading data intact while diagnosing problems. A factory reset or edits to boot partitions are not part of these procedures.

## The app does not start

| Symptom | Check or action |
| --- | --- |
| No Aristotle item in KUAL | The zip must unpack to the drive root. Check for `extensions/aristotle/config.xml`, not an extra enclosing release folder. |
| KUAL reports no app | Check that `aristotle/aristotle` and `aristotle/run-aristotle.sh` exist on the drive. |
| The guard refuses the device | Only PW4 on firmware 5.13.x is supported. Do not use `ARISTOTLE_FORCE=1` as a routine workaround. |
| The stock interface returns immediately | Read the app log. Keep a copy before another launch rotates it. |
| The log reports a newer index version or invalid index JSON | Preserve `.aristotle/` and the log. Use a compatible app version or [restore a matching backup](maintenance.md#restore-a-backup). Do not delete the index to make the error disappear. |
| Autostart runs an older version | Quit to stock, then select **Start Aristotle at boot: on** again to refresh the internal copy. |
| Autostart no longer runs | Check for `aristotle-disable` on the drive. Three failed launches also cause fallback. Inspect boot logs before enabling autostart again. |

If you need the stock interface, open **Settings > Return to Kindle**. **Open KUAL**, beside it, returns to stock and opens the installed KUAL booklet. Neither needs the agent. Holding **Library** for one second in Aristotle's library still quits. KUAL also has **Return to the Kindle interface**. A 30 to 60 second boot splash on return is expected [dev].

If Aristotle says KUAL is unavailable, use **Return to Kindle** and open KUAL from the stock library. Direct opening needs the installed KUAL booklet and the updated Aristotle launcher. If the framework or KUAL launch does not become ready within 90 seconds, the launcher leaves the stock interface active and records the failure in its log. Each device command has a timeout of at most three seconds within that total budget. This handoff has host tests only.

For a boot problem without SSH, use the [kill switch](../device/README.md#kill-switch). If needed, follow [Recovery without SSH](../device/README.md#recovery-without-ssh). The emergency script requires the stock framework to start and the USB drive not to be exported. This is not a verified recovery method for an unbootable PW4.

## Books and reading

| Symptom | Check or action |
| --- | --- |
| A copied book is missing | Put it under `documents/`, eject the drive, and restart Aristotle. Try **Books > All**. Hidden entries, dictionaries, and directory symlinks are skipped. |
| A book is listed but will not open | Check [format support](formats.md). Indexing, downloading, and opening are separate capabilities. Encrypted MOBI cannot be read. |
| A CBZ downloads but will not open | The default release disables MuPDF's CBZ reader. Use a supported format. |
| Text size or custom fonts do not affect a page | PDF uses fixed pages. MuPDF reflow, used by TXT and some MOBI files, does not use the chosen custom font. |
| Page numbers are missing | Wait for background pagination. The current position can be saved before page counts are ready. |
| A highlight is absent from clippings | Check **Save highlights to My Clippings.txt** in settings. Bookmarks are not exported there. |
| Stock highlights are absent from the page or Highlights list | Imported annotations can be stored in the index, but the reader currently displays only annotations created in Aristotle. Stock annotation parsing also lacks device-data verification. |
| A deleted highlight remains in clippings | Expected. New entries append to the file, but deletion does not rewrite it. |
| A Markdown export still shows the last deleted highlight | Expected with the current exporter. Removing the last Aristotle highlight leaves the existing Markdown file unchanged. |
| A dictionary is missing or returns nothing | Check the [complete StarDict file set](dictionaries.md#copy-a-complete-dictionary), folder depth, source language, and indexing status. |

## Power and display

The default screensaver does not suspend the CPU. The power button has no effect while the app holds `preventScreenSaver`. Touch wakes the app's screensaver. Quit to stock when you finish reading. Battery use is unverified.

`ARISTOTLE_POWERD=1` enables an alternative sleep path with host tests only. It is not a verified fix for device suspend or battery use.

For ghosting or flicker, report the transition that causes it, whether night mode is on, and whether you changed the default page-turn mode. Do not change waveforms or clear reading state just to collect a report.

## USB drive mode and SSH

| Symptom | Check or action |
| --- | --- |
| The computer sees a network interface, not a drive | With the agent installed, select **USB drive mode for 10 minutes** in KUAL or use Aristotle's USB setting. Quit to stock first if the computer does not see the drive. |
| SSH drops when drive mode starts | Expected. SSH returns when the agent switches back to network mode. |
| SSH times out | Follow [Host setup](../device/README.md#host-setup). Check the cable, NetworkManager profile, and that USB is in network mode. |
| SSH reports permission denied | Check the dedicated private key used by `tools/host/ssh_config` and the public key installed with the agent. Password login is disabled. |
| SSH breaks only on a particular LAN | Its route may overlap `192.168.15.0/24`. See [The USB link](../device/README.md#the-usb-link). |
| A host-key warning appears after reinstall | Verify that the connected device is yours before updating its entry in `$ARISTOTLE_CACHE/known_hosts`. Do not disable host-key checks. |

## Network features

| Message or symptom | Check or action |
| --- | --- |
| No server or catalog after copying JSON | Imports belong next to the running executable, not the library index. Follow [Choose the correct import location](network.md#choose-the-correct-import-location). |
| Can't read the import file | Check the path shown on screen and JSON syntax. Use double quotes and no trailing commas. |
| Can't reach the server or timeout | Enable Wi-Fi in stock, check the server's LAN address and port, and [inspect the network policy](network.md#check-the-network-policy). USB networking has no internet gateway. |
| Destination not allowed | Check the requested host and port, including redirected downloads. Add a trusted destination as described in [network setup](network.md#check-the-network-policy). |
| Certificate does not fit the device's time | Check the Kindle clock. In `scoped` mode, the stock Amazon NTP destination is blocked. Allowing another NTP server does not reconfigure the stock client. |
| User name or password refused | Verify the account on the same KOSync or catalog server. Re-import credentials if needed. |
| Progress does not match KOReader exactly | EPUB uses chapters and percentage, not exact element paths. Check that both readers use identical book files and the same server account. |
| MOBI, AZW3, or TXT progress never reaches KOReader | These formats receive remote progress by percentage, but Aristotle does not send progress for them. Test sending with an EPUB or PDF. |
| Search asks for a settings file | Typed search is unavailable without the on-screen keyboard. Browse the catalog instead. |

## Find the right log

| How Aristotle was started | Log on the Kindle |
| --- | --- |
| **KUAL > Aristotle > Start Aristotle** | `/mnt/us/.aristotle/app.log`, previous run in `app.log.1` |
| `run-aristotle.sh` over SSH | `/var/tmp/aristotle.log`, unless `ARISTOTLE_LOG` overrides it |
| At boot or `aristotlectl app start` | `/var/local/aristotle/app.log`, launcher details in `/var/local/aristotle/ui.log` |
| Agent | `/var/local/aristotle/log` |
| Refused private directory permissions | `/mnt/us/aristotle-refused.log` and syslog |
| Manual USB networking | `/mnt/us/extensions/aristotle/usbnet.log` |
| After agent removal | `/mnt/us/aristotle-agent.log` |

KUAL's app log is readable from the computer with hidden files visible. Collect `/var/tmp` logs before rebooting. Read private agent logs through [USB SSH](../device/README.md#host-setup), not by changing their permissions.

## Report a bug

Include the following in a [bug report](https://github.com/piotrkowski/aristotle/issues):

- Device model and exact firmware version.
- App version and commit from **Settings > About > Software** or `aristotle/SOURCE.txt`.
- Launch method: KUAL, boot, or SSH.
- Steps to reproduce, the expected result, and the actual result.
- Relevant log lines and, for display problems, a photograph or screenshot.
- Book format and whether a public-domain test book has the same problem.

Review logs before sharing them. Remove passwords, tokens, private server addresses, book titles, or paths you do not want public. Do not attach private books, complete probe output, serial numbers, MAC addresses, Wi-Fi details, or waveform files.

Report suspected vulnerabilities privately through the [security policy](../.github/SECURITY.md), not in a public issue.
