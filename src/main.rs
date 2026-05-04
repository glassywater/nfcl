use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const APP_NAME: &str = "fontctl";
const CONFIG_VERSION: u32 = 1;
const DEFAULT_REPO: &str = "/home/Kyecox/work/font/scoop-nerd-fonts";
const DEFAULT_REMOTE: &str = "https://github.com/matthewjberger/scoop-nerd-fonts.git";
const DEFAULT_CONFIG_FILE: &str = "config.json";
const DEFAULT_INSTALLED_FILE: &str = "installed.json";
const DEFAULT_BUCKET_CACHE_FILE: &str = "bucket.json";
const FONT_EXTENSIONS: &[&str] = &["otf", "ttf", "ttc", "otc", "woff", "woff2"];

type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug)]
struct CliError {
    message: String,
}

impl CliError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CliError {}

impl From<io::Error> for CliError {
    fn from(value: io::Error) -> Self {
        Self::new(value.to_string())
    }
}

fn bail<T>(message: impl Into<String>) -> Result<T> {
    Err(CliError::new(message))
}

#[derive(Debug, Clone)]
struct Options {
    repo_dir: PathBuf,
    bucket_dir: PathBuf,
    config_path: PathBuf,
    installed_path: PathBuf,
    bucket_cache_path: PathBuf,
    font_root: PathBuf,
    cache_dir: PathBuf,
    proxy: Option<String>,
    repo_overridden: bool,
    bucket_overridden: bool,
    installed_overridden: bool,
    bucket_cache_overridden: bool,
    cache_dir_overridden: bool,
    proxy_overridden: bool,
}

impl Options {
    fn from_env() -> Result<Self> {
        let repo_env = env_path("FONTCTL_REPO");
        let bucket_env = env_path("FONTCTL_BUCKET");
        let repo_overridden = repo_env.is_some();
        let bucket_overridden = bucket_env.is_some();
        let repo_dir = repo_env.unwrap_or_else(|| PathBuf::from(DEFAULT_REPO));
        let bucket_dir = bucket_env.unwrap_or_else(|| repo_dir.join("bucket"));
        let config_path = env_path("FONTCTL_CONFIG").unwrap_or_else(|| {
            xdg_config_home()
                .unwrap_or_else(|| home_dir().join(".config"))
                .join(APP_NAME)
                .join(DEFAULT_CONFIG_FILE)
        });
        let installed_env = env_path("FONTCTL_INSTALLED");
        let installed_overridden = installed_env.is_some();
        let installed_path = installed_env.unwrap_or_else(|| {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(DEFAULT_INSTALLED_FILE)
        });
        let bucket_cache_env = env_path("FONTCTL_BUCKET_CACHE");
        let bucket_cache_overridden = bucket_cache_env.is_some();
        let bucket_cache_path = bucket_cache_env.unwrap_or_else(|| {
            config_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(DEFAULT_BUCKET_CACHE_FILE)
        });
        let font_root = env_path("FONTCTL_FONT_DIR").unwrap_or_else(|| {
            xdg_data_home()
                .unwrap_or_else(|| home_dir().join(".local/share"))
                .join("fonts")
                .join(APP_NAME)
        });
        let cache_env = env_path("FONTCTL_CACHE_DIR");
        let cache_dir_overridden = cache_env.is_some();
        let cache_dir = cache_env.unwrap_or_else(|| {
            xdg_cache_home()
                .unwrap_or_else(|| home_dir().join(".cache"))
                .join(APP_NAME)
        });
        let proxy_env = env::var("FONTCTL_PROXY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let proxy_overridden = proxy_env.is_some();
        let proxy = proxy_env;

        Ok(Self {
            repo_dir,
            bucket_dir,
            config_path,
            installed_path,
            bucket_cache_path,
            font_root,
            cache_dir,
            proxy,
            repo_overridden,
            bucket_overridden,
            installed_overridden,
            bucket_cache_overridden,
            cache_dir_overridden,
            proxy_overridden,
        })
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{APP_NAME}: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut options = Options::from_env()?;
    let args = parse_global_args(env::args().skip(1).collect(), &mut options)?;
    if options.config_path == options.installed_path {
        return bail("config JSON and installed JSON must be different files");
    }
    if options.bucket_cache_path == options.config_path
        || options.bucket_cache_path == options.installed_path
    {
        return bail("bucket cache JSON must differ from config and installed JSON paths");
    }

    // `init` is the only command that may run without an existing config
    // directory — its job is to create it. For every other invocation
    // (including `help`, `--help`, no args, and unknown commands) we
    // require ~/.config/fontctl/ to exist and prompt for `--init` if not.
    let is_init = args.first().map(String::as_str) == Some("init");
    if !is_init {
        require_config_dir(&options)?;
    }

    if args.is_empty() {
        print_help();
        return Ok(());
    }

    if matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_help();
        return Ok(());
    }

    if is_init {
        cmd_init(&mut options, &args[1..])?;
        return Ok(());
    }

    ensure_initialized(&mut options)?;

    match args[0].as_str() {
        "list" | "ls" => cmd_list(&options, &args[1..])?,
        "search" => cmd_search(&options, &args[1..])?,
        "info" => cmd_info(&options, &args[1..])?,
        "install" | "add" => cmd_install(&options, &args[1..])?,
        "uninstall" | "remove" | "rm" => cmd_uninstall(&options, &args[1..])?,
        "installed" => cmd_installed(&options)?,
        "update" | "pudate" => cmd_update(&mut options, &args[1..])?,
        "config" => cmd_config(&mut options, &args[1..])?,
        "doctor" => cmd_doctor(&options)?,
        "cache" => cmd_cache(&options, &args[1..])?,
        command => {
            return bail(format!(
                "unknown command '{command}'. Run '{APP_NAME} help' for usage."
            ));
        }
    }

    Ok(())
}

fn require_config_dir(options: &Options) -> Result<()> {
    let config_dir = options
        .config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if !config_dir.is_dir() {
        return bail(format!(
            "{APP_NAME} config directory not found: {}. Run '{APP_NAME} --init' first.",
            config_dir.display()
        ));
    }
    Ok(())
}

fn parse_global_args(raw: Vec<String>, options: &mut Options) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--init" => args.push("init".to_string()),
            "--repo" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--repo requires a path"))?;
                options.repo_dir = PathBuf::from(value);
                options.repo_overridden = true;
                if !options.bucket_overridden {
                    options.bucket_dir = options.repo_dir.join("bucket");
                }
            }
            "--bucket" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--bucket requires a path"))?;
                options.bucket_dir = PathBuf::from(value);
                options.bucket_overridden = true;
            }
            "--config" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--config requires a path"))?;
                options.config_path = PathBuf::from(value);
                if !options.installed_overridden {
                    options.installed_path =
                        config_path_parent(&options.config_path).join(DEFAULT_INSTALLED_FILE);
                }
                if !options.bucket_cache_overridden {
                    options.bucket_cache_path =
                        config_path_parent(&options.config_path).join(DEFAULT_BUCKET_CACHE_FILE);
                }
            }
            "--installed" | "--installed-json" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--installed requires a path"))?;
                options.installed_path = PathBuf::from(value);
                options.installed_overridden = true;
            }
            "--bucket-cache" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--bucket-cache requires a path"))?;
                options.bucket_cache_path = PathBuf::from(value);
                options.bucket_cache_overridden = true;
            }
            "--font-dir" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--font-dir requires a path"))?;
                options.font_root = PathBuf::from(value);
            }
            "--cache-dir" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--cache-dir requires a path"))?;
                options.cache_dir = PathBuf::from(value);
                options.cache_dir_overridden = true;
            }
            _ => args.push(raw[i].clone()),
        }
        i += 1;
    }

    Ok(args)
}

