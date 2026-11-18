# TODO

- [ ] Build the macOS 0.7.0 binary and replace `Src/Bin/Darwin/fastgmad` in WorkshopDecompressor.
- [ ] Run the Linux checks on a Linux host or VM and send back the output: `cargo test --release`, `cargo clippy --release --all-targets`, and the five checks against `Modern-Addon.gma` (good, bit flip, truncated, bit flip with `--strict`, no arguments).
- [ ] Later: CLI polish pass with an AI skill, to make the terminal output look and sound better.

## Not finished here, and why

- Linux tests, clippy and the five checks: this machine has no Linux host, VM, WSL or Docker, so nothing that needs Linux ran.
- Native Linux binary: not built. The Linux artifact is a musl cross-build from Windows and was never executed.
- macOS binary: not built, there is no Apple SDK or toolchain here.
