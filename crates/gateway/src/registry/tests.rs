use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mcp_proxy_core::config::{Category, Config, EnvId, McpId, ProfileId, SessionId, ToolName};
use mcp_proxy_core::filter::{FilterError, OutputFilter, SECRET_MARKER};
use mcp_proxy_core::policy::CategorySource;
use mcp_proxy_core::secrets::{Credential, KeyWrapper, ProtectionFailure, SecretError, SecretStore};
use mcp_proxy_core::settings::UserSettings;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::TryRecvError;
use zeroize::Zeroizing;

use super::*;

const SENTINEL: &str = "Zq7pW3kLx9VbN2mR8tYc4HdF6gJs1AeU";

// --- Helpers ---------------------------------------------------------------------------------------------------------

#[derive(Clone, Default)]
struct BreakableWrapper(Arc<AtomicBool>);

impl KeyWrapper for BreakableWrapper {
    fn wrap(&self, key: &[u8]) -> Result<Vec<u8>, SecretError> {
        self.check()?;
        Ok(key.iter().map(|b| b ^ 0x5a).collect())
    }

    fn unwrap(&self, wrapped: &[u8]) -> Result<Zeroizing<Vec<u8>>, SecretError> {
        self.check()?;
        Ok(Zeroizing::new(wrapped.iter().map(|b| b ^ 0x5a).collect()))
    }
}

impl BreakableWrapper {
    fn check(&self) -> Result<(), SecretError> {
        if self.0.load(Ordering::SeqCst) {
            return Err(SecretError::Protection(ProtectionFailure::Os(-1)));
        }
        Ok(())
    }

    fn break_now(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct Filter {
    filter: Arc<OutputFilter>,
    store: Arc<SecretStore>,
    wrapper: BreakableWrapper,
}

fn filter() -> Filter {
    let wrapper = BreakableWrapper::default();
    let store = Arc::new(SecretStore::new(Box::new(wrapper.clone())).unwrap());
    let filter = Arc::new(OutputFilter::new(Arc::clone(&store), &config().filter).unwrap());
    Filter { filter, store, wrapper }
}

impl Filter {
    fn with_secret(self) -> Self {
        let token = SecretString::from(SENTINEL.to_owned());
        self.store
            .put(&SessionId::new("sso-corp").unwrap(), Credential::Bearer { token })
            .unwrap();
        self
    }
}

#[derive(Clone, Default)]
struct MemoryCache {
    entries: Arc<Mutex<BTreeMap<InstanceId, CachedCatalog>>>,
    stores: Arc<AtomicUsize>,
    fail_load: Arc<AtomicBool>,
    fail_store: Arc<AtomicBool>,
}

impl MemoryCache {
    fn with(self, instance: &InstanceId, tools: Vec<ToolDefinition>) -> Self {
        self.entries
            .lock()
            .unwrap()
            .insert(instance.clone(), CachedCatalog { tools });
        self
    }

    fn get(&self, instance: &InstanceId) -> Option<CachedCatalog> {
        self.entries.lock().unwrap().get(instance).cloned()
    }

    fn stores(&self) -> usize {
        self.stores.load(Ordering::SeqCst)
    }
}

impl CatalogCache for MemoryCache {
    fn load(&self, instance: &InstanceId) -> Result<Option<CachedCatalog>, CacheError> {
        if self.fail_load.load(Ordering::SeqCst) {
            return Err(CacheError::Io("accès refusé".to_owned()));
        }
        Ok(self.get(instance))
    }

    fn store(&self, instance: &InstanceId, catalog: &CachedCatalog) -> Result<(), CacheError> {
        self.stores.fetch_add(1, Ordering::SeqCst);
        if self.fail_store.load(Ordering::SeqCst) {
            return Err(CacheError::Save("disque plein".to_owned()));
        }
        self.entries.lock().unwrap().insert(instance.clone(), catalog.clone());
        Ok(())
    }
}

/// A fresh, empty directory under the system temporary directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("mcp-proxy-catalog-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config() -> Config {
    crate::modules::embedded_config().unwrap()
}

fn inst(env: &str, mcp: &str) -> InstanceId {
    InstanceId::new(EnvId::new(env).unwrap(), McpId::new(mcp).unwrap())
}

fn tn(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

fn registry_with(config: Config, cache: Box<dyn CatalogCache>, filter: &Filter) -> Registry {
    let embedded = EmbeddedCatalog::builtin(&config).unwrap();
    Registry::new(Arc::new(config), &embedded, cache, Arc::clone(&filter.filter)).unwrap()
}

fn registry() -> Registry {
    registry_with(config(), Box::new(MemoryCache::default()), &filter())
}

fn settings_for(profile: &str) -> UserSettings {
    let config = config();
    let mut settings = UserSettings::default();
    settings.choose_profile(&config.profiles[&ProfileId::new(profile).unwrap()]);
    settings
}

fn developer(registry: &mut Registry) {
    registry.apply_settings(&settings_for("developpeur"));
}

fn tool(name: &str, description: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        title: None,
        description: Some(description.to_owned()),
        input_schema: json!({ "type": "object", "properties": {} }),
        output_schema: None,
        annotations: None,
    }
}

/// The embedded snapshot of an instance, as a live list.
fn snapshot(instance: &InstanceId) -> Vec<ToolDefinition> {
    EmbeddedCatalog::builtin(&config())
        .unwrap()
        .for_instance(instance)
        .unwrap()
        .to_vec()
}

fn drain(rx: &mut tokio::sync::broadcast::Receiver<RegistryEvent>) -> Vec<RegistryEvent> {
    let mut events = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(TryRecvError::Empty) => return events,
            Err(e) => panic!("{e:?}"),
        }
    }
}

