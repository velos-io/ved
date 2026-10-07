//! Cooperative resource budgets for trusted local specification loaders.
use crate::{ConfigSnapshot, Result, VedError};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLimitKind {
    SourceBytes,
    TotalSourceBytes,
    CatalogueSources,
    Ticks,
    HeapBytes,
    CallstackFrames,
    Elapsed,
}

#[derive(Debug, thiserror::Error)]
#[error("Specification resource limit {kind:?} exceeded for '{source_path}' (limit {limit})", source_path = source_path.display())]
pub struct ResourceLimitError {
    kind: ResourceLimitKind,
    source_path: PathBuf,
    limit: u64,
}
impl ResourceLimitError {
    pub fn new(kind: ResourceLimitKind, source: impl AsRef<Path>, limit: u64) -> Self {
        Self {
            kind,
            source_path: source.as_ref().to_path_buf(),
            limit,
        }
    }
    pub fn kind(&self) -> ResourceLimitKind {
        self.kind
    }
    pub fn source_path(&self) -> &Path {
        &self.source_path
    }
    pub fn limit(&self) -> u64 {
        self.limit
    }
}

/// Positive limits for trusted inputs; evaluator limits are best-effort, not an OS sandbox.
#[derive(Debug, Clone)]
pub struct ResourceBudget {
    pub max_source_bytes: usize,
    pub max_total_source_bytes: usize,
    pub max_catalogue_sources: usize,
    pub max_ticks: u64,
    pub max_heap_bytes: usize,
    pub max_callstack_frames: usize,
    pub max_elapsed: Duration,
}
impl Default for ResourceBudget {
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024,
            max_total_source_bytes: 16 * 1024 * 1024,
            max_catalogue_sources: 128,
            max_ticks: 1_000_000,
            max_heap_bytes: 64 * 1024 * 1024,
            max_callstack_frames: 256,
            max_elapsed: Duration::from_secs(30),
        }
    }
}
/// An immutable validated budget. A fresh accounting session is created for each load.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    budget: ResourceBudget,
}
impl LoadOptions {
    pub fn new(budget: ResourceBudget) -> Result<Self> {
        if budget.max_source_bytes == 0
            || budget.max_source_bytes == usize::MAX
            || budget.max_total_source_bytes == 0
            || budget.max_catalogue_sources == 0
            || budget.max_ticks == 0
            || budget.max_heap_bytes == 0
            || budget.max_callstack_frames == 0
            || budget.max_elapsed.is_zero()
            || Instant::now().checked_add(budget.max_elapsed).is_none()
        {
            return Err(VedError::config(
                "Resource budgets must be positive and representable",
            ));
        }
        Ok(Self { budget })
    }
    pub fn budget(&self) -> &ResourceBudget {
        &self.budget
    }
    pub fn from_config(config: &ConfigSnapshot) -> Result<Self> {
        let mut budget = ResourceBudget::default();
        fn number(config: &ConfigSnapshot, key: &str, default: u64) -> Result<u64> {
            match config.get_prop_opt(key) {
                None => Ok(default),
                Some(value) => {
                    value.parse::<u64>().ok().filter(|v| *v > 0).ok_or_else(|| {
                        VedError::config(format!("{key} must be a positive integer"))
                    })
                }
            }
        }
        fn size(config: &ConfigSnapshot, key: &str, default: usize) -> Result<usize> {
            usize::try_from(number(config, key, default as u64)?)
                .map_err(|_| VedError::config(format!("{key} exceeds platform size")))
        }
        budget.max_source_bytes = size(config, "VED_MAX_SOURCE_BYTES", budget.max_source_bytes)?;
        budget.max_total_source_bytes = size(
            config,
            "VED_MAX_TOTAL_SOURCE_BYTES",
            budget.max_total_source_bytes,
        )?;
        budget.max_catalogue_sources = size(
            config,
            "VED_MAX_CATALOGUE_SOURCES",
            budget.max_catalogue_sources,
        )?;
        budget.max_ticks = number(config, "VED_MAX_TICKS", budget.max_ticks)?;
        budget.max_heap_bytes = size(config, "VED_MAX_HEAP_BYTES", budget.max_heap_bytes)?;
        budget.max_callstack_frames = size(
            config,
            "VED_MAX_CALLSTACK_FRAMES",
            budget.max_callstack_frames,
        )?;
        budget.max_elapsed = Duration::from_millis(number(
            config,
            "VED_MAX_ELAPSED_MILLIS",
            budget.max_elapsed.as_millis() as u64,
        )?);
        Self::new(budget)
    }
}

