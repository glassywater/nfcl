# fontctl

A small Linux font manager that reuses [scoop-nerd-fonts][repo] manifests as
its catalog. It downloads the upstream archive, verifies its SHA-256, extracts
the font files into `~/.local/share/fonts/fontctl/<name>/`, and refreshes the
fontconfig cache.

[repo]: https://github.com/matthewjberger/scoop-nerd-fonts

## Build

```sh
cargo build --release
# binary lands at: ./target/release/fontctl
```

Run it from `target/release/`, drop it into `~/.local/bin/`, or `cargo install
--path .` — whatever you prefer. Examples below use the bare `fontctl` name.

## Quickstart

```sh
fontctl --init                       # one-time setup
fontctl list --all                   # browse the catalog
fontctl search cascadia              # find something
fontctl info Cascadia-Code           # inspect a manifest
fontctl install Cascadia-Code        # download + verify + install
fontctl list                         # see what's installed
fontctl update                       # check for new versions
fontctl update '*'                   # actually upgrade every outdated font
fontctl uninstall Cascadia-Code      # remove it again
```

## Initialize

The first invocation must run `--init` to create the config directory and
choose a path for the scoop-nerd-fonts git repo:

```sh
fontctl --init                                            # interactive prompt
fontctl --init /home/me/work/font/scoop-nerd-fonts        # non-interactive
```

This writes:

- `~/.config/fontctl/config.json`  — CLI settings (`repo_dir`, `cache_dir`,
  optional `proxy`)
- `~/.config/fontctl/installed.json` — record of fonts installed by `fontctl`
- `~/.config/fontctl/bucket.json`    — aggregated copy of all bucket manifests

If the chosen `repo_dir` doesn't exist yet, `fontctl update` will clone it on
demand.

### Re-init / adoption

If `~/.local/share/fonts/fontctl/<name>/` directories already exist on disk
when you run `--init` (typical after copying dotfiles to a new machine, or
after deleting `installed.json` by accident), `fontctl` will scan that tree
and adopt every directory whose name matches a manifest in the bucket:

- A new entry is added to `installed.json` with the manifest's current
  version, the directory's mtime as `installed_at`, and the actual font files
  on disk.
- Entries already present in `installed.json` are left **untouched** — their
  recorded version / install time are presumed more accurate than anything
  we can recover from disk.
- Directories whose name doesn't match any manifest are reported as a
  warning and never deleted.

After that, `fontctl update` will compare those adopted versions against the
bucket and report `OUTDATED` as usual.

## Commands

### list

```sh
fontctl list            # default: installed fonts (table)
fontctl list --all      # full bucket; `*` marks already-installed entries
fontctl list -a         # short alias of --all
```

The `installed` view is a column-aligned table with `NAME / VERSION /
INSTALLED / FILES / SIZE`, plus a footer with the total disk usage:

```
NAME           VERSION  INSTALLED                FILES       SIZE
Cascadia-Code  2407.24  2026-05-04 06:38:05 UTC     84  114.65 MB
---
1 font installed, 114.65 MB total
```

### search

```sh
fontctl search cascadia
```

Substring + similarity search over manifest names and descriptions. Output is
`name  version  description`, one per line.

### info

```sh
fontctl info Cascadia-Code
```

Prints the manifest's `name`, `version`, `description`, `homepage`, `license`,
`extract_dir`, `manifest` path, and download `URLs`. If the font is installed,
it also lists the install timestamp, recorded version, file count, total size,
and font directory.

### install

```sh
fontctl install Cascadia-Code           # fail if already installed
fontctl install Cascadia-Code --force   # reinstall over an existing entry
```

Pipeline:

1. resolve the manifest from the bucket (alias-tolerant: `cascadia-code`,
   `CascadiaCode`, etc. all match)
2. download each `url` into `~/.cache/fontctl/downloads/<name>/`
   - if a file with the same name already exists and its SHA-256 matches the
     manifest's `hash`, **the download is skipped** and the cached archive is
     reused
3. verify SHA-256 with `sha256sum`
4. extract with `unzip`, `tar`, or `7z` (chosen by extension)
5. copy `*.ttf / *.otf / *.ttc / *.otc / *.woff / *.woff2` into
   `~/.local/share/fonts/fontctl/<name>/`
6. run `fc-cache -f` so fontconfig picks up the new files
7. record the install in `installed.json`

For Nerd Font variant manifests (`*-NF`, `*-NF-Mono`, `*-NF-Propo`) the file
filter keeps only the matching variant — installing `FiraCode-NF-Mono` will
not also drop in the propo / non-mono files that ship in the same archive.

### uninstall

```sh
fontctl uninstall Cascadia-Code   # canonical
fontctl rm Cascadia-Code          # alias
fontctl remove Cascadia-Code      # alias
```

Removes `~/.local/share/fonts/fontctl/<name>/`, drops the entry from
`installed.json`, and runs `fc-cache -f`. **Cached download archives are not
deleted** — drop them with `cache rm` if you want the disk space back. The
font-dir removal is guarded against escapes: if the recorded `font_dir` ever
points outside `~/.local/share/fonts/fontctl/`, the operation is refused.

### installed

```sh
fontctl installed
```

Identical to `list` (without `--all`). Kept as an explicit name for scripts.

### update

```sh
fontctl update                  # report only: pull manifests + show what's stale
fontctl update '*'              # also reinstall every outdated font
fontctl update FiraCode-NF      # only act on these names; up-to-date ones are
fontctl update FontA FontB      # skipped, missing-from-installed names error out
fontctl update --install        # legacy alias of `update *`
```