// --- EX-005-01: naming ----------------------------------------------------------------------------------------------

// Exigence : EX-005-01
#[test]
fn exposed_names_are_prefixed_and_mapped_back() {
    let registry = registry();
    let entry = registry.lookup("prod_kibana_search_logs").unwrap();
    assert_eq!(entry.instance, inst("prod", "kibana-logs"));
    assert_eq!(entry.original_name, tn("search_logs"));
    assert_eq!(
        registry.lookup("dev_k8s_delete_pod").unwrap().instance,
        inst("dev", "k8s-admin")
    );

    let mut names = std::collections::BTreeSet::new();
    for instance in registry.instances() {
        for entry in registry.tools(instance) {
            assert!(entry.exposed_name.len() <= 64);
            assert!(
                entry
                    .exposed_name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
            );
            assert!(names.insert(entry.exposed_name.clone()));
            let back = registry.lookup(&entry.exposed_name).unwrap();
            assert_eq!((&back.instance, &back.original_name), (instance, &entry.original_name));
        }
    }
}

// Exigence : EX-005-01
#[test]
fn server_names_are_sanitized_and_truncated_deterministically() {
    let long = format!("Very.Long_{}", "x".repeat(110));
    let list = || vec![tool("Search.Logs", "Recherche"), tool(&long, "Long")];
    let mut first = registry();
    first.apply_live_list(&inst("dev", "kibana-logs"), list()).unwrap();

    let entry = first.lookup("dev_kibana_search_logs").unwrap();
    assert_eq!(entry.original_name.as_str(), "Search.Logs");

    let long_entry = first
        .tools(&inst("dev", "kibana-logs"))
        .find(|e| e.original_name.as_str() == long)
        .unwrap();
    assert_eq!(long_entry.exposed_name.len(), 64);
    assert!(long_entry.exposed_name.starts_with("dev_kibana_very_long_xxx"));

    let mut second = registry();
    second.apply_live_list(&inst("dev", "kibana-logs"), list()).unwrap();
    let again = second.lookup(&long_entry.exposed_name).unwrap();
    assert_eq!(again.original_name.as_str(), long);
}

// Exigence : EX-005-01
#[test]
fn colliding_names_refuse_the_whole_list() {
    let mut registry = registry();
    let instance = inst("dev", "kibana-logs");
    let before = registry.instance_fingerprint(&instance).cloned();
    let error = registry
        .apply_live_list(&instance, vec![tool("Search.Logs", "a"), tool("search_logs", "b")])
        .unwrap_err();
    assert!(
        matches!(&error, RegistryError::Collision { exposed_name, .. } if exposed_name == "dev_kibana_search_logs")
    );
    assert!(error.to_string().contains("collision"));
    assert_eq!(registry.instance_fingerprint(&instance).cloned(), before);
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Embedded));
}

// Exigence : EX-005-01
#[test]
fn invalid_or_duplicate_names_refuse_the_whole_list() {
    let mut registry = registry();
    let instance = inst("dev", "kibana-logs");
    let error = registry
        .apply_live_list(&instance, vec![tool("ok", "a"), tool("a b", "b")])
        .unwrap_err();
    assert!(matches!(error, RegistryError::InvalidToolName { index: 1, .. }));
    let error = registry
        .apply_live_list(&instance, vec![tool("ok", "a"), tool("ok", "b")])
        .unwrap_err();
    assert!(matches!(error, RegistryError::DuplicateTool { .. }));
    let mut bad_schema = tool("ok", "a");
    bad_schema.input_schema = json!("string");
    assert!(matches!(
        registry.apply_live_list(&instance, vec![bad_schema]),
        Err(RegistryError::InvalidSchema { .. })
    ));
    assert!(matches!(
        registry.apply_live_list(&inst("qua", "gitlab-ops"), vec![tool("ok", "a")]),
        Err(RegistryError::UnknownInstance(_))
    ));
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Embedded));
}

// --- EX-005-02: title, description, annotations -----------------------------------------------------------------

