use super::*;

/// Smallest valid configuration; each test breaks it in one place.
const MINIMAL: &str = r#"
schema_version = 1

[gateway]
call_budget_secs = 50
confirmation_timeout_secs = 30
auth_timeout_secs = 45
upstream_timeout_secs = 120
slow_call_threshold_secs = 5
tool_name_max_len = 64
exposed_tools_warning_threshold = 100
refresh_interval_secs = 300
list_ttl_secs = 60
elicitation_clients = []

[trace]
ring_capacity = 500
ring_max_bytes = 1048576
log_retention_days = 7

[webview]
profile = "in_memory"
os_primary_account_sso = false

[update]
base_url = "https://artifactory.example.internal/artifactory"
repository = "generic-release-local"
path = "mcp-proxy"
check_interval_hours = 6

[[environments]]
id = "dev"
label = "DEV"

[[environments]]
id = "prod"
label = "PROD"
sensitive = true

[[sessions]]
id = "sso-corp"
label = "SSO Entreprise"
scope = "global"
[sessions.auth]
method = "basic"
ttl_hours = 8

[[sessions]]
id = "sso-dev"
label = "SSO DEV"
scope = "env"
environment = "dev"
[sessions.auth]
method = "cookie"
login_url = "https://sso.dev.example.internal/login"
cookie_url = "https://apps.dev.example.internal/"
cookie_names = ["SESSION"]
success_url_pattern = "https://apps.dev.example.internal/*"
allowed_navigation = ["sso.dev.example.internal", "apps.dev.example.internal"]
ttl_hours = 8

[[sessions]]
id = "sso-prod"
label = "SSO PROD"
scope = "env"
environment = "prod"
[sessions.auth]
method = "jwt_header"
login_url = "https://portal.prod.example.internal/"
capture_url_pattern = "https://api.prod.example.internal/*"
capture_header = "Authorization"
capture_scheme = "Bearer"
allowed_navigation = ["portal.prod.example.internal"]
max_ttl_hours = 24

[[mcp]]
id = "logs"
alias = "logs"
label = "Logs"
kind = "remote"
session = "env"
[mcp.instances]
dev = { url = "https://logs.dev.example.internal/mcp" }
prod = { url = "https://logs.prod.example.internal/mcp" }
[mcp.tools]
search = { category = "read" }
purge = { category = "destructive" }

[[mcp]]
id = "tickets"
alias = "tickets"
label = "Tickets"
kind = "local"
module = "jira-local"
session = "sso-corp"
[mcp.instances]
prod = { base_url = "https://jira.example.internal" }
[mcp.tools]
comment = { category = "write" }

[policy]
default = { decision = "confirm", channel = "notification" }
read = { decision = "allow" }

[profiles.strict]
label = "Strict"
strictest = true
unknown_tools = "deny"
"#;

fn load(toml: &str) -> Result<Config, ConfigErrors> {
    Config::from_toml(toml, BUILTIN_MODULES)
}

fn errors(toml: &str) -> Vec<ConfigError> {
    match load(toml) {
        Ok(_) => panic!("configuration unexpectedly valid"),
        Err(e) => e.errors,
    }
}

/// Replaces exactly one occurrence, so that a fixture edit cannot silently miss.
fn edit(from: &str, to: &str) -> String {
    assert_eq!(MINIMAL.matches(from).count(), 1, "fixture snippet not unique: {from}");
    MINIMAL.replace(from, to)
}

fn append(extra: &str) -> String {
    format!("{MINIMAL}\n{extra}")
}

#[track_caller]
fn assert_error(toml: &str, path: &str, fragment: &str) {
    let errors = errors(toml);
    assert!(
        errors.iter().any(|e| e.path == path && e.message.contains(fragment)),
        "no error at `{path}` containing `{fragment}`; got:\n{}",
        errors.iter().map(|e| format!("  {e}")).collect::<Vec<_>>().join("\n")
    );
}

// --- Embedded configuration ---------------------------------------------------------------------------------------

