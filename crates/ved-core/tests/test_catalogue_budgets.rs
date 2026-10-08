#![cfg(all(feature = "host", unix))]
mod support;
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use ved_core::{AppSpec, LoadOptions, ResourceBudget, ResourceLimitKind};
struct Fixture {
    root: PathBuf,
    context: ved_core::WorkspaceContext,
    source: Vec<u8>,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "ved-catalog-budget-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let catalog = root.join("catalog");
        let source = b"services = [Service(name='cache', image='redis')]\n".to_vec();
        for name in ["one", "two"] {
            fs::create_dir_all(catalog.join(name).join("config")).unwrap();
            fs::write(catalog.join(name).join("Vedfile"), &source).unwrap();
            fs::write(catalog.join(name).join("config/asset.txt"), b"asset").unwrap();
        }
        let context = support::context(
            &root,
            BTreeMap::from([("VED_STACKS_PATH".into(), catalog.display().to_string())]),
        );
        Self {
            root,
            context,
            source,
        }
    }
    fn apply(&self, budget: ResourceBudget) -> ved_core::Result<()> {
        let mut spec = AppSpec::default();
        ved_core::apply_stacks_with_context_and_options(
            &["one".into(), "two".into()],
            &mut spec,
            &self.context,
            &LoadOptions::new(budget)?,
        )
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
#[test]
fn catalogue_count_and_total_bytes_share_one_session_before_publication() {
    let fixture = Fixture::new();
    for (budget, kind) in [
        (
            ResourceBudget {
                max_catalogue_sources: 1,
                ..Default::default()
            },
            ResourceLimitKind::CatalogueSources,
        ),
        (
            ResourceBudget {
                max_total_source_bytes: fixture.source.len() * 2 - 1,
                ..Default::default()
            },
            ResourceLimitKind::TotalSourceBytes,
        ),
    ] {
        let error = fixture.apply(budget).unwrap_err();
        assert_eq!(error.resource_limit_error().unwrap().kind(), kind);
        assert!(
            !fixture.root.join(".ved").exists(),
            "failed load published catalogue assets"
        );
    }
    fixture
        .apply(ResourceBudget {
            max_total_source_bytes: fixture.source.len() * 2,
            ..Default::default()
        })
        .unwrap();
    assert!(fixture
        .root
        .join(".ved/stacks/one/config/asset.txt")
        .exists());
    assert!(fixture
        .root
        .join(".ved/stacks/two/config/asset.txt")
        .exists());
}
#[test]
fn catalogue_source_cap_is_typed_and_preserves_no_follow() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let error = fixture
        .apply(ResourceBudget {
            max_source_bytes: fixture.source.len() - 1,
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(
        error.resource_limit_error().unwrap().kind(),
        ResourceLimitKind::SourceBytes
    );
    assert!(!fixture.root.join(".ved").exists());
    fs::remove_file(fixture.root.join("catalog/one/Vedfile")).unwrap();
    let outside = fixture.root.join("outside.star");
    fs::write(&outside, &fixture.source).unwrap();
    symlink(&outside, fixture.root.join("catalog/one/Vedfile")).unwrap();
    assert!(fixture.apply(ResourceBudget::default()).is_err());
    assert!(!fixture.root.join(".ved").exists());
}
