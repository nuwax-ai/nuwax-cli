//! Stage a release before stopping services, then replace only managed files.
//! Persistent trees are never traversed, moved, copied, or removed. The journal
//! contains package files only, and is retained if recovery cannot finish.

use anyhow::{Context, Result};
use client_core::upgrade_strategy::UpgradeStrategy;
use client_core::utils::archive::{self, ArchiveFormat};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct EnvChange {
    path: PathBuf,
    resolved_path: PathBuf,
    original: Option<String>,
    merged: String,
    applied: bool,
}

/// No live package files change during construction. Keep this plan across the
/// stop operation, and call `apply` only after all old services have stopped.
pub struct PackageReplacement {
    archive_path: PathBuf,
    root: PathBuf,
    temporary: Option<TempDir>,
    stage: PathBuf,
    backup: PathBuf,
    protected: Vec<PathBuf>,
    protected_aliases: BTreeMap<PathBuf, (PathBuf, Option<PathBuf>)>,
    protected_critical_paths: BTreeSet<PathBuf>,
    old_files: BTreeSet<PathBuf>,
    old_dirs: BTreeMap<PathBuf, fs::Permissions>,
    incoming_files: BTreeSet<PathBuf>,
    incoming_dirs: BTreeSet<PathBuf>,
    incoming_dir_modes: BTreeMap<PathBuf, u32>,
    original_entries: BTreeMap<PathBuf, Option<fs::Metadata>>,
    moved: Vec<PathBuf>,
    installed: Vec<PathBuf>,
    created_dirs: Vec<PathBuf>,
    removed_dirs: Vec<(PathBuf, fs::Permissions)>,
    environment: Vec<EnvChange>,
    applied: bool,
}