pub(crate) struct LoadSession {
    budget: ResourceBudget,
    total_bytes: usize,
    catalogues: usize,
    ticks: u64,
    deadline: Instant,
}
impl LoadSession {
    pub(crate) fn new(options: &LoadOptions) -> Result<Self> {
        Ok(Self {
            budget: options.budget.clone(),
            total_bytes: 0,
            catalogues: 0,
            ticks: 0,
            deadline: Instant::now()
                .checked_add(options.budget.max_elapsed)
                .ok_or_else(|| VedError::config("Elapsed budget exceeds clock range"))?,
        })
    }
    pub(crate) fn budget(&self) -> &ResourceBudget {
        &self.budget
    }
    pub(crate) fn deadline(&self) -> Instant {
        self.deadline
    }
    pub(crate) fn source_limit(&self) -> usize {
        self.budget.max_source_bytes
    }
    pub(crate) fn remaining_ticks(&self) -> u64 {
        self.budget.max_ticks.saturating_sub(self.ticks)
    }
    #[cfg(test)]
    pub(crate) fn ticks_used(&self) -> u64 {
        self.ticks
    }
    pub(crate) fn elapsed_limit_error(&self, path: &Path) -> VedError {
        ResourceLimitError::new(
            ResourceLimitKind::Elapsed,
            path,
            self.budget
                .max_elapsed
                .as_millis()
                .min(u64::MAX as u128)
                .max(1) as u64,
        )
        .into()
    }
    pub(crate) fn check_elapsed(&self, path: &Path) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(self.elapsed_limit_error(path))
        } else {
            Ok(())
        }
    }
    pub(crate) fn check_representation(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.check_elapsed(path)?;
        if bytes.len() > self.source_limit() {
            return Err(ResourceLimitError::new(
                ResourceLimitKind::SourceBytes,
                path,
                self.source_limit() as u64,
            )
            .into());
        }
        Ok(())
    }
    pub(crate) fn check_source(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.check_representation(path, bytes)?;
        self.total_bytes = self
            .total_bytes
            .checked_add(bytes.len())
            .filter(|n| *n <= self.budget.max_total_source_bytes)
            .ok_or_else(|| {
                VedError::from(ResourceLimitError::new(
                    ResourceLimitKind::TotalSourceBytes,
                    path,
                    self.budget.max_total_source_bytes as u64,
                ))
            })?;
        Ok(())
    }
    pub(crate) fn catalogue_source(&mut self, path: &Path) -> Result<()> {
        self.check_elapsed(path)?;
        self.catalogues = self
            .catalogues
            .checked_add(1)
            .filter(|n| *n <= self.budget.max_catalogue_sources)
            .ok_or_else(|| {
                VedError::from(ResourceLimitError::new(
                    ResourceLimitKind::CatalogueSources,
                    path,
                    self.budget.max_catalogue_sources as u64,
                ))
            })?;
        Ok(())
    }
    pub(crate) fn charge_ticks(&mut self, path: &Path, ticks: u64) -> Result<()> {
        self.ticks = self
            .ticks
            .checked_add(ticks)
            .filter(|n| *n <= self.budget.max_ticks)
            .ok_or_else(|| {
                VedError::from(ResourceLimitError::new(
                    ResourceLimitKind::Ticks,
                    path,
                    self.budget.max_ticks,
                ))
            })?;
        self.check_elapsed(path)
    }
    pub(crate) fn read_source(&mut self, path: &Path) -> Result<String> {
        self.check_elapsed(path)?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|file| {
                file.take(self.source_limit() as u64 + 1)
                    .read_to_end(&mut bytes)
            })
            .map_err(|e| VedError::spec_load(path, crate::SpecLoadPhase::Read, e))?;
        self.check_source(path, &bytes)?;
        String::from_utf8(bytes)
            .map_err(|e| VedError::spec_load(path, crate::SpecLoadPhase::Read, e))
    }
}