fn print_help() {
    println!(
        "\
{APP_NAME} - Linux font manager backed by scoop-nerd-fonts manifests

Usage:
  {APP_NAME} [global-options] <command> [args]

Commands:
  init, --init              Initialize config and choose git repo path
  list [--all]              List installed fonts (use --all to dump the full bucket)
  search <query>            Search bucket manifests
  info <font>               Show manifest details
  install <font> [--force]  Download, verify, extract, and install a font
  uninstall <font>          Remove a font installed by this CLI
  installed                 Show fonts recorded in config JSON
  update [--install]        git pull/clone manifests, then compare installed versions
  cache list                List archives kept in the download cache
  cache rm <font>|--all     Drop a single cached archive or wipe the whole cache
  config [<proxy>]          Print settings, or persist a download proxy
                            (e.g. `{APP_NAME} config 127.0.0.1:7890`,
                             `{APP_NAME} config none` to clear)
  doctor                    Check local tools and paths

Global options:
  --repo <path>             scoop-nerd-fonts git directory
  --bucket <path>           manifest bucket directory
  --config <path>           config JSON path
  --installed <path>        installed font JSON path
  --bucket-cache <path>     aggregated bucket cache JSON path
  --font-dir <path>         root directory for installed font files
  --cache-dir <path>        cache directory for downloads/extraction

Defaults:
  config:        ~/.config/{APP_NAME}/{DEFAULT_CONFIG_FILE}
  installed:     ~/.config/{APP_NAME}/{DEFAULT_INSTALLED_FILE}
  bucket cache:  ~/.config/{APP_NAME}/{DEFAULT_BUCKET_CACHE_FILE}
  repo:          {DEFAULT_REPO}
  remote:        {DEFAULT_REMOTE}
"
    );
}

fn cmd_init(options: &mut Options, args: &[String]) -> Result<()> {
    let mut config = load_config(&options.config_path)?;

    // Preserve any cache_dir the user has previously persisted (or hand-edited
    // into config.json), unless this invocation explicitly overrode it via
    // --cache-dir / FONTCTL_CACHE_DIR.
    if !options.cache_dir_overridden {
        if let Some(saved) = config
            .cache_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            options.cache_dir = expand_home(saved);
        }
    }
    // Same idea for proxy: env var FONTCTL_PROXY wins, otherwise we reuse
    // whatever was previously persisted.
    if !options.proxy_overridden {
        options.proxy = config
            .proxy
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
    }

    let installed_to_create = if options.installed_path.exists() {
        None
    } else {
        Some(
            load_installed(&options.installed_path).unwrap_or_else(|_| InstalledFile {
                version: repo_version_or_unknown(&options.repo_dir),
                installed: BTreeMap::new(),
            }),
        )
    };
    let selected_repo = if let Some(path) = args.iter().find(|arg| !arg.starts_with('-')) {
        expand_home(path)
    } else {
        prompt_repo_path(&options.repo_dir)?
    };

    if selected_repo.as_os_str().is_empty() {
        return bail("repo path cannot be empty");
    }

    options.repo_dir = selected_repo;
    if !options.bucket_overridden {
        options.bucket_dir = options.repo_dir.join("bucket");
    }

    config.repo_dir = Some(options.repo_dir.to_string_lossy().to_string());
    config.cache_dir = Some(options.cache_dir.to_string_lossy().to_string());
    config.proxy = options.proxy.clone();
    save_config(&options.config_path, &config)?;
    if let Some(mut installed) = installed_to_create {
        installed.version = repo_version_or_unknown(&options.repo_dir);
        save_installed(&options.installed_path, &installed)?;
    }
    let cache_built = refresh_bucket_cache(options)?;

    println!("Initialized {APP_NAME}.");
    println!("Config:         {}", options.config_path.display());
    println!("Installed JSON: {}", options.installed_path.display());
    println!("Bucket cache:   {}", options.bucket_cache_path.display());
    println!("Repo:           {}", options.repo_dir.display());
    println!("Cache dir:      {}", options.cache_dir.display());
    if !cache_built {
        println!(
            "Bucket directory not found yet; run '{APP_NAME} update' to clone the repo and build the bucket cache."
        );
    }
    println!(
        "Run '{APP_NAME} update' to git pull or clone scoop-nerd-fonts, then compare installed versions."
    );
    Ok(())
}

fn ensure_initialized(options: &mut Options) -> Result<()> {
    // The config directory existence is checked once up front in `run()` for
    // every non-init command, so by the time we get here the directory is
    // guaranteed to exist; only the file itself still needs verifying.
    if !options.config_path.exists() {
        return bail(format!(
            "{APP_NAME} is not initialized. Run '{APP_NAME} --init' first. Expected config: {}",
            options.config_path.display()
        ));
    }

    let config = load_config(&options.config_path)?;
    let repo_dir = config
        .repo_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CliError::new(format!(
                "{APP_NAME} config has no repo_dir. Run '{APP_NAME} --init' to choose the git repo path."
            ))
        })?;

    if !options.repo_overridden {
        options.repo_dir = expand_home(repo_dir);
    }
    if !options.bucket_overridden {
        options.bucket_dir = options.repo_dir.join("bucket");
    }
    if !options.cache_dir_overridden {
        if let Some(cache) = config
            .cache_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            options.cache_dir = expand_home(cache);
        }
    }
    if !options.proxy_overridden {
        options.proxy = config
            .proxy
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }

    // Lazy migration: if the on-disk config predates the cache_dir field,
    // snapshot the current resolution into config.json so it shows up on
    // future inspections and so users can edit one file to relocate it.
    if config.cache_dir.is_none() {
        let mut migrated = config;
        migrated.cache_dir = Some(options.cache_dir.to_string_lossy().to_string());
        save_config(&options.config_path, &migrated)?;
    }

    Ok(())
}

fn prompt_repo_path(default_repo: &Path) -> Result<PathBuf> {
    println!("Initializing {APP_NAME}.");
    println!(
        "Choose the scoop-nerd-fonts git repo path. 'update' will run git pull there, or clone into it if missing."
    );
    print!("Repo path [{}]: ", default_repo.display());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default_repo.to_path_buf())
    } else {
        Ok(expand_home(trimmed))
    }
}

fn cmd_list(options: &Options, args: &[String]) -> Result<()> {
    // Default view is "what did I install" — that's what feels right for a
    // package manager prompt. `--all` (or `-a`) opts into the full manifest
    // catalog with `*` markers for already-installed fonts. We deliberately
    // don't accept `--bucket`/`--installed` here because both are taken as
    // global path overrides by parse_global_args.
    let want_bucket = args.iter().any(|arg| arg == "--all" || arg == "-a");
    if !want_bucket {
        return cmd_installed(options);
    }

    let index = load_bucket_index(&options.bucket_dir)?;
    let config = load_installed(&options.installed_path)?;

    for entry in &index.entries {
        let marker = if config.installed.contains_key(&entry.name) {
            "*"
        } else {
            " "
        };
        println!("{marker} {}", entry.name);
    }
    println!("{} manifests", index.entries.len());
    Ok(())
}