impl PackageReplacement {
    /// Archive staging can read many gigabytes. Keep that work off Tokio's
    /// runtime threads; cancellation cannot mutate live files because this task
    /// completes preparation only, before service stop and before `apply`.
    pub async fn prepare_async(
        archive_path: &Path,
        strategy: &UpgradeStrategy,
        docker_root: &Path,
        env_path: &Path,
        merge_selected_env: bool,
    ) -> Result<Self> {
        let archive_path = archive_path.to_path_buf();
        let strategy = strategy.clone();
        let docker_root = docker_root.to_path_buf();
        let env_path = env_path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            Self::prepare(
                &archive_path,
                &strategy,
                &docker_root,
                &env_path,
                merge_selected_env,
            )
        })
        .await
        .context("Package preparation task failed before stopping services")?
    }

    pub fn prepare(
        archive_path: &Path,
        strategy: &UpgradeStrategy,
        docker_root: &Path,
        env_path: &Path,
        merge_selected_env: bool,
    ) -> Result<Self> {
        let root = super::absolute_clean_path(docker_root)?;
        let parent = root.parent().context("Package root has no parent")?;
        reject_link(&root)?;
        let temporary = tempfile::Builder::new()
            .prefix(".nuwax-package-")
            .tempdir_in(parent)
            .context("Cannot prepare a package transaction before stopping services")?;
        let stage = temporary.path().join("incoming");
        let backup = temporary.path().join("previous");
        fs::create_dir(&stage)?;
        fs::create_dir(&backup)?;
        let selected = super::absolute_clean_path(env_path)?;
        let mut protected = vec![selected.clone()];
        let mut protected_aliases = BTreeMap::new();
        if env_path.exists() {
            protected.push(fs::canonicalize(env_path)?);
        }
        let lexical_env = if env_path.is_absolute() {
            env_path.to_path_buf()
        } else {
            std::env::current_dir()?.join(env_path)
        };
        for ancestor in lexical_env
            .ancestors()
            .take_while(|ancestor| ancestor.starts_with(&root))
        {
            if fs::symlink_metadata(ancestor)
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                fs::canonicalize(ancestor)
                    .context("Selected environment alias must resolve before stopping services")?;
                protected.push(ancestor.to_path_buf());
            }
        }
        // Protect the whole existing persistent tree, including absent children.
        // Do not traverse it: a root-owned database tree can be unreadable.
        if root.exists() {
            for name in client_core::constants::docker::EXCLUDE_DIRS {
                let path = root.join(name);
                if fs::symlink_metadata(&path).is_ok() {
                    if name != ".env" && fs::symlink_metadata(&path)?.file_type().is_symlink() {
                        protected_aliases.insert(
                            path.clone(),
                            (fs::read_link(&path)?, fs::canonicalize(&path).ok()),
                        );
                    }
                    if path.exists() {
                        let target = fs::canonicalize(&path)?;
                        if name != ".env" && root.starts_with(&target) {
                            anyhow::bail!(
                                "Persistent path {} resolves to the package root or an ancestor; refusing replacement before stopping services",
                                path.display()
                            );
                        }
                        protected.push(target);
                    }
                    protected.push(path);
                }
            }
        }
        let mut plan = Self {
            archive_path: archive_path.to_path_buf(),
            root,
            temporary: Some(temporary),
            stage,
            backup,
            protected,
            protected_aliases,
            protected_critical_paths: BTreeSet::new(),
            old_files: BTreeSet::new(),
            old_dirs: BTreeMap::new(),
            incoming_files: BTreeSet::new(),
            incoming_dirs: BTreeSet::new(),
            incoming_dir_modes: BTreeMap::new(),
            original_entries: BTreeMap::new(),
            moved: Vec::new(),
            installed: Vec::new(),
            created_dirs: Vec::new(),
            removed_dirs: Vec::new(),
            environment: Vec::new(),
            applied: false,
        };
        if plan
            .protected_aliases
            .keys()
            .any(|alias| fs::canonicalize(alias).is_ok_and(|target| target.starts_with(&plan.root)))
        {
            plan.protected_critical_paths = critical_release_paths(archive_path)?;
        }
        #[cfg(unix)]
        if plan.root.exists() {
            use std::os::unix::fs::MetadataExt;
            if fs::metadata(&plan.root)?.dev() != fs::metadata(&plan.stage)?.dev() {
                anyhow::bail!(
                    "Package root and transaction staging directory are on different filesystems; refusing managed-file renames before stopping services"
                );
            }
        }
        super::validate_archive_paths(archive_path)?;
        let format = archive::detect_format_by_magic(archive_path)?;
        let mut selection = Selection::new(strategy)?;
        let mut defaults = None;
        let mut seen = BTreeSet::new();
        match format {
            ArchiveFormat::Zip => {
                let mut zip = zip::ZipArchive::new(File::open(archive_path)?)?;
                if !selection.full {
                    if super::optional_zip_text(&mut zip, "docker-compose.yml")?.is_none()
                        && !super::legacy_schema::can_retain(
                            "docker-compose.yml",
                            &selection
                                .affected_roots()
                                .iter()
                                .map(|path| path.to_string_lossy().into_owned())
                                .collect::<Vec<_>>(),
                        )
                    {
                        anyhow::bail!("Patch changes Compose but does not include its replacement");
                    }
                    selection.critical = super::patch_critical_files(&mut zip)?
                        .into_iter()
                        .map(PathBuf::from)
                        .collect();
                }
                for index in 0..zip.len() {
                    let mut entry = zip.by_index(index)?;
                    let raw = entry.enclosed_name().context("Unsafe package path")?;
                    let relative = normalize_entry(&raw)?;
                    let mode = entry.unix_mode().unwrap_or(0o644);
                    let kind = mode & 0o170000;
                    if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                        anyhow::bail!(
                            "Package links or special files are forbidden: {}",
                            relative.display()
                        );
                    }
                    plan.stage_entry(
                        &relative,
                        entry.is_dir(),
                        mode,
                        &mut entry,
                        &selection,
                        &mut seen,
                        &mut defaults,
                        &selected,
                    )?;
                }
            }
            ArchiveFormat::TarGz => {
                if !selection.full {
                    anyhow::bail!("Incremental package application requires ZIP");
                }
                let decoder = flate2::read::GzDecoder::new(File::open(archive_path)?);
                let mut tar = tar::Archive::new(decoder);
                for entry in tar.entries()? {
                    let mut entry = entry?;
                    let relative = normalize_entry(&entry.path()?)?;
                    let kind = entry.header().entry_type();
                    if !kind.is_file() && !kind.is_dir() {
                        anyhow::bail!(
                            "Package links or special files are forbidden: {}",
                            relative.display()
                        );
                    }
                    let mode = entry.header().mode()?;
                    plan.stage_entry(
                        &relative,
                        kind.is_dir(),
                        mode,
                        &mut entry,
                        &selection,
                        &mut seen,
                        &mut defaults,
                        &selected,
                    )?;
                }
            }
        }
        for required in selection.required_files() {
            if !seen.contains(&required) && !plan.is_protected(&required) {
                anyhow::bail!(
                    "Package replacement file is missing: {}",
                    required.display()
                );
            }
        }
        for directory in &selection.directories {
            if !plan.is_protected(directory)
                && !seen.iter().any(|entry| entry.starts_with(directory))
            {
                anyhow::bail!(
                    "Package replacement directory is missing: {}",
                    directory.display()
                );
            }
        }
        if merge_selected_env {
            plan.prepare_env(&selected, defaults.as_deref().unwrap_or_default())?;
        }
        // Enumerate only managed paths which the release is allowed to replace.
        if selection.full {
            plan.collect_old(Path::new(""))?;
        } else {
            for path in selection.affected_roots() {
                plan.collect_old(&path)?;
            }
        }
        for path in plan.incoming_files.clone() {
            plan.collect_old(&path)?;
        }
        plan.validate_destinations()?;
        for relative in plan.old_files.union(&plan.incoming_files) {
            let metadata = match fs::symlink_metadata(plan.root.join(relative)) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error)
                        .context("Cannot snapshot managed path before stopping services");
                }
            };
            plan.original_entries.insert(relative.clone(), metadata);
        }
        let recovery_metadata = serde_json::json!({
            "transaction_version": 1,
            "docker_root": plan.root,
            "protected_paths": plan.protected,
            "managed_previous_files": plan.old_files,
            "environment_snapshots": plan.environment.iter().enumerate().map(|(index, change)| {
                serde_json::json!({"destination": change.path, "snapshot": change.original.as_ref().map(|_| format!("previous/env-{index}.snapshot"))})
            }).collect::<Vec<_>>(),
        });
        client_core::atomic_file::write_atomic(
            &plan
                .stage
                .parent()
                .context("Transaction directory missing")?
                .join("transaction.json"),
            &serde_json::to_vec_pretty(&recovery_metadata)?,
            client_core::atomic_file::PermissionsPolicy::Private,
        )?;
        plan.record_phase("prepared")?;
        Ok(plan)
    }

    fn is_protected(&self, relative: &Path) -> bool {
        let path = self.root.join(relative);
        self.protected
            .iter()
            .any(|protected| path.starts_with(protected))
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_entry(
        &mut self,
        relative: &Path,
        directory: bool,
        mode: u32,
        input: &mut impl Read,
        selection: &Selection,
        seen: &mut BTreeSet<PathBuf>,
        defaults: &mut Option<String>,
        selected: &Path,
    ) -> Result<()> {
        if relative.as_os_str().is_empty() {
            return Ok(());
        }
        if !seen.insert(relative.to_path_buf()) {
            anyhow::bail!("Duplicate package path: {}", relative.display());
        }
        if super::should_skip_file(&relative.to_string_lossy()) {
            return Ok(());
        }
        if relative == Path::new(".env") && !directory {
            let mut text = String::new();
            input
                .read_to_string(&mut text)
                .context("Package .env is not valid UTF-8")?;
            // Validate defaults even when an existing .env is protected.
            super::env_merge::merge_env_contents("", &text)?;
            *defaults = Some(text.clone());
            let target = self.root.join(relative);
            if target.exists() {
                if target != selected || !selection.full {
                    self.prepare_env(&target, &text)?;
                }
            } else {
                self.write_staged(relative, text.as_bytes(), mode)?;
            }
            return Ok(());
        }
        if self.is_protected(relative) && self.protected_critical_paths.contains(relative) {
            anyhow::bail!(
                "Persistent alias protects required release file {}; cannot safely install this package before stopping services",
                relative.display()
            );
        }
        if self.is_protected(relative) || !selection.includes(relative) {
            return Ok(());
        }
        if directory {
            fs::create_dir_all(self.stage.join(relative))?;
            self.incoming_dirs.insert(relative.to_path_buf());
            self.incoming_dir_modes.insert(relative.to_path_buf(), mode);
        } else {
            let target = self.root.join(relative);
            if target.file_name().is_some_and(|name| name == ".env") && target.is_file() {
                let mut text = String::new();
                input.read_to_string(&mut text)?;
                self.prepare_env(&target, &text)?;
            } else {
                let stage = self.stage.join(relative);
                fs::create_dir_all(stage.parent().context("Staged file has no parent")?)?;
                let mut file = File::create(&stage)?;
                std::io::copy(input, &mut file).with_context(|| {
                    format!(
                        "Failed to stage {} before stopping services",
                        relative.display()
                    )
                })?;
                file.sync_all()?;
                set_mode(&stage, mode)?;
                self.incoming_files.insert(relative.to_path_buf());
            }
        }
        Ok(())
    }

    fn write_staged(&mut self, relative: &Path, contents: &[u8], mode: u32) -> Result<()> {
        let target = self.stage.join(relative);
        fs::create_dir_all(target.parent().context("Staged file has no parent")?)?;
        fs::write(&target, contents)?;
        set_mode(&target, mode)?;
        self.incoming_files.insert(relative.to_path_buf());
        Ok(())
    }

    fn prepare_env(&mut self, path: &Path, defaults: &str) -> Result<()> {
        if self.environment.iter().any(|change| change.path == path) {
            return Ok(());
        }
        let original = match fs::read_to_string(path) {
            Ok(contents) => Some(contents),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && fs::symlink_metadata(path).is_err() =>
            {
                None
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Cannot read environment {} before stopping services",
                        path.display()
                    )
                });
            }
        };
        let merged = super::env_merge::merge_env_contents(
            original.as_deref().unwrap_or_default(),
            defaults,
        )?;
        let target = if path.exists() {
            fs::canonicalize(path)?
        } else {
            path.to_path_buf()
        };
        #[cfg(unix)]
        if let Ok(metadata) = fs::metadata(&target) {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != nix::unistd::geteuid().as_raw() {
                anyhow::bail!(
                    "Environment {} is owned by another user; refusing an atomic replacement which would change its owner before stopping services",
                    path.display()
                );
            }
            let probe = tempfile::NamedTempFile::new_in(
                target
                    .parent()
                    .context("Environment target has no parent")?,
            )
            .with_context(|| {
                format!(
                    "Environment parent is not writable before stopping services: {}",
                    target.display()
                )
            })?;
            if metadata.gid() != probe.as_file().metadata()?.gid() {
                anyhow::bail!(
                    "Environment {} has a group that an atomic replacement would not retain; refusing before stopping services",
                    path.display()
                );
            }
        }
        validate_writable_parent(&target)?;
        if let Some(contents) = original.as_ref() {
            let snapshot = self
                .backup
                .join(format!("env-{}.snapshot", self.environment.len()));
            // The transaction directory is 0700; environment snapshots are 0600.
            client_core::atomic_file::write_atomic(
                &snapshot,
                contents.as_bytes(),
                client_core::atomic_file::PermissionsPolicy::Private,
            )?;
        }
        self.protected.push(path.to_path_buf());
        self.protected.push(target.clone());
        self.environment.push(EnvChange {
            path: path.to_path_buf(),
            resolved_path: target,
            original,
            merged,
            applied: false,
        });
        Ok(())
    }

    fn collect_old(&mut self, relative: &Path) -> Result<()> {
        let path = self.root.join(relative);
        if self.is_protected(relative) {
            return Ok(());
        }
        reject_link(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                if !relative.as_os_str().is_empty() {
                    self.old_dirs
                        .insert(relative.to_path_buf(), metadata.permissions());
                }
                for entry in fs::read_dir(&path)? {
                    let entry = entry?;
                    self.collect_old(&relative.join(entry.file_name()))?;
                }
            }
            Ok(metadata) if metadata.is_file() => {
                self.old_files.insert(relative.to_path_buf());
            }
            Ok(_) => anyhow::bail!("Unsupported live package file: {}", path.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Cannot inspect managed path {} before stopping services",
                        path.display()
                    )
                });
            }
        }
        Ok(())
    }

    fn validate_destinations(&self) -> Result<()> {
        let mut parents = BTreeSet::new();
        for relative in self
            .old_files
            .iter()
            .chain(self.old_dirs.keys())
            .chain(self.incoming_files.iter())
            .chain(self.incoming_dirs.iter())
        {
            let path = self.root.join(relative);
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if let Ok(metadata) = fs::symlink_metadata(&path)
                    && metadata.dev() != fs::metadata(&self.stage)?.dev()
                {
                    anyhow::bail!(
                        "Managed package path {} is on another filesystem; refusing replacement before stopping services",
                        path.display()
                    );
                }
            }
            #[cfg(unix)]
            if self.old_files.contains(relative) {
                use std::os::unix::fs::MetadataExt;
                let leaf = fs::metadata(&path)?;
                let parent = fs::metadata(path.parent().context("Managed file has no parent")?)?;
                let current_uid = nix::unistd::geteuid().as_raw();
                if current_uid != 0
                    && parent.mode() & 0o1000 != 0
                    && parent.uid() != current_uid
                    && leaf.uid() != current_uid
                {
                    anyhow::bail!(
                        "Sticky managed parent prevents replacing {} as the current user before stopping services",
                        path.display()
                    );
                }
            }
            for ancestor in path
                .ancestors()
                .take_while(|ancestor| ancestor.starts_with(&self.root))
            {
                reject_link(ancestor)?;
            }
            if path.is_dir()
                && self.incoming_files.contains(relative)
                && self
                    .protected
                    .iter()
                    .any(|protected| protected.starts_with(&path))
            {
                anyhow::bail!(
                    "Package file cannot replace a directory containing protected state: {}",
                    path.display()
                );
            }
            let mut parent = path.parent().context("Package path has no parent")?;
            while !parent.is_dir() {
                if parent.is_file() {
                    let relative_parent = parent.strip_prefix(&self.root)?;
                    if !self.old_files.contains(relative_parent) {
                        anyhow::bail!(
                            "Managed file blocks package directory: {}",
                            parent.display()
                        );
                    }
                }
                parent = parent.parent().context("No existing package ancestor")?;
            }
            parents.insert(parent.to_path_buf());
        }
        parents.insert(if self.root.is_dir() {
            self.root.clone()
        } else {
            self.root
                .parent()
                .context("Package root has no parent")?
                .to_path_buf()
        });
        for parent in parents {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if fs::metadata(&parent)?.dev() != fs::metadata(&self.stage)?.dev() {
                    anyhow::bail!(
                        "Managed directory {} is on another filesystem; refusing package renames before stopping services",
                        parent.display()
                    );
                }
            }
            probe_directory(&parent)?;
            // Bind mounts can have the same st_dev yet disallow cross-mount
            // rename. Exercise the exact managed-file journal direction with a
            // disposable caller-owned file before stopping any service.
            let probe = tempfile::NamedTempFile::new_in(&parent)?;
            let destination = self
                .stage
                .join(format!(".rename-probe-{}", uuid::Uuid::new_v4()));
            let file = probe.persist(&destination).map_err(|error| error.error)
                .with_context(|| format!("Managed directory {} cannot rename into package staging before stopping services", parent.display()))?;
            drop(file);
            fs::remove_file(destination)?;
        }
        Ok(())
    }

    pub fn apply(&mut self) -> Result<()> {
        if self.applied {
            anyhow::bail!("Package transaction was already applied");
        }
        // Windows volume preparation (or an operator) may have created a
        // persistent directory after staging. Protect it now as a whole too.
        for (alias, (expected_link, expected_target)) in &self.protected_aliases {
            if fs::read_link(alias).as_ref().ok() != Some(expected_link)
                || fs::canonicalize(alias).ok() != *expected_target
            {
                anyhow::bail!(
                    "Persistent alias {} changed after preflight; previous package files are untouched",
                    alias.display()
                );
            }
        }
        let mut newly_protected = Vec::new();
        let mut new_internal_alias = false;
        for name in client_core::constants::docker::EXCLUDE_DIRS {
            if name == ".env" {
                continue;
            }
            let path = self.root.join(name);
            if fs::symlink_metadata(&path).is_ok() && !self.protected.contains(&path) {
                newly_protected.push(path.clone());
                self.protected.push(path);
                if let Ok(target) = fs::canonicalize(self.root.join(name)) {
                    if self.root.starts_with(&target) {
                        anyhow::bail!(
                            "New persistent alias resolves to the package root or an ancestor"
                        );
                    }
                    if fs::symlink_metadata(self.root.join(name))?
                        .file_type()
                        .is_symlink()
                        && target.starts_with(&self.root)
                    {
                        new_internal_alias = true;
                    }
                    newly_protected.push(target.clone());
                    self.protected.push(target);
                }
            }
        }
        if new_internal_alias {
            let critical = critical_release_paths(&self.archive_path)?;
            for relative in &self.incoming_files {
                if critical.contains(relative)
                    && newly_protected
                        .iter()
                        .any(|path| self.root.join(relative).starts_with(path))
                {
                    anyhow::bail!(
                        "New persistent alias protects required release file {}; previous package files are untouched",
                        relative.display()
                    );
                }
            }
        }
        let root = &self.root;
        self.incoming_files.retain(|relative| {
            !newly_protected
                .iter()
                .any(|path| root.join(relative).starts_with(path))
        });
        self.incoming_dirs.retain(|relative| {
            !newly_protected
                .iter()
                .any(|path| root.join(relative).starts_with(path))
        });
        self.old_files.retain(|relative| {
            !newly_protected
                .iter()
                .any(|path| root.join(relative).starts_with(path))
        });
        self.old_dirs.retain(|relative, _| {
            !newly_protected
                .iter()
                .any(|path| root.join(relative).starts_with(path))
        });
        self.validate_destinations()?;
        for (relative, original) in &self.original_entries {
            if self.is_protected(relative) && relative != Path::new(".env") {
                continue;
            }
            let current = match fs::symlink_metadata(self.root.join(relative)) {
                Ok(metadata) => Some(metadata),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(error).context("Cannot revalidate managed package path"),
            };
            if !same_entry(original.as_ref(), current.as_ref()) {
                anyhow::bail!(
                    "Managed package path {} changed after preflight; previous package files are untouched",
                    self.root.join(relative).display()
                );
            }
        }
        for change in &self.environment {
            let resolved = if change.path.exists() {
                fs::canonicalize(&change.path)?
            } else {
                change.path.clone()
            };
            if resolved != change.resolved_path {
                anyhow::bail!(
                    "Environment alias {} changed after preflight; previous package files are untouched",
                    change.path.display()
                );
            }
            let current = match fs::read_to_string(&change.path) {
                Ok(contents) => Some(contents),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("Cannot revalidate environment {}", change.path.display())
                    });
                }
            };
            if current != change.original {
                anyhow::bail!(
                    "Environment {} changed after preflight; previous package files are untouched",
                    change.path.display()
                );
            }
        }
        self.record_phase("applying")?;
        let result = self.apply_inner(&mut || Ok(()));
        if let Err(error) = result {
            return self.recover_error(error);
        }
        self.applied = true;
        if let Err(error) = self.record_phase("files_applied") {
            return self.recover_error(error);
        }
        Ok(())
    }

    fn apply_inner(&mut self, checkpoint: &mut impl FnMut() -> Result<()>) -> Result<()> {
        if !self.root.exists() {
            fs::create_dir(&self.root)?;
            self.created_dirs.push(self.root.clone());
        }
        for relative in &self.old_files {
            let old = self.root.join(relative);
            let backup = self.backup.join("files").join(relative);
            fs::create_dir_all(backup.parent().context("Backup file has no parent")?)?;
            fs::rename(&old, &backup).with_context(|| {
                format!("Failed to journal managed package file {}", old.display())
            })?;
            self.moved.push(relative.clone());
            checkpoint()?;
        }
        // A patch directory delete/replacement includes empty directories too.
        // Remove only directories emptied by this transaction, deepest first;
        // a remaining protected child prevents removal and stays in place.
        let mut old_dirs = self
            .old_dirs
            .iter()
            .map(|(relative, permissions)| (relative.clone(), permissions.clone()))
            .collect::<Vec<_>>();
        old_dirs.sort_by_key(|(relative, _)| std::cmp::Reverse(relative.components().count()));
        for (relative, permissions) in old_dirs {
            let path = self.root.join(&relative);
            if self
                .protected
                .iter()
                .any(|protected| protected.starts_with(&path))
            {
                continue;
            }
            match fs::remove_dir(&path) {
                Ok(()) => {
                    self.removed_dirs.push((path, permissions));
                    checkpoint()?;
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!(
                            "Cannot remove journaled managed directory {}",
                            path.display()
                        )
                    });
                }
            }
        }
        let mut directories = self.incoming_dirs.clone();
        for relative in &self.incoming_files {
            for parent in relative
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                directories.insert(parent.to_path_buf());
            }
        }
        for relative in directories {
            let path = self.root.join(relative);
            if !path.exists() {
                fs::create_dir(&path)?;
                self.created_dirs.push(path);
            }
        }
        for relative in &self.incoming_files {
            let target = self.root.join(relative);
            if target.is_dir() {
                remove_empty_tree(&target, &mut self.removed_dirs)?;
            }
            fs::rename(self.stage.join(relative), &target).with_context(|| {
                format!(
                    "Failed to install managed package file {}",
                    target.display()
                )
            })?;
            self.installed.push(relative.clone());
            checkpoint()?;
        }
        for change in &mut self.environment {
            // A sync error can happen after rename, so rollback must also include
            // the environment currently being attempted.
            change.applied = true;
            if change.original.as_ref() != Some(&change.merged) {
                super::env_merge::write_env_contents(&change.path, &change.merged, None)?;
            }
            checkpoint()?;
        }
        // TAR directory header modes (including restrictive modes) are applied
        // after their children are installed. Existing protected parents retain
        // their original permissions and ownership.
        for (relative, mode) in &self.incoming_dir_modes {
            let path = self.root.join(relative);
            if self.incoming_dirs.contains(relative) && self.created_dirs.contains(&path) {
                set_mode(&path, *mode)?;
            }
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        #[cfg(unix)]
        for directory in &self.created_dirs {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(directory) {
                let mode = metadata.permissions().mode();
                if let Err(error) =
                    fs::set_permissions(directory, fs::Permissions::from_mode(mode | 0o700))
                {
                    failures.push(anyhow::Error::new(error).context(format!(
                        "Cannot make transaction-created directory recoverable {}",
                        directory.display()
                    )));
                }
            }
        }
        for change in self
            .environment
            .iter_mut()
            .rev()
            .filter(|change| change.applied)
        {
            let result = match &change.original {
                Some(contents) => {
                    if fs::read_to_string(&change.path).is_ok_and(|current| current == *contents) {
                        Ok(())
                    } else {
                        super::env_merge::write_env_contents(&change.path, contents, None)
                    }
                }
                None => {
                    if change.path.exists() {
                        fs::remove_file(&change.path).map_err(Into::into)
                    } else {
                        Ok(())
                    }
                }
            };
            match result {
                Ok(()) => change.applied = false,
                Err(error) => failures.push(error.context(format!(
                    "Cannot restore environment {}",
                    change.path.display()
                ))),
            }
        }
        // An inaccessible external env must not prevent restoring the remaining
        // managed files. Attempt every journal action, retaining failed entries.
        let mut pending_installed = Vec::new();
        for relative in self.installed.iter().rev() {
            let path = self.root.join(relative);
            if path.exists()
                && let Err(error) = fs::remove_file(&path)
            {
                pending_installed.push(relative.clone());
                failures.push(anyhow::Error::new(error).context(format!(
                    "Cannot remove installed managed file {}",
                    path.display()
                )));
            }
        }
        self.installed = pending_installed;
        // Never recurse into directories created by this transaction: external
        // processes may have begun using them during a failed deployment.
        let mut pending_created = Vec::new();
        for path in self.created_dirs.iter().rev() {
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => {
                    pending_created.push(path.clone());
                    failures.push(anyhow::Error::new(error).context(format!(
                        "Cannot remove empty transaction directory {}",
                        path.display()
                    )));
                }
            }
        }
        self.created_dirs = pending_created;
        let mut pending_removed = Vec::new();
        for (path, permissions) in self.removed_dirs.iter().rev() {
            let result = fs::create_dir_all(path)
                .and_then(|()| fs::set_permissions(path, permissions.clone()));
            if let Err(error) = result {
                pending_removed.push((path.clone(), permissions.clone()));
                failures.push(anyhow::Error::new(error).context(format!(
                    "Cannot restore managed directory {}",
                    path.display()
                )));
            }
        }
        self.removed_dirs = pending_removed;
        let mut pending_moved = Vec::new();
        for relative in self.moved.iter().rev() {
            let destination = self.root.join(relative);
            let result = (|| -> Result<()> {
                fs::create_dir_all(
                    destination
                        .parent()
                        .context("Restored file has no parent")?,
                )?;
                fs::rename(self.backup.join("files").join(relative), &destination)?;
                Ok(())
            })();
            if let Err(error) = result {
                pending_moved.push(relative.clone());
                failures.push(error.context(format!(
                    "Cannot restore managed package file {}",
                    destination.display()
                )));
            }
        }
        self.moved = pending_moved;
        self.applied = false;
        if !failures.is_empty() {
            anyhow::bail!(
                "Managed-file rollback encountered {} error(s): {}",
                failures.len(),
                failures
                    .iter()
                    .map(|error| format!("{error:#}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        Ok(())
    }

    pub fn recover_error(&mut self, error: anyhow::Error) -> Result<()> {
        match self.rollback() {
            Ok(()) => Err(error.context("Package preparation failed; previous managed files restored and protected data left in place")),
            Err(recovery) => {
                let directory = self.retain_for_recovery();
                Err(error.context(format!("Managed-file recovery failed: {recovery:#}; inspect {}; protected data remains at {}", directory.display(), self.root.display())))
            }
        }
    }

    /// After MySQL startup, database changes may have happened. Retain the old
    /// managed files for inspection, rather than rolling back files or data.
    pub fn retain_for_recovery(&mut self) -> PathBuf {
        self.temporary
            .take()
            .map(TempDir::keep)
            .unwrap_or_else(|| self.backup.clone())
    }

    pub fn mark_database_starting(&self) -> Result<()> {
        self.record_phase("mysql_starting")
    }

    fn record_phase(&self, phase: &str) -> Result<()> {
        let directory = self
            .temporary
            .as_ref()
            .context("Transaction recovery directory is unavailable")?
            .path();
        let state = serde_json::json!({
            "phase": phase,
            "docker_root": self.root,
            "moved_files": self.moved,
            "installed_files": self.installed,
            "environment_applied": self.environment.iter().filter(|change| change.applied).map(|change| &change.path).collect::<Vec<_>>(),
        });
        client_core::atomic_file::write_atomic(
            &directory.join("phase.json"),
            &serde_json::to_vec_pretty(&state)?,
            client_core::atomic_file::PermissionsPolicy::Private,
        )
        .context("Cannot persist package transaction phase")
    }

    pub fn finish(mut self) -> Result<()> {
        self.applied = false;
        self.moved.clear();
        for change in &mut self.environment {
            change.applied = false;
        }
        if let Some(directory) = self.temporary.take() {
            let recovery_path = directory.path().to_path_buf();
            if let Err(error) = directory.close() {
                tracing::warn!(%error, recovery_path = %recovery_path.display(), "Deployment completed, but old managed package files could not be cleaned up");
            }
        }
        Ok(())
    }
}

impl Drop for PackageReplacement {
    fn drop(&mut self) {
        // Never silently delete the only old package copy on an unhandled exit.
        if self.temporary.is_some()
            && (!self.moved.is_empty() || self.environment.iter().any(|change| change.applied))
        {
            let path = self.retain_for_recovery();
            tracing::warn!(recovery_path = %path.display(), "Retained managed package recovery files; persistent state was not moved");
        }
    }
}

struct Selection {
    full: bool,
    files: BTreeSet<PathBuf>,
    directories: Vec<PathBuf>,
    deletes: Vec<PathBuf>,
    critical: BTreeSet<PathBuf>,
}

impl Selection {
    fn new(strategy: &UpgradeStrategy) -> Result<Self> {
        let mut selection = Self {
            full: false,
            files: BTreeSet::new(),
            directories: Vec::new(),
            deletes: Vec::new(),
            critical: BTreeSet::new(),
        };
        match strategy {
            UpgradeStrategy::FullUpgrade { .. } => selection.full = true,
            UpgradeStrategy::PatchUpgrade { patch_info, .. } => {
                super::legacy_schema::validate_changed_paths(&patch_info.get_changed_files())?;
                if let Some(replace) = &patch_info.operations.replace {
                    selection
                        .files
                        .extend(replace.files.iter().map(PathBuf::from));
                    selection
                        .directories
                        .extend(replace.directories.iter().map(PathBuf::from));
                }
                if let Some(delete) = &patch_info.operations.delete {
                    selection.deletes.extend(
                        delete
                            .files
                            .iter()
                            .chain(&delete.directories)
                            .map(PathBuf::from),
                    );
                }
            }
            UpgradeStrategy::NoUpgrade { .. } => {
                anyhow::bail!("No-upgrade strategy does not extract a package")
            }
        }
        Ok(selection)
    }

    fn includes(&self, path: &Path) -> bool {
        self.full
            || self.files.contains(path)
            || self.critical.contains(path)
            || self
                .directories
                .iter()
                .any(|directory| path.starts_with(directory))
    }

    fn required_files(&self) -> BTreeSet<PathBuf> {
        self.files.union(&self.critical).cloned().collect()
    }

    fn affected_roots(&self) -> BTreeSet<PathBuf> {
        self.files
            .iter()
            .chain(&self.critical)
            .chain(&self.directories)
            .chain(&self.deletes)
            .cloned()
            .collect()
    }
}

fn normalize_entry(path: &Path) -> Result<PathBuf> {
    if super::contains_unsafe_component(path) || path.to_string_lossy().contains(['\\', ':']) {
        anyhow::bail!("Unsafe package path: {}", path.display());
    }
    Ok(path
        .strip_prefix("docker")
        .unwrap_or(path)
        .components()
        .collect())
}

fn reject_link(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => anyhow::bail!(
            "Package replacement refuses live symlink: {}",
            path.display()
        ),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "Cannot inspect package path {} before stopping services",
                path.display()
            )
        }),
    }
}