#[test]
fn embedded_config_loads_and_validates() {
    let config = match Config::embedded() {
        Ok(config) => config,
        Err(e) => panic!("{e}"),
    };
    assert_eq!(config.environments.len(), 8);
    assert_eq!(config.mcp.len(), 5);
    assert_eq!(config.profiles.len(), 3);
    let global = config
        .sessions
        .iter()
        .filter(|s| s.scope == SessionScope::Global)
        .count();
    assert_eq!(global, 2);
    assert_eq!(config.sessions.len(), 2 + 8);
    assert_eq!(config.strictest_profile().map(|p| p.id.as_str()), Some("lecture-seule"));

    let prod = EnvId::new("prod").unwrap();
    let kibana = McpId::new("kibana-logs").unwrap();
    let session = config.session_for(&kibana, &prod).unwrap();
    assert_eq!(session.id.as_str(), "sso-prod");
    let origins: Vec<_> = session.allowed_origins.iter().map(Origin::as_str).collect();
    assert_eq!(
        origins,
        [
            "https://k8s-mcp.prod.example.internal",
            "https://kibana-mcp.prod.example.internal"
        ]
    );

    let tools = config.session(&SessionId::new("sso-tools").unwrap()).unwrap();
    let origins: Vec<_> = tools.allowed_origins.iter().map(Origin::as_str).collect();
    assert_eq!(origins, ["https://jira.example.internal"]);

    let gitlab = config.mcp(&McpId::new("gitlab-ops").unwrap()).unwrap();
    let uncategorized = gitlab.tools[&ToolName::new("get_merge_request").unwrap()].category;
    assert_eq!(uncategorized, None);
}

#[test]
fn minimal_fixture_is_valid() {
    if let Err(e) = load(MINIMAL) {
        panic!("{e}");
    }
}

// --- Structure ----------------------------------------------------------------------------------------------------

#[test]
fn unknown_key_is_rejected() {
    let toml = edit("list_ttl_secs = 60", "list_ttl_secs = 60\nlist_ttl = 60");
    assert_error(&toml, "gateway.list_ttl", "unknown field `list_ttl`");
}

#[test]
fn unknown_key_is_rejected_deep_in_the_tree() {
    let toml = edit(
        "purge = { category = \"destructive\" }",
        "purge = { category = \"destructive\", color = 1 }",
    );
    assert_error(&toml, "mcp[0].tools.purge.color", "unknown field `color`");
}

#[test]
fn unknown_key_is_rejected_in_auth() {
    let toml = edit("method = \"basic\"\n", "method = \"basic\"\npassword = \"x\"\n");
    assert_error(&toml, "sessions[0].auth", "password");
}

#[test]
fn unknown_auth_method_is_rejected() {
    let toml = edit("method = \"basic\"", "method = \"ntlm\"");
    assert_error(&toml, "sessions[0].auth.method", "unknown variant `ntlm`");
}

#[test]
fn toml_syntax_error_has_a_line() {
    let toml = edit("ring_capacity = 500", "ring_capacity = = 500");
    assert_error(&toml, "", "ligne");
}

#[test]
fn several_errors_are_reported_together() {
    let toml = edit("id = \"dev\"\nlabel = \"DEV\"", "id = \"Dev\"\nlabel = \"\"");
    let toml = toml.replace("call_budget_secs = 50", "call_budget_secs = 70");
    let toml = toml.replace("read = { decision = \"allow\" }", "read = { decision = \"maybe\" }");
    let errors = errors(&toml);
    let paths: Vec<_> = errors.iter().map(|e| e.path.as_str()).collect();
    for expected in [
        "environments[0].id",
        "environments[0].label",
        "gateway.call_budget_secs",
        "policy.read.decision",
    ] {
        assert!(paths.contains(&expected), "missing {expected} in {paths:?}");
    }
}

#[test]
fn display_lists_every_error_with_its_path() {
    let toml = edit("id = \"dev\"\nlabel = \"DEV\"", "id = \"Dev\"\nlabel = \"\"");
    let err = load(&toml).unwrap_err();
    let text = err.to_string();
    assert!(text.starts_with("config invalide ("), "{text}");
    assert!(
        text.contains("environments[0].id : identifiant « Dev » invalide"),
        "{text}"
    );
    assert!(text.contains("environments[0].label : valeur vide"), "{text}");
}