fn cmd_search(options: &Options, args: &[String]) -> Result<()> {
    let query = first_positional(args, "search requires a query")?;
    let needle = normalize(query);

    let mut matches = Vec::new();
    let mut used_cache = false;
    if options.bucket_cache_path.exists() {
        match load_bucket_cache(&options.bucket_cache_path) {
            Ok(cache) => {
                used_cache = true;
                for (name, font) in &cache.fonts {
                    let haystack = format!("{} {}", normalize(name), normalize(&font.description));
                    if haystack.contains(&needle) {
                        let version_text = if font.version.is_empty() {
                            "unknown".to_string()
                        } else {
                            font.version.clone()
                        };
                        matches.push((name.clone(), version_text, font.description.clone()));
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "warning: failed to read bucket cache ({err}); falling back to manifest scan. Run '{APP_NAME} update' to rebuild it."
                );
            }
        }
    }
    if !used_cache {
        let index = load_bucket_index(&options.bucket_dir)?;
        for entry in &index.entries {
            let manifest = read_manifest(&entry.path)?;
            let description = manifest.description.as_deref().unwrap_or_default();
            let haystack = format!("{} {}", normalize(&entry.name), normalize(description));
            if haystack.contains(&needle) {
                matches.push((
                    entry.name.clone(),
                    manifest.version_text(),
                    description.to_string(),
                ));
            }
        }
    }

    if matches.is_empty() {
        println!("No manifest matched '{query}'.");
        return Ok(());
    }

    for (name, version, description) in matches {
        if description.is_empty() {
            println!("{name}  {version}");
        } else {
            println!("{name}  {version}  {description}");
        }
    }
    Ok(())
}

fn cmd_info(options: &Options, args: &[String]) -> Result<()> {
    let query = first_positional(args, "info requires a font name")?;
    let index = load_bucket_index(&options.bucket_dir)?;
    let entry = resolve_manifest(&index, query)?;
    let manifest = read_manifest(&entry.path)?;
    let config = load_installed(&options.installed_path)?;

    println!("Name:        {}", manifest.name);
    println!("Version:     {}", manifest.version_text());
    if let Some(description) = &manifest.description {
        println!("Description: {description}");
    }
    if let Some(homepage) = &manifest.homepage {
        println!("Homepage:    {homepage}");
    }
    if let Some(license) = &manifest.license {
        println!("License:     {license}");
    }
    if let Some(extract_dir) = &manifest.extract_dir {
        println!("Extract dir: {extract_dir}");
    }
    println!("Manifest:    {}", manifest.path.display());
    println!("URLs:");
    for url in &manifest.urls {
        println!("  {url}");
    }
    let installed = config.installed.get(&manifest.name);
    match installed {
        Some(record) => {
            let size = installed_size(record);
            println!("Installed:    yes");
            println!("Installed at: {}", parse_install_time(&record.installed_at));
            println!("Installed ver: {}", record.version);
            println!("Files:        {}", record.files.len());
            println!("Size:         {}", format_size(size));
            println!("Font dir:     {}", record.font_dir);
        }
        None => println!("Installed:    no"),
    }
    Ok(())
}

fn cmd_install(options: &Options, args: &[String]) -> Result<()> {
    let force = args.iter().any(|arg| arg == "--force" || arg == "-f");
    let query = first_positional(args, "install requires a font name")?;
    let record = install_by_query(options, query, force)?;
    println!(
        "Installed {} {} with {} font files.",
        record.name,
        record.version,
        record.files.len()
    );
    println!("Config: {}", options.config_path.display());
    Ok(())
}

fn cmd_uninstall(options: &Options, args: &[String]) -> Result<()> {
    let query = first_positional(args, "uninstall requires a font name")?;
    let mut config = load_installed(&options.installed_path)?;
    let key = resolve_installed_key(&config, query).or_else(|| {
        load_bucket_index(&options.bucket_dir)
            .ok()
            .and_then(|index| resolve_manifest(&index, query).ok())
            .and_then(|entry| {
                if config.installed.contains_key(&entry.name) {
                    Some(entry.name)
                } else {
                    None
                }
            })
    });

    let key =
        key.ok_or_else(|| CliError::new(format!("'{query}' is not installed by {APP_NAME}")))?;
    let record = config
        .installed
        .remove(&key)
        .ok_or_else(|| CliError::new(format!("'{key}' is not installed by {APP_NAME}")))?;

    remove_installed_dir(&record.font_dir, &options.font_root)?;
    save_installed(&options.installed_path, &config)?;
    refresh_font_cache(&options.font_root);
    println!("Uninstalled {key}.");
    println!("Installed JSON: {}", options.installed_path.display());
    Ok(())
}

fn cmd_installed(options: &Options) -> Result<()> {
    let config = load_installed(&options.installed_path)?;
    if config.installed.is_empty() {
        println!("No fonts installed by {APP_NAME}.");
        return Ok(());
    }

    struct Row {
        name: String,
        version: String,
        installed_at: String,
        files: String,
        size_text: String,
    }

    let mut rows: Vec<Row> = Vec::with_capacity(config.installed.len());
    let mut total_size = 0u64;
    for record in config.installed.values() {
        let size = installed_size(record);
        total_size += size;
        rows.push(Row {
            name: record.name.clone(),
            version: record.version.clone(),
            installed_at: parse_install_time(&record.installed_at),
            files: record.files.len().to_string(),
            size_text: format_size(size),
        });
    }

    let header = ("NAME", "VERSION", "INSTALLED", "FILES", "SIZE");
    let w_name = rows
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max(header.0.len());
    let w_ver = rows
        .iter()
        .map(|r| r.version.len())
        .max()
        .unwrap_or(0)
        .max(header.1.len());
    let w_date = rows
        .iter()
        .map(|r| r.installed_at.len())
        .max()
        .unwrap_or(0)
        .max(header.2.len());
    let w_files = rows
        .iter()
        .map(|r| r.files.len())
        .max()
        .unwrap_or(0)
        .max(header.3.len());
    let w_size = rows
        .iter()
        .map(|r| r.size_text.len())
        .max()
        .unwrap_or(0)
        .max(header.4.len());

    println!(
        "{:<w_name$}  {:<w_ver$}  {:<w_date$}  {:>w_files$}  {:>w_size$}",
        header.0,
        header.1,
        header.2,
        header.3,
        header.4,
        w_name = w_name,
        w_ver = w_ver,
        w_date = w_date,
        w_files = w_files,
        w_size = w_size,
    );

    for row in &rows {
        println!(
            "{:<w_name$}  {:<w_ver$}  {:<w_date$}  {:>w_files$}  {:>w_size$}",
            row.name,
            row.version,
            row.installed_at,
            row.files,
            row.size_text,
            w_name = w_name,
            w_ver = w_ver,
            w_date = w_date,
            w_files = w_files,
            w_size = w_size,
        );
    }

    println!(
        "---\n{} font{} installed, {} total",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" },
        format_size(total_size)
    );
    Ok(())
}

fn cmd_update(options: &mut Options, args: &[String]) -> Result<()> {
    let apply_install = args.iter().any(|arg| arg == "--install");
    sync_bucket_repo(options)?;
    options.bucket_dir = options.repo_dir.join("bucket");

    let mut config = load_installed(&options.installed_path)?;
    config.version = git_repo_version(&options.repo_dir)?;
    save_installed(&options.installed_path, &config)?;
    let bucket_cache = build_and_save_bucket_cache(options)?;
    println!(
        "Bucket cache: {} ({} fonts)",
        options.bucket_cache_path.display(),
        bucket_cache.fonts.len()
    );

    if config.installed.is_empty() {
        println!(
            "No installed fonts in {}.",
            options.installed_path.display()
        );
        return Ok(());
    }

    let index = load_bucket_index(&options.bucket_dir)?;
    let mut outdated = Vec::new();
    let mut missing = Vec::new();
    let mut current = 0usize;

    for record in config.installed.values() {
        let entry = match resolve_manifest(&index, &record.name) {
            Ok(entry) => entry,
            Err(_) => {
                missing.push(record.name.clone());
                continue;
            }
        };
        let manifest = read_manifest(&entry.path)?;
        let manifest_version = manifest.version_text();
        if manifest_version == record.version {
            current += 1;
            println!("OK       {} {}", record.name, record.version);
        } else {
            println!(
                "OUTDATED {} installed={} bucket={}",
                record.name, record.version, manifest_version
            );
            outdated.push(manifest.name.clone());
        }
    }

    for name in &missing {
        println!("MISSING  {name} manifest not found after git update");
    }

    println!(
        "Checked {} installed fonts: {current} current, {} outdated, {} missing.",
        config.installed.len(),
        outdated.len(),
        missing.len()
    );

    if apply_install {
        for name in outdated {
            let record = install_by_query(options, &name, true)?;
            println!(
                "Updated {} to {} with {} font files.",
                record.name,
                record.version,
                record.files.len()
            );
        }
    } else if !outdated.is_empty() {
        println!("Run '{APP_NAME} update --install' to reinstall outdated fonts.");
    }

    Ok(())
}

fn cmd_config(options: &mut Options, args: &[String]) -> Result<()> {
    // `fontctl config` (no args)             -> print current settings
    // `fontctl config <host:port|url>`       -> persist proxy = <value>
    // `fontctl config none|off|-|""`         -> clear the persisted proxy
    if !args.is_empty() {
        if args.len() > 1 {
            return bail("config takes at most one argument: the proxy value (or 'none' to clear)");
        }
        let raw = args[0].trim();
        let mut on_disk = load_config(&options.config_path)?;
        let new_proxy: Option<String> = match raw {
            "" | "none" | "off" | "-" | "clear" => None,
            other => Some(other.to_string()),
        };
        on_disk.proxy = new_proxy.clone();
        // repo_dir/cache_dir come from current Options so save_config doesn't
        // overwrite them with stale on-disk values it just loaded back.
        on_disk.repo_dir = Some(options.repo_dir.to_string_lossy().to_string());
        on_disk.cache_dir = Some(options.cache_dir.to_string_lossy().to_string());
        save_config(&options.config_path, &on_disk)?;
        // Reflect the change in this process's Options too — useful if another
        // command (e.g. install) is chained later in scripts.
        if !options.proxy_overridden {
            options.proxy = new_proxy.clone();
        }
        match new_proxy {
            Some(value) => println!("proxy = {value}"),
            None => println!("proxy cleared"),
        }
        return Ok(());
    }

    println!("Repo:         {}", options.repo_dir.display());
    println!("Bucket:       {}", options.bucket_dir.display());
    println!("Config:       {}", options.config_path.display());
    println!("Installed:    {}", options.installed_path.display());
    println!("Bucket cache: {}", options.bucket_cache_path.display());
    println!("Fonts:        {}", options.font_root.display());
    println!("Cache:        {}", options.cache_dir.display());
    println!(
        "Proxy:        {}",
        options.proxy.as_deref().unwrap_or("(none)")
    );
    println!("Remote:       {DEFAULT_REMOTE}");
    Ok(())
}

fn cmd_doctor(options: &Options) -> Result<()> {
    println!("Repo:         {}", path_status(&options.repo_dir));
    println!("Bucket:       {}", path_status(&options.bucket_dir));
    println!("Config:       {}", options.config_path.display());
    println!("Installed:    {}", options.installed_path.display());
    println!("Bucket cache: {}", path_status(&options.bucket_cache_path));
    println!("Fonts:        {}", options.font_root.display());
    println!("Cache:        {}", options.cache_dir.display());
    println!("git:          {}", command_status("git"));
    println!("curl:         {}", command_status("curl"));
    println!("wget:         {}", command_status("wget"));
    println!("sha256:       {}", command_status("sha256sum"));
    println!("unzip:        {}", command_status("unzip"));
    println!("tar:          {}", command_status("tar"));
    println!("7z:           {}", command_status("7z"));
    println!("fc-cache:     {}", command_status("fc-cache"));
    Ok(())
}

fn cmd_cache(options: &Options, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("list") | Some("ls") => cmd_cache_list(options),
        Some("rm") | Some("clean") | Some("clear") => cmd_cache_rm(options, &args[1..]),
        Some(other) => bail(format!(
            "unknown cache subcommand '{other}'. Try 'cache list' or 'cache rm <name>|--all'."
        )),
        None => bail("cache requires a subcommand: list, rm"),
    }
}