fn validate_writable_parent(path: &Path) -> Result<()> {
    let mut parent = path.parent().context("Environment file has no parent")?;
    while !parent.exists() {
        parent = parent
            .parent()
            .context("Environment path has no existing ancestor")?;
    }
    if !parent.is_dir() {
        anyhow::bail!(
            "Environment parent is not a directory: {}",
            parent.display()
        );
    }
    probe_directory(parent)
}

fn probe_directory(path: &Path) -> Result<()> {
    let temporary = tempfile::NamedTempFile::new_in(path).with_context(|| {
        format!(
            "Managed directory {} is not writable before stopping services",
            path.display()
        )
    })?;
    temporary.as_file().sync_all()?;
    temporary.close()?;
    Ok(())
}

fn remove_empty_tree(path: &Path, removed: &mut Vec<(PathBuf, fs::Permissions)>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            anyhow::bail!(
                "Unjournaled file prevents replacing directory {}",
                path.display()
            );
        }
        remove_empty_tree(&entry.path(), removed)?;
    }
    let permissions = fs::metadata(path)?.permissions();
    fs::remove_dir(path)?;
    removed.push((path.to_path_buf(), permissions));
    Ok(())
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn same_entry(original: Option<&fs::Metadata>, current: Option<&fs::Metadata>) -> bool {
    match (original, current) {
        (None, None) => true,
        (Some(original), Some(current)) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if original.dev() != current.dev() || original.ino() != current.ino() {
                    return false;
                }
            }
            original.file_type() == current.file_type()
                && (original.is_dir()
                    || (original.len() == current.len()
                        && original.modified().ok() == current.modified().ok()))
        }
        _ => false,
    }
}