#[test]
fn unsupported_schema_version() {
    assert_error(
        &edit("schema_version = 1", "schema_version = 2"),
        "schema_version",
        "non prise en charge",
    );
}

// --- Identifiers --------------------------------------------------------------------------------------------------

#[test]
fn malformed_identifier() {
    assert_error(
        &edit("id = \"logs\"", "id = \"Logs_1\""),
        "mcp[0].id",
        "caractère interdit",
    );
}

#[test]
fn duplicate_environment() {
    assert_error(
        &edit("id = \"prod\"\nlabel", "id = \"dev\"\nlabel"),
        "environments[1].id",
        "en double",
    );
}

#[test]
fn duplicate_session() {
    assert_error(
        &edit("id = \"sso-dev\"", "id = \"sso-corp\""),
        "sessions[1].id",
        "en double",
    );
}

#[test]
fn reserved_session_id() {
    assert_error(&edit("id = \"sso-corp\"", "id = \"env\""), "sessions[0].id", "réservé");
}

#[test]
fn duplicate_mcp() {
    assert_error(&edit("id = \"tickets\"", "id = \"logs\""), "mcp[1].id", "en double");
}

#[test]
fn duplicate_alias() {
    assert_error(
        &edit("alias = \"tickets\"", "alias = \"logs\""),
        "mcp[1].alias",
        "déjà utilisé",
    );
}

#[test]
fn malformed_tool_name() {
    let toml = edit("search = { category", "\"search logs\" = { category");
    assert_error(&toml, "mcp[0].tools.\"search logs\"", "caractère interdit");
}

#[test]
fn malformed_profile_id() {
    let toml = append("[profiles.Dev]\nlabel = \"Dev\"\nunknown_tools = \"deny\"");
    assert_error(&toml, "profiles.Dev", "caractère interdit");
}

// --- Cross references ---------------------------------------------------------------------------------------------

#[test]
fn unknown_session_reference() {
    assert_error(
        &edit("session = \"sso-corp\"", "session = \"sso-nope\""),
        "mcp[1].session",
        "inconnue",
    );
}

#[test]
fn env_session_referenced_by_id() {
    assert_error(
        &edit("session = \"sso-corp\"", "session = \"sso-prod\""),
        "mcp[1].session",
        "utiliser « env »",
    );
}

#[test]
fn unknown_environment_in_session() {
    assert_error(
        &edit("environment = \"dev\"", "environment = \"qa\""),
        "sessions[1].environment",
        "inconnu",
    );
}

#[test]
fn unknown_environment_in_instance() {
    let toml = edit("dev = { url = \"https://logs.dev", "qa = { url = \"https://logs.dev");
    assert_error(&toml, "mcp[0].instances.qa", "inconnu");
}

#[test]
fn unknown_local_module() {
    assert_error(
        &edit("module = \"jira-local\"", "module = \"confluence\""),
        "mcp[1].module",
        "inconnu",
    );
}

#[test]
fn local_mcp_requires_module_and_base_url() {
    let toml = edit("module = \"jira-local\"\n", "");
    assert_error(&toml, "mcp[1].module", "obligatoire");
    let toml = edit("{ base_url = \"https://jira", "{ url = \"https://jira");
    assert_error(&toml, "mcp[1].instances.prod.base_url", "obligatoire");
    assert_error(&toml, "mcp[1].instances.prod.url", "interdit");
}

#[test]
fn mcp_without_instance() {
    let toml = edit("prod = { base_url = \"https://jira.example.internal\" }\n", "");
    assert_error(&toml, "mcp[1].instances", "au moins une instance");
}

#[test]
fn unknown_environment_in_profile() {
    let toml = edit(
        "strictest = true",
        "strictest = true\ndefault_environments = [\"dev\", \"qa\"]",
    );
    assert_error(&toml, "profiles.strict.default_environments[1]", "inconnu");
}

#[test]
fn unknown_environment_in_policy() {
    let toml = append("[policy.env.qa]\nread = { decision = \"deny\" }");
    assert_error(&toml, "policy.env.qa", "inconnu");
}

#[test]
fn unknown_mcp_in_policy() {
    let toml = append("[policy.env.prod.mcp.metrics]\nread = { decision = \"deny\" }");
    assert_error(&toml, "policy.env.prod.mcp.metrics", "inconnu");
}

