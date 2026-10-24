use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use indicatif::HumanBytes;
use serde::Serialize;

use crate::{absolute_path, find_workspace_root, workspace_directories, ReportFormat, Ui};

#[derive(Debug, Args)]
pub(crate) struct CleanCommand {
    /// Project directory.
    #[arg(default_value = ".")]
    pub(crate) path: PathBuf,
    /// Show what would be removed without changing the workspace.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Cleanup report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Debug, Args)]
pub(crate) struct CacheCommand {
    #[command(subcommand)]
    pub(crate) command: CacheSubcommand,
}

#[derive(Debug, Subcommand)]
pub(crate) enum CacheSubcommand {
    /// Print the shared and language-server cache locations.
    Dir(CacheOutput),
    /// Report cache usage by area.
    Status(CacheOutput),
    /// Remove one cache area or every known cache area.
    Clear(CacheClear),
    /// Remove cache entries older than a configured number of days.
    Prune(CachePrune),
}

#[derive(Debug, Args)]
pub(crate) struct CacheOutput {
    /// Output format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Debug, Args)]
pub(crate) struct CacheClear {
    /// Cache area to clear.
    #[arg(value_enum)]
    pub(crate) area: CacheArea,
    /// Show what would be removed without deleting it.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Cleanup report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Debug, Args)]
pub(crate) struct CachePrune {
    /// Cache area to prune.
    #[arg(value_enum, default_value_t = CacheArea::All)]
    pub(crate) area: CacheArea,
    /// Remove entries not modified within this many days.
    #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u64).range(1..))]
    pub(crate) older_than: u64,
    /// Show what would be removed without deleting it.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Cleanup report format.
    #[arg(long, value_enum, default_value_t = ReportFormat::Human)]
    pub(crate) format: ReportFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CacheArea {
    Dependencies,
    Catalog,
    Audit,
    Imports,
    Tools,
    Lsp,
    All,
}

impl CacheArea {
    const fn name(self) -> &'static str {
        match self {
            Self::Dependencies => "dependencies",
            Self::Catalog => "catalog",
            Self::Audit => "audit",
            Self::Imports => "imports",
            Self::Tools => "tools",
            Self::Lsp => "lsp",
            Self::All => "all",
        }
    }