Three argument shapes, one underlying pipeline:

1. **`fontctl update`** — runs `git -C <repo> pull --ff-only` (or `git clone`
   if the repo doesn't exist yet), rebuilds `bucket.json`, then walks every
   font in `installed.json` and prints `OK` / `OUTDATED` / `MISSING`. The
   font directories are not touched.
2. **`fontctl update '*'`** (quote it so the shell doesn't glob) — same
   report, then reinstalls every `OUTDATED` entry by calling
   `install --force` under the hood. Cached download archives whose SHA-256
   still matches the new manifest are reused; otherwise they're re-downloaded.
   `fontctl update --install` is kept as a synonym for old scripts.
3. **`fontctl update <name> [<name>...]`** — same sync step, but the
   `OK / OUTDATED` walk and the reinstall are scoped to just the listed
   names. Up-to-date names are skipped with `OK ... (already current)`.
   Names that aren't in `installed.json` cause an error.

Combining `*` (or `--install`) with explicit names is rejected — pick one.

What `update` does **not** do:
- It will not act on `MISSING` entries (manifest deleted upstream); use
  `fontctl uninstall <name>` to clear them out.
- It will not delete cached old-version archives. Old zips stay in
  `~/.cache/fontctl/downloads/` until you `fontctl cache rm <name>` or
  `fontctl cache rm --all`.

### cache list / cache rm

```sh
fontctl cache list                # what's parked in ~/.cache/fontctl/downloads/
fontctl cache rm Cascadia-Code    # drop one font's cached archive(s)
fontctl cache rm --all            # wipe the whole download cache
```

Removing a cache entry only frees disk space — the installed fonts themselves
are untouched.

### config

```sh
fontctl config                              # print every resolved path + proxy
fontctl config proxy 127.0.0.1:7890         # write "proxy": "127.0.0.1:7890"
fontctl config proxy http://10.0.0.5:3128
fontctl config proxy none                   # remove the proxy key
                                            # (also accepts off / - / clear / "")

# The form is generic — any key/value pair lands in config.json:
fontctl config editor neovim                # writes "editor": "neovim"
fontctl config editor none                  # removes it
```

`config` is just a typed editor for `~/.config/fontctl/config.json`. fontctl
itself acts on a few known keys (`proxy`, `repo_dir`, `cache_dir`); anything
else you set is round-tripped verbatim — kept on disk for your own scripts
or future fontctl versions, never silently dropped by the next `init`.

`version` and `remote` are managed by fontctl and refuse manual writes.

The `proxy` value is read by `install` (and anything else that calls
`download_payload`) and threaded into `curl -x …` or, when only `wget` is
available, into the `http_proxy` / `https_proxy` env vars. Bare `host:port`
is fine; a missing scheme is auto-prefixed with `http://` for `wget`.

Resolution order: the env var **`FONTCTL_PROXY`** wins, otherwise the value
persisted in `config.json` is used. So you can do an ad-hoc one-off install
without persisting:

```sh
FONTCTL_PROXY=127.0.0.1:7890 fontctl install Cascadia-Code
```

### doctor

```sh
fontctl doctor
```

Prints whether each path exists and whether each external tool fontctl shells
out to is available (`git`, `curl`/`wget`, `sha256sum`, `unzip`, `tar`, `7z`,
`fc-cache`).

## Paths

Defaults:

| What          | Path                                |
| ------------- | ----------------------------------- |
| repo          | `/home/Kyecox/work/font/scoop-nerd-fonts` |
| bucket        | `<repo>/bucket`                     |
| config JSON   | `~/.config/fontctl/config.json`     |
| installed JSON| `~/.config/fontctl/installed.json`  |
| bucket cache  | `~/.config/fontctl/bucket.json`     |
| fonts root    | `~/.local/share/fonts/fontctl`      |
| download cache| `~/.cache/fontctl`                  |

Every path can be overridden either with a CLI flag or an env var:

| Flag              | Env var               |
| ----------------- | --------------------- |
| `--repo <path>`   | `FONTCTL_REPO`        |
| `--bucket <path>` | `FONTCTL_BUCKET`      |
| `--config <path>` | `FONTCTL_CONFIG`      |
| `--installed <p>` | `FONTCTL_INSTALLED`   |
| `--bucket-cache <p>` | `FONTCTL_BUCKET_CACHE` |
| `--font-dir <p>`  | `FONTCTL_FONT_DIR`    |
| `--cache-dir <p>` | `FONTCTL_CACHE_DIR`   |
| —                 | `FONTCTL_PROXY`       |

```sh
fontctl --repo /tmp/sf update
fontctl --font-dir /tmp/fonts install FiraCode-NF
FONTCTL_CACHE_DIR=/tmp/c fontctl install Cascadia-Code
```

CLI flag takes precedence over env, env takes precedence over `config.json`.

## Required external tools

`fontctl` is a thin orchestrator and shells out to:

- **`git`** — for `update` (pull/clone)
- **`curl`** *(preferred)* or **`wget`** — for downloads
- **`sha256sum`** — for archive verification
- **`unzip`**, **`tar`**, and/or **`7z`** — for extraction (which one is used
  depends on the archive extension)
- **`fc-cache`** — to refresh fontconfig after install/uninstall

Run `fontctl doctor` to check.