fn critical_release_paths(archive: &Path) -> Result<BTreeSet<PathBuf>> {
    let mut paths = [
        "docker-compose.yml",
        "DELIVERY_MANIFEST.json",
        "config/mysql-schema-manifest.json",
    ]
    .into_iter()
    .chain(
        client_core::constants::sql::CRITICAL_UPGRADE_FILES
            .iter()
            .copied(),
    )
    .chain(
        client_core::constants::sql::OPTIONAL_SCHEMA_SQL_FILES
            .iter()
            .copied(),
    )
    .map(PathBuf::from)
    .collect::<BTreeSet<_>>();
    let entries = super::read_archive_entries(
        archive,
        &[
            "docker-compose.yml",
            "DELIVERY_MANIFEST.json",
            "config/mysql-schema-manifest.json",
        ],
    )?;
    if let Some(bytes) = entries.get("DELIVERY_MANIFEST.json") {
        let delivery = client_core::container::preflight::parse_delivery_manifest(
            std::str::from_utf8(bytes)?,
        )?;
        paths.insert(PathBuf::from(delivery.compose.path));
        paths.extend(delivery.mysql.files.into_keys().map(PathBuf::from));
        paths.extend(
            delivery
                .components
                .into_values()
                .flat_map(|component| component.artifacts.into_keys())
                .map(PathBuf::from),
        );
    }
    if let Some(bytes) = entries.get("config/mysql-schema-manifest.json") {
        let manifest =
            client_core::mysql_manifest::parse_schema_manifest(std::str::from_utf8(bytes)?)?;
        paths.extend(manifest.referenced_paths().into_iter().map(PathBuf::from));
    }
    if let Some(bytes) = entries.get("docker-compose.yml") {
        paths.extend(
            super::legacy_schema::component_entrypoints(std::str::from_utf8(bytes)?)?
                .into_iter()
                .map(PathBuf::from),
        );
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use client_core::upgrade_strategy::DownloadType;
    use std::io::Write;

    fn full() -> Result<UpgradeStrategy> {
        Ok(UpgradeStrategy::FullUpgrade {
            url: String::new(),
            hash: String::new(),
            signature: String::new(),
            target_version: "0.0.108".parse()?,
            download_type: DownloadType::Full,
        })
    }

    fn zip(path: &Path, entries: &[(&str, &str)]) -> Result<()> {
        let mut writer = zip::ZipWriter::new(File::create(path)?);
        for (name, contents) in entries {
            writer.start_file(
                *name,
                zip::write::SimpleFileOptions::default().unix_permissions(0o755),
            )?;
            writer.write_all(contents.as_bytes())?;
        }
        writer.finish()?;
        Ok(())
    }

    fn fixture(root: &Path) -> Result<()> {
        fs::create_dir_all(root.join("data/mysql"))?;
        fs::create_dir_all(root.join("project_workspace"))?;
        fs::create_dir_all(root.join("logs/rcoder"))?;
        fs::create_dir_all(root.join("upload"))?;
        fs::write(root.join("data/mysql/row"), "persistent database contents")?;
        fs::write(root.join("project_workspace/project"), "workspace")?;
        fs::write(root.join("logs/rcoder/api.log"), "existing log")?;
        fs::write(root.join(".env"), "# operator\nA='value # literal'\n")?;
        fs::write(root.join("docker-compose.yml"), "old compose")?;
        Ok(())
    }

    fn archive(path: &Path) -> Result<()> {
        zip(
            path,
            &[
                ("docker/docker-compose.yml", "new compose"),
                ("docker/config/start.sh", "#!/bin/sh\nexit 0\n"),
                ("docker/.env", "A=package\nNEW_KEY=added\n"),
                ("docker/data/mysql/row", "must not overwrite"),
                ("docker/data/mysql/new-package-row", "must not add"),
                ("docker/project_workspace/placeholder", "must not add"),
                ("docker/logs/rcoder/package.log", "must not add"),
            ],
        )
    }

    #[cfg(unix)]
    #[derive(Debug, PartialEq, Eq)]
    struct FsIdentity {
        device: u64,
        inode: u64,
        uid: u32,
        gid: u32,
        mode: u32,
    }

    #[cfg(unix)]
    fn identities(root: &Path) -> Result<Vec<FsIdentity>> {
        use std::os::unix::fs::MetadataExt;
        [
            "",
            "data",
            "data/mysql",
            "data/mysql/row",
            "project_workspace",
            "project_workspace/project",
            "logs",
            "logs/rcoder/api.log",
            "upload",
        ]
        .iter()
        .map(|relative| {
            let metadata = fs::metadata(root.join(relative))?;
            Ok(FsIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
                uid: metadata.uid(),
                gid: metadata.gid(),
                mode: metadata.mode(),
            })
        })
        .collect()
    }

    #[test]
    fn preparation_does_not_change_the_live_package_or_environment() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join(".env"))?,
            "# operator\nA='value # literal'\n"
        );
        assert!(!root.join("config").exists());
        drop(plan);
        assert_eq!(fs::read_dir(temporary.path())?.count(), 2);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn protected_tree_identity_and_contents_survive_full_replacement() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let before = identities(&root)?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(identities(&root)?, before);
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "new compose"
        );
        assert_eq!(
            fs::read_to_string(root.join("data/mysql/row"))?,
            "persistent database contents"
        );
        assert!(!root.join("data/mysql/new-package-row").exists());
        assert!(!root.join("project_workspace/placeholder").exists());
        assert!(!root.join("logs/rcoder/package.log").exists());
        assert_eq!(
            fs::read_to_string(root.join(".env"))?,
            "# operator\nA='value # literal'\nNEW_KEY=added\n"
        );
        Ok(())
    }

    #[test]
    fn failure_after_each_replacement_and_env_merge_restores_managed_files() -> Result<()> {
        for failure_step in 1..=5 {
            let temporary = tempfile::tempdir()?;
            let root = temporary.path().join("docker");
            fixture(&root)?;
            let package = temporary.path().join("package.zip");
            archive(&package)?;
            let mut plan =
                PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
            let mut step = 0;
            let result = plan.apply_inner(&mut || {
                step += 1;
                if step == failure_step {
                    anyhow::bail!("injected managed-file/env failure");
                }
                Ok(())
            });
            // This fixture has one journal, two installations, and one env write.
            if failure_step > 4 {
                assert!(result.is_ok());
            } else {
                assert!(result.is_err());
            }
            plan.rollback()?;
            assert_eq!(
                fs::read_to_string(root.join("docker-compose.yml"))?,
                "old compose"
            );
            assert_eq!(
                fs::read_to_string(root.join(".env"))?,
                "# operator\nA='value # literal'\n"
            );
            assert!(!root.join("config/start.sh").exists());
            assert_eq!(
                fs::read_to_string(root.join("data/mysql/row"))?,
                "persistent database contents"
            );
        }
        Ok(())
    }

    #[test]
    fn invalid_defaults_and_truncated_archive_fail_without_changing_old_files() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let package = temporary.path().join("bad.zip");
        zip(&package, &[("docker/.env", "NEW='unterminated\n")])?;
        assert!(
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)
                .is_err()
        );
        fs::write(&package, b"PK\x03\x04truncated")?;
        assert!(
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join(".env"))?,
            "# operator\nA='value # literal'\n"
        );
        Ok(())
    }

    #[test]
    fn external_env_and_missing_protected_directories_are_supported() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        let env = temporary.path().join("secrets/operator.env");
        fs::create_dir_all(env.parent().context("fixture parent")?)?;
        fs::write(&env, "A=external\n")?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let mut plan = PackageReplacement::prepare(&package, &full()?, &root, &env, true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(fs::read_to_string(&env)?, "A=external\nNEW_KEY=added\n");
        assert_eq!(
            fs::read_to_string(root.join(".env"))?,
            "A=package\nNEW_KEY=added\n"
        );
        assert!(root.join("data/mysql/new-package-row").is_file());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn external_env_symlink_is_preserved_and_live_managed_links_are_rejected() -> Result<()> {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let target = temporary.path().join("operator.env");
        fs::write(&target, "A=external\n")?;
        let alias = temporary.path().join("alias.env");
        symlink("operator.env", &alias)?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let mut plan = PackageReplacement::prepare(&package, &full()?, &root, &alias, true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(fs::read_link(&alias)?, Path::new("operator.env"));
        assert_eq!(fs::read_to_string(&target)?, "A=external\nNEW_KEY=added\n");
        symlink(&target, root.join("config/unsafe-link"))?;
        assert!(PackageReplacement::prepare(&package, &full()?, &root, &alias, true).is_err());
        assert_eq!(fs::read_to_string(&target)?, "A=external\nNEW_KEY=added\n");
        Ok(())
    }

    #[test]
    fn operator_env_change_after_preflight_is_rejected_before_replacement() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        fs::write(root.join(".env"), "A=operator-updated\n")?;
        assert!(plan.apply().is_err());
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join(".env"))?,
            "A=operator-updated\n"
        );
        Ok(())
    }

    #[test]
    fn abandoned_applied_transaction_retains_managed_recovery_files() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        let recovery = plan.retain_for_recovery();
        drop(plan);
        assert_eq!(
            fs::read_to_string(recovery.join("previous/files/docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join("data/mysql/row"))?,
            "persistent database contents"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn protected_links_preserve_internal_targets_and_dangling_aliases() -> Result<()> {
        use std::os::unix::fs::{MetadataExt, symlink};
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        fs::create_dir_all(root.join("config"))?;
        fs::rename(root.join("data"), root.join("config/persistent"))?;
        symlink("config/persistent", root.join("data"))?;
        fs::remove_dir_all(root.join("logs"))?;
        symlink("missing-log-volume", root.join("logs"))?;
        let before_data = fs::symlink_metadata(root.join("data"))?.ino();
        let before_logs = fs::symlink_metadata(root.join("logs"))?.ino();
        let before_target = fs::metadata(root.join("config/persistent/mysql/row"))?.ino();
        let package = temporary.path().join("package.zip");
        zip(
            &package,
            &[
                (
                    "docker/config/persistent/mysql/row",
                    "must not replace data",
                ),
                ("docker/logs/new.log", "must not follow dangling link"),
                ("docker/config/start.sh", "managed script"),
            ],
        )?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(fs::symlink_metadata(root.join("data"))?.ino(), before_data);
        assert_eq!(fs::symlink_metadata(root.join("logs"))?.ino(), before_logs);
        assert_eq!(
            fs::metadata(root.join("config/persistent/mysql/row"))?.ino(),
            before_target
        );
        assert_eq!(
            fs::read_to_string(root.join("data/mysql/row"))?,
            "persistent database contents"
        );
        assert_eq!(
            fs::read_link(root.join("logs"))?,
            Path::new("missing-log-volume")
        );
        assert!(!root.join("missing-log-volume").exists());
        fs::remove_file(root.join("data"))?;
        symlink(".", root.join("data"))?;
        assert!(
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn patch_delete_removes_old_empty_directories_and_rollback_restores_them() -> Result<()> {
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fs::create_dir_all(root.join("config/retired/empty"))?;
        fs::write(root.join("docker-compose.yml"), "services: {}\n")?;
        fs::write(
            root.join("config/init_mysql.sql"),
            "USE agent_platform;\nCREATE TABLE users (id INT);\n",
        )?;
        let package = temporary.path().join("patch.zip");
        zip(
            &package,
            &[(
                "docker/config/init_mysql.sql",
                "USE agent_platform;\nCREATE TABLE users (id INT);\n",
            )],
        )?;
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: None,
                    delete: Some(ReplaceOperations {
                        files: Vec::new(),
                        directories: vec!["config/retired".into()],
                    }),
                },
            },
            target_version: "0.0.108.1".parse()?,
            download_type: DownloadType::Patch,
        };
        let mut plan =
            PackageReplacement::prepare(&package, &strategy, &root, &root.join(".env"), true)?;
        plan.apply()?;
        assert!(!root.join("config/retired").exists());
        plan.rollback()?;
        assert!(root.join("config/retired/empty").is_dir());
        let mut plan =
            PackageReplacement::prepare(&package, &strategy, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert!(!root.join("config/retired").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rollback_restores_existing_empty_directories_and_original_modes() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        fs::create_dir_all(root.join("config/empty"))?;
        fs::set_permissions(root.join("config"), fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(root.join("config/empty"), fs::Permissions::from_mode(0o710))?;
        fs::write(root.join("config/previous"), "previous config")?;
        let package = temporary.path().join("package.zip");
        archive(&package)?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.rollback()?;
        assert_eq!(
            fs::read_to_string(root.join("config/previous"))?,
            "previous config"
        );
        assert!(root.join("config/empty").is_dir());
        assert_eq!(
            fs::metadata(root.join("config"))?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join("config/empty"))?
                .permissions()
                .mode()
                & 0o777,
            0o710
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn tar_directory_header_modes_are_retained_for_new_managed_directories() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        let package = temporary.path().join("package.tar.gz");
        let encoder =
            flate2::write::GzEncoder::new(File::create(&package)?, flate2::Compression::default());
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_size(0);
        header.set_mode(0o700);
        header.set_cksum();
        tar.append_data(&mut header, "docker/config", std::io::empty())?;
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "docker/config/script", b"script".as_slice())?;
        tar.into_inner()?.finish()?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(
            fs::metadata(root.join("config"))?.permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(root.join("config/script"))?
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn new_persistent_alias_after_prepare_does_not_move_its_managed_target() -> Result<()> {
        use std::os::unix::fs::{MetadataExt, symlink};
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        fs::remove_dir_all(root.join("data"))?;
        fs::create_dir_all(root.join("config/persistent"))?;
        fs::write(root.join("config/persistent/row"), "canary")?;
        let before = fs::metadata(root.join("config/persistent/row"))?.ino();
        let package = temporary.path().join("package.zip");
        zip(
            &package,
            &[
                ("docker/config/persistent/row", "package must not overwrite"),
                ("docker/config/start", "managed"),
            ],
        )?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        symlink("config/persistent", root.join("data"))?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(fs::metadata(root.join("data/row"))?.ino(), before);
        assert_eq!(fs::read_to_string(root.join("data/row"))?, "canary");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persistent_alias_overlapping_release_sql_is_rejected_before_apply() -> Result<()> {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fixture(&root)?;
        fs::remove_dir_all(root.join("data"))?;
        fs::create_dir(root.join("config"))?;
        fs::write(root.join("config/init_mysql.sql"), "preserved data")?;
        symlink("config", root.join("data"))?;
        let package = temporary.path().join("package.zip");
        zip(
            &package,
            &[(
                "docker/config/init_mysql.sql",
                "USE agent_platform;\nCREATE TABLE users (id INT);\n",
            )],
        )?;
        let error =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)
                .err()
                .context(
                    "SQL overlapping a persistent alias must refuse before stopping services",
                )?;
        assert!(format!("{error:#}").contains("required release file"));
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join("config/init_mysql.sql"))?,
            "preserved data"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn selected_env_alias_inside_package_keeps_its_link_and_merges_target_atomically() -> Result<()>
    {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir()?;
        let root = temporary.path().join("docker");
        fs::create_dir_all(root.join("config"))?;
        fs::write(root.join("config/operator.env"), "A=operator\n")?;
        fs::write(root.join("config/obsolete"), "old")?;
        symlink("config/operator.env", root.join(".env"))?;
        let package = temporary.path().join("package.zip");
        zip(
            &package,
            &[
                ("docker/.env", "A=package\nNEW=added\n"),
                ("docker/config/new", "new"),
            ],
        )?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(
            fs::read_link(root.join(".env"))?,
            Path::new("config/operator.env")
        );
        assert_eq!(
            fs::read_to_string(root.join("config/operator.env"))?,
            "A=operator\nNEW=added\n"
        );
        assert!(!root.join("config/obsolete").exists());
        assert!(root.join("config/new").is_file());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires isolated root-owned fixture and a non-root process; run scripts/test-package-preservation-linux.sh"]
    fn native_root_owned_fixture() -> Result<()> {
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            nix::unistd::geteuid().as_raw(),
            0,
            "root cannot prove ordinary-user permissions"
        );
        let root = PathBuf::from(
            std::env::var("NUWAX_TEST_ROOT_OWNED_FIXTURE")
                .context("Isolated root-owned fixture path is required")?,
        );
        assert!(
            root.parent()
                .context("fixture parent")?
                .join(".nuwax-owned-permission-fixture")
                .is_file()
        );
        for protected in ["data", "project_workspace", "logs"] {
            let metadata = fs::metadata(root.join(protected))?;
            assert_eq!(metadata.uid(), 0);
            assert_eq!(metadata.mode() & 0o777, 0o755);
        }
        let before = identities(&root)?;
        let package = root.parent().context("fixture parent")?.join("package.zip");
        archive(&package)?;
        let mut plan =
            PackageReplacement::prepare(&package, &full()?, &root, &root.join(".env"), true)?;
        plan.apply()?;
        plan.finish()?;
        assert_eq!(identities(&root)?, before);
        assert!(!root.join("data/mysql/new-package-row").exists());
        // A root-owned unwritable MANAGED parent must refuse before stop/apply.
        let package = root
            .parent()
            .context("fixture parent")?
            .join("rejected.zip");
        zip(&package, &[("docker/unwritable-managed/file", "new")])?;
        let rejected_root = root
            .parent()
            .context("fixture parent")?
            .join("rejected/docker");
        assert!(
            PackageReplacement::prepare(
                &package,
                &full()?,
                &rejected_root,
                &rejected_root.join(".env"),
                true
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(rejected_root.join("docker-compose.yml"))?,
            "old compose"
        );
        assert_eq!(
            fs::read_to_string(root.join("docker-compose.yml"))?,
            "new compose"
        );
        assert_eq!(identities(&root)?, before);
        Ok(())
    }
}