// Exigence : EX-005-02
#[test]
fn title_and_description_name_the_environment() {
    let registry = registry();
    let entry = registry.lookup("prod_kibana_search_logs").unwrap();
    assert_eq!(entry.title, "PROD · Kibana Logs · search_logs");
    assert_eq!(
        entry.description,
        "[PROD — Production] Searches log entries matching a Lucene query over a time range."
    );
    let dev = registry.lookup("dev_kibana_search_logs").unwrap();
    assert!(dev.description.starts_with("[DEV — Développement] "));

    let definition = entry.exposed_definition();
    assert_eq!(definition.name, "prod_kibana_search_logs");
    assert_eq!(definition.title.as_deref(), Some(entry.title.as_str()));
    assert_eq!(definition.description.as_deref(), Some(entry.description.as_str()));
    assert_eq!(definition.input_schema, entry.input_schema);
}

// Exigence : EX-005-02
#[test]
fn description_without_server_text_is_the_environment_alone() {
    let mut registry = registry();
    let mut bare = tool("search_logs", "");
    bare.description = None;
    registry
        .apply_live_list(&inst("rec", "kibana-logs"), vec![bare])
        .unwrap();
    assert_eq!(
        registry.lookup("rec_kibana_search_logs").unwrap().description,
        "[REC — Recette]"
    );
}

fn hints(read_only: Option<bool>, destructive: Option<bool>) -> ToolHints {
    ToolHints {
        read_only_hint: read_only,
        destructive_hint: destructive,
        ..ToolHints::default()
    }
}

// Exigence : EX-005-02
#[test]
fn annotations_follow_the_retained_category() {
    let registry = registry();
    let check = |name: &str, category: Category, source: CategorySource, expected: ToolHints| {
        let entry = registry.lookup(name).unwrap();
        assert_eq!((entry.category, entry.category_source), (category, source), "{name}");
        assert_eq!(entry.annotations, expected, "{name}");
        assert_eq!(entry.exposed_definition().annotations, Some(expected), "{name}");
    };
    let read = hints(Some(true), None);
    let write = hints(Some(false), Some(false));
    let destructive = hints(Some(false), Some(true));
    check(
        "dev_kibana_search_logs",
        Category::Read,
        CategorySource::Config,
        read.clone(),
    );
    check(
        "dev_gitlab_retry_job",
        Category::Write,
        CategorySource::Config,
        write.clone(),
    );
    check(
        "dev_kibana_delete_index",
        Category::Destructive,
        CategorySource::Config,
        destructive,
    );
    // Declared without a category: the server annotations decide (S11).
    check(
        "dev_gitlab_get_merge_request",
        Category::Read,
        CategorySource::Annotation,
        read,
    );
    check(
        "dev_gitlab_add_merge_request_note",
        Category::Write,
        CategorySource::Annotation,
        write,
    );
    // The server's other hints are never relayed.
    let scale = registry.lookup("dev_k8s_scale_deployment").unwrap();
    assert_eq!(scale.server_annotations.as_ref().unwrap().idempotent_hint, Some(true));
    assert_eq!(scale.annotations.idempotent_hint, None);
}

// Exigence : EX-005-02
#[test]
fn server_annotations_never_override_the_configuration() {
    let mut registry = registry();
    let mut search = tool("search_logs", "x");
    search.annotations = Some(hints(Some(false), Some(true)));
    let mut delete = tool("delete_index", "x");
    delete.annotations = Some(hints(Some(true), None));
    let mut unknown = tool("drop_everything", "Harmless, really");
    unknown.annotations = Some(hints(Some(true), Some(false)));
    registry
        .apply_live_list(&inst("dev", "kibana-logs"), vec![search, delete, unknown])
        .unwrap();

    assert_eq!(
        registry.lookup("dev_kibana_search_logs").unwrap().annotations,
        hints(Some(true), None)
    );
    assert_eq!(
        registry.lookup("dev_kibana_delete_index").unwrap().annotations,
        hints(Some(false), Some(true))
    );
    let unknown = registry.lookup("dev_kibana_drop_everything").unwrap();
    assert!(!unknown.known);
    assert_eq!(unknown.category, Category::Destructive);
    assert_eq!(unknown.category_source, CategorySource::Unknown);
    assert_eq!(unknown.annotations, hints(Some(false), Some(true)));
}

// Exigence : EX-005-02
#[test]
fn declared_tool_without_category_nor_annotations_is_destructive() {
    let mut registry = registry();
    registry
        .apply_live_list(&inst("dev", "gitlab-ops"), vec![tool("get_merge_request", "x")])
        .unwrap();
    let entry = registry.lookup("dev_gitlab_get_merge_request").unwrap();
    assert!(entry.known);
    assert_eq!(
        (entry.category, entry.category_source),
        (Category::Destructive, CategorySource::Default)
    );
    assert_eq!(entry.annotations.destructive_hint, Some(true));
}