    fn concrete(self) -> &'static [Self] {
        match self {
            Self::Dependencies => &[Self::Dependencies],
            Self::Catalog => &[Self::Catalog],
            Self::Audit => &[Self::Audit],
            Self::Imports => &[Self::Imports],
            Self::Tools => &[Self::Tools],
            Self::Lsp => &[Self::Lsp],
            Self::All => &[
                Self::Dependencies,
                Self::Catalog,
                Self::Audit,
                Self::Imports,
                Self::Tools,
                Self::Lsp,
            ],
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageStats {
    bytes: u64,
    files: u64,
    directories: u64,
}

impl StorageStats {
    fn add(&mut self, other: &Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.files = self.files.saturating_add(other.files);
        self.directories = self.directories.saturating_add(other.directories);
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CleanReport {
    workspace: PathBuf,
    dry_run: bool,
    modules: usize,
    removed: StorageStats,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheDirectories {
    shared: PathBuf,
    lsp: PathBuf,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheAreaStatus {
    area: CacheArea,
    bytes: u64,
    files: u64,
    directories: u64,
    paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheStatus {
    shared_root: PathBuf,
    lsp_root: PathBuf,
    bytes: u64,
    files: u64,
    directories: u64,
    areas: Vec<CacheAreaStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheMutationReport {
    operation: &'static str,
    area: CacheArea,
    dry_run: bool,
    older_than_days: Option<u64>,
    removed: StorageStats,
    paths: Vec<PathBuf>,
}

pub(crate) async fn clean_project(arguments: &CleanCommand, ui: &Ui) -> Result<()> {
    let target = absolute_path(&arguments.path)?;
    if !target.exists() {
        bail!("cannot clean {}: path does not exist", target.display());
    }
    let directory = if target.is_file() {
        target
            .parent()
            .context("cleanup path has no parent directory")?
    } else {
        &target
    };
    let manifest = nearest_manifest(directory).with_context(|| {
        format!(
            "cannot clean {}: no jman.toml was found in this directory or its ancestors",
            target.display()
        )
    })?;
    let project = manifest
        .parent()
        .context("jman.toml has no parent directory")?;
    let workspace = find_workspace_root(project);
    let module_directories = workspace_directories(&workspace).await?;
    let paths = module_directories
        .iter()
        .map(|directory| directory.join(".jman"))
        .collect::<Vec<_>>();
    let activity = ui.activity(if arguments.dry_run {
        "Inspecting disposable workspace output"
    } else {
        "Removing disposable workspace output"
    });
    let mut removed = StorageStats::default();
    let mut present = Vec::new();
    for path in paths {
        let stats = inspect_path(&path)?;
        if stats.files == 0 && stats.directories == 0 {
            continue;
        }
        removed.add(&stats);
        present.push(path.clone());
        if !arguments.dry_run {
            remove_path(&path)?;
        }
    }
    let report = CleanReport {
        workspace,
        dry_run: arguments.dry_run,
        modules: present.len(),
        removed,
        paths: present,
    };
    match arguments.format {
        ReportFormat::Human => {
            let action = if arguments.dry_run {
                "Would clean"
            } else {
                "Cleaned"
            };
            activity.finish_clean(format!(
                "{action} {} module{} · {} reclaimed",
                report.modules,
                if report.modules == 1 { "" } else { "s" },
                HumanBytes(report.removed.bytes)
            ));
            if ui.is_verbose() {
                for path in &report.paths {
                    ui.detail(path.display().to_string());
                }
            }
        }
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    Ok(())
}

pub(crate) fn cache_command(arguments: &CacheCommand, ui: &Ui) -> Result<()> {
    let roots = cache_directories()?;
    match &arguments.command {
        CacheSubcommand::Dir(output) => print_cache_directories(&roots, output.format),
        CacheSubcommand::Status(output) => print_cache_status(&roots, output.format),
        CacheSubcommand::Clear(clear) => clear_cache(&roots, clear, ui),
        CacheSubcommand::Prune(prune) => prune_cache(&roots, prune, ui),
    }
}

fn print_cache_directories(roots: &CacheDirectories, format: ReportFormat) -> Result<()> {
    match format {
        ReportFormat::Human => {
            println!("Shared  {}", roots.shared.display());
            println!("LSP     {}", roots.lsp.display());
        }
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(roots)?),
    }
    Ok(())
}

fn print_cache_status(roots: &CacheDirectories, format: ReportFormat) -> Result<()> {
    let status = inspect_cache(roots)?;
    match format {
        ReportFormat::Human => {
            println!(
                "JMAN cache · {} · {} files",
                HumanBytes(status.bytes),
                status.files
            );
            for area in &status.areas {
                println!(
                    "  {:<13} {:>11}  {:>8} files",
                    area.area.name(),
                    HumanBytes(area.bytes),
                    area.files
                );
            }
            println!("\nShared  {}", status.shared_root.display());
            println!("LSP     {}", status.lsp_root.display());
        }
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(&status)?),
    }
    Ok(())
}

fn clear_cache(roots: &CacheDirectories, arguments: &CacheClear, ui: &Ui) -> Result<()> {
    validate_cache_root(&roots.shared)?;
    ensure_lsp_is_idle(arguments.area)?;
    let paths = area_paths(roots, arguments.area);
    let activity = ui.activity(if arguments.dry_run {
        "Inspecting cache entries"
    } else {
        "Clearing cache entries"
    });
    let mut removed = StorageStats::default();
    let mut present = Vec::new();
    for path in paths {
        let stats = inspect_path(&path)?;
        if stats.files == 0 && stats.directories == 0 {
            continue;
        }
        removed.add(&stats);
        present.push(path.clone());
        if !arguments.dry_run {
            remove_path(&path)?;
        }
    }
    let report = CacheMutationReport {
        operation: "clear",
        area: arguments.area,
        dry_run: arguments.dry_run,
        older_than_days: None,
        removed,
        paths: present,
    };
    print_cache_mutation(&report, activity, ui, arguments.format)
}

fn prune_cache(roots: &CacheDirectories, arguments: &CachePrune, ui: &Ui) -> Result<()> {
    validate_cache_root(&roots.shared)?;
    ensure_lsp_is_idle(arguments.area)?;
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            arguments.older_than.saturating_mul(24 * 60 * 60),
        ))
        .context("cache retention period is too large")?;
    let paths = area_paths(roots, arguments.area);
    let activity = ui.activity(if arguments.dry_run {
        "Inspecting stale cache entries"
    } else {
        "Pruning stale cache entries"
    });
    let mut removed = StorageStats::default();
    let mut affected = Vec::new();
    for path in paths {
        let stats = prune_path(&path, cutoff, arguments.dry_run)?;
        if stats.files == 0 && stats.directories == 0 {
            continue;
        }
        removed.add(&stats);
        affected.push(path);
    }
    let report = CacheMutationReport {
        operation: "prune",
        area: arguments.area,
        dry_run: arguments.dry_run,
        older_than_days: Some(arguments.older_than),
        removed,
        paths: affected,
    };
    print_cache_mutation(&report, activity, ui, arguments.format)
}

fn print_cache_mutation(
    report: &CacheMutationReport,
    activity: crate::ui::Activity,
    ui: &Ui,
    format: ReportFormat,
) -> Result<()> {
    match format {
        ReportFormat::Human => {
            let action = match (report.operation, report.dry_run) {
                ("clear", true) => "Would clear",
                ("clear", false) => "Cleared",
                ("prune", true) => "Would prune",
                ("prune", false) => "Pruned",
                _ => "Processed",
            };
            activity.finish_clean(format!(
                "{action} {} · {} · {} files",
                report.area.name(),
                HumanBytes(report.removed.bytes),
                report.removed.files
            ));
            if ui.is_verbose() {
                for path in &report.paths {
                    ui.detail(path.display().to_string());
                }
            }
        }
        ReportFormat::Json => println!("{}", serde_json::to_string_pretty(report)?),
    }
    Ok(())
}

fn cache_directories() -> Result<CacheDirectories> {
    let shared = absolute_path(&jman_build::toolchain::default_cache_dir())?;
    let lsp = absolute_path(&jman_java_lsp::cache_root())?;
    if lsp != shared.join("lsp") {
        bail!(
            "language-server cache {} is outside the JMAN cache root {}",
            lsp.display(),
            shared.display()
        );
    }
    Ok(CacheDirectories { shared, lsp })
}

fn validate_cache_root(root: &Path) -> Result<()> {
    let resolved = if root.exists() {
        fs::canonicalize(root)
            .with_context(|| format!("could not canonicalize cache root {}", root.display()))?
    } else {
        root.to_owned()
    };
    if resolved
        .parent()
        .is_none_or(|parent| parent.parent().is_none())
    {
        bail!(
            "refusing to manage unsafe cache root {}; set JMAN_CACHE_DIR to a dedicated directory",
            resolved.display()
        );
    }
    if dirs::home_dir().is_some_and(|home| resolved == home)
        || resolved == jman_build::toolchain::default_data_dir()
        || resolved == jman_build::toolchain::default_config_dir()
    {
        bail!(
            "refusing to manage protected directory {} as a cache root",
            resolved.display()
        );
    }
    let current = std::env::current_dir().context("could not determine current directory")?;
    let current = fs::canonicalize(&current).unwrap_or(current);
    if current.starts_with(&resolved) {
        bail!(
            "refusing to manage cache root {} because it contains the current working directory",
            resolved.display()
        );
    }
    Ok(())
}

fn inspect_cache(roots: &CacheDirectories) -> Result<CacheStatus> {
    let mut total = StorageStats::default();
    let mut areas = Vec::new();
    for area in CacheArea::All.concrete() {
        let paths = area_paths(roots, *area);
        let mut stats = StorageStats::default();
        for path in &paths {
            stats.add(&inspect_path(path)?);
        }
        total.add(&stats);
        areas.push(CacheAreaStatus {
            area: *area,
            bytes: stats.bytes,
            files: stats.files,
            directories: stats.directories,
            paths,
        });
    }
    Ok(CacheStatus {
        shared_root: roots.shared.clone(),
        lsp_root: roots.lsp.clone(),
        bytes: total.bytes,
        files: total.files,
        directories: total.directories,
        areas,
    })
}

fn area_paths(roots: &CacheDirectories, area: CacheArea) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for area in area.concrete() {
        match area {
            CacheArea::Dependencies => {
                paths.push(roots.shared.join("repository"));
                paths.push(roots.shared.join("artifacts"));
            }
            CacheArea::Catalog => paths.push(roots.shared.join("catalog")),
            CacheArea::Audit => paths.push(roots.shared.join("audit")),
            CacheArea::Imports => paths.push(roots.shared.join("imports")),
            CacheArea::Tools => paths.push(roots.shared.join("tools")),
            CacheArea::Lsp => paths.push(roots.lsp.clone()),
            CacheArea::All => unreachable!("all expands into concrete cache areas"),
        }
    }
    paths
}

fn ensure_lsp_is_idle(area: CacheArea) -> Result<()> {
    if area.concrete().contains(&CacheArea::Lsp) && jman_java_lsp::cache_is_active() {
        bail!(
            "the Java language server is using its cache; close active editor sessions before clearing or pruning LSP cache data"
        );
    }
    Ok(())
}

fn nearest_manifest(directory: &Path) -> Option<PathBuf> {
    directory
        .ancestors()
        .map(|ancestor| ancestor.join("jman.toml"))
        .find(|path| path.is_file())
}

fn inspect_path(path: &Path) -> Result<StorageStats> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StorageStats::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", path.display()));
        }
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        return Ok(StorageStats {
            bytes: metadata.len(),
            files: 1,
            directories: 0,
        });
    }
    if !metadata.is_dir() {
        return Ok(StorageStats::default());
    }
    let mut stats = StorageStats {
        directories: 1,
        ..StorageStats::default()
    };
    let entries = fs::read_dir(path)
        .with_context(|| format!("could not read directory {}", path.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("could not read entry in {}", path.display()))?;
        stats.add(&inspect_path(&entry.path())?);
    }
    Ok(stats)
}

fn remove_path(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", path.display()));
        }
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
            .with_context(|| format!("could not remove directory {}", path.display()))
    } else {
        fs::remove_file(path).with_context(|| format!("could not remove {}", path.display()))
    }
}

fn prune_path(path: &Path, cutoff: SystemTime, dry_run: bool) -> Result<StorageStats> {
    let (stats, _) = prune_node(path, cutoff, dry_run, false)?;
    Ok(stats)
}

fn prune_node(
    path: &Path,
    cutoff: SystemTime,
    dry_run: bool,
    remove_self: bool,
) -> Result<(StorageStats, bool)> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((StorageStats::default(), true));
        }
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", path.display()));
        }
    };
    let old = metadata.modified().is_ok_and(|modified| modified <= cutoff);
    if metadata.file_type().is_symlink() || metadata.is_file() {
        if !old {
            return Ok((StorageStats::default(), false));
        }
        if !dry_run {
            fs::remove_file(path)
                .with_context(|| format!("could not remove {}", path.display()))?;
        }
        return Ok((
            StorageStats {
                bytes: metadata.len(),
                files: 1,
                directories: 0,
            },
            true,
        ));
    }
    if !metadata.is_dir() {
        return Ok((StorageStats::default(), false));
    }
    let mut entries = fs::read_dir(path)
        .with_context(|| format!("could not read directory {}", path.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("could not read entries in {}", path.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    let mut stats = StorageStats::default();
    let mut empty_after = true;
    for entry in entries {
        let (child, removed) = prune_node(&entry.path(), cutoff, dry_run, true)?;
        stats.add(&child);
        empty_after &= removed;
    }
    if remove_self && old && empty_after {
        if !dry_run {
            fs::remove_dir(path)
                .with_context(|| format!("could not remove directory {}", path.display()))?;
        }
        stats.directories = stats.directories.saturating_add(1);
        Ok((stats, true))
    } else {
        Ok((stats, false))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::UNIX_EPOCH};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn cache_areas_are_explicit_and_never_include_durable_java_data() {
        let root = PathBuf::from("/cache/jman");
        let roots = CacheDirectories {
            shared: root.clone(),
            lsp: root.join("lsp"),
        };
        let paths = area_paths(&roots, CacheArea::All);
        assert!(paths.contains(&root.join("repository")));
        assert!(paths.contains(&root.join("artifacts")));
        assert!(paths.contains(&root.join("lsp")));
        assert!(paths.iter().all(|path| !path.ends_with("jdks")));
        assert!(paths.iter().all(|path| !path.ends_with("shims")));
        assert!(paths.iter().all(|path| !path.ends_with("current")));
    }

    #[test]
    fn cache_root_safety_rejects_filesystem_roots_and_worktree_ancestors() {
        let filesystem_root = std::env::current_dir()
            .expect("current directory")
            .ancestors()
            .last()
            .expect("filesystem root")
            .to_owned();
        assert!(validate_cache_root(&filesystem_root).is_err());
        let current = std::env::current_dir().expect("current directory");
        assert!(validate_cache_root(&current).is_err());

        let dedicated = tempfile::tempdir().expect("dedicated cache parent");
        assert!(validate_cache_root(&dedicated.path().join("jman")).is_ok());
    }

    #[test]
    fn inspection_and_removal_do_not_follow_symlinks() {
        let temporary = tempdir().expect("temporary directory");
        let outside = temporary.path().join("outside");
        let cache = temporary.path().join("cache");
        fs::create_dir_all(&outside).expect("outside directory");
        fs::create_dir_all(&cache).expect("cache directory");
        fs::write(outside.join("keep.txt"), "keep").expect("outside file");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, cache.join("linked")).expect("cache symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(&outside, cache.join("linked")).expect("cache symlink");

        let stats = inspect_path(&cache).expect("cache stats");
        assert_eq!(stats.files, 1);
        remove_path(&cache).expect("remove cache");
        assert_eq!(
            fs::read_to_string(outside.join("keep.txt")).expect("outside survives"),
            "keep"
        );
    }

    #[test]
    fn prune_dry_run_is_exact_and_does_not_write() {
        let temporary = tempdir().expect("temporary directory");
        let cache = temporary.path().join("cache");
        fs::create_dir_all(cache.join("nested")).expect("cache directory");
        fs::write(cache.join("nested/old.bin"), "old").expect("old file");
        let stats = prune_path(&cache, SystemTime::now(), true).expect("dry-run prune");
        assert_eq!(stats.files, 1);
        assert_eq!(stats.directories, 1);
        assert!(cache.join("nested/old.bin").is_file());

        let stats = prune_path(&cache, SystemTime::now(), false).expect("prune");
        assert_eq!(stats.files, 1);
        assert_eq!(stats.directories, 1);
        assert!(cache.is_dir());
        assert!(!cache.join("nested").exists());
    }

    #[test]
    fn prune_keeps_entries_newer_than_the_cutoff() {
        let temporary = tempdir().expect("temporary directory");
        let cache = temporary.path().join("cache");
        fs::create_dir_all(&cache).expect("cache directory");
        fs::write(cache.join("new.bin"), "new").expect("new file");
        let stats = prune_path(&cache, UNIX_EPOCH, false).expect("prune");
        assert_eq!(stats.files, 0);
        assert!(cache.join("new.bin").is_file());
    }
}
