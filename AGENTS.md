# fastgmad

Drop-in replacement for Garry's Mod `gmad.exe`: extracts `.gma` addons (magic `GMAD`, format version 3). Only extraction exists, there is no creation.

## Commands

```sh
cargo build --release -p fastgmad-bin --bin fastgmad   # shipping binary at target/release/fastgmad
cargo test --release                                    # whole workspace, includes the bin contract tests
cargo test --release -p fastgmad --test extract randomized_roundtrip_of_60_gmas
cargo clippy --release --all-targets
cargo fmt
```

`cargo build` alone builds the workspace but the binary is only useful from `fastgmad-bin`. The `-p` name for the lib is `fastgmad`, not `fastgmad-lib` (directory and crate names differ; imports say `fastgmad::`). `cargo test -p fastgmad` silently skips `fastgmad-bin/tests/summary_line.rs`; run the workspace.

## Layout and boundaries

- `fastgmad-lib` (crate `fastgmad`) holds all logic: `extract` (header/index parsing, both engines, path sanitizing, `ExtractGmaConfig`, `ExtractSummary`), `error` (`FastGmadError` + `FastGmadErrorKind`), private `util` (`BufReadEx`, `ReadSkip`). Format constants are in `src/lib.rs`.
- CLI arg parsing (`ExtractGmaConfig::from_args`, `ExtractGmadIn`, `PrintHelp`) also lives in the lib, gated behind the `binary` feature. `fastgmad-bin` depends on `fastgmad` with `features = ["binary"]` and must stay a thin shell: logger to stderr, dispatch, and the status line. Put new behavior in the lib.
- Both crates share one version number and are bumped together. CI reads the release version from `fastgmad-lib/Cargo.toml`. `extract_gma` and `extract_gma_file` return `ExtractSummary` since 0.7.0; that is the only public API break so far.

## Extraction rules to preserve

- Two engines, one shared parse phase (`read_header`, `write_addon_json`, `read_index`, `create_output_dirs` all run before any worker):
  - `extract_gma(conf, impl BufRead)` for stdin. Sequential copy when `max_io_threads == 1`, otherwise a producer/worker queue bounded by `max_io_memory_usage` (default 256 MiB); single entries larger than the cap stream inline.
  - `extract_gma_file(conf, path)` for real files. Each worker opens its own handle and uses positional reads (`read_at`/`seek_read`) into one reusable 1 MiB buffer, dispatched by an `AtomicUsize` cursor. `max_io_memory_usage` does not apply here.
- Error policy: read errors and truncation abort hard as `InvalidGma` naming the entry; per-entry write failures only warn and count, then the run exits non-zero. Keep this split, tests depend on it.
- Unsafe or duplicate entry names (traversal, non-UTF-8, duplicates, Windows device names) go to `badnames/N.unk`, never dropped. GMA format versions above 3 are rejected, older ones only warn.
- `addon.json` is the GMA description JSON with `title` injected, or a `{title, description}` stub when the description is not JSON.

## Result contract

Subprocess callers (WorkshopDecompressor) read the exit code, one stdout line and stderr. Do not change any of it without bumping `v`.

- Exit codes: 0 success, 1 failure with the reason on stderr, 2 bad arguments and no arguments (usage on stderr, stdout empty either way).
- On success, when stdout is not a terminal, print exactly one line and nothing else: `{"v":1,"entries":N,"bad_names":N,"crc_checked":N,"crc_mismatch":N}`. Field names and order are fixed. Nothing on stdout on failure, and nothing extra when stdout is a terminal.
- `entries` is the file index size, `bad_names` the count routed to `badnames/`, `crc_checked` the entries whose stored CRC was compared, `crc_mismatch` those that failed. A stored CRC of 0 means not recorded and is never counted.
- `--strict` turns any mismatch into exit 1 after all files are written, naming the first mismatching file. Without it a mismatch only warns on stderr.

## CRC policy

- The file index stores a CRC32 per entry. Both engines hash the data while copying (`crc32fast`, no second pass) and compare after the file is written, so a failed write is never counted.
- A mismatch warns and never fails by default. gmad itself never verifies these CRCs (`AddonReader.h` reads `iCRC` and ignores it; `create_gmad.cpp` has `-nocrc` and calls them unused by the game), so rejecting a file gmad accepts would break the drop-in promise. `--strict` is the caller's opt-in.
- The 4 bytes after the last entry are a whole-file CRC of everything before them (`create_gmad.cpp`). It is deliberately not checked: WorkshopDecompressor's `Src/Test/Modern-Addon.gma` stores the last entry's CRC there instead, so a trailer mismatch does not prove corruption. fastgmad has no `.gma` fixture of its own.

## Testing

- `fastgmad-lib/tests/extract.rs` is the suite: both engines, badnames, duplicates, truncation, Windows device names, zero-size entries, CRC counting and `--strict`, and a seeded 60-GMA randomized roundtrip. Extend it when touching extraction.
- `fastgmad-bin/tests/summary_line.rs` pins the status line, the exit codes, `--strict` and the no-argument case by running the real binary. `src/main.rs` has a unit test that the line is suppressed when stdout is a terminal.
- `src/extract/conf.rs` has a unit test pinning the default thread count (`cores - 2`, capped at 3/4 of cores, clamped 1..=32). Change the formula and that test, not just the code.
- The roundtrip test writes hundreds of MB under the temp dir; it is the slow one, run it alone when iterating.

## Build quirks

- `fastgmad-bin/build.rs` rewrites the README usage block between `<!--BEGINUSAGE><!-->` and `<!--ENDUSAGE><!-->` from `fastgmad-bin/src/usage.txt` on every build. Edit `usage.txt`, never that README block, and expect README churn after building.
- `.cargo/config.toml` hardcodes `lld-link` on Windows MSVC, `clang` + `mold` on Linux, and `-C target-cpu=x86-64-v3` (AVX2 baseline) / `apple-m1` on macOS. Builds fail without those linkers, and the resulting binary will not run on pre-AVX2 CPUs. That is intentional for speed, do not relax it to fix a local toolchain.
- Release and dev profiles use `panic = "abort"`, thin LTO, `codegen-units = 1`, stripped symbols.

## CI

`.github/workflows/build.yml` builds nightly artifacts for Windows x64, Linux x64 and macOS arm64, then republishes the rolling `nightly` release. It only triggers on pushes to `updated-fastgmad`, the daily schedule, or manual dispatch, and every job is gated on `github.repository == 'Mitra-88/fastgmad'`, so it stays silent in forks. A schedule run is skipped when nothing changed since the last `nightly` tag.

## Conventions

- Rustfmt: edition 2024, hard tabs, `max_width = 150`, crate-granularity imports. `.editorconfig` sets LF and tab indent, spaces in YAML.
- No inline comments or doc comments for rationale: names should explain the code, and durable knowledge belongs in this file. Deliberate simplifications get a one-line `shortcut:` comment naming the limit and the upgrade path.
- Conventional Commits 1.0.0: `type(scope): description`, lowercase imperative, no trailing period, `!` plus a `BREAKING CHANGE:` footer for breaks.
- Never use em dashes in any file or message; use commas, colons or plain hyphens.