// --- EX-005-03: exposure ----------------------------------------------------------------------------------------

// Exigence : EX-005-03
#[test]
fn nothing_is_exposed_without_a_chosen_profile() {
    let mut registry = registry();
    assert_eq!(registry.exposed_count(), 0);
    assert!(registry.resolve("rec_kibana_search_logs").is_none());

    // Environments listed while no profile is chosen are not exposed either.
    let mut settings = UserSettings::default();
    settings.exposed_environments.insert(EnvId::new("rec").unwrap());
    registry.apply_settings(&settings);
    assert_eq!(registry.exposed_count(), 0);
    assert!(registry.exposed_tools().is_empty());

    // A chosen profile that no longer exists counts as none.
    let mut settings = settings_for("developpeur");
    settings.profile = Some(ProfileId::new("disparu").unwrap());
    registry.apply_settings(&settings);
    assert_eq!(registry.exposed_count(), 0);
    assert!(registry.exposure().profile().is_none());
}

// Exigence : EX-005-03
#[test]
fn exposure_follows_environments_and_disabled_mcps() {
    let mut registry = registry();
    developer(&mut registry);
    // DEV, INT and REC: 12 Kibana, 14 GitLab, 15 Kubernetes, 6 database, 3 Jira.
    assert_eq!(registry.exposed_count(), 50);
    assert!(registry.resolve("dev_kibana_search_logs").is_some());
    // Environment not exposed: the tool exists but does not resolve.
    assert!(registry.lookup("prod_kibana_search_logs").is_some());
    assert!(registry.resolve("prod_kibana_search_logs").is_none());
    // No instance of GitLab in QUA, even once QUA is exposed.
    let mut settings = settings_for("developpeur");
    settings.exposed_environments.insert(EnvId::new("qua").unwrap());
    registry.apply_settings(&settings);
    assert!(registry.resolve("qua_kibana_search_logs").is_some());
    assert!(registry.lookup("qua_gitlab_list_pipelines").is_none());
    assert!(registry.resolve("qua_gitlab_list_pipelines").is_none());

    // Disabled MCP: none of its tools, in any environment.
    settings.disabled_mcp.insert(McpId::new("kibana-logs").unwrap());
    registry.apply_settings(&settings);
    assert!(registry.resolve("dev_kibana_search_logs").is_none());
    assert!(registry.resolve("qua_kibana_search_logs").is_none());
    assert!(registry.resolve("dev_k8s_list_pods").is_some());
    assert!(
        registry
            .exposed_tools()
            .iter()
            .all(|e| e.instance.mcp.as_str() != "kibana-logs")
    );
    let names: Vec<&str> = registry
        .exposed_tools()
        .iter()
        .map(|e| e.exposed_name.as_str())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
}

// Exigence : EX-005-03
#[test]
fn suspending_the_gateway_does_not_change_the_list() {
    let mut registry = registry();
    let mut settings = settings_for("support");
    registry.apply_settings(&settings);
    let count = registry.exposed_count();
    assert!(count > 0);
    settings.gateway_suspended = true;
    registry.apply_settings(&settings);
    assert_eq!(registry.exposed_count(), count);
}

// Exigence : EX-005-03
#[test]
fn exposure_changes_are_announced_once() {
    let mut registry = registry();
    let mut rx = registry.subscribe();
    developer(&mut registry);
    developer(&mut registry);
    assert_eq!(drain(&mut rx), vec![RegistryEvent::ExposureChanged]);
    registry.set_exposure(Exposure::none());
    assert_eq!(drain(&mut rx), vec![RegistryEvent::ExposureChanged]);
    assert_eq!(registry.exposed_count(), 0);
}

// --- EX-005-04: source preference -------------------------------------------------------------------------------

// Exigence : EX-005-04
#[test]
fn embedded_snapshot_is_used_without_cache() {
    let registry = registry();
    for instance in registry.instances() {
        assert_eq!(
            registry.instance_source(instance),
            Some(CatalogSource::Embedded),
            "{instance}"
        );
        assert!(registry.tools(instance).all(|e| e.origin == CatalogSource::Embedded));
    }
    assert!(registry.cache_warnings().is_empty());
}

// Exigence : EX-005-04
#[test]
fn cache_wins_over_the_embedded_snapshot() {
    let instance = inst("dev", "kibana-logs");
    let mut cached = snapshot(&instance);
    cached.push(tool("tail_logs", "Seen by an earlier run"));
    let cache = MemoryCache::default().with(&instance, cached);
    let registry = registry_with(config(), Box::new(cache), &filter());

    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Cache));
    assert_eq!(
        registry.instance_source(&inst("rec", "kibana-logs")),
        Some(CatalogSource::Embedded)
    );
    let tail = registry.lookup("dev_kibana_tail_logs").unwrap();
    assert_eq!(tail.origin, CatalogSource::Cache);
    // Seen in the cache, still unknown: only the configuration makes a tool known.
    assert!(!tail.known);
    assert_eq!(tail.category_source, CategorySource::Unknown);
    assert!(registry.lookup("dev_kibana_search_logs").unwrap().known);
}

