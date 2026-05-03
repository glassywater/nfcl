use std::collections::{BTreeMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const APP_NAME: &str = "fontctl";
const CONFIG_VERSION: u32 = 1;
const DEFAULT_REPO: &str = "/home/Kyecox/work/font/scoop-nerd-fonts";
const DEFAULT_REMOTE: &str = "https://github.com/matthewjberger/scoop-nerd-fonts.git";
const DEFAULT_CONFIG_FILE: &str = "config.json";
const DEFAULT_INSTALLED_FILE: &str = "installed.json";
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
    font_root: PathBuf,
    cache_dir: PathBuf,
    repo_overridden: bool,
    bucket_overridden: bool,
    installed_overridden: bool,
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
        let font_root = env_path("FONTCTL_FONT_DIR").unwrap_or_else(|| {
            xdg_data_home()
                .unwrap_or_else(|| home_dir().join(".local/share"))
                .join("fonts")
                .join(APP_NAME)
        });
        let cache_dir = env_path("FONTCTL_CACHE_DIR").unwrap_or_else(|| {
            xdg_cache_home()
                .unwrap_or_else(|| home_dir().join(".cache"))
                .join(APP_NAME)
        });

        Ok(Self {
            repo_dir,
            bucket_dir,
            config_path,
            installed_path,
            font_root,
            cache_dir,
            repo_overridden,
            bucket_overridden,
            installed_overridden,
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

    if args.is_empty() {
        print_help();
        return Ok(());
    }

    if matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_help();
        return Ok(());
    }

    if matches!(args[0].as_str(), "init") {
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
        "config" => cmd_config(&options)?,
        "doctor" => cmd_doctor(&options)?,
        command => {
            return bail(format!(
                "unknown command '{command}'. Run '{APP_NAME} help' for usage."
            ));
        }
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
            }
            "--installed" | "--installed-json" => {
                i += 1;
                let value = raw
                    .get(i)
                    .ok_or_else(|| CliError::new("--installed requires a path"))?;
                options.installed_path = PathBuf::from(value);
                options.installed_overridden = true;
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
  list [--installed]        List bucket fonts or installed fonts
  search <query>            Search bucket manifests
  info <font>               Show manifest details
  install <font> [--force]  Download, verify, extract, and install a font
  uninstall <font>          Remove a font installed by this CLI
  installed                 Show fonts recorded in config JSON
  update [--install]        git pull/clone manifests, then compare installed versions
  config                    Print paths used by the CLI
  doctor                    Check local tools and paths

Global options:
  --repo <path>             scoop-nerd-fonts git directory
  --bucket <path>           manifest bucket directory
  --config <path>           config JSON path
  --installed <path>        installed font JSON path
  --font-dir <path>         root directory for installed font files
  --cache-dir <path>        cache directory for downloads/extraction

Defaults:
  config:  ~/.config/{APP_NAME}/{DEFAULT_CONFIG_FILE}
  installed: ~/.config/{APP_NAME}/{DEFAULT_INSTALLED_FILE}
  repo:    {DEFAULT_REPO}
  remote:  {DEFAULT_REMOTE}
"
    );
}

fn cmd_init(options: &mut Options, args: &[String]) -> Result<()> {
    let mut config = load_config(&options.config_path)?;
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
    save_config(&options.config_path, &config)?;
    if let Some(mut installed) = installed_to_create {
        installed.version = repo_version_or_unknown(&options.repo_dir);
        save_installed(&options.installed_path, &installed)?;
    }

    println!("Initialized {APP_NAME}.");
    println!("Config: {}", options.config_path.display());
    println!("Installed JSON: {}", options.installed_path.display());
    println!("Repo:   {}", options.repo_dir.display());
    println!(
        "Run '{APP_NAME} update' to git pull or clone scoop-nerd-fonts, then compare installed versions."
    );
    Ok(())
}

fn ensure_initialized(options: &mut Options) -> Result<()> {
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
    if args.iter().any(|arg| arg == "--installed") {
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
    let index = load_bucket_index(&options.bucket_dir)?;

    let mut matches = Vec::new();
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
            println!("Installed:   yes");
            println!("Font dir:    {}", record.font_dir);
            println!("Files:       {}", record.files.len());
        }
        None => println!("Installed:   no"),
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

    for record in config.installed.values() {
        println!(
            "{}  {}  {} files  {}",
            record.name,
            record.version,
            record.files.len(),
            record.font_dir
        );
    }
    Ok(())
}

fn cmd_update(options: &mut Options, args: &[String]) -> Result<()> {
    let apply_install = args.iter().any(|arg| arg == "--install");
    sync_bucket_repo(options)?;
    options.bucket_dir = options.repo_dir.join("bucket");

    let mut config = load_installed(&options.installed_path)?;
    config.version = git_repo_version(&options.repo_dir)?;
    save_installed(&options.installed_path, &config)?;

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

fn cmd_config(options: &Options) -> Result<()> {
    println!("Repo:    {}", options.repo_dir.display());
    println!("Bucket:  {}", options.bucket_dir.display());
    println!("Config:  {}", options.config_path.display());
    println!("Installed: {}", options.installed_path.display());
    println!("Fonts:   {}", options.font_root.display());
    println!("Cache:   {}", options.cache_dir.display());
    println!("Remote:  {DEFAULT_REMOTE}");
    Ok(())
}

fn cmd_doctor(options: &Options) -> Result<()> {
    println!("Repo:    {}", path_status(&options.repo_dir));
    println!("Bucket:  {}", path_status(&options.bucket_dir));
    println!("Config:  {}", options.config_path.display());
    println!("Installed: {}", options.installed_path.display());
    println!("Fonts:   {}", options.font_root.display());
    println!("Cache:   {}", options.cache_dir.display());
    println!("git:     {}", command_status("git"));
    println!("curl:    {}", command_status("curl"));
    println!("wget:    {}", command_status("wget"));
    println!("sha256:  {}", command_status("sha256sum"));
    println!("unzip:   {}", command_status("unzip"));
    println!("tar:     {}", command_status("tar"));
    println!("7z:      {}", command_status("7z"));
    println!("fc-cache: {}", command_status("fc-cache"));
    Ok(())
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

fn read_manifest(path: &Path) -> Result<Manifest> {
    let text = fs::read_to_string(path)?;
    let json = parse_json(&text)?;
    let object = json.as_object().ok_or_else(|| {
        CliError::new(format!("manifest is not a JSON object: {}", path.display()))
    })?;

    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| CliError::new(format!("invalid manifest path: {}", path.display())))?
        .to_string();
    let urls = json_string_or_array(object.get("url"))
        .ok_or_else(|| CliError::new(format!("manifest missing url: {}", path.display())))?;
    let hash_values = json_string_or_array(object.get("hash")).unwrap_or_default();
    let hashes = normalize_hashes(urls.len(), hash_values);

    Ok(Manifest {
        name,
        path: path.to_path_buf(),
        version: json_string(object.get("version")),
        description: json_string(object.get("description")),
        homepage: json_string(object.get("homepage")),
        license: json_string(object.get("license")),
        urls,
        hashes,
        extract_dir: json_string(object.get("extract_dir")),
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

#[derive(Debug, Clone)]
struct CliConfig {
    repo_dir: Option<String>,
}

#[derive(Debug, Clone)]
struct InstalledFile {
    version: String,
    installed: BTreeMap<String, InstalledFont>,
}

#[derive(Debug, Clone)]
struct InstalledFont {
    name: String,
    version: String,
    manifest: String,
    urls: Vec<String>,
    hashes: Vec<String>,
    installed_at: String,
    font_dir: String,
    files: Vec<String>,
}

fn load_config(path: &Path) -> Result<CliConfig> {
    if !path.exists() {
        return Ok(CliConfig { repo_dir: None });
    }

    let text = fs::read_to_string(path)?;
    let json = parse_json(&text)?;
    let object = json
        .as_object()
        .ok_or_else(|| CliError::new(format!("config is not a JSON object: {}", path.display())))?;

    Ok(CliConfig {
        repo_dir: json_string(object.get("repo_dir")),
    })
}

fn save_config(path: &Path, config: &CliConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"version\": {CONFIG_VERSION},\n"));
    push_json_string_field(
        &mut out,
        "repo_dir",
        config.repo_dir.as_deref().unwrap_or(DEFAULT_REPO),
        2,
        true,
    );
    push_json_string_field(&mut out, "remote", DEFAULT_REMOTE, 2, false);
    out.push_str("}\n");
    fs::write(path, out)?;
    Ok(())
}

fn load_installed(path: &Path) -> Result<InstalledFile> {
    if !path.exists() {
        return Ok(InstalledFile {
            version: "unknown".to_string(),
            installed: BTreeMap::new(),
        });
    }

    let text = fs::read_to_string(path)?;
    let json = parse_json(&text)?;
    let object = json.as_object().ok_or_else(|| {
        CliError::new(format!(
            "installed JSON is not a JSON object: {}",
            path.display()
        ))
    })?;
    let version = json_string(object.get("version")).unwrap_or_else(|| "unknown".to_string());
    let mut installed = BTreeMap::new();

    if let Some(installed_obj) = object.get("installed").and_then(Json::as_object) {
        for (key, value) in installed_obj {
            let Some(record_obj) = value.as_object() else {
                continue;
            };
            let name = json_string(record_obj.get("name")).unwrap_or_else(|| key.clone());
            let record = InstalledFont {
                name: name.clone(),
                version: json_string(record_obj.get("version"))
                    .unwrap_or_else(|| "unknown".to_string()),
                manifest: json_string(record_obj.get("manifest")).unwrap_or_default(),
                urls: json_string_or_array(record_obj.get("urls")).unwrap_or_default(),
                hashes: json_string_or_array(record_obj.get("hashes")).unwrap_or_default(),
                installed_at: json_string(record_obj.get("installed_at")).unwrap_or_default(),
                font_dir: json_string(record_obj.get("font_dir")).unwrap_or_default(),
                files: json_string_or_array(record_obj.get("files")).unwrap_or_default(),
            };
            installed.insert(name, record);
        }
    }

    Ok(InstalledFile { version, installed })
}

fn save_installed(path: &Path, config: &InstalledFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut out = String::new();
    out.push_str("{\n");
    push_json_string_field(&mut out, "version", &config.version, 2, true);
    out.push_str("  \"installed\": {\n");
    for (idx, (key, record)) in config.installed.iter().enumerate() {
        out.push_str(&format!("    \"{}\": {{\n", json_escape(key)));
        push_json_string_field(&mut out, "name", &record.name, 6, true);
        push_json_string_field(&mut out, "version", &record.version, 6, true);
        push_json_string_field(&mut out, "manifest", &record.manifest, 6, true);
        push_json_array_field(&mut out, "urls", &record.urls, 6, true);
        push_json_array_field(&mut out, "hashes", &record.hashes, 6, true);
        push_json_string_field(&mut out, "installed_at", &record.installed_at, 6, true);
        push_json_string_field(&mut out, "font_dir", &record.font_dir, 6, true);
        push_json_array_field(&mut out, "files", &record.files, 6, false);
        if idx + 1 == config.installed.len() {
            out.push_str("    }\n");
        } else {
            out.push_str("    },\n");
        }
    }
    out.push_str("  }\n");
    out.push_str("}\n");
    fs::write(path, out)?;
    Ok(())
}

fn push_json_string_field(out: &mut String, key: &str, value: &str, indent: usize, comma: bool) {
    out.push_str(&" ".repeat(indent));
    out.push_str(&format!(
        "\"{}\": \"{}\"",
        json_escape(key),
        json_escape(value)
    ));
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_array_field(
    out: &mut String,
    key: &str,
    values: &[String],
    indent: usize,
    comma: bool,
) {
    out.push_str(&" ".repeat(indent));
    out.push_str(&format!("\"{}\": [", json_escape(key)));
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("\"{}\"", json_escape(value)));
    }
    out.push(']');
    if comma {
        out.push(',');
    }
    out.push('\n');
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
        download_payload(url, &payload_path)?;
        if let Some(expected) = manifest.hashes.get(idx).and_then(|value| value.as_deref()) {
            verify_sha256(&payload_path, expected)?;
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

fn download_payload(url: &str, target: &Path) -> Result<()> {
    let clean_url = download_url(url);
    if command_exists("curl") {
        run_command(
            Command::new("curl")
                .arg("-L")
                .arg("--fail")
                .arg("--output")
                .arg(target)
                .arg(clean_url),
            "curl download",
        )
    } else if command_exists("wget") {
        run_command(
            Command::new("wget").arg("-O").arg(target).arg(clean_url),
            "wget download",
        )
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

fn json_string(value: Option<&Json>) -> Option<String> {
    value.and_then(Json::as_str).map(ToOwned::to_owned)
}

fn json_string_or_array(value: Option<&Json>) -> Option<Vec<String>> {
    match value? {
        Json::String(value) => Some(vec![value.clone()]),
        Json::Array(values) => Some(
            values
                .iter()
                .filter_map(Json::as_str)
                .map(ToOwned::to_owned)
                .collect(),
        ),
        _ => None,
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Json {
    Null,
    Bool(()),
    Number(()),
    String(String),
    Array(Vec<Json>),
    Object(BTreeMap<String, Json>),
}

impl Json {
    fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(value) => Some(value),
            _ => None,
        }
    }

    fn as_object(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Object(value) => Some(value),
            _ => None,
        }
    }
}

fn parse_json(input: &str) -> Result<Json> {
    let mut parser = JsonParser {
        chars: input.chars().collect(),
        pos: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if !parser.is_eof() {
        return bail("trailing content after JSON value");
    }
    Ok(value)
}

struct JsonParser {
    chars: Vec<char>,
    pos: usize,
}

impl JsonParser {
    fn parse_value(&mut self) -> Result<Json> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => self.parse_string().map(Json::String),
            Some('t') => {
                self.expect_literal("true")?;
                Ok(Json::Bool(()))
            }
            Some('f') => {
                self.expect_literal("false")?;
                Ok(Json::Bool(()))
            }
            Some('n') => {
                self.expect_literal("null")?;
                Ok(Json::Null)
            }
            Some(ch) if ch == '-' || ch.is_ascii_digit() => self.parse_number(),
            Some(ch) => bail(format!("unexpected JSON character '{ch}'")),
            None => bail("unexpected end of JSON"),
        }
    }

    fn parse_object(&mut self) -> Result<Json> {
        self.expect('{')?;
        let mut object = BTreeMap::new();
        self.skip_ws();
        if self.consume('}') {
            return Ok(Json::Object(object));
        }

        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            self.expect(':')?;
            let value = self.parse_value()?;
            object.insert(key, value);
            self.skip_ws();
            if self.consume('}') {
                break;
            }
            self.expect(',')?;
        }

        Ok(Json::Object(object))
    }

    fn parse_array(&mut self) -> Result<Json> {
        self.expect('[')?;
        let mut array = Vec::new();
        self.skip_ws();
        if self.consume(']') {
            return Ok(Json::Array(array));
        }

        loop {
            array.push(self.parse_value()?);
            self.skip_ws();
            if self.consume(']') {
                break;
            }
            self.expect(',')?;
        }

        Ok(Json::Array(array))
    }

    fn parse_string(&mut self) -> Result<String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let Some(ch) = self.bump() else {
                return bail("unterminated JSON string");
            };
            match ch {
                '"' => break,
                '\\' => {
                    let Some(escaped) = self.bump() else {
                        return bail("unterminated JSON escape");
                    };
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.parse_unicode_escape()?),
                        other => return bail(format!("invalid JSON escape '\\{other}'")),
                    }
                }
                ch => out.push(ch),
            }
        }
        Ok(out)
    }

    fn parse_unicode_escape(&mut self) -> Result<char> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(ch) = self.bump() else {
                return bail("unterminated unicode escape");
            };
            value = value * 16
                + ch.to_digit(16)
                    .ok_or_else(|| CliError::new(format!("invalid unicode escape digit '{ch}'")))?;
        }
        Ok(char::from_u32(value).unwrap_or('\u{fffd}'))
    }

    fn parse_number(&mut self) -> Result<Json> {
        let start = self.pos;
        while let Some(ch) = self.peek() {
            if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return bail("expected JSON number");
        }
        Ok(Json::Number(()))
    }

    fn expect_literal(&mut self, literal: &str) -> Result<()> {
        for expected in literal.chars() {
            self.expect(expected)?;
        }
        Ok(())
    }

    fn expect(&mut self, expected: char) -> Result<()> {
        match self.bump() {
            Some(ch) if ch == expected => Ok(()),
            Some(ch) => bail(format!("expected '{expected}', got '{ch}'")),
            None => bail(format!("expected '{expected}', got end of JSON")),
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_ws(&mut self) {
        while self.peek().map(|ch| ch.is_whitespace()).unwrap_or(false) {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += 1;
        Some(ch)
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_parser_reads_manifest_shape() {
        let json = parse_json(
            r#"{"version":"1","url":["a.ttf","b.ttf"],"hash":["aa","bb"],"extract_dir":"ttf"}"#,
        )
        .unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(json_string(object.get("version")).unwrap(), "1");
        assert_eq!(json_string_or_array(object.get("url")).unwrap().len(), 2);
        assert_eq!(json_string(object.get("extract_dir")).unwrap(), "ttf");
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
