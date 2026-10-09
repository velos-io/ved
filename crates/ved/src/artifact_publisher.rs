//! Transactional publication and freshness metadata for generated artifacts.
use crate::error::{CliError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const MANIFEST_VERSION: u32 = 1;
const CACHE_VERSION: u32 = 1;
const JOURNAL_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "artifact-manifest.json";
const CACHE_FILE: &str = "artifact-cache.json";
const JOURNAL_FILE: &str = "artifact-transaction.json";

#[derive(Debug, Clone)]
pub(crate) struct ScopeUpdate {
    /// Explicitly configured artifact root. Descendants are publisher-owned only via manifest.
    pub root: PathBuf,
    /// Relative paths beneath `root` and their rendered content.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Manifest {
    version: u32,
    scopes: BTreeMap<String, ManifestScope>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestScope {
    root: PathBuf,
    files: BTreeMap<PathBuf, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Cache {
    version: u32,
    targets: BTreeMap<String, CacheEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    fingerprint: String,
    outputs: Vec<ExpectedOutput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
struct ExpectedOutput {
    path: PathBuf,
    digest: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    version: u32,
    transaction_dir: PathBuf,
    entries: Vec<JournalEntry>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalEntry {
    target: PathBuf,
    backup: Option<PathBuf>,
    mode: Option<u32>,
}

/// A per-workspace lock, durable manifest, and rollback journal.
pub(crate) struct ArtifactPublisher {
    coordinator: PathBuf,
}
impl ArtifactPublisher {
    pub(crate) fn new(coordinator: impl Into<PathBuf>) -> Self {
        Self {
            coordinator: coordinator.into(),
        }
    }

    #[cfg(test)]
    pub(crate) fn publish(
        &self,
        updates: BTreeMap<String, ScopeUpdate>,
        cache_updates: BTreeMap<String, String>,
        trusted_roots: &BTreeMap<String, PathBuf>,
    ) -> Result<()> {
        let lease = self.lease(trusted_roots)?;
        let result = lease.publish(updates, cache_updates);
        drop(lease);
        result
    }

    pub(crate) fn lease(&self, trusted_roots: &BTreeMap<String, PathBuf>) -> Result<ArtifactLease> {
        if fs::symlink_metadata(&self.coordinator).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(CliError::validation_failed(
                &self.coordinator,
                "symlink publisher root rejected",
            ));
        }
        fs::create_dir_all(&self.coordinator)?;
        if fs::symlink_metadata(&self.coordinator)?
            .file_type()
            .is_symlink()
        {
            return Err(CliError::validation_failed(
                &self.coordinator,
                "symlink publisher root rejected",
            ));
        }
        let _lock = WorkspaceLock::acquire(&self.coordinator.join("artifact-publish.lock"))?;
        self.recover_locked(trusted_roots)?;
        Ok(ArtifactLease {
            publisher: ArtifactPublisher {
                coordinator: self.coordinator.clone(),
            },
            trusted_roots: trusted_roots.clone(),
            _lock,
        })
    }

    fn publish_locked(
        &self,
        updates: BTreeMap<String, ScopeUpdate>,
        cache_updates: BTreeMap<String, String>,
        trusted_roots: &BTreeMap<String, PathBuf>,
    ) -> Result<()> {
        let manifest_path = self.coordinator.join(MANIFEST_FILE);
        let manifest_was_present = fs::symlink_metadata(&manifest_path)
            .is_ok_and(|metadata| metadata.file_type().is_file());
        let metadata_was_corrupt = fs::read(&manifest_path)
            .ok()
            .is_some_and(|bytes| serde_json::from_slice::<Manifest>(&bytes).is_err());
        let cache_path = self.coordinator.join(CACHE_FILE);
        let malformed_existing_manifest = manifest_was_present && metadata_was_corrupt;
        let reject_unmanaged = !malformed_existing_manifest;
        let mut manifest = read_manifest(&manifest_path)?;
        validate_manifest_paths(&manifest)?;
        manifest.scopes.retain(|name, scope| {
            trusted_roots
                .get(name)
                .is_some_and(|root| root == &scope.root)
        });
        let mut cache = read_cache(&cache_path)?;
        let mut changes = BTreeMap::<PathBuf, Option<Vec<u8>>>::new();

        for (name, update) in &updates {
            if trusted_roots.get(name) != Some(&update.root) {
                return Err(CliError::validation_failed(
                    &update.root,
                    format!("artifact scope '{name}' differs from configured root"),
                ));
            }
            fs::create_dir_all(&update.root)?;
            if fs::symlink_metadata(&update.root)?.file_type().is_symlink() {
                return Err(CliError::validation_failed(
                    &update.root,
                    "symlink artifact root rejected",
                ));
            }
            validate_relative_scope_files(&update.root, &update.files)?;
            if let Some(previous) = manifest.scopes.get(name) {
                if previous.root == update.root {
                    for rel in previous.files.keys() {
                        validate_relative(rel)?;
                        reject_symlink_below_root(&update.root, rel)?;
                        if !update.files.contains_key(rel) {
                            changes.insert(update.root.join(rel), None);
                        }
                    }
                }
            }
            let mut files = BTreeMap::new();
            for (relative, bytes) in &update.files {
                let destination = update.root.join(relative);
                let previous_owner = manifest.scopes.get(name).is_some_and(|scope| {
                    scope.root == update.root && scope.files.contains_key(relative)
                });
                if !previous_owner
                    && reject_unmanaged
                    && fs::symlink_metadata(&destination)
                        .is_ok_and(|meta| meta.file_type().is_file())
                    && fs::read(&destination)? != *bytes
                {
                    return Err(CliError::validation_failed(&destination, "refusing to replace an unmanaged file; remove it or move it before compiling"));
                }
                files.insert(relative.clone(), digest(bytes));
                changes.insert(destination.clone(), Some(bytes.clone()));
            }
            manifest.scopes.insert(
                name.clone(),
                ManifestScope {
                    root: update.root.clone(),
                    files,
                },
            );
        }

        // Manifest and cache metadata are committed in the same recoverable transaction.
        for target in cache.targets.keys().cloned().collect::<Vec<_>>() {
            let changed = cache.targets.get(&target).is_some_and(|entry| {
                entry
                    .outputs
                    .iter()
                    .any(|output| match changes.get(&output.path) {
                        Some(Some(bytes)) => digest(bytes) != output.digest,
                        Some(None) => true,
                        None => false,
                    })
            });
            if changed {
                cache.targets.remove(&target);
            }
        }
        for (target, fingerprint) in cache_updates {
            let mut outputs = expected_outputs_for_target(&manifest, &target);
            outputs.retain(|output| !matches!(changes.get(&output.path), Some(None)));
            for output in &mut outputs {
                if let Some(Some(bytes)) = changes.get(&output.path) {
                    output.digest = digest(bytes);
                }
            }
            outputs.sort();
            cache.targets.insert(
                target,
                CacheEntry {
                    fingerprint,
                    outputs,
                },
            );
        }
        manifest.version = MANIFEST_VERSION;
        cache.version = CACHE_VERSION;
        changes.insert(
            manifest_path.clone(),
            Some(serde_json::to_vec_pretty(&manifest).map_err(json_error)?),
        );
        changes.insert(
            cache_path.clone(),
            Some(serde_json::to_vec_pretty(&cache).map_err(json_error)?),
        );

        self.apply_locked(changes, trusted_roots)
    }

    pub(crate) fn is_fresh(
        &self,
        target: &str,
        fingerprint: &str,
        trusted_roots: &BTreeMap<String, PathBuf>,
    ) -> bool {
        self.lease(trusted_roots)
            .is_ok_and(|lease| lease.is_fresh(target, fingerprint))
    }

    fn is_fresh_locked(
        &self,
        target: &str,
        fingerprint: &str,
        trusted_roots: &BTreeMap<String, PathBuf>,
    ) -> bool {
        let cache = match read_cache(&self.coordinator.join(CACHE_FILE)) {
            Ok(cache) if cache.version == CACHE_VERSION => cache,
            _ => return false,
        };
        let manifest = match read_manifest(&self.coordinator.join(MANIFEST_FILE)) {
            Ok(manifest) if manifest.version == MANIFEST_VERSION => manifest,
            _ => return false,
        };
        if validate_manifest_paths(&manifest).is_err() {
            return false;
        }
        if manifest
            .scopes
            .iter()
            .any(|(name, scope)| trusted_roots.get(name) != Some(&scope.root))
        {
            return false;
        }
        let Some(entry) = cache.targets.get(target) else {
            return false;
        };
        if entry.fingerprint != fingerprint || entry.outputs.is_empty() {
            return false;
        }
        let manifest_outputs = expected_outputs_for_target(&manifest, target);
        let expected = entry
            .outputs
            .iter()
            .map(|output| output.path.clone())
            .collect::<BTreeSet<_>>();
        manifest_outputs
            .iter()
            .map(|output| output.path.clone())
            .collect::<BTreeSet<_>>()
            == expected
            && entry.outputs == manifest_outputs
            && entry.outputs.iter().all(|output| {
                target_scope_root(&manifest, target, &output.path).is_some_and(
                    |(root, relative)| {
                        if reject_symlink_below_root(root, &relative).is_err() {
                            return false;
                        }
                        fs::symlink_metadata(&output.path)
                            .is_ok_and(|meta| meta.file_type().is_file())
                            && fs::read(&output.path)
                                .is_ok_and(|bytes| digest(&bytes) == output.digest)
                    },
                )
            })
    }

    fn recover_locked(&self, trusted_roots: &BTreeMap<String, PathBuf>) -> Result<()> {
        let journal_path = self.coordinator.join(JOURNAL_FILE);
        if fs::symlink_metadata(&journal_path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(CliError::validation_failed(
                &journal_path,
                "symlink transaction journal rejected",
            ));
        }
        let bytes = match fs::read(&journal_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes).map_err(json_error)?;
        if journal.version != JOURNAL_VERSION {
            return Err(CliError::validation_failed(
                &journal_path,
                "unsupported transaction journal version",
            ));
        }
        if !journal.transaction_dir.starts_with(&self.coordinator)
            || journal.transaction_dir.parent() != Some(self.coordinator.as_path())
            || !journal
                .transaction_dir
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".artifact-txn-"))
        {
            return Err(CliError::validation_failed(
                &journal.transaction_dir,
                "unsafe transaction directory",
            ));
        }
        for entry in &journal.entries {
            validate_journal_entry(entry, &self.coordinator, trusted_roots)?;
            if let Some(backup) = &entry.backup {
                if !fs::symlink_metadata(backup).is_ok_and(|meta| meta.file_type().is_file()) {
                    return Err(CliError::validation_failed(
                        backup,
                        "transaction backup is missing or unsafe",
                    ));
                }
            }
        }
        for entry in journal.entries.iter().rev() {
            match &entry.backup {
                Some(backup) => {
                    let bytes = fs::read(backup)?;
                    write_atomic(&entry.target, &bytes, entry.mode)?;
                }
                None => remove_file_if_present(&entry.target)?,
            }
        }
        fs::remove_dir_all(&journal.transaction_dir)?;
        remove_file_if_present(&journal_path)?;
        if let Some(parent) = journal_path.parent() {
            sync_dir(parent)?;
        }
        Ok(())
    }

    fn apply_locked(
        &self,
        changes: BTreeMap<PathBuf, Option<Vec<u8>>>,
        trusted_roots: &BTreeMap<String, PathBuf>,
    ) -> Result<()> {
        static NEXT_TXN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let txn_dir = self.coordinator.join(format!(
            ".artifact-txn-{}-{}",
            std::process::id(),
            NEXT_TXN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&txn_dir)?;
        let mut entries = Vec::with_capacity(changes.len());
        for (index, (target, new_content)) in changes.iter().enumerate() {
            validate_target(target)?;
            validate_trusted_target(target, &self.coordinator, trusted_roots)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if fs::symlink_metadata(target).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(CliError::validation_failed(
                    target,
                    "symlink artifact destination rejected",
                ));
            }
            let old = match fs::symlink_metadata(target) {
                Ok(meta) => {
                    if !meta.file_type().is_file() {
                        return Err(CliError::validation_failed(
                            target,
                            "artifact destination is not a regular file",
                        ));
                    }
                    let data = fs::read(target)?;
                    let backup = txn_dir.join(format!("backup-{index}"));
                    write_new_file(&backup, &data)?;
                    Some((backup, metadata_mode(&meta)))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let (backup, mode) = old.map(|(b, m)| (Some(b), m)).unwrap_or((None, None));
            entries.push(JournalEntry {
                target: target.clone(),
                backup,
                mode,
            });
            if let Some(bytes) = new_content {
                let staged = txn_dir.join(format!("staged-{index}"));
                write_new_file(&staged, bytes)?;
                if let Some(parent) = target.parent() {
                    sync_dir(parent)?;
                }
            }
        }
        sync_dir(&txn_dir)?;
        let journal = Journal {
            version: JOURNAL_VERSION,
            transaction_dir: txn_dir.clone(),
            entries,
        };
        write_atomic(
            &self.coordinator.join(JOURNAL_FILE),
            &serde_json::to_vec(&journal).map_err(json_error)?,
            None,
        )?;
        // The journal must be durable before any output is replaced or removed.
        sync_dir(&self.coordinator)?;

        let result = (|| -> Result<()> {
            for (index, (target, new_content)) in changes.iter().enumerate() {
                match new_content {
                    Some(_) => {
                        let staged = txn_dir.join(format!("staged-{index}"));
                        let sibling = target
                            .with_extension(format!("ved-publish-{}-{index}", std::process::id()));
                        let mut source = File::open(staged)?;
                        let mut data = Vec::new();
                        use std::io::Read;
                        source.read_to_end(&mut data)?;
                        write_new_file(&sibling, &data)?;
                        fs::rename(&sibling, target)?;
                    }
                    None => remove_file_if_present(target)?,
                }
                if let Some(parent) = target.parent() {
                    sync_dir(parent)?;
                }
                test_failpoint(index)?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            let recovery = self.recover_locked(trusted_roots);
            let _ = fs::remove_dir_all(&txn_dir);
            return match recovery {
                Ok(()) => Err(error),
                Err(rollback) => Err(CliError::validation_failed(
                    &self.coordinator,
                    format!("publish failed ({error}); rollback failed ({rollback})"),
                )),
            };
        }
        remove_file_if_present(&self.coordinator.join(JOURNAL_FILE))?;
        sync_dir(&self.coordinator)?;
        fs::remove_dir_all(txn_dir)?;
        Ok(())
    }
}

/// Holds the workspace lock across freshness checks, rendering, and publication.
pub(crate) struct ArtifactLease {
    publisher: ArtifactPublisher,
    trusted_roots: BTreeMap<String, PathBuf>,
    _lock: WorkspaceLock,
}
impl ArtifactLease {
    pub(crate) fn is_fresh(&self, target: &str, fingerprint: &str) -> bool {
        self.publisher
            .is_fresh_locked(target, fingerprint, &self.trusted_roots)
    }
    pub(crate) fn publish(
        &self,
        updates: BTreeMap<String, ScopeUpdate>,
        cache_updates: BTreeMap<String, String>,
    ) -> Result<()> {
        self.publisher
            .publish_locked(updates, cache_updates, &self.trusted_roots)
    }
}

fn expected_outputs_for_target(manifest: &Manifest, target: &str) -> Vec<ExpectedOutput> {
    let scopes: &[&str] = match target {
        "compose" => &["compose", "generated-env", "stack-assets"],
        "k8s" => &["k8s", "generated-env", "stack-assets"],
        "helm" => &["helm", "stack-assets"],
        "all" => &["compose", "k8s", "helm", "generated-env", "stack-assets"],
        _ => &[],
    };
    let mut outputs = scopes
        .iter()
        .filter_map(|scope| manifest.scopes.get(*scope))
        .flat_map(|scope| {
            scope.files.iter().map(|(relative, digest)| ExpectedOutput {
                path: scope.root.join(relative),
                digest: digest.clone(),
            })
        })
        .collect::<Vec<_>>();
    outputs.sort();
    outputs.dedup_by(|a, b| a.path == b.path);
    outputs
}
fn target_scope_root<'a>(
    manifest: &'a Manifest,
    target: &str,
    path: &Path,
) -> Option<(&'a Path, PathBuf)> {
    let scopes: &[&str] = match target {
        "compose" => &["compose", "generated-env", "stack-assets"],
        "k8s" => &["k8s", "generated-env", "stack-assets"],
        "helm" => &["helm", "stack-assets"],
        "all" => &["compose", "k8s", "helm", "generated-env", "stack-assets"],
        _ => &[],
    };
    scopes
        .iter()
        .filter_map(|scope| manifest.scopes.get(*scope))
        .find_map(|scope| {
            path.strip_prefix(&scope.root)
                .ok()
                .map(|relative| (scope.root.as_path(), relative.to_path_buf()))
        })
}
fn read_manifest(path: &Path) -> Result<Manifest> {
    if fs::symlink_metadata(path).is_ok_and(|meta| !meta.file_type().is_file()) {
        return Err(CliError::validation_failed(
            path,
            "artifact manifest is not a regular file",
        ));
    }
    match fs::read(path) {
        Ok(b) => match serde_json::from_slice::<Manifest>(&b) {
            Ok(manifest) if manifest.version == MANIFEST_VERSION => Ok(manifest),
            // Treat unknown or malformed ownership metadata as empty. No old file is
            // then eligible for deletion; requested paths are adopted only if absent
            // or already contain the exact rendered bytes.
            Ok(_) | Err(_) => Ok(Manifest {
                version: MANIFEST_VERSION,
                ..Default::default()
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest {
            version: MANIFEST_VERSION,
            ..Default::default()
        }),
        Err(e) => Err(e.into()),
    }
}
fn read_cache(path: &Path) -> Result<Cache> {
    if fs::symlink_metadata(path).is_ok_and(|meta| !meta.file_type().is_file()) {
        return Err(CliError::validation_failed(
            path,
            "artifact cache is not a regular file",
        ));
    }
    match fs::read(path) {
        Ok(b) => match serde_json::from_slice::<Cache>(&b) {
            Ok(cache) if cache.version == CACHE_VERSION => Ok(cache),
            Ok(_) | Err(_) => Ok(Cache {
                version: CACHE_VERSION,
                ..Default::default()
            }),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Cache {
            version: CACHE_VERSION,
            ..Default::default()
        }),
        Err(e) => Err(e.into()),
    }
}
fn validate_relative(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(CliError::validation_failed(
            path,
            "unsafe artifact manifest path",
        ));
    }
    Ok(())
}
fn validate_relative_scope_files(root: &Path, files: &BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
    for path in files.keys() {
        validate_relative(path)?;
        reject_symlink_below_root(root, path)?;
    }
    if root.as_os_str().is_empty() {
        return Err(CliError::validation_failed(root, "empty artifact root"));
    }
    Ok(())
}
fn validate_manifest_paths(manifest: &Manifest) -> Result<()> {
    if manifest.version != MANIFEST_VERSION {
        return Err(CliError::validation_failed(
            "artifact manifest",
            "unsupported manifest version",
        ));
    }
    for (name, scope) in &manifest.scopes {
        if !matches!(
            name.as_str(),
            "compose" | "k8s" | "helm" | "generated-env" | "stack-assets"
        ) || !scope.root.is_absolute()
        {
            return Err(CliError::validation_failed(
                &scope.root,
                "invalid artifact manifest scope",
            ));
        }
        for relative in scope.files.keys() {
            validate_relative(relative)?;
        }
        if name == "stack-assets"
            && scope.files.keys().any(|path| {
                path.components().next() != Some(Component::Normal(std::ffi::OsStr::new("stacks")))
            })
        {
            return Err(CliError::validation_failed(
                &scope.root,
                "stack asset manifest path is outside the stacks directory",
            ));
        }
    }
    Ok(())
}
fn validate_trusted_target(
    path: &Path,
    coordinator: &Path,
    trusted_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    if path == coordinator.join(MANIFEST_FILE) || path == coordinator.join(CACHE_FILE) {
        if fs::symlink_metadata(coordinator).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(CliError::validation_failed(
                coordinator,
                "symlink publisher root rejected",
            ));
        }
        return Ok(());
    }
    for (name, root) in trusted_roots {
        if let Ok(relative) = path.strip_prefix(root) {
            if name == "stack-assets"
                && relative.components().next()
                    != Some(Component::Normal(std::ffi::OsStr::new("stacks")))
            {
                continue;
            }
            if relative.as_os_str().is_empty() {
                return Err(CliError::validation_failed(
                    path,
                    "artifact target must be a file below its scope",
                ));
            }
            reject_symlink_below_root(root, relative)?;
            return Ok(());
        }
    }
    Err(CliError::validation_failed(
        path,
        "transaction journal target is outside configured artifact scopes",
    ))
}
fn validate_journal_entry(
    entry: &JournalEntry,
    coordinator: &Path,
    trusted_roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    validate_target(&entry.target)?;
    validate_trusted_target(&entry.target, coordinator, trusted_roots)?;
    if let Some(backup) = &entry.backup {
        let parent = backup.parent().ok_or_else(|| {
            CliError::validation_failed(backup, "invalid transaction backup path")
        })?;
        if parent.parent() != Some(coordinator)
            || !parent
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(".artifact-txn-"))
            || backup.parent() != Some(parent)
            || backup.file_name().is_none()
        {
            return Err(CliError::validation_failed(
                backup,
                "transaction backup is outside publisher staging",
            ));
        }
    }
    Ok(())
}
fn validate_target(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(CliError::validation_failed(
            path,
            "artifact destination must be absolute",
        ));
    }
    Ok(())
}
fn reject_symlink_below_root(root: &Path, relative: &Path) -> Result<()> {
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(CliError::validation_failed(
            root,
            "symlink artifact root rejected",
        ));
    }
    let mut current = root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for component in components.iter().take(components.len().saturating_sub(1)) {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(CliError::validation_failed(
                    &current,
                    "symlink artifact path rejected",
                ))
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
fn write_atomic(path: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
        return Err(CliError::validation_failed(
            path,
            "symlink artifact path rejected",
        ));
    }
    let temp = path.with_extension(format!(
        "ved-recover-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    file.write_all(data)?;
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
    }
    file.sync_all()?;
    fs::rename(temp, path)?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}
fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
fn json_error(error: serde_json::Error) -> CliError {
    CliError::validation_failed("artifact metadata", error)
}
static NEXT_TEMP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
#[cfg(test)]
thread_local! { static FAIL_AFTER_MUTATIONS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) }; }
#[cfg(test)]
fn test_failpoint(index: usize) -> Result<()> {
    FAIL_AFTER_MUTATIONS.with(|setting| {
        if setting.get() == Some(index) {
            setting.set(None);
            Err(CliError::Io(std::io::Error::other(
                "injected publisher failure",
            )))
        } else {
            Ok(())
        }
    })
}
#[cfg(not(test))]
fn test_failpoint(_index: usize) -> Result<()> {
    Ok(())
}
#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    Some(metadata.mode() & 0o7777)
}
#[cfg(not(unix))]
fn metadata_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

struct WorkspaceLock(File);
impl WorkspaceLock {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(CliError::validation_failed(
                path,
                "symlink publisher lock rejected",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        #[cfg(unix)]
        {
            rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
                .map_err(std::io::Error::from)?;
        }
        #[cfg(not(unix))]
        {
            return Err(CliError::validation_failed(
                path,
                "transactional artifact locking is unsupported on this platform",
            ));
        }
        Ok(Self(file))
    }
}
impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    fn temp() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ved-publisher-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn scope(root: &Path, name: &str, files: &[(&str, &[u8])]) -> (String, ScopeUpdate) {
        let mut map = BTreeMap::new();
        for (path, bytes) in files {
            map.insert(PathBuf::from(path), bytes.to_vec());
        }
        (
            name.to_string(),
            ScopeUpdate {
                root: root.to_path_buf(),
                files: map,
            },
        )
    }
    #[test]
    fn publishes_and_cleans_only_manifest_owned_stale_files() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("user.txt"), b"keep").unwrap();
        let publisher = ArtifactPublisher::new(root.join(".ved"));
        let roots = BTreeMap::from([("k8s".into(), out.clone())]);
        let update = scope(
            &out,
            "k8s",
            &[("base/a.yaml", b"a"), ("base/old.yaml", b"old")],
        );
        publisher
            .publish(
                BTreeMap::from([update]),
                BTreeMap::from([("k8s".into(), "fp1".into())]),
                &roots,
            )
            .unwrap();
        let update = scope(&out, "k8s", &[("base/a.yaml", b"new")]);
        publisher
            .publish(
                BTreeMap::from([update]),
                BTreeMap::from([("k8s".into(), "fp2".into())]),
                &roots,
            )
            .unwrap();
        assert_eq!(fs::read(out.join("base/a.yaml")).unwrap(), b"new");
        assert!(!out.join("base/old.yaml").exists());
        assert_eq!(fs::read(out.join("user.txt")).unwrap(), b"keep");
        let roots = BTreeMap::from([("k8s".into(), out.clone())]);
        assert!(publisher.is_fresh("k8s", "fp2", &roots));
        assert!(!publisher.is_fresh("k8s", "fp1", &roots));
    }
    #[test]
    fn read_only_unmanaged_files_survive_and_unsafe_manifest_path_fails_closed() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let publisher = ArtifactPublisher::new(root.join(".ved"));
        let update = scope(&out, "compose", &[("compose.yaml", b"ok")]);
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        publisher
            .publish(
                BTreeMap::from([update]),
                BTreeMap::from([("compose".into(), "fp".into())]),
                &roots,
            )
            .unwrap();
        let manifest_path = root.join(".ved").join(MANIFEST_FILE);
        let mut manifest = read_manifest(&manifest_path).unwrap();
        manifest
            .scopes
            .get_mut("compose")
            .unwrap()
            .files
            .insert(PathBuf::from("../outside"), "hash".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let update = scope(&out, "compose", &[("compose.yaml", b"next")]);
        assert!(publisher
            .publish(BTreeMap::from([update]), BTreeMap::new(), &roots)
            .is_err());
    }

    #[test]
    fn publisher_refuses_to_overwrite_an_unmanaged_destination() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let unmanaged = out.join("compose.yaml");
        fs::write(&unmanaged, b"user data").unwrap();
        let publisher = ArtifactPublisher::new(root.join(".ved"));
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        let attempt = publisher.publish(
            BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"generated")])]),
            BTreeMap::new(),
            &roots,
        );
        assert!(attempt.is_err());
        assert_eq!(fs::read(unmanaged).unwrap(), b"user data");
    }

    #[test]
    fn malformed_metadata_is_stale_and_replaced_on_successful_publish() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let publisher = ArtifactPublisher::new(root.join(".ved"));
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        publisher
            .publish(
                BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"old")])]),
                BTreeMap::from([("compose".into(), "fp1".into())]),
                &roots,
            )
            .unwrap();
        fs::write(root.join(".ved").join(MANIFEST_FILE), b"corrupt").unwrap();
        fs::write(root.join(".ved").join(CACHE_FILE), b"corrupt").unwrap();
        assert!(!publisher.is_fresh("compose", "fp1", &roots));
        publisher
            .publish(
                BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"new")])]),
                BTreeMap::from([("compose".into(), "fp2".into())]),
                &roots,
            )
            .unwrap();
        assert_eq!(fs::read(out.join("compose.yaml")).unwrap(), b"new");
        assert!(publisher.is_fresh("compose", "fp2", &roots));
    }

    #[test]
    fn injected_mid_publish_failure_restores_the_previous_bundle() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let publisher = ArtifactPublisher::new(root.join(".ved"));
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        publisher
            .publish(
                BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"old")])]),
                BTreeMap::from([("compose".into(), "old-fp".into())]),
                &roots,
            )
            .unwrap();
        FAIL_AFTER_MUTATIONS.with(|setting| setting.set(Some(2)));
        assert!(publisher
            .publish(
                BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"new")])]),
                BTreeMap::from([("compose".into(), "new-fp".into())]),
                &roots
            )
            .is_err());
        assert_eq!(fs::read(out.join("compose.yaml")).unwrap(), b"old");
        assert!(publisher.is_fresh("compose", "old-fp", &roots));
    }

    #[test]
    fn restart_recovers_a_journal_before_reporting_freshness() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let coordinator = root.join(".ved");
        let publisher = ArtifactPublisher::new(&coordinator);
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        publisher
            .publish(
                BTreeMap::from([scope(&out, "compose", &[("compose.yaml", b"committed")])]),
                BTreeMap::from([("compose".into(), "fp".into())]),
                &roots,
            )
            .unwrap();
        let txn_dir = coordinator.join(".artifact-txn-recovery-test");
        fs::create_dir_all(&txn_dir).unwrap();
        let backup = txn_dir.join("backup-0");
        fs::write(&backup, b"committed").unwrap();
        let target = out.join("compose.yaml");
        fs::write(&target, b"partial").unwrap();
        let journal = Journal {
            version: JOURNAL_VERSION,
            transaction_dir: txn_dir.clone(),
            entries: vec![JournalEntry {
                target: target.clone(),
                backup: Some(backup),
                mode: None,
            }],
        };
        fs::write(
            coordinator.join(JOURNAL_FILE),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        assert!(publisher.is_fresh("compose", "fp", &roots));
        assert_eq!(fs::read(target).unwrap(), b"committed");
        assert!(!txn_dir.exists());
    }

    #[test]
    fn concurrent_publishers_serialize_and_leave_one_complete_bundle() {
        let root = temp();
        let out = root.join("out");
        fs::create_dir_all(&out).unwrap();
        let coordinator = root.join(".ved");
        let roots = BTreeMap::from([("compose".into(), out.clone())]);
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for (label, content) in [("a", vec![b'A'; 8192]), ("b", vec![b'B'; 8192])] {
            let publisher = ArtifactPublisher::new(&coordinator);
            let roots = roots.clone();
            let out = out.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                publisher
                    .publish(
                        BTreeMap::from([scope(
                            &out,
                            "compose",
                            &[("compose.yaml", content.as_slice())],
                        )]),
                        BTreeMap::from([("compose".into(), label.into())]),
                        &roots,
                    )
                    .unwrap();
            }));
        }
        barrier.wait();
        for handle in handles {
            handle.join().unwrap();
        }
        let bytes = fs::read(out.join("compose.yaml")).unwrap();
        assert!(bytes == vec![b'A'; 8192] || bytes == vec![b'B'; 8192]);
        let publisher = ArtifactPublisher::new(coordinator);
        assert!(
            publisher.is_fresh("compose", "a", &roots)
                || publisher.is_fresh("compose", "b", &roots)
        );
    }
}