#[test]
fn mcp_without_instance_in_policy_environment() {
    let toml = append("[policy.env.dev.mcp.tickets]\nread = { decision = \"deny\" }");
    assert_error(&toml, "policy.env.dev.mcp.tickets", "pas d'instance");
}

#[test]
fn undeclared_tool_in_policy() {
    let toml = append("[policy.env.prod.mcp.logs.tool.purge_all]\nrule = { decision = \"deny\" }");
    assert_error(&toml, "policy.env.prod.mcp.logs.tool.purge_all", "non déclaré");
}

#[test]
fn unknown_environment_in_profile_policy() {
    let toml = append("[profiles.strict.policy.env.qa]\nread = { decision = \"deny\" }");
    assert_error(&toml, "profiles.strict.policy.env.qa", "inconnu");
}

// --- Sessions -----------------------------------------------------------------------------------------------------

#[test]
fn two_env_sessions_for_one_environment() {
    let toml = edit("environment = \"prod\"", "environment = \"dev\"");
    assert_error(&toml, "sessions[2].environment", "déjà sa session");
    assert_error(&toml, "environments[1]", "aucune session");
}

#[test]
fn environment_without_env_session() {
    let toml = edit("scope = \"env\"\nenvironment = \"prod\"", "scope = \"global\"");
    assert_error(&toml, "environments[1]", "aucune session scope = \"env\"");
}

#[test]
fn no_global_session() {
    let toml = edit("scope = \"global\"", "scope = \"env\"\nenvironment = \"dev\"");
    assert_error(&toml, "sessions", "au moins une session scope = \"global\"");
}

#[test]
fn env_session_requires_environment() {
    assert_error(
        &edit("environment = \"dev\"\n", ""),
        "sessions[1].environment",
        "obligatoire",
    );
}

#[test]
fn global_session_forbids_environment() {
    let toml = edit("scope = \"global\"", "scope = \"global\"\nenvironment = \"dev\"");
    assert_error(&toml, "sessions[0].environment", "interdit");
}

#[test]
fn unknown_scope() {
    assert_error(
        &edit("scope = \"global\"", "scope = \"world\""),
        "sessions[0].scope",
        "inconnu",
    );
}

#[test]
fn session_without_origin() {
    let toml = append(
        "[[sessions]]\nid = \"sso-unused\"\nlabel = \"Inutilisée\"\nscope = \"global\"\n\
         [sessions.auth]\nmethod = \"basic\"\nttl_hours = 8",
    );
    assert_error(&toml, "sessions[3]", "aucune origine autorisée");
}

// --- URLs ---------------------------------------------------------------------------------------------------------

#[test]
fn plain_http_is_refused() {
    let toml = edit(
        "https://logs.dev.example.internal/mcp",
        "http://logs.dev.example.internal/mcp",
    );
    assert_error(&toml, "mcp[0].instances.dev.url", "https obligatoire");
}

#[test]
fn http_is_tolerated_for_loopback() {
    let toml = edit("https://logs.dev.example.internal/mcp", "http://127.0.0.1:8080/mcp");
    let toml = toml.replace("https://jira.example.internal", "http://localhost:9000");
    let config = load(&toml).unwrap();
    let dev = config.session(&SessionId::new("sso-dev").unwrap()).unwrap();
    let origins: Vec<_> = dev.allowed_origins.iter().map(Origin::as_str).collect();
    assert_eq!(origins, ["http://127.0.0.1:8080"]);
}

#[test]
fn other_scheme_is_refused() {
    assert_error(
        &edit("https://artifactory", "ftp://artifactory"),
        "update.base_url",
        "schéma « ftp »",
    );
}

#[test]
fn credentials_in_url_are_refused() {
    let toml = edit(
        "https://logs.prod.example.internal/mcp",
        "https://user:pw@logs.prod.example.internal/mcp",
    );
    assert_error(&toml, "mcp[0].instances.prod.url", "identifiants interdits");
}

#[test]
fn malformed_url() {
    assert_error(
        &edit("https://sso.dev.example.internal/login", "not a url"),
        "sessions[1].auth.login_url",
        "URL invalide",
    );
}