// Exigence : EX-005-04
#[test]
fn live_list_wins_over_cache_and_snapshot() {
    let instance = inst("dev", "kibana-logs");
    let cache = MemoryCache::default().with(&instance, vec![tool("search_logs", "cached")]);
    let mut registry = registry_with(config(), Box::new(cache.clone()), &filter());
    registry
        .apply_live_list(&instance, vec![tool("search_logs", "live"), tool("tail_logs", "new")])
        .unwrap();
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Live));
    let entry = registry.lookup("dev_kibana_search_logs").unwrap();
    assert_eq!(entry.origin, CatalogSource::Live);
    assert_eq!(entry.server_description.as_deref(), Some("live"));
    assert!(!registry.lookup("dev_kibana_tail_logs").unwrap().known);
    // The cache now holds the live list.
    let stored = cache.get(&instance).unwrap();
    assert_eq!(stored.tools.len(), 2);
    assert!(stored.tools.iter().all(|t| !t.name.starts_with("dev_")));
}

// Exigence : EX-005-04
#[test]
fn unusable_cache_falls_back_to_the_snapshot() {
    let instance = inst("dev", "kibana-logs");
    let collide = vec![tool("Search.Logs", "a"), tool("search_logs", "b")];
    let cache = MemoryCache::default().with(&instance, collide);
    let registry = registry_with(config(), Box::new(cache), &filter());
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Embedded));
    assert_eq!(registry.cache_warnings().len(), 1);
    assert_eq!(registry.cache_warnings()[0].instance, instance);
    assert_eq!(registry.cache_warnings()[0].operation, CacheOperation::Load);

    let cache = MemoryCache::default();
    cache.fail_load.store(true, Ordering::SeqCst);
    let registry = registry_with(config(), Box::new(cache), &filter());
    assert!(
        registry
            .instances()
            .all(|i| registry.instance_source(i) == Some(CatalogSource::Embedded))
    );
    assert_eq!(registry.cache_warnings().len(), registry.instances().count());
}

// Exigence : EX-005-04
#[test]
fn environment_specific_snapshot_overrides_the_mcp_one() {
    let config = config();
    let generic = r#"[{"name":"search_logs","inputSchema":{"type":"object"}}]"#;
    let prod = r#"[{"name":"search_logs_v2","inputSchema":{"type":"object"}}]"#;
    let catalog = EmbeddedCatalog::from_files(
        &config,
        &[("kibana-logs.json", generic), ("kibana-logs.prod.json", prod)],
    )
    .unwrap();
    assert_eq!(
        catalog.for_instance(&inst("prod", "kibana-logs")).unwrap()[0].name,
        "search_logs_v2"
    );
    assert_eq!(
        catalog.for_instance(&inst("dev", "kibana-logs")).unwrap()[0].name,
        "search_logs"
    );
    assert!(catalog.for_instance(&inst("dev", "k8s-admin")).is_none());

    let registry = Registry::new(
        Arc::new(config.clone()),
        &catalog,
        Box::new(MemoryCache::default()),
        filter().filter,
    )
    .unwrap();
    assert!(registry.lookup("prod_kibana_search_logs_v2").is_some());
    assert_eq!(registry.instance_source(&inst("dev", "k8s-admin")), None);
    assert_eq!(registry.tools(&inst("dev", "k8s-admin")).count(), 0);

    let refused = |files: &[(&str, &str)]| EmbeddedCatalog::from_files(&config, files).unwrap_err();
    assert!(matches!(
        refused(&[("nope.json", generic)]),
        CatalogError::UnknownMcp { .. }
    ));
    assert!(matches!(
        refused(&[("gitlab-ops.qua.json", generic)]),
        CatalogError::NoInstance { .. }
    ));
    assert!(matches!(
        refused(&[("kibana-logs.txt", generic)]),
        CatalogError::FileName { .. }
    ));
    assert!(matches!(
        refused(&[("Kibana.json", generic)]),
        CatalogError::FileName { .. }
    ));
    assert!(matches!(
        refused(&[("kibana-logs.json", "[{")]),
        CatalogError::Json { .. }
    ));
}

