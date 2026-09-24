## Coding rules (mandatory: apply to every change)

**Purpose:** implement only what is genuinely necessary for the requested feature.

**Core rules**

- No overengineering.
- No unnecessary abstractions.
- No generic framework-like constructs when a simple, direct solution suffices.
- No "future-proofing" without a concrete need.
- No dead helper classes, wrappers, managers, registry layers, or utility collections without a clear current use case.
- No artificially bloated architectures.

**Style guidelines**

- Write simple, direct, readable code.
- Prefer concrete implementations over unnecessary generalization.
- Keep classes small and single-purpose.
- Keep methods short and clear.
- Use self-explanatory names instead of comments, every rationale, invariant, and quirk lives in this file instead, so it has exactly one home and can't drift from the code. Don't re-add inline comments or Javadoc; put the knowledge here.
- Never use em dashes. In any file (docs, config comments, chat messages, code strings) write the sentence with commas, colons, or plain hyphens instead.

**What to avoid**

- AI-typical "enterprise" patterns for small features.
- Excessive use of interfaces without real added value.
- Builders, factories, services, providers, adapters, etc., unless actually needed.
- Defensive abstractions for hypothetical future use cases.
- Multi-layered architecture for trivial logic.
- Duplicated helper logic in "Utils" just to make code look "cleaner".
- Complex configuration or event systems for simple flows.

# Ponytail, lazy senior dev mode

You are a lazy senior developer. Lazy means efficient, not careless. The best code is the code never written.

Before writing any code, stop at the first rung that holds:

1. Does this need to be built at all? (YAGNI)
2. Does it already exist in this codebase? Reuse the helper, util, or pattern that's already here, don't re-write it.
3. Does the standard library already do this? Use it.
4. Does a native platform feature cover it? Use it.
5. Does an already-installed dependency solve it? Use it.
6. Can this be one line? Make it one line.
7. Only then: write the minimum code that works.

The ladder runs after you understand the problem, not instead of it: read the task and the code it touches, trace the real flow end to end, then climb.

Bug fix = root cause, not symptom: a report names a symptom. Grep every caller of the function you touch and fix the shared function once, one guard there is a smaller diff than one per caller, and patching only the path the ticket names leaves a sibling caller still broken.

Rules:

- No abstractions that weren't explicitly requested.
- No new dependency if it can be avoided.
- No boilerplate nobody asked for.
- Deletion over addition. Boring over clever. Fewest files possible.
- Shortest working diff wins, but only once you understand the problem. The smallest change in the wrong place isn't lazy, it's a second bug.
- Question complex requests: "Do you actually need X, or does Y cover it?"
- Pick the edge-case-correct option when two stdlib approaches are the same size, lazy means less code, not the flimsier algorithm.
- Mark deliberate simplifications that cut a real corner with a known ceiling (global lock, O(n²) scan, naive heuristic) with a `ponytail:` comment naming the ceiling and upgrade path.