fn cmd_cache_list(options: &Options) -> Result<()> {
    let downloads_dir = options.cache_dir.join("downloads");
    let work_dir = options.cache_dir.join("work");

    let mut entries: BTreeMap<String, (u64, Vec<String>, bool)> = BTreeMap::new();
    if downloads_dir.is_dir() {
        for entry in fs::read_dir(&downloads_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let mut size = 0u64;
            let mut files = Vec::new();
            for f in fs::read_dir(&path)? {
                let f = f?;
                let p = f.path();
                if p.is_file() {
                    size += p.metadata()?.len();
                    if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                        files.push(name.to_string());
                    }
                }
            }
            files.sort();
            entries.insert(name, (size, files, false));
        }
    }
    if work_dir.is_dir() {
        for entry in fs::read_dir(&work_dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                entries
                    .entry(name.to_string())
                    .or_insert_with(|| (0, Vec::new(), false))
                    .2 = true;
            }
        }
    }

    if entries.is_empty() {
        println!("Cache is empty ({}).", options.cache_dir.display());
        return Ok(());
    }

    let mut total = 0u64;
    for (name, (size, files, has_work)) in &entries {
        total += size;
        let work_marker = if *has_work { " [+work]" } else { "" };
        let archives = if files.is_empty() {
            "(no archives)".to_string()
        } else {
            files.join(", ")
        };
        println!(
            "{name}{work_marker}  {}  {archives}",
            format_size(*size)
        );
    }
    println!("---");
    println!(
        "{} cache entr{}, {} total",
        entries.len(),
        if entries.len() == 1 { "y" } else { "ies" },
        format_size(total)
    );
    Ok(())
}

fn cmd_cache_rm(options: &Options, args: &[String]) -> Result<()> {
    let all = args.iter().any(|a| a == "--all");
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .map(|a| a.as_str())
        .collect();

    let downloads_dir = options.cache_dir.join("downloads");
    let work_dir = options.cache_dir.join("work");

    if all {
        if !positional.is_empty() {
            return bail("cache rm: --all and <name> are mutually exclusive");
        }
        let mut freed = 0u64;
        let mut removed_any = false;
        for sub in [&downloads_dir, &work_dir] {
            if sub.is_dir() {
                freed += dir_size(sub).unwrap_or(0);
                fs::remove_dir_all(sub)?;
                removed_any = true;
            }
        }
        if !removed_any {
            println!("Cache was already empty.");
        } else {
            println!("Cleared cache ({} freed).", format_size(freed));
        }
        return Ok(());
    }

    if positional.is_empty() {
        return bail("cache rm requires a font name or --all");
    }

    let mut total_freed = 0u64;
    let mut removed = 0usize;
    for query in positional {
        let mut hit = false;
        let mut entry_freed = 0u64;
        let mut display_name = query.to_string();
        for sub in [&downloads_dir, &work_dir] {
            if let Some(actual) = resolve_cache_name(sub, query) {
                display_name = actual.clone();
                let target = sub.join(&actual);
                let size = dir_size(&target).unwrap_or(0);
                fs::remove_dir_all(&target)?;
                entry_freed += size;
                hit = true;
            }
        }
        if hit {
            removed += 1;
            total_freed += entry_freed;
            println!("Removed {display_name} ({} freed)", format_size(entry_freed));
        } else {
            eprintln!("warning: no cache entry for '{query}'");
        }
    }

    if removed > 0 {
        println!("---");
        println!(
            "Removed {removed} entr{}, {} freed",
            if removed == 1 { "y" } else { "ies" },
            format_size(total_freed)
        );
    }
    Ok(())
}

/// Resolve a user-supplied cache name to an actual on-disk subdirectory under
/// `parent`. Tries exact match first, then case-insensitive. Returns the
/// real on-disk name, so callers can both report and `remove_dir_all` it.
fn resolve_cache_name(parent: &Path, query: &str) -> Option<String> {
    if !parent.is_dir() {
        return None;
    }
    if parent.join(query).is_dir() {
        return Some(query.to_string());
    }
    let q_lower = query.to_lowercase();
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if name.to_lowercase() == q_lower {
            return Some(name);
        }
    }
    None
}

