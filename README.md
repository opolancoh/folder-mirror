# folder-mirror

One-way folder mirroring for Windows, Linux and macOS. The **source** is always right; the
**destination** (the mirror) is made identical: new files are copied, edited files overwritten,
removed files deleted.

## Usage

One command does everything: it scans both folders, shows what will change in the destination,
asks `Proceed with sync?`, and applies the changes.

```bash
# Analyze, ask, apply
mirror -s \\server\share\work -d E:\mirror\work

# Only show the differences and write the report (never changes anything)
mirror -s \\server\share\work -d E:\mirror\work --dry-run

# Scheduled / unattended: apply without asking
mirror --profile weekly --confirm false

# Review first, then apply exactly the reviewed result file
mirror -s ... -d ... --dry-run -o plan.json
mirror --plan plan.json
```

`--confirm` is `true` by default. Run `mirror` without folders to see how the tool works and its
default settings, and `mirror --help` for every option. Every run starts with a short summary of
how it will work (confirmation, match mode, deletions, safety limits, exclusions...).

Files that exist only in the destination (for example, added there by mistake) are treated
like files removed from the source: they are deleted from the mirror (to the Recycle Bin by
default). The summary lists them under **Delete** before asking, so you can review them.

### Comparison modes (`--mode`)

| Mode | A file is "modified" when | Reads content |
|---|---|---|
| `name` | never (only added / removed) | no |
| `metadata` | size or modification time differs | no |
| `smart` (default) | size differs, or time differs **and** a full hash confirms it | only uncertain files |
| `quick-hash` | size or a hash of 3 sampled blocks differs | partially |
| `full-hash` | size or a hash of the whole file differs | everything |

Times within `--mtime-tolerance` (default 2 s) count as equal (FAT/exFAT round to 2 s).
After a copy, the destination gets the source modification time, so later runs stay fast.

### Deletions (`--delete-mode`)

- `recycle-bin` (default): OS Recycle Bin / Trash. Where it does not exist (network or
  removable drives, headless Linux) files go to the trash folder instead, and the summary says so.
- `trash-folder`: `<destination>/.mirror-trash/<date>/…`, keeping the last `--trash-keep` runs.
- `permanent`.

The old version of each overwritten file is handled the same way (disable with
`--no-backup-overwritten`).

### Safety

- Nothing changes until you confirm (`--confirm false` skips the question); `--dry-run` never changes anything.
- `--max-delete` (default 20%) blocks a sync that would delete too much of the mirror
  (for example when the source path is wrong or a share is temporarily empty).
- Free space is checked before starting.
- Files are copied to a temporary file and renamed into place: no half-written files.
- If a source folder cannot be read, its counterpart in the mirror is **never deleted**.
- Source and destination may not be inside each other.
- Ctrl+C stops after the files in progress; press it twice to quit immediately.

### Problems

The sync never stops on a failed file (unless `--stop-on-error`). Locked files are retried.
At the end, up to 7 problems (`--max-errors-shown`) are listed with their paths; above that,
counts by reason are shown. The result file always lists every problem with full paths.

OS files are excluded on both sides by default (`Thumbs.db`, `desktop.ini`, `.DS_Store`,
`$RECYCLE.BIN`, `System Volume Information`, …). Symbolic links, special files and
online-only cloud files are skipped and listed.

### Result file

Every run writes a result file named with the run's start time and time zone, for example
`folder-mirror-2026-10-09T143000-0500.json`, in the current folder or in `-o <folder>` (also
`output_dir` in a profile). Names are always unique: if one is taken, `-2`, `-3`... is added.
`--format csv` writes CSV instead of JSON. The file holds the summary, every action (full source
and destination paths, reason, status) and every problem; its full path is printed at the end of
the run. A JSON result can be applied later with `mirror --plan <file>`.

The report is saved right after the analysis and updated in place after the sync, so the
analysis is kept even if the sync is interrupted. It has two parts: **analysis** (when, how long,
planned changes) and **execution** (when, how long, confirmed or automatic, done / failed per
line), or the reason it was not executed (dry run, declined, blocked...).

An HTML report with the same name (`.html`) is written next to it: summary
cards, problems, and every action with filters and search. It is a single file; open it in any
browser.

### Exit codes

`0` in sync / success · `1` differences found, not applied · `2` completed with problems ·
`3` aborted (blocked by a safety check, no terminal to confirm, or fatal error).

### Profiles

Copy `mirror.example.toml` to `mirror.toml` and use `--profile <name>` (or `--config <file>`).

## Building