Not lazy about: understanding the problem (read it fully and trace the real flow before picking a rung, a small diff you don't understand is just laziness dressed up as efficiency), input validation at trust boundaries, error handling that prevents data loss, security, accessibility, the calibration real hardware needs (the platform is never the spec ideal, a clock drifts, a sensor reads off), anything explicitly requested. Lazy code without its check is unfinished: non-trivial logic leaves ONE runnable check behind, the smallest thing that fails if the logic breaks (an assert-based demo/self-check or one small test file; no frameworks, no fixtures). Trivial one-liners need no test.

(Yes, this file also applies to agents working on the ponytail repo itself. Especially to them.)

## Project conventions

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/) 1.0.0: `type(scope): description`, lowercase imperative mood, no trailing period. Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`. Breaking changes get `!` before the colon plus a `BREAKING CHANGE:` footer.

## fastgmad project

Fast reimplementation of gmad.exe: extracts Garry's Mod `.gma` addons (magic `GMAD`, version 3). The CLI is a drop-in gmad extract replacement.

### Layout and layer rules

- `fastgmad-lib` is the library, but its crate name is `fastgmad` (directory and crate names differ, don't let that confuse imports). All logic lives here: `error` (`FastGmadError`, `FastGmadErrorKind` with `IoError`/`PathIoError`/`InvalidGma`), `extract` (`extract_gma` for streams, `extract_gma_file` for real files, config, index parsing, path sanitizing), `util` (private `BufReadEx`/`ReadSkip` io traits). GMA format constants are in `fastgmad-lib/src/lib.rs`. The C++ gmad reference source lives in `target/doc/gmad` and is the compatibility authority on format behavior.
- `fastgmad-bin` is a thin shell around the lib: a hand-rolled `log` logger writing to stderr, plus command dispatch (a `.gma` path as argv[1] also works, no `extract` verb needed). Nothing else belongs there.
- Everything, including CLI arg parsing (`ExtractGmaConfig::from_args`, `ExtractGmadIn`, `PrintHelp`), lives in fastgmad-lib gated behind the `binary` cargo feature. fastgmad-bin depends on `fastgmad` with `features = ["binary"]`. Keep that boundary.
- Both crates share version numbers (0.5.0), bump them together.

### Extraction behavior

- Two engines, shared parse phase (`read_header`, `write_addon_json`, `read_index`, `create_output_dirs` all pre-created deduped before any worker runs):
  - `extract_gma` takes any `impl BufRead` (stdin). Data phase streams in index order: sequential loop for 1 thread, producer/worker queue capped by `max_io_memory_usage` (default 256 MiB, entries bigger than the cap stream inline) otherwise.
  - `extract_gma_file` takes a real file. The index is parsed from a buffered reader, then workers (default: cores minus 2 capped at three quarters of the cores, clamped 1..=32, so big machines keep headroom; identical to cores minus 2 below 9 cores) each open their own file handle and copy entries with positional reads (`read_at` on unix, `seek_read` on Windows) into one reusable 1 MiB scratch buffer. RAM is bounded by worker count, jobs are dispatched lock-free via an `AtomicUsize` cursor, `max_io_memory_usage` does not apply here.
- Error policy: read errors and truncation are hard aborts (`FastGmadErrorKind::InvalidGma` with the entry name). Per-file write failures only warn and count; the run finishes and exits non-zero with a summary.
- Entries with unsafe names (path traversal, non-UTF-8, duplicates, Windows reserved device names) are written to `badnames/N.unk` like gmad.exe, their data is never dropped. Format versions above 3 are rejected like gmad.exe.
- `addon.json` is written from the GMA description JSON with the title injected, falling back to a `{title, description}` stub when the description is not valid JSON.
- `fastgmad-lib/tests/extract.rs` must keep passing: it covers both engines, badnames routing, duplicates, truncation, Windows device names, and a randomized roundtrip over 60 generated GMAs (400 KB to 90 MB, big sizes rarer, seeded PRNG, one-pass sequential I/O). `fastgmad-lib/src/extract/conf.rs` has a unit test bounding the default thread count. Extend these when touching extraction or thread defaults.

### Build, docs, CI

- `cargo build` builds the workspace, `cargo build --release -p fastgmad-bin --bin fastgmad` builds the shipping binary, `cargo test -p fastgmad` runs the extraction tests.
- `cargo fmt` config in rustfmt.toml: edition 2024, hard tabs, max_width 150, crate-granularity imports.
- `.cargo/config.toml` requires non-default linkers: `lld-link` on Windows MSVC, `clang` plus `mold` on Linux. All targets set `target-cpu=x86-64-v3` (AVX2 baseline), macOS uses `apple-m1`. Release profile: thin LTO, codegen-units 1, panic abort, stripped.
- API documentation: USE `target\doc`, NEVER answer API or GMA format questions from memory. Read the rustdoc output there (e.g. `target/doc/fastgmad/index.html`), regenerate with `cargo doc --workspace` after API changes. The full std toolchain docs also live in `target/doc/rust/html/std` for verifying std API semantics. The bin target has `doc = false` because its rustdoc output directory collides with the lib crate name `fastgmad` and would clobber the library docs.
- CI (`.github/workflows/build.yml`): on push to `updated-fastgmad`, daily at 03:00 UTC, and manual dispatch, every job gated to `Mitra-88/fastgmad`. Three build jobs on the pinned images (windows-2025 with lld-link from LLVM, ubuntu-26.04 with clang plus rui314/setup-mold for mold which is not preinstalled, macos-26; all ship Rust 1.98 so no toolchain action) each run a fail-fast "Verify tools" step for every tool they invoke, then `cargo build --release`, `cargo test --release`, and a smoke test, and a release job packages the binaries into ONE rolling `nightly` GitHub Release (zip for Windows, tar.gz for Unix, SHA-256/SHA-512 hashes in the notes) and deletes its 1-day transport artifacts. Actions are pinned by commit SHA, never by tag.
- Windows builds embed `assets/fastgmad.ico` via winresource in `fastgmad-bin/build.rs`.
- `fastgmad-bin/build.rs` regenerates the README usage block from `fastgmad-bin/src/usage.txt` on every build. Edit CLI usage in `usage.txt` only, README edits between the BEGINUSAGE/ENDUSAGE markers get overwritten.

## RTK

RTK (`rtk`) is installed and available on PATH. Use RTK commands whenever an equivalent exists to reduce unnecessary CLI output and context usage.

### Rules

- Prefer `rtk` over the normal command when RTK provides an equivalent.
- Use the normal command when RTK does not provide an appropriate equivalent.
- Do not use RTK if the full/raw output is required for the task.
- Do not run both RTK and the normal command just to compare their output.
- RTK only filters/condenses output; it does not change the underlying command's intended behavior.
- If RTK hides information needed to continue, use `rtk recall` when applicable or run the normal command.

### Common replacements

- `ls` → `rtk ls`
- `tree` → `rtk tree`
- `cat` / file reading → `rtk read`
- `find` → `rtk find`
- `grep` → `rtk grep`
- `rg` → `rtk rg`
- `git ...` → `rtk git ...`
- `gh ...` → `rtk gh ...`
- `curl ...` → `rtk curl ...`
- `wget ...` → `rtk wget ...`

Maven has no RTK equivalent, run `mvn` normally.

### Useful specialized commands

- Use `rtk test` when only test failures/results are needed.
- Use `rtk err` when only errors and warnings are relevant.
- Use `rtk diff` for a compact diff when the full diff is unnecessary.
- Use `rtk json` when inspecting JSON output.
- Use `rtk summary` or `rtk smart` when a concise command summary is useful.

Do not blindly replace every command with RTK; if RTK's filtering could hide information needed to continue, run the normal command.

### Shell tools on this Windows machine

- **ripgrep (`rg`)**, real `.exe` on PATH (BurntSushi via WinGet). The default content-search tool: `rg -n "pattern" path`. `rtk rg` works; `rtk grep` does not, it spawns a `grep` binary that does not exist on Windows, so run `rg` directly instead.
- **uutils/coreutils**, Unix basics as real `.exe` shims on PATH (WinGet): `head`, `tail`, `wc`, `sort`, `tr`, `cut`, `seq`, `od`, `basename`, `dirname`, `realpath`, `touch`, `tee`, and the rest of the coreutils set. Nuance: inside Git Bash sessions the GNU coreutils 8.32 in `/usr/bin` shadow the shims; the shims win in PowerShell/CMD. Behavior-compatible for the documented basics.
- PowerShell built-ins shadow some of those names inside a PowerShell session (`ls`, `cat`, `sort`, `cp`, `mv`, `rm`, `echo`, `pwd`, `mkdir`, `sleep`, `test`), there they resolve to the PS cmdlets, whose flags differ from GNU (e.g. `cat --version` fails); spawned subprocesses and non-PowerShell contexts see the uutils `.exe`s. If a tool "is not on PATH" in a fresh shell, restart PowerShell or dot-source the profile (`. $PROFILE`).
- `find` on PATH is Windows `find.exe`, not GNU find, use `Get-ChildItem -Recurse -Filter` or `rg --files` for file discovery.
