# Compare reader latency

Use this procedure before and after a latency change. It measures the
current release-driven gestures with original test text. Read the
[metric definitions](benchmarks.md#method) before comparing results.

## Prepare the fixture

1. Build both app bundles with the same options. Record their commits,
   firmware, battery level, and display settings alongside the results.
2. Build the observer and generate the fixture on the host:

   ```sh
	. tools/env.sh
	export CARGO_TARGET_DIR="$ARISTOTLE_CACHE/target"
	cargo build --release --locked --target armv7-unknown-linux-musleabihf -p aristotle-observe
	tools/host/make-latency-fixture.sh "$ARISTOTLE_CACHE/latency-fixture"
   ```

3. Arrange an exclusive, reversible app test session on the device. Exit
   the owner's app first. A paused app still holds the touchscreen grab.
   Do not terminate the boot runner or change protected persistent state.
4. Copy the observer, one app bundle, the generated `books/` directory, and
   `tools/scenarios/aristotle-latency.txt` into
   `/var/tmp/aristotle-latency`. Keep the stock-restoration procedure ready.

## Measure each build

1. Use a fresh fixture library for each build. Run the app in the prepared
   test session with isolated accounts, fonts, dictionaries, and clippings:

   ```sh
	T=/var/tmp/aristotle-latency
	mkdir -p "$T/library" "$T/empty" "$T/accounts"
	: > "$T/empty-clippings.txt"
	ARISTOTLE_ACCOUNTS_DIR="$T/accounts" "$T/aristotle" \
		--data "$T/data" --library "$T/library" --root "$T/books" \
		--dict "$T/empty" --fonts "$T/empty" \
		--test-clippings "$T/empty-clippings.txt" --lang en \
		--turn-mode 9 --open 'Latency fixture' > "$T/app.log" 2>&1
   ```

2. Verify that the test book is on its first page, in day mode, with the
   reader menu closed. Do not capture or index the owner's library.
3. In a second device shell, run the same scenario for both builds:

   ```sh
	T=/var/tmp/aristotle-latency
	"$T/aristotle-observe" run "$T/aristotle-latency.txt" \
		--trace "$T/before.json" > "$T/before.txt"
   ```

   Use `after.json` and `after.txt` for the candidate. Trace paths must not
   exist. Each run warms up, turns six pages forward and six back, and
   opens and closes the reader menu. Its waits leave a 1.5 s quiet window
   after the expected panel update. It does not take screenshots.
4. Copy the traces and app logs to `$ARISTOTLE_CACHE`. Stop only your test
   process, restore the previous device state, and remove your device files.
5. Replay each trace on the host, without accessing a device:

   ```sh
	cargo run --locked -p aristotle-observe -- replay \
		"$ARISTOTLE_CACHE/before.json" --json > "$ARISTOTLE_CACHE/before-report.json"
   ```

## Check the evidence

`--trace` saves touch timestamps, labels, coordinates, and parsed EPDC
updates, including waveform, full mode, rectangle, and completion time.
It saves no framebuffer, book text, account data, or library path.
`replay --json` reports microsecond values and an explicit `valid` field.
Missing completion metrics are `null`. Replay exits unsuccessfully for
incomplete captures, log overruns, unsupported schemas, or invalid timestamps.

New captures record `captured_until_us` and `drain_timed_out`. The observer
drains until all submitted updates finish and the 1.5 s quiet window expires.
Its final drain has a 10 s budget. A timeout preserves the raw trace, marks
it invalid, and suppresses completion metrics. Replay checks the recorded
end time against the rolling quiet deadline instead of trusting the flag.

Each measurement reports `settlement_verified`. A new capture reports `true`
only when it proves the quiet horizon. An interrupted or truncated horizon
reports `false` and no settled time. Legacy traces remain readable and retain
their observed completion values, with `settlement_verified: null` and a
warning in the text report. That unknown acquisition evidence does not
invalidate their recorded first-update or full-screen completions.

Reject a run if `valid` is false or either page-turn label has fewer than
six completed samples. Inspect every `page-forward` and `page-back` update:
each must remain one full-screen, full-mode `glkw16` update (mode 9).
Inspect menu updates separately. Do not compare a combined median of
page turns and menu actions.

Compare medians of `page_done_us` for each page-turn label and
`first_done_us` and `settled_us` for each menu label. For an even number
of samples, average the middle pair. Repeat both builds if the difference
exceeds 10 % without an intended performance change. Preserve both traces
and exact build settings with any published result.