// Exigence : EX-005-04
#[test]
fn file_cache_round_trip_through_a_new_registry() {
    let dir = TempDir::new();
    let f = filter();
    let instance = inst("rec", "jira");
    let live = vec![
        tool("search_issues", "Live search"),
        tool("link_issues", "New on the server"),
    ];
    {
        let cache = FileCatalogCache::new(&dir.0, Arc::clone(&f.filter));
        let mut registry = registry_with(config(), Box::new(cache), &f);
        registry.apply_live_list(&instance, live).unwrap();
        assert!(registry.cache_warnings().is_empty());
    }
    assert!(dir.0.join("jira.rec.json").is_file());

    let cache = FileCatalogCache::new(&dir.0, Arc::clone(&f.filter));
    let registry = registry_with(config(), Box::new(cache), &f);
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Cache));
    assert_eq!(
        registry.instance_source(&inst("prod", "jira")),
        Some(CatalogSource::Embedded)
    );
    let entry = registry.lookup("rec_jira_search_issues").unwrap();
    assert_eq!(entry.server_description.as_deref(), Some("Live search"));
    assert!(entry.known);
    assert!(!registry.lookup("rec_jira_link_issues").unwrap().known);
}

// Exigence : EX-005-04
#[test]
fn file_cache_of_another_instance_or_version_is_ignored() {
    let dir = TempDir::new();
    let f = filter();
    let cache = FileCatalogCache::new(&dir.0, Arc::clone(&f.filter));
    let rec = inst("rec", "jira");
    let prod = inst("prod", "jira");
    cache
        .store(
            &rec,
            &CachedCatalog {
                tools: vec![tool("search_issues", "x")],
            },
        )
        .unwrap();
    std::fs::copy(cache.path_for(&rec), cache.path_for(&prod)).unwrap();
    assert!(matches!(cache.load(&prod), Err(CacheError::Invalid(_))));
    assert!(cache.load(&rec).unwrap().is_some());
    assert!(cache.load(&inst("dev", "jira")).unwrap().is_none());

    let text = std::fs::read_to_string(cache.path_for(&rec)).unwrap();
    let mut value: Value = serde_json::from_str(&text).unwrap();
    value["schemaVersion"] = json!(2);
    std::fs::write(cache.path_for(&rec), value.to_string()).unwrap();
    assert!(matches!(cache.load(&rec), Err(CacheError::Invalid(_))));
    std::fs::write(cache.path_for(&rec), "{ not json").unwrap();
    let error = cache.load(&rec).unwrap_err();
    assert!(!error.to_string().contains("not json"));

    let registry = registry_with(config(), Box::new(cache), &f);
    assert_eq!(registry.instance_source(&rec), Some(CatalogSource::Embedded));
    assert_eq!(registry.instance_source(&prod), Some(CatalogSource::Embedded));
    assert_eq!(registry.cache_warnings().len(), 2);
}

// --- EX-005-05: live lists and fingerprints ---------------------------------------------------------------------

// Exigence : EX-005-05
#[test]
fn live_list_diff_reports_added_removed_and_modified() {
    let instance = inst("dev", "kibana-logs");
    let mut registry = registry();
    developer(&mut registry);
    let mut rx = registry.subscribe();

    // Same list as the snapshot: nothing changes, nothing is announced.
    let diff = registry.apply_live_list(&instance, snapshot(&instance)).unwrap();
    assert!(diff.is_empty());
    assert!(drain(&mut rx).is_empty());

    let mut list = snapshot(&instance);
    list.retain(|t| t.name != "delete_index");
    list.push(tool("tail_logs", "Follows new entries"));
    list[0].description = Some("Changed description".to_owned());
    list[1].input_schema["properties"]["extra"] = json!({ "type": "boolean" });
    let diff = registry.apply_live_list(&instance, list).unwrap();
    assert_eq!(diff.added, vec![tn("tail_logs")]);
    assert_eq!(diff.removed, vec![tn("delete_index")]);
    assert_eq!(diff.modified, vec![tn("get_log_entry"), tn("search_logs")]);
    assert_eq!(
        drain(&mut rx),
        vec![RegistryEvent::ToolsChanged {
            instance: instance.clone(),
            diff
        }]
    );
    // A removed tool no longer resolves, a new one does (unknown, hence destructive).
    assert!(registry.resolve("dev_kibana_delete_index").is_none());
    assert!(
        registry
            .resolve("dev_kibana_tail_logs")
            .unwrap()
            .annotations
            .destructive_hint
            == Some(true)
    );
}

