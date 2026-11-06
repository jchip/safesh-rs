# brush policy spike

Throwaway prototype for P0 of `notes/rust-runner-design.md`. Results are in `notes/rust-runner-spike-results.md`.

It embeds brush-core with a `PolicyLauncher`:
- It checks every external exec. This includes deny-by-name, `git push --force`, and path args of file-changing commands.
- It checks every redirect and `source` open, through a proposed `check_open` hook.
- It re-routes nested `bash`/`sh` back into itself.

## Build

```sh
git clone https://github.com/reubeno/brush ../../.temp/brush-spike/brush
cd ../../.temp/brush-spike/brush
git checkout b2f7782cc941d8c7777940aef1a046986c45c0c3
git apply ../../../spikes/brush-policy/brush-check-open.patch
cd -
CARGO_TARGET_DIR=../../.temp/brush-spike/policy-target cargo +stable build --release
```

## Run

- `hook-tests.sh`: hook coverage cases.
- `latency.py`: cold-start timing. Run it with `python3 -I latency.py`.

Both use absolute paths under `.temp/brush-spike/`.