fn dir_size(path: &Path) -> Result<u64> {
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_dir() {
            total += dir_size(&p)?;
        } else if let Ok(meta) = p.metadata() {
            total += meta.len();
        }
    }
    Ok(total)
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.2} MB", b / MB)
    } else if b >= KB {
        format!("{:.2} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Render a unix timestamp (seconds since 1970-01-01 UTC) as
/// "YYYY-MM-DD HH:MM:SS UTC" using Howard Hinnant's `civil_from_days`
/// algorithm — no extra crate, handles negatives.
fn format_unix_timestamp(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if m <= 2 {
        y += 1;
    }

    let h = (secs_of_day / 3600) as u32;
    let min = ((secs_of_day % 3600) / 60) as u32;
    let s = (secs_of_day % 60) as u32;
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02} UTC")
}

fn parse_install_time(value: &str) -> String {
    value
        .parse::<i64>()
        .map(format_unix_timestamp)
        .unwrap_or_else(|_| "(unknown)".to_string())
}

fn installed_size(record: &InstalledFont) -> u64 {
    record
        .files
        .iter()
        .filter_map(|f| fs::metadata(f).ok())
        .map(|m| m.len())
        .sum()
}

#[derive(Debug, Clone)]
struct BucketEntry {
    name: String,
    path: PathBuf,
}

#[derive(Debug, Clone)]
struct BucketIndex {
    entries: Vec<BucketEntry>,
}

fn load_bucket_index(bucket_dir: &Path) -> Result<BucketIndex> {
    if !bucket_dir.is_dir() {
        return bail(format!(
            "bucket directory not found: {}",
            bucket_dir.display()
        ));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(bucket_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("json")) {
            if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                entries.push(BucketEntry {
                    name: stem.to_string(),
                    path,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(BucketIndex { entries })
}

fn resolve_manifest(index: &BucketIndex, query: &str) -> Result<BucketEntry> {
    let query = query.trim().trim_end_matches(".json");
    if query.is_empty() {
        return bail("empty font name");
    }

    for entry in &index.entries {
        if entry.name.eq_ignore_ascii_case(query) {
            return Ok(entry.clone());
        }
    }

    let query_norm = normalize(query);
    let mut alias_hits: Vec<(u8, BucketEntry)> = Vec::new();
    let mut aliases = Vec::new();

    for (idx, entry) in index.entries.iter().enumerate() {
        for (alias, priority) in aliases_for_name(&entry.name) {
            aliases.push((alias.clone(), idx, priority));
            if alias == query_norm {
                alias_hits.push((priority, entry.clone()));
            }
        }
    }

    if !alias_hits.is_empty() {
        alias_hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.cmp(&b.1.name)));
        let best_priority = alias_hits[0].0;
        let best: Vec<_> = alias_hits
            .into_iter()
            .filter(|(priority, _)| *priority == best_priority)
            .map(|(_, entry)| entry)
            .collect();
        if best.len() == 1 {
            return Ok(best[0].clone());
        }
        return bail(format!(
            "'{query}' is ambiguous: {}",
            best.iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let mut scored = Vec::new();
    for (alias, idx, priority) in aliases {
        let score = similarity(&query_norm, &alias);
        if score >= 0.82 {
            scored.push((score, priority, index.entries[idx].clone()));
        }
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
            .then(a.2.name.cmp(&b.2.name))
    });

    if let Some((best_score, best_priority, best_entry)) = scored.first().cloned() {
        let same_best = scored
            .iter()
            .filter(|(score, priority, _)| {
                (*score - best_score).abs() < 0.0001 && *priority == best_priority
            })
            .map(|(_, _, entry)| entry.name.clone())
            .collect::<Vec<_>>();
        if same_best.len() == 1 {
            return Ok(best_entry);
        }
    }

    let suggestions = nearest_names(index, &query_norm, 5);
    if suggestions.is_empty() {
        bail(format!("font manifest not found: {query}"))
    } else {
        bail(format!(
            "font manifest not found: {query}. Did you mean: {}?",
            suggestions.join(", ")
        ))
    }
}

fn aliases_for_name(name: &str) -> Vec<(String, u8)> {
    let mut values = Vec::new();
    push_alias(&mut values, &name.to_lowercase(), 0);
    push_alias(&mut values, &normalize(name), 0);

    let lower = name.to_lowercase();
    if lower.ends_with("-nf") {
        let base = &name[..name.len() - 3];
        push_alias(&mut values, &normalize(base), 2);
        push_alias(&mut values, &base.to_lowercase(), 2);
    } else if lower.ends_with("-nf-mono") {
        let base = &name[..name.len() - 8];
        push_alias(&mut values, &format!("{}mono", normalize(base)), 2);
        push_alias(&mut values, &format!("{}-mono", base.to_lowercase()), 2);
    } else if lower.ends_with("-nf-propo") {
        let base = &name[..name.len() - 9];
        push_alias(&mut values, &format!("{}propo", normalize(base)), 2);
        push_alias(&mut values, &format!("{}-propo", base.to_lowercase()), 2);
    }

    values.sort();
    values.dedup();
    values
}

fn push_alias(values: &mut Vec<(String, u8)>, alias: &str, priority: u8) {
    let alias = alias.trim().to_string();
    if !alias.is_empty() {
        values.push((alias, priority));
    }
}

fn nearest_names(index: &BucketIndex, query_norm: &str, limit: usize) -> Vec<String> {
    let mut scored = index
        .entries
        .iter()
        .map(|entry| {
            (
                similarity(query_norm, &normalize(&entry.name)),
                entry.name.clone(),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name)| name)
        .collect()
}

#[derive(Debug, Clone)]
struct Manifest {
    name: String,
    path: PathBuf,
    version: Option<String>,
    description: Option<String>,
    homepage: Option<String>,
    license: Option<String>,
    urls: Vec<String>,
    hashes: Vec<Option<String>>,
    extract_dir: Option<String>,
}

impl Manifest {
    fn version_text(&self) -> String {
        self.version
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[derive(Debug, Default, Deserialize)]
struct ManifestRaw {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default, deserialize_with = "deserialize_license")]
    license: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    url: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_or_array")]
    hash: Vec<String>,
    #[serde(default)]
    extract_dir: Option<String>,
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let text = fs::read_to_string(path)?;
    let raw: ManifestRaw = serde_json::from_str(&text).map_err(|err| {
        CliError::new(format!("invalid manifest {}: {err}", path.display()))
    })?;

    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CliError::new(format!("invalid manifest path: {}", path.display())))?
        .to_string();
    if raw.url.is_empty() {
        return bail(format!("manifest missing url: {}", path.display()));
    }
    let hashes = normalize_hashes(raw.url.len(), raw.hash);

    Ok(Manifest {
        name,
        path: path.to_path_buf(),
        version: raw.version,
        description: raw.description,
        homepage: raw.homepage,
        license: raw.license,
        urls: raw.url,
        hashes,
        extract_dir: raw.extract_dir,
    })
}

fn normalize_hashes(url_len: usize, values: Vec<String>) -> Vec<Option<String>> {
    if values.is_empty() {
        return vec![None; url_len];
    }
    if values.len() == 1 && url_len > 1 {
        return (0..url_len).map(|_| Some(values[0].clone())).collect();
    }
    (0..url_len)
        .map(|idx| {
            values
                .get(idx)
                .cloned()
                .filter(|value| !value.trim().is_empty())
        })
        .collect()
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CliConfig {
    #[serde(default)]
    repo_dir: Option<String>,
    #[serde(default)]
    cache_dir: Option<String>,
    #[serde(default)]
    proxy: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct InstalledFile {
    #[serde(default = "unknown_version", deserialize_with = "deserialize_string_lenient")]
    version: String,
    #[serde(default)]
    installed: BTreeMap<String, InstalledFont>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct InstalledFont {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    manifest: String,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    hashes: Vec<String>,
    #[serde(default)]
    installed_at: String,
    #[serde(default)]
    font_dir: String,
    #[serde(default)]
    files: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct BucketCache {
    #[serde(default = "unknown_version")]
    version: String,
    #[serde(default, rename = "bucket")]
    fonts: BTreeMap<String, CachedFont>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct CachedFont {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    homepage: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    extract_dir: String,
    #[serde(default)]
    urls: Vec<String>,
    #[serde(default)]
    hashes: Vec<String>,
    #[serde(default)]
    manifest: String,
}

fn unknown_version() -> String {
    "unknown".to_string()
}

/// Manifest `url` / `hash` may be either a plain string or an array; flatten
/// both into `Vec<String>`.
fn deserialize_string_or_array<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(s) => vec![s],
        serde_json::Value::Array(arr) => arr
            .into_iter()
            .filter_map(|v| match v {
                serde_json::Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// Accept the legacy `"version": <number>` shape that older fontctl builds
/// wrote into installed.json before we switched to git versions.
fn deserialize_string_lenient<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(match value {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        _ => String::new(),
    })
}

/// Scoop manifests sometimes write `license` as an object
/// `{"identifier":"MIT","url":"..."}` rather than a plain string. Pull
/// the identifier out so we still surface something useful.
fn deserialize_license<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(serde_json::Value::String(s)) => Some(s),
        Some(serde_json::Value::Object(map)) => map
            .get("identifier")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        _ => None,
    })
}

fn load_config(path: &Path) -> Result<CliConfig> {
    if !path.exists() {
        return Ok(CliConfig {
            repo_dir: None,
            cache_dir: None,
            proxy: None,
        });
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::new(format!("invalid config {}: {err}", path.display())))
}

fn save_config(path: &Path, config: &CliConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut value = serde_json::Map::new();
    value.insert("version".to_string(), serde_json::json!(CONFIG_VERSION));
    value.insert(
        "repo_dir".to_string(),
        serde_json::Value::String(
            config
                .repo_dir
                .as_deref()
                .unwrap_or(DEFAULT_REPO)
                .to_string(),
        ),
    );
    value.insert(
        "remote".to_string(),
        serde_json::Value::String(DEFAULT_REMOTE.to_string()),
    );
    if let Some(cache_dir) = config.cache_dir.as_deref().filter(|s| !s.is_empty()) {
        value.insert(
            "cache_dir".to_string(),
            serde_json::Value::String(cache_dir.to_string()),
        );
    }
    if let Some(proxy) = config.proxy.as_deref().filter(|s| !s.is_empty()) {
        value.insert(
            "proxy".to_string(),
            serde_json::Value::String(proxy.to_string()),
        );
    }
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(value))
        .map_err(|err| CliError::new(format!("config serialize: {err}")))?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

fn load_installed(path: &Path) -> Result<InstalledFile> {
    if !path.exists() {
        return Ok(InstalledFile {
            version: unknown_version(),
            installed: BTreeMap::new(),
        });
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::new(format!("invalid installed JSON {}: {err}", path.display())))
}

fn save_installed(path: &Path, config: &InstalledFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(config)
        .map_err(|err| CliError::new(format!("installed serialize: {err}")))?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

fn build_bucket_cache(bucket_dir: &Path, repo_dir: &Path) -> Result<BucketCache> {
    let index = load_bucket_index(bucket_dir)?;
    let mut fonts = BTreeMap::new();
    for entry in &index.entries {
        match read_manifest(&entry.path) {
            Ok(manifest) => {
                let cached = CachedFont {
                    name: manifest.name.clone(),
                    version: manifest.version.clone().unwrap_or_default(),
                    description: manifest.description.clone().unwrap_or_default(),
                    homepage: manifest.homepage.clone().unwrap_or_default(),
                    license: manifest.license.clone().unwrap_or_default(),
                    extract_dir: manifest.extract_dir.clone().unwrap_or_default(),
                    urls: manifest.urls.clone(),
                    hashes: manifest
                        .hashes
                        .iter()
                        .map(|value| value.clone().unwrap_or_default())
                        .collect(),
                    manifest: manifest.path.to_string_lossy().to_string(),
                };
                fonts.insert(manifest.name, cached);
            }
            Err(err) => {
                eprintln!(
                    "warning: skipped manifest {}: {err}",
                    entry.path.display()
                );
            }
        }
    }
    Ok(BucketCache {
        version: repo_version_or_unknown(repo_dir),
        fonts,
    })
}

fn save_bucket_cache(path: &Path, cache: &BucketCache) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut text = serde_json::to_string_pretty(cache)
        .map_err(|err| CliError::new(format!("bucket cache serialize: {err}")))?;
    text.push('\n');
    fs::write(path, text)?;
    Ok(())
}

fn load_bucket_cache(path: &Path) -> Result<BucketCache> {
    if !path.exists() {
        return Ok(BucketCache {
            version: unknown_version(),
            fonts: BTreeMap::new(),
        });
    }
    let text = fs::read_to_string(path)?;
    serde_json::from_str(&text)
        .map_err(|err| CliError::new(format!("invalid bucket cache {}: {err}", path.display())))
}

/// Rebuild the bucket cache only when the repo bucket directory exists yet.
/// Returns `Ok(true)` when the cache was (re)written, `Ok(false)` when
/// the repo bucket isn't present (e.g. right after `init` before `update`).
fn refresh_bucket_cache(options: &Options) -> Result<bool> {
    if !options.bucket_dir.is_dir() {
        return Ok(false);
    }
    build_and_save_bucket_cache(options)?;
    Ok(true)
}

fn build_and_save_bucket_cache(options: &Options) -> Result<BucketCache> {
    let cache = build_bucket_cache(&options.bucket_dir, &options.repo_dir)?;
    save_bucket_cache(&options.bucket_cache_path, &cache)?;
    Ok(cache)
}

fn install_by_query(options: &Options, query: &str, force: bool) -> Result<InstalledFont> {
    let index = load_bucket_index(&options.bucket_dir)?;
    let entry = resolve_manifest(&index, query)?;
    let manifest = read_manifest(&entry.path)?;
    install_manifest(options, &manifest, force)
}

fn install_manifest(options: &Options, manifest: &Manifest, force: bool) -> Result<InstalledFont> {
    if manifest.urls.is_empty() {
        return bail(format!("manifest has no URLs: {}", manifest.name));
    }

    let mut config = load_installed(&options.installed_path)?;
    config.version = repo_version_or_unknown(&options.repo_dir);
    if config.installed.contains_key(&manifest.name) && !force {
        return bail(format!(
            "{} is already installed. Use --force to reinstall.",
            manifest.name
        ));
    }

    ensure_tool_for_download()?;
    ensure_command("sha256sum")?;

    let work_dir = options.cache_dir.join("work").join(&manifest.name);
    let extracted_dir = work_dir.join("extracted");
    if work_dir.exists() {
        fs::remove_dir_all(&work_dir)?;
    }
    fs::create_dir_all(&extracted_dir)?;

    let download_dir = options.cache_dir.join("downloads").join(&manifest.name);
    fs::create_dir_all(&download_dir)?;

    for (idx, url) in manifest.urls.iter().enumerate() {
        let filename = scoop_filename(url, idx);
        let payload_path = download_dir.join(format!("{:02}-{}", idx + 1, filename));
        let expected = manifest.hashes.get(idx).and_then(|value| value.as_deref());

        // Cache short-circuit: if we already have the archive and its sha256
        // matches the manifest, skip the download. Without a manifest hash we
        // can't trust a stale file, so redownload to be safe.
        let cached = payload_path.exists()
            && match expected {
                Some(hash) => verify_sha256(&payload_path, hash).is_ok(),
                None => false,
            };
        if cached {
            println!(
                "Using cached {} for {}",
                payload_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| payload_path.display().to_string()),
                manifest.name
            );
        } else {
            if payload_path.exists() {
                fs::remove_file(&payload_path)?;
            }
            download_payload(url, &payload_path, options.proxy.as_deref())?;
            if let Some(hash) = expected {
                verify_sha256(&payload_path, hash)?;
            }
        }

        let target = extracted_dir.join(format!("{:02}", idx + 1));
        fs::create_dir_all(&target)?;
        unpack_payload(&payload_path, &target, &filename)?;
    }

    let roots = font_search_roots(&extracted_dir, manifest.extract_dir.as_deref());
    let mut font_files = Vec::new();
    for root in roots {
        if root.exists() {
            collect_font_files(&root, &mut font_files)?;
        }
    }

    font_files = filter_manifest_fonts(manifest, font_files);
    font_files.sort();
    font_files.dedup();

    if font_files.is_empty() {
        return bail(format!("no font files found for {}", manifest.name));
    }

    let target_dir = options.font_root.join(&manifest.name);
    if target_dir.exists() {
        fs::remove_dir_all(&target_dir)?;
    }
    fs::create_dir_all(&target_dir)?;

    let mut copied = Vec::new();
    let mut used_names = HashSet::new();
    for source in &font_files {
        let filename = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| CliError::new(format!("invalid font filename: {}", source.display())))?;
        let target_name = unique_filename(filename, &mut used_names);
        let target = target_dir.join(target_name);
        fs::copy(source, &target)?;
        copied.push(target.to_string_lossy().to_string());
    }

    refresh_font_cache(&target_dir);

    // Drop the extraction scratch directory now that the fonts have been
    // copied to font_root. The downloaded archive in `download_dir` is kept
    // so subsequent reinstalls can short-circuit; the user can drop it via
    // `fontctl cache rm <name>`.
    if work_dir.exists() {
        let _ = fs::remove_dir_all(&work_dir);
    }

    let record = InstalledFont {
        name: manifest.name.clone(),
        version: manifest.version_text(),
        manifest: manifest.path.to_string_lossy().to_string(),
        urls: manifest.urls.clone(),
        hashes: manifest
            .hashes
            .iter()
            .map(|value| value.clone().unwrap_or_default())
            .collect(),
        installed_at: now_unix_seconds(),
        font_dir: target_dir.to_string_lossy().to_string(),
        files: copied,
    };

    config
        .installed
        .insert(manifest.name.clone(), record.clone());
    config.version = repo_version_or_unknown(&options.repo_dir);
    save_installed(&options.installed_path, &config)?;
    Ok(record)
}

fn sync_bucket_repo(options: &Options) -> Result<()> {
    ensure_command("git")?;
    if options.repo_dir.exists() {
        if !options.repo_dir.join(".git").is_dir() {
            return bail(format!(
                "repo path exists but is not a git repository: {}",
                options.repo_dir.display()
            ));
        }
        run_command(
            Command::new("git")
                .arg("-C")
                .arg(&options.repo_dir)
                .arg("pull")
                .arg("--ff-only"),
            "git pull",
        )?;
    } else {
        if let Some(parent) = options.repo_dir.parent() {
            fs::create_dir_all(parent)?;
        }
        run_command(
            Command::new("git")
                .arg("clone")
                .arg(DEFAULT_REMOTE)
                .arg(&options.repo_dir),
            "git clone",
        )?;
    }
    Ok(())
}

fn git_repo_version(repo_dir: &Path) -> Result<String> {
    ensure_command("git")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .arg("describe")
        .arg("--tags")
        .arg("--always")
        .arg("--dirty")
        .output()?;

    if !output.status.success() {
        return bail(format!(
            "failed to read git version for {}",
            repo_dir.display()
        ));
    }

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        bail(format!("empty git version for {}", repo_dir.display()))
    } else {
        Ok(version)
    }
}

fn repo_version_or_unknown(repo_dir: &Path) -> String {
    if repo_dir.join(".git").is_dir() {
        git_repo_version(repo_dir).unwrap_or_else(|_| "unknown".to_string())
    } else {
        "unknown".to_string()
    }
}

fn unpack_payload(payload: &Path, target: &Path, original_filename: &str) -> Result<()> {
    if is_font_file(payload) {
        fs::copy(payload, target.join(sanitize_filename(original_filename)))?;
        return Ok(());
    }

    let lower = payload
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_lowercase();

    if lower.ends_with(".zip") {
        ensure_command("unzip")?;
        run_command(
            Command::new("unzip")
                .arg("-q")
                .arg("-o")
                .arg(payload)
                .arg("-d")
                .arg(target),
            "unzip",
        )?;
    } else if lower.ends_with(".tar.gz")
        || lower.ends_with(".tgz")
        || lower.ends_with(".tar.xz")
        || lower.ends_with(".txz")
        || lower.ends_with(".tar.bz2")
        || lower.ends_with(".tbz2")
        || lower.ends_with(".tar")
    {
        ensure_command("tar")?;
        run_command(
            Command::new("tar")
                .arg("-xf")
                .arg(payload)
                .arg("-C")
                .arg(target),
            "tar",
        )?;
    } else if lower.ends_with(".7z") {
        ensure_command("7z")?;
        run_command(
            Command::new("7z")
                .arg("x")
                .arg("-y")
                .arg(format!("-o{}", target.display()))
                .arg(payload),
            "7z",
        )?;
    } else {
        return bail(format!(
            "unsupported payload type: {}",
            payload
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("?")
        ));
    }

    Ok(())
}

fn download_payload(url: &str, target: &Path, proxy: Option<&str>) -> Result<()> {
    let clean_url = download_url(url);
    if command_exists("curl") {
        let mut cmd = Command::new("curl");
        cmd.arg("-L").arg("--fail");
        if let Some(p) = proxy {
            // curl accepts host:port without a scheme and assumes HTTP.
            cmd.arg("-x").arg(p);
        }
        cmd.arg("--output").arg(target).arg(clean_url);
        run_command(&mut cmd, "curl download")
    } else if command_exists("wget") {
        let mut cmd = Command::new("wget");
        if let Some(p) = proxy {
            // wget needs full URLs in its proxy env vars; add a scheme if the
            // user only typed host:port.
            let normalized = if p.contains("://") {
                p.to_string()
            } else {
                format!("http://{p}")
            };
            cmd.env("http_proxy", &normalized)
                .env("https_proxy", &normalized)
                .arg("-e")
                .arg("use_proxy=yes");
        }
        cmd.arg("-O").arg(target).arg(clean_url);
        run_command(&mut cmd, "wget download")
    } else {
        bail("missing downloader: install curl or wget")
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<()> {
    let expected = expected.trim().to_ascii_lowercase();
    if expected.is_empty() || expected == "skip" {
        return Ok(());
    }
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return bail(format!("sha256sum failed for {}", path.display()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let actual = stdout.split_whitespace().next().unwrap_or_default();
    if actual != expected {
        return bail(format!(
            "hash mismatch for {}: expected {}, got {}",
            path.display(),
            expected,
            actual
        ));
    }
    Ok(())
}

fn font_search_roots(root: &Path, extract_dir: Option<&str>) -> Vec<PathBuf> {
    let Some(extract_dir) = extract_dir else {
        return vec![root.to_path_buf()];
    };

    let mut roots = Vec::new();
    let direct = root.join(extract_dir);
    if direct.exists() {
        roots.push(direct);
    }
    if let Ok(children) = fs::read_dir(root) {
        for child in children.flatten() {
            let candidate = child.path().join(extract_dir);
            if candidate.exists() {
                roots.push(candidate);
            }
        }
    }

    if roots.is_empty() {
        roots.push(root.to_path_buf());
    }
    roots
}

fn collect_font_files(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_font_files(&path, out)?;
        } else if is_font_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn filter_manifest_fonts(manifest: &Manifest, files: Vec<PathBuf>) -> Vec<PathBuf> {
    let name = manifest.name.to_ascii_lowercase();
    if name.ends_with("-nf-mono") {
        return files
            .into_iter()
            .filter(|path| filename_contains(path, "nerdfontmono"))
            .collect();
    }
    if name.ends_with("-nf-propo") {
        return files
            .into_iter()
            .filter(|path| filename_contains(path, "nerdfontpropo"))
            .collect();
    }
    if name.ends_with("-nf") {
        return files
            .into_iter()
            .filter(|path| {
                filename_contains(path, "nerdfont")
                    && !filename_contains(path, "nerdfontmono")
                    && !filename_contains(path, "nerdfontpropo")
            })
            .collect();
    }
    files
}

fn filename_contains(path: &Path, needle: &str) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|name| normalize(name).contains(needle))
        .unwrap_or(false)
}

fn is_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|ext| FONT_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn unique_filename(filename: &str, used: &mut HashSet<String>) -> String {
    if used.insert(filename.to_string()) {
        return filename.to_string();
    }

    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("font");
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    for idx in 2.. {
        let candidate = if ext.is_empty() {
            format!("{stem}-{idx}")
        } else {
            format!("{stem}-{idx}.{ext}")
        };
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
}

fn remove_installed_dir(recorded_dir: &str, expected_root: &Path) -> Result<()> {
    if recorded_dir.trim().is_empty() {
        return Ok(());
    }
    let path = PathBuf::from(recorded_dir);
    if !path.exists() {
        return Ok(());
    }

    let canonical_path = fs::canonicalize(&path)?;
    let canonical_root = fs::canonicalize(expected_root)?;
    if canonical_path == canonical_root || !canonical_path.starts_with(&canonical_root) {
        return bail(format!(
            "refusing to remove path outside font root: {}",
            canonical_path.display()
        ));
    }
    fs::remove_dir_all(canonical_path)?;
    Ok(())
}

fn refresh_font_cache(path: &Path) {
    if !command_exists("fc-cache") {
        eprintln!("warning: fc-cache not found; fontconfig cache was not refreshed");
        return;
    }

    match Command::new("fc-cache")
        .arg("-f")
        .arg(path)
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) if status.success() || status.code() == Some(1) => {}
        Ok(status) => eprintln!("warning: fc-cache exited with {status}"),
        Err(err) => eprintln!("warning: failed to run fc-cache: {err}"),
    }
}

fn resolve_installed_key(config: &InstalledFile, query: &str) -> Option<String> {
    let query_norm = normalize(query.trim().trim_end_matches(".json"));
    for key in config.installed.keys() {
        if key.eq_ignore_ascii_case(query) || normalize(key) == query_norm {
            return Some(key.clone());
        }
    }

    let mut best = None;
    let mut best_score = 0.0;
    for key in config.installed.keys() {
        let score = similarity(&query_norm, &normalize(key));
        if score > best_score {
            best_score = score;
            best = Some(key.clone());
        }
    }
    if best_score >= 0.82 { best } else { None }
}

fn first_positional<'a>(args: &'a [String], message: &str) -> Result<&'a str> {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(|arg| arg.as_str())
        .ok_or_else(|| CliError::new(message))
}

fn ensure_tool_for_download() -> Result<()> {
    if command_exists("curl") || command_exists("wget") {
        Ok(())
    } else {
        bail("missing downloader: install curl or wget")
    }
}

fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        Ok(())
    } else {
        bail(format!("missing command: {command}"))
    }
}

fn command_exists(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|dir| dir.join(command).is_file()))
        .unwrap_or(false)
}