// --- Authentication parameters ------------------------------------------------------------------------------------

#[test]
fn missing_auth_parameter() {
    let toml = edit("capture_scheme = \"Bearer\"\n", "");
    assert_error(&toml, "sessions[2].auth", "missing field `capture_scheme`");
}

#[test]
fn login_host_must_be_navigable() {
    let toml = edit(
        "allowed_navigation = [\"portal.prod.example.internal\"]",
        "allowed_navigation = [\"other.example.internal\"]",
    );
    assert_error(&toml, "sessions[2].auth.login_url", "allowed_navigation");
}

#[test]
fn success_host_must_be_navigable() {
    let toml = edit(
        "allowed_navigation = [\"sso.dev.example.internal\", \"apps.dev.example.internal\"]",
        "allowed_navigation = [\"sso.dev.example.internal\"]",
    );
    assert_error(&toml, "sessions[1].auth.success_url_pattern", "allowed_navigation");
}

#[test]
fn navigation_host_must_be_a_plain_host() {
    let toml = edit(
        "[\"portal.prod.example.internal\"]",
        "[\"portal.prod.example.internal\", \"*.example.internal\"]",
    );
    assert_error(&toml, "sessions[2].auth.allowed_navigation[1]", "nom d'hôte valide");
}

#[test]
fn wildcard_only_at_end_of_pattern() {
    let toml = edit(
        "https://api.prod.example.internal/*",
        "https://*.prod.example.internal/",
    );
    assert_error(&toml, "sessions[2].auth.capture_url_pattern", "qu'en fin de motif");
}

#[test]
fn pattern_prefix_must_be_canonical() {
    let toml = edit(
        "https://api.prod.example.internal/*",
        "https://api.prod.example.internal*",
    );
    assert_error(&toml, "sessions[2].auth.capture_url_pattern", "non canonique");
}

#[test]
fn cookie_names_required() {
    assert_error(
        &edit("cookie_names = [\"SESSION\"]", "cookie_names = []"),
        "sessions[1].auth.cookie_names",
        "au moins un",
    );
}

#[test]
fn invalid_capture_header() {
    let toml = edit("capture_header = \"Authorization\"", "capture_header = \"Auth header\"");
    assert_error(&toml, "sessions[2].auth.capture_header", "invalide");
}

#[test]
fn ttl_is_capped_at_one_day() {
    assert_error(
        &edit("max_ttl_hours = 24", "max_ttl_hours = 48"),
        "sessions[2].auth.max_ttl_hours",
        "24",
    );
    assert_error(
        &edit("method = \"basic\"\nttl_hours = 8", "method = \"basic\"\nttl_hours = 0"),
        "sessions[0].auth.ttl_hours",
        "24",
    );
}

#[test]
fn probe_url_must_be_https() {
    let toml = edit(
        "ttl_hours = 8\n\n[[sessions]]\nid = \"sso-dev\"",
        "ttl_hours = 8\nprobe_url = \"http://jira.example.internal/x\"\n\n[[sessions]]\nid = \"sso-dev\"",
    );
    assert_error(&toml, "sessions[0].auth.probe_url", "https obligatoire");
}

// --- Policy -------------------------------------------------------------------------------------------------------

#[test]
fn root_default_rule_is_mandatory() {
    let toml = edit("default = { decision = \"confirm\", channel = \"notification\" }\n", "");
    assert_error(&toml, "policy.default", "obligatoire");
}

#[test]
fn unknown_decision() {
    assert_error(
        &edit("read = { decision = \"allow\" }", "read = { decision = \"ok\" }"),
        "policy.read.decision",
        "inconnue",
    );
}

#[test]
fn unknown_channel() {
    let toml = edit("channel = \"notification\" }", "channel = \"email\" }");
    assert_error(&toml, "policy.default.channel", "inconnu");
}

#[test]
fn unknown_category() {
    assert_error(
        &edit("search = { category = \"read\" }", "search = { category = \"lookup\" }"),
        "mcp[0].tools.search.category",
        "inconnue",
    );
}