// Exigence : EX-005-05
#[test]
fn fingerprints_ignore_key_and_tool_order() {
    let instance = inst("dev", "kibana-logs");
    let mut registry = registry();
    let before = registry.instance_fingerprint(&instance).cloned().unwrap();
    let prints: BTreeMap<ToolName, Fingerprint> = registry
        .tools(&instance)
        .map(|e| (e.original_name.clone(), e.fingerprint.clone()))
        .collect();

    // Same content, keys and tools in another order.
    let mut list: Vec<ToolDefinition> = snapshot(&instance)
        .into_iter()
        .map(|t| {
            let text = serde_json::to_string(&t).unwrap();
            let value: Value = serde_json::from_str(&text).unwrap();
            let Value::Object(map) = value else { unreachable!() };
            let reversed: serde_json::Map<String, Value> = map.into_iter().rev().collect();
            serde_json::from_value(Value::Object(reversed)).unwrap()
        })
        .collect();
    list.reverse();
    let reordered_schema = r#"{"required":["index","id"],"properties":{"id":{"type":"string"},"index":{"type":"string"}},"type":"object"}"#;
    let entry = list.iter_mut().find(|t| t.name == "get_log_entry").unwrap();
    entry.input_schema = serde_json::from_str(reordered_schema).unwrap();

    assert!(registry.apply_live_list(&instance, list).unwrap().is_empty());
    assert_eq!(registry.instance_fingerprint(&instance), Some(&before));
    for entry in registry.tools(&instance) {
        assert_eq!(prints[&entry.original_name], entry.fingerprint);
    }
    // Instances with different tools have different fingerprints.
    assert_ne!(registry.instance_fingerprint(&inst("dev", "k8s-admin")), Some(&before));
}

// Exigence : EX-005-05
#[test]
fn cache_is_written_only_when_needed() {
    let instance = inst("dev", "kibana-logs");
    let cache = MemoryCache::default();
    let mut registry = registry_with(config(), Box::new(cache.clone()), &filter());
    registry.apply_live_list(&instance, snapshot(&instance)).unwrap();
    assert_eq!(cache.stores(), 1, "first live list replaces the embedded snapshot");
    registry.apply_live_list(&instance, snapshot(&instance)).unwrap();
    assert_eq!(cache.stores(), 1, "same fingerprint: nothing written");
    registry
        .apply_live_list(&instance, vec![tool("search_logs", "x")])
        .unwrap();
    assert_eq!(cache.stores(), 2);

    cache.fail_store.store(true, Ordering::SeqCst);
    let diff = registry
        .apply_live_list(&instance, vec![tool("search_logs", "y")])
        .unwrap();
    assert_eq!(diff.modified, vec![tn("search_logs")]);
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Live));
    let warning = registry.cache_warnings().last().unwrap();
    assert_eq!(warning.operation, CacheOperation::Store);
    assert_eq!(warning.instance, instance);
}

// --- EX-005-06: warning threshold -------------------------------------------------------------------------------

// Exigence : EX-005-06
#[test]
fn threshold_is_exceeded_only_beyond_its_value() {
    let with_threshold = |threshold: usize| {
        let mut config = config();
        config.gateway.exposed_tools_warning_threshold = threshold;
        let mut registry = registry_with(config, Box::new(MemoryCache::default()), &filter());
        developer(&mut registry);
        registry
    };
    assert_eq!(with_threshold(50).exposed_count(), 50);
    assert!(!with_threshold(50).over_threshold());
    assert!(with_threshold(49).over_threshold());
    // The embedded threshold (100) is not reached by the example configuration.
    let mut registry = registry();
    developer(&mut registry);
    assert!(!registry.over_threshold());
    // Nothing exposed is never over the threshold.
    let mut low = with_threshold(1);
    low.set_exposure(Exposure::none());
    assert!(!low.over_threshold());
}

// --- EX-005-07: embedded snapshots ------------------------------------------------------------------------------

// Exigence : EX-005-07
#[test]
fn every_snapshot_file_is_embedded() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config/catalog");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let embedded: Vec<String> = EmbeddedCatalog::builtin_file_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(on_disk, embedded);
}

// Exigence : EX-005-07
#[test]
fn snapshots_list_exactly_the_declared_tools() {
    let config = config();
    let registry = registry();
    for mcp in &config.mcp {
        for env in mcp.instances.keys() {
            let instance = InstanceId::new(env.clone(), mcp.id.clone());
            let listed: Vec<&ToolName> = registry.tools(&instance).map(|e| &e.original_name).collect();
            let declared: Vec<&ToolName> = mcp.tools.keys().collect();
            assert_eq!(listed, declared, "{instance}");
            assert!(registry.tools(&instance).all(|e| e.known));
            assert!(
                registry
                    .tools(&instance)
                    .all(|e| e.category_source != CategorySource::Default),
                "{instance}: every declared tool without a category is annotated"
            );
        }
    }
}

// --- EX-005-08: output filter -----------------------------------------------------------------------------------