fn command_status(command: &str) -> String {
    if command_exists(command) {
        "ok".to_string()
    } else {
        "missing".to_string()
    }
}

fn path_status(path: &Path) -> String {
    if path.exists() {
        format!("ok ({})", path.display())
    } else {
        format!("missing ({})", path.display())
    }
}

fn run_command(command: &mut Command, label: &str) -> Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        bail(format!("{label} failed with status {status}"))
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn config_path_parent(path: &Path) -> PathBuf {
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn expand_home(value: &str) -> PathBuf {
    let value = value.trim();
    if value == "~" {
        return home_dir();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(value)
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn xdg_config_home() -> Option<PathBuf> {
    env_path("XDG_CONFIG_HOME")
}

fn xdg_data_home() -> Option<PathBuf> {
    env_path("XDG_DATA_HOME")
}

fn xdg_cache_home() -> Option<PathBuf> {
    env_path("XDG_CACHE_HOME")
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let distance = levenshtein(a, b) as f64;
    let max_len = a.chars().count().max(b.chars().count()) as f64;
    1.0 - distance / max_len
}

fn levenshtein(a: &str, b: &str) -> usize {
    let b_chars = b.chars().collect::<Vec<_>>();
    let mut costs = (0..=b_chars.len()).collect::<Vec<_>>();

    for (i, ca) in a.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = i + 1;
        for (j, cb) in b_chars.iter().enumerate() {
            let temp = costs[j + 1];
            let substitution = if ca == *cb { previous } else { previous + 1 };
            costs[j + 1] = (costs[j + 1] + 1).min(costs[j] + 1).min(substitution);
            previous = temp;
        }
    }

    costs[b_chars.len()]
}

fn scoop_filename(url: &str, idx: usize) -> String {
    if let Some(pos) = url.find("#/") {
        return sanitize_filename(&url[pos + 2..]);
    }

    let without_hash = url.split('#').next().unwrap_or(url);
    let without_query = without_hash.split('?').next().unwrap_or(without_hash);
    let filename = without_query
        .rsplit('/')
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("payload");
    let filename = sanitize_filename(filename);
    if filename == "payload" {
        format!("payload-{}", idx + 1)
    } else {
        filename
    }
}

fn sanitize_filename(value: &str) -> String {
    let mut out = value
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '\0' => '_',
            _ => ch,
        })
        .collect::<String>();
    if out.trim().is_empty() {
        out = "payload".to_string();
    }
    out
}

