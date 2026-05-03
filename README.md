# fontctl

Linux font manager backed by Scoop manifests from
`/home/Kyecox/work/font/scoop-nerd-fonts/bucket`.

## Commands

```sh
cargo run -- --init
cargo run -- list
cargo run -- search firacode
cargo run -- info firecode
cargo run -- install firecode
cargo run -- uninstall firecode
cargo run -- installed
cargo run -- update
cargo run -- update --install
```

First use must initialize the CLI:

```sh
cargo run -- --init
```

Initialization creates `~/.config/fontctl/config.json` for CLI settings and
`~/.config/fontctl/installed.json` for installed font records. It prompts for
the local `scoop-nerd-fonts` git repo path, and saves that path as `repo_dir` in
`config.json`. You can also pass it directly:

```sh
cargo run -- init /home/Kyecox/work/font/scoop-nerd-fonts
```

`update` syncs `/home/Kyecox/work/font/scoop-nerd-fonts` first:

- if the repo exists, it runs `git pull --ff-only`
- if the repo does not exist, it runs
  `git clone https://github.com/matthewjberger/scoop-nerd-fonts.git`
- after that it compares installed versions in the config JSON with the updated
  bucket manifests

`update --install` also reinstalls outdated fonts.

## Paths

Defaults:

- repo: `/home/Kyecox/work/font/scoop-nerd-fonts`
- bucket: `/home/Kyecox/work/font/scoop-nerd-fonts/bucket`
- config: `~/.config/fontctl/config.json`
- installed: `~/.config/fontctl/installed.json`
- fonts: `~/.local/share/fonts/fontctl`
- cache: `~/.cache/fontctl`

Overrides:

```sh
fontctl --repo /path/to/scoop-nerd-fonts update
fontctl --bucket /path/to/bucket install FiraCode-NF
fontctl --config /path/to/config.json config
fontctl --installed /path/to/installed.json installed
fontctl --font-dir /path/to/fonts install FiraCode-NF
```

Environment variables with the same purpose are also supported:

- `FONTCTL_REPO`
- `FONTCTL_BUCKET`
- `FONTCTL_CONFIG`
- `FONTCTL_INSTALLED`
- `FONTCTL_FONT_DIR`
- `FONTCTL_CACHE_DIR`

## Install behavior

`install` reads the manifest `url`, `hash`, and optional `extract_dir`, then:

1. downloads with `curl` or `wget`
2. verifies SHA-256 with `sha256sum`
3. extracts with `unzip`, `tar`, or `7z`
4. copies font files into `~/.local/share/fonts/fontctl/<manifest-name>`
5. runs `fc-cache -f`
6. writes the installed record to `~/.config/fontctl/installed.json`

For Nerd Font variant manifests such as `FiraCode-NF`, `FiraCode-NF-Mono`,
and `FiraCode-NF-Propo`, font files are filtered to match the requested
variant.