/// Adds `inspect`, a declared tool without static category, to the `logs` MCP.
fn with_uncategorized_tool() -> String {
    edit(
        "search = { category = \"read\" }",
        "search = { category = \"read\" }\ninspect = {}",
    )
}

#[test]
fn tool_without_category_is_known() {
    let config = load(&with_uncategorized_tool()).unwrap();
    let logs = config.mcp(&McpId::new("logs").unwrap()).unwrap();
    assert_eq!(logs.tools[&ToolName::new("inspect").unwrap()].category, None);
    assert_eq!(
        logs.tools[&ToolName::new("search").unwrap()].category,
        Some(Category::Read)
    );
}

#[test]
fn invalid_category_is_refused_even_though_category_is_optional() {
    let toml = edit("purge = { category = \"destructive\" }", "purge = { category = \"\" }");
    assert_error(&toml, "mcp[0].tools.purge.category", "inconnue");
}

#[test]
fn locked_category_does_not_cover_uncategorized_tool_rule() {
    let toml = with_uncategorized_tool()
        + "\n[policy.env.prod]\nwrite = { decision = \"deny\", locked = true }\n\
           [policy.env.prod.mcp.logs.tool.inspect]\nrule = { decision = \"allow\" }";
    if let Err(e) = load(&toml) {
        panic!("{e}");
    }
}

#[test]
fn locked_default_covers_uncategorized_tool_rule() {
    let toml = with_uncategorized_tool()
        + "\n[policy.env.prod]\ndefault = { decision = \"deny\", locked = true }\n\
           [policy.env.prod.mcp.logs.tool.inspect]\nrule = { decision = \"allow\" }";
    assert_error(
        &toml,
        "policy.env.prod.mcp.logs.tool.inspect.rule",
        "la règle verrouillée policy.env.prod.default la couvre",
    );
}

#[test]
fn locked_tool_rule_covers_uncategorized_tool_rule_in_profile() {
    let toml = with_uncategorized_tool()
        + "\n[policy.env.prod.mcp.logs.tool.inspect]\nrule = { decision = \"deny\", locked = true }\n\
           [profiles.strict.policy.env.prod.mcp.logs.tool.inspect]\nrule = { decision = \"allow\" }";
    assert_error(
        &toml,
        "profiles.strict.policy.env.prod.mcp.logs.tool.inspect.rule",
        "au moins aussi stricte",
    );
}

#[test]
fn confirm_requires_channel() {
    let toml = edit(
        "default = { decision = \"confirm\", channel = \"notification\" }",
        "default = { decision = \"confirm\" }",
    );
    assert_error(&toml, "policy.default.channel", "obligatoire");
}

#[test]
fn channel_only_with_confirm() {
    let toml = edit(
        "read = { decision = \"allow\" }",
        "read = { decision = \"allow\", channel = \"elicitation\" }",
    );
    assert_error(&toml, "policy.read.channel", "n'a de sens");
}

#[test]
fn unknown_tools_value() {
    assert_error(
        &edit("unknown_tools = \"deny\"", "unknown_tools = \"allow\""),
        "profiles.strict.unknown_tools",
        "inconnu",
    );
}

#[test]
fn rule_softer_than_covering_locked_rule() {
    let toml = append(
        "[policy.env.prod]\nwrite = { decision = \"confirm\", channel = \"notification\", locked = true }\n\
         [policy.env.prod.mcp.tickets.tool.comment]\nrule = { decision = \"allow\" }",
    );
    assert_error(
        &toml,
        "policy.env.prod.mcp.tickets.tool.comment.rule",
        "verrouillée policy.env.prod.write",
    );
}

#[test]
fn locked_default_covers_deeper_categories() {
    let toml = edit(
        "read = { decision = \"allow\" }",
        "read = { decision = \"allow\" }\n[policy.env.prod]\ndefault = { decision = \"deny\", locked = true }",
    ) + "\n[policy.env.prod.mcp.logs]\nread = { decision = \"confirm\", channel = \"elicitation\" }";
    assert_error(
        &toml,
        "policy.env.prod.mcp.logs.read",
        "verrouillée policy.env.prod.default",
    );
}

