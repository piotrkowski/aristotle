# Set up sync and catalogs

Aristotle can share reading positions with KOReader through a KOSync server and download books from OPDS 1.2 catalogs. Both features are off until configured. Reading local books needs no network.

These steps use the standard device installation with the [optional agent](install.md#the-agent-optional). Configure Wi-Fi through the stock Kindle interface before starting Aristotle. The USB SSH link has no gateway and does not provide internet access.

There is no on-screen keyboard. Prepare the JSON files on a computer, then import them from the network screens. Use HTTPS where available. HTTP sends requests without transport encryption.

## Choose the correct import location

For a manual KUAL launch, put the files in `aristotle/` on the Kindle drive, next to the `aristotle` executable. Do not put them in `.aristotle/`.

The app reads imports next to the executable that is running. The boot copy runs from `/var/local/aristotle/app/`, not the USB drive. If you use autostart, quit to the stock interface, prepare the USB files, and choose **KUAL > Aristotle > Start Aristotle** to import them. The saved accounts are shared with the boot copy.

Imports run at app startup. The **Load sync.json** and **Load catalogs.json** actions read the files again without a restart. Each screen shows its actual import path.

## Connect progress sync

You need the address of a KOSync-compatible server and an account on it. Use the same server and account in KOReader. Server installation is outside this guide.

1. Quit Aristotle and connect the Kindle in USB drive mode.
2. Create `aristotle/sync.json` on the drive. Replace the example values with your server and credentials:

   ```json
   {"server":"https://sync.example.org","username":"anna","password":"replace-this","auto":true}
   ```

3. Eject the drive and choose **KUAL > Aristotle > Start Aristotle**.
4. Open **Settings > Network > Progress sync**.
5. Check that the server and username appear, then tap **Log in**. Use **Register** only if the server permits registration and you need a new account.
6. Wait for a successful login message.
7. Open an EPUB or PDF, turn a page, and return to the library.
8. Check **Progress sync** for a connection error. Open the identical book file in KOReader with progress sync enabled to check the saved position.

Use identical book files on both devices. Reconverted or edited files can have different document IDs. Automatic sync fetches progress on open and sends changed progress on close for supported formats. A different remote position on another page asks for confirmation before a jump. Failed sends are not queued.

EPUB sync is chapter-based, refined by percentage where possible. It does not reproduce KOReader's exact element position. MOBI, AZW3, and TXT can receive a remote percentage, but Aristotle sends no progress for these formats. See [format support](formats.md#progress-sync).

## Add a calibre catalog

Start calibre's content server on a computer reachable from the Kindle's Wi-Fi network. Find its LAN address and listening port. `localhost` on the Kindle means the Kindle itself, not your computer.

1. Quit Aristotle and connect the Kindle in USB drive mode.
2. Create `aristotle/catalogs.json`. For a server at `192.168.1.10:8080`, use:

   ```json
   {"catalogs":[{"name":"calibre","url":"http://192.168.1.10:8080/opds"}]}
   ```

3. If the server requires authentication, add `username` and `password` to the catalog object. Use a trusted LAN for this HTTP example.
4. Eject the drive and choose **KUAL > Aristotle > Start Aristotle**.
5. Open **Settings > Network > Catalogs** and tap **calibre**.
6. Browse to a book and choose **Download EPUB**.
7. Wait for the saved-path message, then tap **Read now** or return to the library.

The file appears under `documents/calibre/`. Existing files are not replaced. Downloads are limited to 200 MiB and must leave 20 MiB free. The app can list acquisition formats that the default reader build cannot open, including CBZ. Choose a format from the [reader support table](formats.md#default-release-bundle).

Catalog browsing and downloads work without typed search. Although the protocol supports search, entering a query is unavailable without the on-screen keyboard.

## Check the network policy

Importing an account adds its server to the agent's allowlist. It does not enable `scoped` filtering. In the default `off` mode, the stock firewall still permits outbound traffic from other processes.

With [USB SSH configured](../device/README.md#host-setup), inspect the policy from the host:

```sh
. tools/env.sh
ssh -F tools/host/ssh_config kindle 'sh /var/local/aristotle/aristotlectl net status'
```

To enable the existing allowlist, run:

```sh
ssh -F tools/host/ssh_config kindle 'sh /var/local/aristotle/aristotlectl net scoped'
```

Redirects and download links can use a different host or port. If a request reports a blocked destination, allow that specific destination only if you trust it:

```sh
ssh -F tools/host/ssh_config kindle \
	'sh /var/local/aristotle/aristotlectl net allow add books.example.org:443 opds'
```

Filtering is by IP address, not by process or TLS hostname. DNS remains allowed, and stock rules apply until the agent starts. Read the [network policy and limits](../device/README.md#network) before treating `scoped` as an isolation boundary.

## Set the clock without internet access

In `scoped` mode, the stock Amazon NTP server is blocked. A wrong date can
cause TLS certificate errors. Allowing another NTP server does not change
the stock client's configuration.

Check that your computer's clock is correct, then connect the Kindle over
[USB SSH](../device/README.md#host-setup). Compare its clock with the host:

```sh
tools/host/sync-clock.sh --check
```

The output reports the device's time minus the host's time in seconds.
To set the device's runtime UTC clock from the host, run:

```sh
tools/host/sync-clock.sh
```

The helper checks the PW4 firmware guard before changing time. It does not
contact a time server, change NTP settings, write the hardware clock or
enable Wi-Fi. It sends whole seconds over SSH, so connection latency limits
accuracy. Check again after reboot; this is a manual correction, not
automatic time synchronization.

## Change or remove an account

Edit the import file with the new values and use its load action, or restart the manual KUAL app. Imports remove plaintext password fields from successfully read USB files. Credentials are saved privately in `/var/local/aristotle/accounts.json`, not in `.aristotle/`.

Keep the sync server and username unchanged to reuse the saved sync key. If either changes, include the password again.

Catalog imports merge entries by name, ignoring case. Leaving an entry out of `catalogs.json` does not delete the saved catalog. A catalog keeps its stored password only when its URL and username also match. Supply the password again when changing either.

Use **Forget this server** on the sync screen or the remove button beside a catalog to remove saved accounts and their tagged allowlist entries. After forgetting a sync server, delete its `sync.json` import file. After removing a catalog, remove its entry from `catalogs.json` too. This prevents a later manual import from recreating the account. Removing the agent deletes the private credentials. Re-enter credentials after a reinstall rather than restoring the whole agent directory.

For connection errors, see [Troubleshooting](troubleshooting.md#network-features).
