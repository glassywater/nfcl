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
fontctl update            # git pull/clone + rebuild bucket cache + report stale
fontctl update --install  # also reinstall any font whose bucket version moved
```

If `repo_dir` already exists it runs `git pull --ff-only` there, otherwise it
clones from the configured `remote` (default
`https://github.com/matthewjberger/scoop-nerd-fonts.git`). After syncing it
walks `installed.json` and prints which entries are now out of date relative
to the bucket. With `--install`, outdated fonts are reinstalled.

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
fontctl config                     # print every resolved path + proxy
fontctl config 127.0.0.1:7890      # persist a download proxy
fontctl config http://10.0.0.5:3128
fontctl config none                # clear the persisted proxy
                                   # (also accepts off / - / clear / "")
```

The proxy is read by `install` (and any path that calls `download_payload`)
and threaded into `curl -x …` or, when only `wget` is available, into the
`http_proxy` / `https_proxy` env vars. Bare `host:port` is fine; a missing
scheme is auto-prefixed with `http://` for `wget`.

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