Requires Rust (https://rustup.rs).

### Versions and releases

The version lives only in `Cargo.toml` (`version = "0.1.0"`, [Semantic Versioning](https://semver.org):
`MAJOR.MINOR.PATCH`). It is shown by `mirror --version`, at the start of every run, in the reports,
and in the release file names.

To publish a version, bump `version` in `Cargo.toml`, then run the release script. Every platform
goes into one folder per version:

```
dist/0.1.0/
  mirror-0.1.0-windows-x64.exe   scripts\release.ps1 on Windows
  mirror-0.1.0-linux-x64         scripts\release.ps1 on Windows (needs zig) or release.sh on Linux
  mirror-0.1.0-macos-arm64       scripts/release.sh on a Mac (Apple Silicon)
  mirror-0.1.0-macos-x64         scripts/release.sh on a Mac (Intel)
  SHA256SUMS.txt                 checksums of every file in the folder, regenerated by each script
```

| OS | Command |
|---|---|
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\release.ps1` |
| macOS / Linux | `sh scripts/release.sh` |

macOS binaries can only be built on a Mac (they need Apple's SDK); copy them into the same
`dist/<version>/` folder. To build Linux from Windows, install zig and cargo-zigbuild once:

```bash
winget install zig.zig
```
```bash
cargo install --locked cargo-zigbuild
```

Each binary is the only file needed; copy it anywhere. Everything under `target/` is build cache.
On Windows the C runtime is linked in (see `.cargo/config.toml`), so it runs on any PC without the
Visual C++ Redistributable; the Linux build from Windows is fully static (musl).

### Publishing a new version

Once all changes are done and tested:

1. **Start from a clean, up-to-date `main`.** `git status` must show no pending changes.
   ```bash
   git checkout main
   ```
   ```bash
   git pull
   ```
2. **Check the code.** It must finish with no errors or warnings (and `cargo test`, once there are tests).
   ```bash
   cargo clippy --release
   ```
3. **Pick the new version** following `MAJOR.MINOR.PATCH`:
   - `PATCH` (0.1.0 → 0.1.1) for bug fixes only.
   - `MINOR` (0.1.1 → 0.2.0) for new features that keep existing commands working.
   - `MAJOR` (1.4.2 → 2.0.0) for changes that break existing usage (removed or renamed options).
     While the version is `0.x`, breaking changes may go in a `MINOR` bump.
4. **Set it in `Cargo.toml`** (`version = "0.2.0"`), then build once so `Cargo.lock` is updated too:
   ```bash
   cargo build --release
   ```
5. **Build the release files.** On Windows (Windows, plus Linux if zig is installed):
   ```bash
   powershell -ExecutionPolicy Bypass -File scripts\release.ps1
   ```
   For macOS, run `sh scripts/release.sh` on a Mac with the same commit, copy its files into
   `dist/<version>/`, and run the Windows script again so `SHA256SUMS.txt` includes them.
6. **Smoke-test the published binary.** It must print the new version; then try a `--dry-run`:
   ```bash
   .\dist\0.2.0\mirror-0.2.0-windows-x64.exe --version
   ```
7. **Commit, tag and push the version:**
   ```bash
   git commit -am "Release 0.2.0"
   ```
   ```bash
   git tag v0.2.0
   ```
   ```bash
   git push origin main v0.2.0
   ```
8. **Share the `dist/0.2.0/` folder** (it is not in git). The tag `v0.2.0` marks exactly which code
   each binary was built from.

On Windows, use the MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`) with the
Visual Studio Build Tools "Desktop development with C++" workload:

```bash
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

The GNU toolchain also needs a full MinGW-w64 install (`dlltool` and `as`) to link the Windows
APIs used for the Recycle Bin.

## Project layout

```
src/
  main.rs, lib.rs, app.rs   entry point, library root, use cases (analyze / load_plan / sync)
  features/                 business logic, one folder per feature
    scan/                   walk a folder into a snapshot; exclusions
    compare/                snapshot vs snapshot → diff; strategies/ = one file per mode
    plan/                   diff → ordered actions, summary, safety checks
    sync/                   execute the plan: copy, delete (recycle bin / trash / permanent), retries
    report/                 result file: JSON (re-executable), CSV, and the HTML report
  shared/                   reusable code: errors, paths, files, hashing, config, progress
  cli/                      command line only: args, commands, console output, progress bars
```

Rules: features depend on `shared`, never on `cli`; data flows scan → compare → plan → sync →
report; a new interface (TUI, GUI) only calls `app`.