// Exigence : EX-005-08
#[test]
fn definitions_are_filtered_before_entering_the_registry() {
    let f = filter().with_secret();
    let mut registry = registry_with(config(), Box::new(MemoryCache::default()), &f);
    let mut leaky = tool("search_logs", &format!("Use token {SENTINEL} to search"));
    leaky.input_schema = json!({
        "type": "object",
        "properties": { "q": { "type": "string", "default": SENTINEL } }
    });
    leaky.output_schema = Some(json!({ "type": "object", "description": SENTINEL }));
    registry
        .apply_live_list(&inst("dev", "kibana-logs"), vec![leaky])
        .unwrap();

    let entry = registry.lookup("dev_kibana_search_logs").unwrap();
    let exposed = serde_json::to_string(&entry.exposed_definition()).unwrap();
    assert!(!exposed.contains(SENTINEL));
    assert!(exposed.contains(SECRET_MARKER));
    assert!(
        !serde_json::to_string(&entry.server_definition())
            .unwrap()
            .contains(SENTINEL)
    );
    assert!(!format!("{entry:?}").contains(SENTINEL));

    // A name cannot be masked: the list is refused.
    let error = registry
        .apply_live_list(&inst("dev", "kibana-logs"), vec![tool(&format!("x{SENTINEL}"), "x")])
        .unwrap_err();
    assert!(matches!(error, RegistryError::MaskedName { .. }));
    assert!(!error.to_string().contains(SENTINEL));
}

// Exigence : EX-005-08
#[test]
fn failing_filter_changes_nothing() {
    let f = filter();
    let cache = MemoryCache::default();
    let mut registry = registry_with(config(), Box::new(cache.clone()), &f);
    let instance = inst("dev", "kibana-logs");
    let before = registry.instance_fingerprint(&instance).cloned();
    f.wrapper.break_now();
    let error = registry
        .apply_live_list(&instance, vec![tool("search_logs", "x")])
        .unwrap_err();
    assert!(matches!(error, RegistryError::Filter(FilterError::Unavailable(_))));
    assert_eq!(registry.instance_fingerprint(&instance).cloned(), before);
    assert_eq!(registry.instance_source(&instance), Some(CatalogSource::Embedded));
    assert_eq!(cache.stores(), 0);

    // Without a working filter, no registry at all.
    let embedded = EmbeddedCatalog::builtin(&config()).unwrap();
    let built = Registry::new(
        Arc::new(config()),
        &embedded,
        Box::new(MemoryCache::default()),
        Arc::clone(&f.filter),
    );
    assert!(matches!(built, Err(RegistryError::Filter(_))));
}

// Exigence : EX-005-08
#[test]
fn file_cache_is_filtered_and_written_only_if_the_filter_works() {
    let dir = TempDir::new();
    let f = filter().with_secret();
    let cache = FileCatalogCache::new(&dir.0, Arc::clone(&f.filter));
    let instance = inst("rec", "jira");
    let leaky = CachedCatalog {
        tools: vec![tool("search_issues", &format!("token={SENTINEL}"))],
    };
    cache.store(&instance, &leaky).unwrap();
    let text = std::fs::read_to_string(cache.path_for(&instance)).unwrap();
    assert!(!text.contains(SENTINEL));
    assert!(text.contains("search_issues"));

    f.wrapper.break_now();
    let other = inst("prod", "jira");
    let error = cache.store(&other, &leaky).unwrap_err();
    assert!(matches!(error, CacheError::Filter(_)));
    assert!(!cache.path_for(&other).exists());
    let leftovers: Vec<_> = std::fs::read_dir(&dir.0).unwrap().collect();
    assert_eq!(leftovers.len(), 1, "no temporary file either");
}

// Exigence : EX-005-08
#[test]
fn seal_is_applied_to_the_bytes_of_the_file() {
    struct Reverse;
    impl CacheSeal for Reverse {
        fn seal(&self, instance: &InstanceId, mut plain: Vec<u8>) -> Result<Vec<u8>, CacheError> {
            plain.reverse();
            plain.extend_from_slice(instance.to_string().as_bytes());
            Ok(plain)
        }
        fn open(&self, instance: &InstanceId, mut sealed: Vec<u8>) -> Result<Vec<u8>, CacheError> {
            let tag = instance.to_string();
            if !sealed.ends_with(tag.as_bytes()) {
                return Err(CacheError::Seal("autre instance".to_owned()));
            }
            sealed.truncate(sealed.len() - tag.len());
            sealed.reverse();
            Ok(sealed)
        }
    }
    let dir = TempDir::new();
    let f = filter();
    let cache = FileCatalogCache::with_seal(&dir.0, Arc::clone(&f.filter), Box::new(Reverse));
    let rec = inst("rec", "jira");
    let catalog = CachedCatalog {
        tools: vec![tool("search_issues", "x")],
    };
    cache.store(&rec, &catalog).unwrap();
    assert!(serde_json::from_slice::<Value>(&std::fs::read(cache.path_for(&rec)).unwrap()).is_err());
    assert_eq!(cache.load(&rec).unwrap(), Some(catalog));
    let prod = inst("prod", "jira");
    std::fs::copy(cache.path_for(&rec), cache.path_for(&prod)).unwrap();
    assert!(matches!(cache.load(&prod), Err(CacheError::Seal(_))));
}

#[test]
fn registry_can_be_shared_between_tasks() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Registry>();
    send_sync::<RegistryEvent>();
}