#[test]
fn profile_rule_softer_than_base_locked_rule() {
    let toml = append(
        "[policy.env.prod]\nwrite = { decision = \"confirm\", channel = \"notification\", locked = true }\n\
         [profiles.strict.policy.env.prod]\nwrite = { decision = \"allow\" }",
    );
    assert_error(
        &toml,
        "profiles.strict.policy.env.prod.write",
        "verrouillée policy.env.prod.write",
    );
}

#[test]
fn stricter_rules_under_a_lock_are_accepted() {
    let toml = append(
        "[policy.env.prod]\nwrite = { decision = \"confirm\", channel = \"elicitation\", locked = true }\n\
         [policy.env.prod.mcp.tickets.tool.comment]\nrule = { decision = \"deny\" }\n\
         [profiles.strict.policy.env.prod]\nwrite = { decision = \"confirm\", channel = \"notification\" }\n\
         [policy.env.dev]\nwrite = { decision = \"allow\" }",
    );
    if let Err(e) = load(&toml) {
        panic!("{e}");
    }
}

#[test]
fn locked_category_does_not_cover_other_categories() {
    let toml = append(
        "[policy.env.prod]\nwrite = { decision = \"deny\", locked = true }\n\
         [policy.env.prod.mcp.logs]\nread = { decision = \"allow\" }\ndefault = { decision = \"allow\" }",
    );
    if let Err(e) = load(&toml) {
        panic!("{e}");
    }
}

// --- Profiles -----------------------------------------------------------------------------------------------------

#[test]
fn no_strictest_profile() {
    assert_error(&edit("strictest = true\n", ""), "profiles", "aucun trouvé");
}

#[test]
fn two_strictest_profiles() {
    let toml = append("[profiles.zz]\nlabel = \"Autre\"\nstrictest = true\nunknown_tools = \"deny\"");
    assert_error(&toml, "profiles.zz.strictest", "exactement un profil");
}

#[test]
fn strictest_profile_denies_unknown_tools() {
    let toml = edit("unknown_tools = \"deny\"", "unknown_tools = \"confirm\"");
    assert_error(&toml, "profiles.strict.unknown_tools", "\"deny\" exigé");
}

// --- Exposed names ------------------------------------------------------------------------------------------------

#[test]
fn exposed_name_collision() {
    let toml = edit(
        "search = { category = \"read\" }",
        "search = { category = \"read\" }\nSearch = { category = \"read\" }",
    );
    assert_error(&toml, "mcp[0].tools.search", "nom exposé « dev_logs_search »");
}

#[test]
fn long_tool_names_are_truncated_without_collision() {
    let long = "a".repeat(100);
    let toml = edit(
        "search = { category = \"read\" }",
        &format!("{long}x = {{ category = \"read\" }}\n{long}y = {{ category = \"read\" }}"),
    );
    if let Err(e) = load(&toml) {
        panic!("{e}");
    }
}

#[test]
fn tool_name_max_len_must_allow_names() {
    assert_error(
        &edit("tool_name_max_len = 64", "tool_name_max_len = 8"),
        "gateway.tool_name_max_len",
        "compris entre",
    );
}

// --- Gateway timings ----------------------------------------------------------------------------------------------

#[test]
fn call_budget_below_client_cutoff() {
    assert_error(
        &edit("call_budget_secs = 50", "call_budget_secs = 60"),
        "gateway.call_budget_secs",
        "60 s",
    );
}

#[test]
fn confirmation_timeout_within_budget() {
    let toml = edit("confirmation_timeout_secs = 30", "confirmation_timeout_secs = 51");
    assert_error(&toml, "gateway.confirmation_timeout_secs", "call_budget_secs");
}

#[test]
fn auth_timeout_within_budget() {
    assert_error(
        &edit("auth_timeout_secs = 45", "auth_timeout_secs = 55"),
        "gateway.auth_timeout_secs",
        "call_budget_secs",
    );
}

#[test]
fn zero_values_are_refused() {
    assert_error(
        &edit("ring_capacity = 500", "ring_capacity = 0"),
        "trace.ring_capacity",
        "au moins 1",
    );
}

#[test]
fn update_path_cannot_escape() {
    assert_error(
        &edit("path = \"mcp-proxy\"", "path = \"../mcp-proxy\""),
        "update.path",
        "segments",
    );
}