fn download_url(url: &str) -> &str {
    url.split("#/").next().unwrap_or(url)
}

fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_raw_deserializes_string_or_array() {
        let raw: ManifestRaw = serde_json::from_str(
            r#"{"version":"1","url":["a.ttf","b.ttf"],"hash":["aa","bb"],"extract_dir":"ttf"}"#,
        )
        .unwrap();
        assert_eq!(raw.version.as_deref(), Some("1"));
        assert_eq!(raw.url, vec!["a.ttf".to_string(), "b.ttf".to_string()]);
        assert_eq!(raw.hash, vec!["aa".to_string(), "bb".to_string()]);
        assert_eq!(raw.extract_dir.as_deref(), Some("ttf"));

        let single: ManifestRaw =
            serde_json::from_str(r#"{"url":"only.ttf","hash":"single"}"#).unwrap();
        assert_eq!(single.url, vec!["only.ttf".to_string()]);
        assert_eq!(single.hash, vec!["single".to_string()]);
    }

    #[test]
    fn manifest_raw_accepts_object_license() {
        let raw: ManifestRaw = serde_json::from_str(
            r#"{"license":{"identifier":"OFL-1.1","url":"https://example.com/license"},"url":"x.zip"}"#,
        )
        .unwrap();
        assert_eq!(raw.license.as_deref(), Some("OFL-1.1"));

        let plain: ManifestRaw =
            serde_json::from_str(r#"{"license":"MIT","url":"x.zip"}"#).unwrap();
        assert_eq!(plain.license.as_deref(), Some("MIT"));

        let missing: ManifestRaw = serde_json::from_str(r#"{"url":"x.zip"}"#).unwrap();
        assert!(missing.license.is_none());
    }

    #[test]
    fn format_unix_timestamp_known_dates() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(
            format_unix_timestamp(946_684_800),
            "2000-01-01 00:00:00 UTC"
        );
        // leap day (2024-02-29 12:34:56 UTC = 1709210096)
        assert_eq!(
            format_unix_timestamp(1_709_210_096),
            "2024-02-29 12:34:56 UTC"
        );
        // pre-epoch: -1 = 1969-12-31 23:59:59 UTC
        assert_eq!(format_unix_timestamp(-1), "1969-12-31 23:59:59 UTC");
    }

    #[test]
    fn parse_install_time_handles_garbage() {
        assert_eq!(parse_install_time(""), "(unknown)");
        assert_eq!(parse_install_time("not-a-number"), "(unknown)");
        assert_eq!(parse_install_time("0"), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn installed_file_accepts_legacy_numeric_version() {
        // older fontctl wrote "version": 1 (number); we now expect a string,
        // but should still load the file rather than fail outright.
        let installed: InstalledFile =
            serde_json::from_str(r#"{"version": 1, "installed": {}}"#).unwrap();
        assert_eq!(installed.version, "1");
        assert!(installed.installed.is_empty());
    }

    #[test]
    fn alias_resolves_base_nerd_font_name() {
        let aliases = aliases_for_name("FiraCode-NF");
        assert!(aliases.iter().any(|(alias, _)| alias == "firacode"));
        let mono_aliases = aliases_for_name("FiraCode-NF-Mono");
        assert!(
            mono_aliases
                .iter()
                .any(|(alias, _)| alias == "firacodemono")
        );
        assert!(!mono_aliases.iter().any(|(alias, _)| alias == "firacode"));
    }

    #[test]
    fn scoop_fragment_filename_wins() {
        assert_eq!(
            scoop_filename("https://example.test/download?x=1#/font.7z", 0),
            "font.7z"
        );
    }

    #[test]
    fn normal_nerd_filter_excludes_mono_and_propo() {
        let manifest = Manifest {
            name: "FiraCode-NF".to_string(),
            path: PathBuf::new(),
            version: None,
            description: None,
            homepage: None,
            license: None,
            urls: vec![],
            hashes: vec![],
            extract_dir: None,
        };
        let files = vec![
            PathBuf::from("FiraCodeNerdFont-Regular.ttf"),
            PathBuf::from("FiraCodeNerdFontMono-Regular.ttf"),
            PathBuf::from("FiraCodeNerdFontPropo-Regular.ttf"),
        ];
        let filtered = filter_manifest_fonts(&manifest, files);
        assert_eq!(
            filtered,
            vec![PathBuf::from("FiraCodeNerdFont-Regular.ttf")]
        );
    }
}
