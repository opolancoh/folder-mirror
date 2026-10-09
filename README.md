# folder-mirror

One-way folder mirroring for Windows, Linux and macOS. The **source** is always right; the
**destination** (the mirror) is made identical: new files are copied, edited files overwritten,
removed files deleted.

## Usage

```bash
# 1. See what would change (changes nothing, writes a result file)
mirror analyze -s \\server\share\work -d E:\mirror\work

# 2. Sync (shows the summary and asks for confirmation)
mirror sync -s \\server\share\work -d E:\mirror\work

# Scheduled / unattended
mirror sync --profile weekly --yes

# Review a plan first, then apply exactly that plan
mirror analyze -s ... -d ... -o plan.json
mirror sync --plan plan.json
```

Run `mirror` without a command to see how the tool works and its default settings, and
`mirror analyze --help` / `mirror sync --help` for every option. Every run starts with a short
summary of how it will work (match mode, deletions, safety limits, exclusions...).

Files that exist only in the destination (for example, added there by mistake) are treated
like files removed from the source: they are deleted from the mirror (to the Recycle Bin by
default). `analyze` lists them under **Removed** first, so you can review them.

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

- `analyze` never changes anything; `sync` asks for confirmation unless `--yes`.
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

Every run writes `mirror-result-<date>.json` in the current folder (or `-o file.json` /
`--format csv`) with the summary, every action (full source and destination paths, reason,
status) and every problem. The full path is printed at the end of the run. A JSON result can be
executed later with `sync --plan`.

An HTML report with the same name (`mirror-result-<date>.html`) is written next to it: summary
cards, problems, and every action with filters and search. It is a single file; open it in any
browser.

### Exit codes

`0` in sync / success · `1` differences found (analyze) · `2` completed with problems ·
`3` aborted (blocked by a safety check, not confirmed, or fatal error).

### Profiles

Copy `mirror.example.toml` to `mirror.toml` and use `--profile <name>` (or `--config <file>`).

## Building

Requires Rust (https://rustup.rs).

```bash
cargo build --release
```

The binary is `target/release/mirror` (`mirror.exe` on Windows). Build on each OS for that OS,
or cross-compile with `--target`. It is a single file; copy it anywhere to use it.

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
