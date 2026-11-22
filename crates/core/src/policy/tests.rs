use proptest::prelude::*;

use super::*;
use crate::config::{
    Channel, Config, EnvId, McpId, PolicyTree, ProfileId, Rule, RulePosition, Selector, ToolConfig, ToolName,
};

const ALLOW: Decision = Decision::Allow;
const CE: Decision = Decision::Confirm(Channel::Elicitation);
const CN: Decision = Decision::Confirm(Channel::Notification);
const DENY: Decision = Decision::Deny;

const fn r(decision: Decision) -> Rule {
    Rule {
        decision,
        locked: false,
    }
}

const fn l(decision: Decision) -> Rule {
    Rule { decision, locked: true }
}

fn env(id: &str) -> EnvId {
    EnvId::new(id).unwrap()
}

fn mcp(id: &str) -> McpId {
    McpId::new(id).unwrap()
}

fn tool(name: &str) -> ToolName {
    ToolName::new(name).unwrap()
}

fn selector(name: &str) -> Selector {
    match name {
        "default" => Selector::Default,
        "read" => Selector::Category(Category::Read),
        "write" => Selector::Category(Category::Write),
        "destructive" => Selector::Category(Category::Destructive),
        other => panic!("unknown selector {other}"),
    }
}

/// `default`, `env.prod.write`, `env.prod.mcp.k8s-admin.destructive`, `env.prod.mcp.k8s-admin.tool.delete_pod`.
fn position(path: &str) -> RulePosition {
    let parts: Vec<&str> = path.split('.').collect();
    match parts.as_slice() {
        [s] => RulePosition {
            env: None,
            mcp: None,
            selector: selector(s),
        },
        ["env", e, s] => RulePosition {
            env: Some(env(e)),
            mcp: None,
            selector: selector(s),
        },
        ["env", e, "mcp", m, s] => RulePosition {
            env: Some(env(e)),
            mcp: Some(mcp(m)),
            selector: selector(s),
        },
        ["env", e, "mcp", m, "tool", t] => RulePosition {
            env: Some(env(e)),
            mcp: Some(mcp(m)),
            selector: Selector::Tool(tool(t)),
        },
        _ => panic!("bad path {path}"),
    }
}

pub(crate) fn insert(tree: &mut PolicyTree, position: &RulePosition, rule: Rule) {
    let set = match (&position.env, &position.mcp) {
        (None, _) => &mut tree.rules,
        (Some(e), None) => &mut tree.env.entry(e.clone()).or_default().rules,
        (Some(e), Some(m)) => {
            let node = tree.env.entry(e.clone()).or_default().mcp.entry(m.clone()).or_default();
            if let Selector::Tool(t) = &position.selector {
                node.tools.insert(t.clone(), rule);
                return;
            }
            &mut node.rules
        }
    };
    let slot = match position.selector {
        Selector::Default => &mut set.default,
        Selector::Category(Category::Read) => &mut set.read,
        Selector::Category(Category::Write) => &mut set.write,
        Selector::Category(Category::Destructive) => &mut set.destructive,
        Selector::Tool(_) => unreachable!("tool rule without env and mcp"),
    };
    *slot = Some(rule);
}

fn tree(rules: &[(&str, Rule)]) -> PolicyTree {
    let mut out = PolicyTree::default();
    for (path, rule) in rules {
        insert(&mut out, &position(path), *rule);
    }
    out
}

/// The embedded configuration with every policy tree replaced.
fn config_with(base: PolicyTree, profile: PolicyTree) -> Config {
    let mut config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    config.policy = base;
    for p in config.profiles.values_mut() {
        p.policy = profile.clone();
    }
    config
}

fn resolve_in(
    config: &Config,
    profile: &str,
    user: &PolicyTree,
    call: (&str, &str, &str),
    annotations: Option<ToolAnnotations>,
) -> Resolution {
    let profile = &config.profiles[&ProfileId::new(profile).unwrap()];
    PolicyEngine::new(config, profile, user).resolve(&env(call.0), &mcp(call.1), &tool(call.2), annotations.as_ref())
}

const DEV: &str = "developpeur"; // unknown_tools = confirm (notification)
const RO: &str = "lecture-seule"; // unknown_tools = deny

const SEARCH_LOGS: (&str, &str, &str) = ("prod", "kibana-logs", "search_logs"); // read
const DELETE_INDEX: (&str, &str, &str) = ("prod", "kibana-logs", "delete_index"); // destructive
const SCALE: (&str, &str, &str) = ("prod", "k8s-admin", "scale_deployment"); // write
const DELETE_POD: (&str, &str, &str) = ("prod", "k8s-admin", "delete_pod"); // destructive
const MERGE_REQUEST: (&str, &str, &str) = ("prod", "gitlab-ops", "get_merge_request"); // no static category
const MYSTERY: (&str, &str, &str) = ("prod", "k8s-admin", "drop_cluster"); // unknown to the config

const fn ann(read_only_hint: Option<bool>, destructive_hint: Option<bool>) -> Option<ToolAnnotations> {
    Some(ToolAnnotations {
        read_only_hint,
        destructive_hint,
    })
}

#[derive(Debug, Clone, Copy)]
enum By {
    NoRule,
    Winner(bool, &'static str),
    Floor(bool, &'static str),
    Unknown,
    Hardening(&'static str),
}

const BASE: bool = false;
const PROFILE: bool = true;

struct Case {
    name: &'static str,
    base: &'static [(&'static str, Rule)],
    profile: &'static [(&'static str, Rule)],
    user: &'static [(&'static str, Rule)],
    profile_id: &'static str,
    call: (&'static str, &'static str, &'static str),
    annotations: Option<ToolAnnotations>,
    decision: Decision,
    category: (Category, CategorySource),
    by: By,
}

const READ_CFG: (Category, CategorySource) = (Category::Read, CategorySource::Config);
const WRITE_CFG: (Category, CategorySource) = (Category::Write, CategorySource::Config);
const DESTR_CFG: (Category, CategorySource) = (Category::Destructive, CategorySource::Config);
const UNKNOWN: (Category, CategorySource) = (Category::Destructive, CategorySource::Unknown);

fn origin_matches(origin: &RuleOrigin, profile: bool, path: &str, profile_id: &str) -> bool {
    let layer = if profile {
        Layer::Profile(ProfileId::new(profile_id).unwrap())
    } else {
        Layer::Base
    };
    origin.layer == layer && origin.position == position(path)
}

fn check_by(case_name: &str, profile_id: &str, resolution: &Resolution, by: By) {
    let decided_by = resolution.provenance.decided_by();
    let ok = match (by, decided_by) {
        (By::NoRule, DecidedBy::NoRule) => true,
        (By::Winner(p, path), DecidedBy::Winner(o)) | (By::Floor(p, path), DecidedBy::Floor(o)) => {
            origin_matches(o, p, path, profile_id)
        }
        (By::Unknown, DecidedBy::UnknownTools(u)) => u.profile.as_str() == profile_id,
        (By::Hardening(path), DecidedBy::Hardening(o)) => o.layer == Layer::User && o.position == position(path),
        _ => false,
    };
    assert!(ok, "{case_name}: expected {by:?}, got {decided_by:?}");
}

const CASES: &[Case] = &[
    // --- Selectors.
    Case {
        name: "root default alone",
        base: &[("default", r(CN))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: CN,
        category: READ_CFG,
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "category beats default on the same node",
        base: &[("default", r(ALLOW)), ("read", r(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: DENY,
        category: READ_CFG,
        by: By::Winner(BASE, "read"),
    },
    Case {
        name: "a category rule of another category is no candidate",
        base: &[("default", r(CE)), ("write", r(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: CE,
        category: READ_CFG,
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "root category beats a deeper default",
        base: &[
            ("read", r(ALLOW)),
            ("default", r(DENY)),
            ("env.prod.mcp.kibana-logs.default", r(DENY)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: ALLOW,
        category: READ_CFG,
        by: By::Winner(BASE, "read"),
    },
    Case {
        name: "tool rule beats a deeper-or-equal category",
        base: &[
            ("default", r(CN)),
            ("env.prod.mcp.kibana-logs.read", r(DENY)),
            ("env.prod.mcp.kibana-logs.tool.search_logs", r(ALLOW)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: ALLOW,
        category: READ_CFG,
        by: By::Winner(BASE, "env.prod.mcp.kibana-logs.tool.search_logs"),
    },
    Case {
        name: "tool rule applies whatever the category",
        base: &[
            ("default", r(CN)),
            ("destructive", r(DENY)),
            ("env.prod.mcp.kibana-logs.tool.delete_index", r(CE)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: DELETE_INDEX,
        annotations: None,
        decision: CE,
        category: DESTR_CFG,
        by: By::Winner(BASE, "env.prod.mcp.kibana-logs.tool.delete_index"),
    },
    Case {
        name: "a tool rule of another tool is no candidate",
        base: &[
            ("default", r(CN)),
            ("env.prod.mcp.kibana-logs.tool.delete_index", r(ALLOW)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: CN,
        category: READ_CFG,
        by: By::Winner(BASE, "default"),
    },
    // --- Depths.
    Case {
        name: "environment default beats root default",
        base: &[("default", r(DENY)), ("env.prod.default", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: ALLOW,
        category: READ_CFG,
        by: By::Winner(BASE, "env.prod.default"),
    },
    Case {
        name: "MCP category beats environment category",
        base: &[
            ("default", r(CN)),
            ("env.prod.write", r(DENY)),
            ("env.prod.mcp.k8s-admin.write", r(CE)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Winner(BASE, "env.prod.mcp.k8s-admin.write"),
    },
    Case {
        name: "rules of another environment are no candidates",
        base: &[
            ("default", r(CN)),
            ("env.dev.default", r(ALLOW)),
            ("env.dev.mcp.k8s-admin.write", r(ALLOW)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CN,
        category: WRITE_CFG,
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "rules of another MCP are no candidates",
        base: &[("default", r(CN)), ("env.prod.mcp.kibana-logs.default", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CN,
        category: WRITE_CFG,
        by: By::Winner(BASE, "default"),
    },
    // --- Layers.
    Case {
        name: "profile beats base on the same node and selector",
        base: &[("default", r(CN)), ("env.prod.write", r(DENY))],
        profile: &[("env.prod.write", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: ALLOW,
        category: WRITE_CFG,
        by: By::Winner(PROFILE, "env.prod.write"),
    },
    Case {
        name: "depth comes before layer",
        base: &[("default", r(CN)), ("env.prod.mcp.k8s-admin.default", r(DENY))],
        profile: &[("env.prod.default", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::Winner(BASE, "env.prod.mcp.k8s-admin.default"),
    },
    Case {
        name: "selector comes before layer",
        base: &[("default", r(CN)), ("write", r(CE))],
        profile: &[("env.prod.mcp.k8s-admin.default", r(DENY))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Winner(BASE, "write"),
    },
    Case {
        name: "profile root default beats base root default",
        base: &[("default", r(CN))],
        profile: &[("default", r(ALLOW))],
        user: &[],
        profile_id: RO,
        call: SCALE,
        annotations: None,
        decision: ALLOW,
        category: WRITE_CFG,
        by: By::Winner(PROFILE, "default"),
    },
    // --- Locks at each level, in each layer.
    Case {
        name: "root lock is a floor",
        base: &[("default", r(CN)), ("write", l(CN))],
        profile: &[("env.prod.mcp.k8s-admin.tool.scale_deployment", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CN,
        category: WRITE_CFG,
        by: By::Floor(BASE, "write"),
    },
    Case {
        name: "environment lock is a floor",
        base: &[("default", r(ALLOW)), ("env.prod.default", l(CE))],
        profile: &[("env.prod.mcp.k8s-admin.write", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Floor(BASE, "env.prod.default"),
    },
    Case {
        name: "MCP lock raises a more precise but shallower winner",
        base: &[
            ("default", r(CN)),
            ("read", r(ALLOW)),
            ("env.prod.mcp.kibana-logs.default", l(DENY)),
        ],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: None,
        decision: DENY,
        category: READ_CFG,
        by: By::Floor(BASE, "env.prod.mcp.kibana-logs.default"),
    },
    Case {
        name: "tool lock of the base under a softer profile tool rule",
        base: &[
            ("default", r(CN)),
            ("env.prod.mcp.kibana-logs.tool.delete_index", l(DENY)),
        ],
        profile: &[("env.prod.mcp.kibana-logs.tool.delete_index", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: DELETE_INDEX,
        annotations: None,
        decision: DENY,
        category: DESTR_CFG,
        by: By::Floor(BASE, "env.prod.mcp.kibana-logs.tool.delete_index"),
    },
    Case {
        name: "profile lock raises a base rule",
        base: &[("default", r(CN)), ("env.prod.write", r(CN))],
        profile: &[("write", l(DENY))],
        user: &[],
        profile_id: RO,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::Floor(PROFILE, "write"),
    },
    Case {
        name: "the most severe lock is the floor",
        base: &[("default", l(CE)), ("env.prod.write", l(CN))],
        profile: &[("env.prod.mcp.k8s-admin.tool.scale_deployment", r(ALLOW))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CN,
        category: WRITE_CFG,
        by: By::Floor(BASE, "env.prod.write"),
    },
    Case {
        name: "a stricter winner is not lowered by the floor",
        base: &[("default", r(CN)), ("env.prod.write", l(CE))],
        profile: &[("env.prod.mcp.k8s-admin.tool.scale_deployment", r(DENY))],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::Winner(PROFILE, "env.prod.mcp.k8s-admin.tool.scale_deployment"),
    },
    Case {
        name: "a locked winner decides by itself",
        base: &[("default", r(CN)), ("env.prod.write", l(CN))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CN,
        category: WRITE_CFG,
        by: By::Winner(BASE, "env.prod.write"),
    },
    Case {
        name: "a lock of another environment is no floor",
        base: &[("default", r(ALLOW)), ("env.dev.default", l(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: ALLOW,
        category: WRITE_CFG,
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "a lock of another category is no floor",
        base: &[("default", r(ALLOW)), ("destructive", l(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: ALLOW,
        category: WRITE_CFG,
        by: By::Winner(BASE, "default"),
    },
    // --- Unknown tools, under each kind of profile.
    Case {
        name: "unknown tool under a confirm profile",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MYSTERY,
        annotations: None,
        decision: CN,
        category: UNKNOWN,
        by: By::Unknown,
    },
    Case {
        name: "unknown tool under a deny profile",
        base: &[("default", r(CN))],
        profile: &[],
        user: &[],
        profile_id: RO,
        call: MYSTERY,
        annotations: None,
        decision: DENY,
        category: UNKNOWN,
        by: By::Unknown,
    },
    Case {
        name: "unknown tool already denied by the config",
        base: &[("default", r(ALLOW)), ("destructive", r(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MYSTERY,
        annotations: None,
        decision: DENY,
        category: UNKNOWN,
        by: By::Winner(BASE, "destructive"),
    },
    Case {
        name: "unknown tool stays destructive whatever its annotations",
        base: &[("default", r(CN)), ("read", r(ALLOW)), ("destructive", r(CE))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MYSTERY,
        annotations: ann(Some(true), Some(false)),
        decision: CN,
        category: UNKNOWN,
        by: By::Unknown,
    },
    Case {
        name: "declared tool of an MCP without instance in this environment is unknown",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: RO,
        call: ("dev", "db-readonly", "run_select"),
        annotations: None,
        decision: DENY,
        category: UNKNOWN,
        by: By::Unknown,
    },
    // --- Category sources.
    Case {
        name: "declared category wins over annotations",
        base: &[("default", r(CN)), ("read", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SEARCH_LOGS,
        annotations: ann(Some(false), Some(true)),
        decision: ALLOW,
        category: READ_CFG,
        by: By::Winner(BASE, "read"),
    },
    Case {
        name: "readOnlyHint gives read",
        base: &[("default", r(CN)), ("read", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: ann(Some(true), Some(true)),
        decision: ALLOW,
        category: (Category::Read, CategorySource::Annotation),
        by: By::Winner(BASE, "read"),
    },
    Case {
        name: "destructiveHint false gives write",
        base: &[("default", r(CN)), ("write", r(CE))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: ann(None, Some(false)),
        decision: CE,
        category: (Category::Write, CategorySource::Annotation),
        by: By::Winner(BASE, "write"),
    },
    Case {
        name: "other hints give destructive",
        base: &[("default", r(CN)), ("destructive", r(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: ann(Some(false), None),
        decision: DENY,
        category: (Category::Destructive, CategorySource::Annotation),
        by: By::Winner(BASE, "destructive"),
    },
    Case {
        name: "annotations without hints give the default",
        base: &[("default", r(CN)), ("destructive", r(DENY))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: ann(None, None),
        decision: DENY,
        category: (Category::Destructive, CategorySource::Default),
        by: By::Winner(BASE, "destructive"),
    },
    Case {
        name: "no annotations give the default",
        base: &[("default", r(CN)), ("read", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: None,
        decision: CN,
        category: (Category::Destructive, CategorySource::Default),
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "a category rule applies to a tool whose category comes from annotations",
        base: &[("default", r(ALLOW)), ("env.prod.read", l(CE))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: MERGE_REQUEST,
        annotations: ann(Some(true), None),
        decision: CE,
        category: (Category::Read, CategorySource::Annotation),
        by: By::Winner(BASE, "env.prod.read"),
    },
    // --- User hardening.
    Case {
        name: "user hardening raises",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[("env.prod.default", r(CE))],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Hardening("env.prod.default"),
    },
    Case {
        name: "a softer user rule never lowers",
        base: &[("default", r(DENY))],
        profile: &[],
        user: &[
            ("default", r(ALLOW)),
            ("env.prod.mcp.k8s-admin.tool.scale_deployment", r(ALLOW)),
        ],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::Winner(BASE, "default"),
    },
    Case {
        name: "user hardening above a lock",
        base: &[("default", r(ALLOW)), ("env.prod.write", l(CN))],
        profile: &[],
        user: &[("env.prod.mcp.k8s-admin.tool.scale_deployment", r(DENY))],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::Hardening("env.prod.mcp.k8s-admin.tool.scale_deployment"),
    },
    Case {
        name: "the most precise user rule is the hardening",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[("default", r(DENY)), ("env.prod.write", r(CE))],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Hardening("env.prod.write"),
    },
    Case {
        name: "user hardening of an unknown tool",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[("env.prod.mcp.k8s-admin.tool.drop_cluster", r(DENY))],
        profile_id: DEV,
        call: MYSTERY,
        annotations: None,
        decision: DENY,
        category: UNKNOWN,
        by: By::Hardening("env.prod.mcp.k8s-admin.tool.drop_cluster"),
    },
    Case {
        name: "a user lock flag has no floor effect",
        base: &[("default", r(ALLOW))],
        profile: &[],
        user: &[("default", l(DENY)), ("env.prod.default", r(CE))],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: CE,
        category: WRITE_CFG,
        by: By::Hardening("env.prod.default"),
    },
    // --- No candidate at all (a tree validation would reject): fail closed.
    Case {
        name: "no root default",
        base: &[("env.dev.default", r(ALLOW))],
        profile: &[],
        user: &[],
        profile_id: DEV,
        call: SCALE,
        annotations: None,
        decision: DENY,
        category: WRITE_CFG,
        by: By::NoRule,
    },
];

// Exigence : EX-004-01
// Exigence : EX-004-07
#[test]
fn table_of_cases() {
    for case in CASES {
        let config = config_with(tree(case.base), tree(case.profile));
        let user = tree(case.user);
        let resolution = resolve_in(&config, case.profile_id, &user, case.call, case.annotations);
        assert_eq!(resolution.decision, case.decision, "{}", case.name);
        assert_eq!(resolution.provenance.decision(), case.decision, "{}", case.name);
        assert_eq!(
            (resolution.category, resolution.category_source),
            case.category,
            "{}",
            case.name
        );
        check_by(case.name, case.profile_id, &resolution, case.by);
        // The rendering never fails and always names the decision.
        let text = resolution.describe(&config);
        assert!(text.starts_with(case.decision.label()), "{}: {text}", case.name);
        assert!(
            resolution.describe_detailed(&config).starts_with(&text),
            "{}",
            case.name
        );
    }
}

// Exigence : EX-004-01
#[test]
fn every_branch_of_the_table_is_covered() {
    let mut seen = std::collections::BTreeSet::new();
    for case in CASES {
        let config = config_with(tree(case.base), tree(case.profile));
        let resolution = resolve_in(&config, case.profile_id, &tree(case.user), case.call, case.annotations);
        let by = match resolution.provenance.decided_by() {
            DecidedBy::NoRule => "none".to_owned(),
            DecidedBy::Winner(o) => format!(
                "winner {:?} {:?} {}",
                o.level(),
                o.position.selector.rank(),
                o.layer.rank()
            ),
            DecidedBy::Floor(o) => format!("floor {:?} {}", o.level(), o.layer.rank()),
            DecidedBy::UnknownTools(u) => format!("unknown {:?}", u.decision),
            DecidedBy::Hardening(_) => "hardening".to_owned(),
        };
        seen.insert(by);
        seen.insert(format!("source {:?}", resolution.category_source));
    }
    for expected in [
        "none",
        "hardening",
        "unknown Deny",
        "unknown Confirm(Notification)",
        "source Config",
        "source Annotation",
        "source Default",
        "source Unknown",
        "floor Root 0",
        "floor Root 1",
        "floor Env 0",
        "floor Mcp 0",
        "floor Tool 0",
        "winner Root 0 0",
        "winner Root 0 1",
        "winner Root 1 0",
        "winner Env 0 0",
        "winner Env 1 1",
        "winner Mcp 0 0",
        "winner Mcp 1 0",
        "winner Tool 2 0",
        "winner Tool 2 1",
    ] {
        assert!(seen.contains(expected), "no case decided by {expected}: {seen:?}");
    }
}

// Exigence : EX-004-02
#[test]
fn the_five_examples_of_the_spec() {
    let user = PolicyTree::default();

    // Root `destructive = deny`, MCP `default = allow`: a category rule beats a deeper default.
    let base = [
        ("default", r(CN)),
        ("destructive", r(DENY)),
        ("env.prod.mcp.k8s-admin.default", r(ALLOW)),
    ];
    let config = config_with(tree(&base), PolicyTree::default());
    let res = resolve_in(&config, DEV, &user, DELETE_POD, None);
    assert_eq!(res.decision, DENY);
    check_by("example 1", DEV, &res, By::Winner(BASE, "destructive"));

    // Same config + MCP `destructive = confirm`: the deeper category wins.
    let mut with_confirm = base.to_vec();
    with_confirm.push(("env.prod.mcp.k8s-admin.destructive", r(CN)));
    let config = config_with(tree(&with_confirm), PolicyTree::default());
    let res = resolve_in(&config, DEV, &user, DELETE_POD, None);
    assert_eq!(res.decision, CN);
    check_by(
        "example 2",
        DEV,
        &res,
        By::Winner(BASE, "env.prod.mcp.k8s-admin.destructive"),
    );

    // Env `write = confirm(notification), locked`; tool `rule = allow`: the lock is a floor. Validation would
    // reject this tool rule in a configuration (I1.21); the engine still applies the floor.
    let base = [
        ("default", r(ALLOW)),
        ("env.prod.write", l(CN)),
        ("env.prod.mcp.k8s-admin.tool.scale_deployment", r(ALLOW)),
    ];
    let config = config_with(tree(&base), PolicyTree::default());
    let res = resolve_in(&config, DEV, &user, SCALE, None);
    assert_eq!(res.decision, CN);
    check_by("example 3", DEV, &res, By::Floor(BASE, "env.prod.write"));
    assert_eq!(
        res.describe(&config),
        "confirmation (notification) — règle « write » de PROD, verrouillée"
    );

    // Env `write = confirm(notification), locked`; tool `rule = deny`: hardening under a lock.
    let base = [
        ("default", r(ALLOW)),
        ("env.prod.write", l(CN)),
        ("env.prod.mcp.k8s-admin.tool.scale_deployment", r(DENY)),
    ];
    let config = config_with(tree(&base), PolicyTree::default());
    let res = resolve_in(&config, DEV, &user, SCALE, None);
    assert_eq!(res.decision, DENY);
    check_by(
        "example 4",
        DEV,
        &res,
        By::Winner(BASE, "env.prod.mcp.k8s-admin.tool.scale_deployment"),
    );

    // Base `read = allow`; user `env.prod.default = confirm(notification)`: read in PROD.
    let config = config_with(tree(&[("default", r(CN)), ("read", r(ALLOW))]), PolicyTree::default());
    let hardened = tree(&[("env.prod.default", r(CN))]);
    let res = resolve_in(&config, DEV, &hardened, SEARCH_LOGS, None);
    assert_eq!(res.decision, CN);
    check_by("example 5", DEV, &res, By::Hardening("env.prod.default"));
    assert_eq!(
        res.describe(&config),
        "confirmation (notification) — durcissement utilisateur « default » de PROD"
    );
    // Elsewhere than PROD, the base rule still applies.
    let res = resolve_in(&config, DEV, &hardened, ("rec", "kibana-logs", "search_logs"), None);
    assert_eq!(res.decision, ALLOW);
}

// Exigence : EX-004-07
#[test]
fn provenance_renders_in_french() {
    let base = tree(&[
        ("default", r(CN)),
        ("env.prod.mcp.kibana-logs.tool.delete_index", l(DENY)),
        ("env.prod.mcp.k8s-admin.default", r(ALLOW)),
    ]);
    let mut config = config_with(base, PolicyTree::default());
    config.profiles.get_mut(&ProfileId::new(RO).unwrap()).unwrap().policy = tree(&[("default", l(DENY))]);
    let none = PolicyTree::default();

    let res = resolve_in(&config, DEV, &none, DELETE_INDEX, None);
    assert_eq!(
        res.describe(&config),
        "refusé — règle de l'outil « delete_index » de Kibana Logs en PROD, verrouillée"
    );
    assert_eq!(
        res.describe_detailed(&config),
        "refusé — règle de l'outil « delete_index » de Kibana Logs en PROD, verrouillée ; \
         catégorie « destructive » (déclarée dans la config)"
    );

    let res = resolve_in(&config, RO, &none, SCALE, None);
    assert_eq!(
        res.describe(&config),
        "refusé — règle « default » de la racine, profil « Lecture seule », verrouillée"
    );
    assert_eq!(
        res.describe_detailed(&config),
        "refusé — règle « default » de la racine, profil « Lecture seule », verrouillée ; \
         règle la plus précise : règle « default » de Kubernetes Admin en PROD (autorisé) ; \
         catégorie « write » (déclarée dans la config)"
    );

    let res = resolve_in(&config, DEV, &none, MYSTERY, ann(Some(true), None));
    assert_eq!(
        res.describe(&config),
        "confirmation (notification) — outil inconnu de la config, profil « Développeur »"
    );
    assert!(
        res.describe_detailed(&config)
            .ends_with("catégorie « destructive » (outil inconnu de la config)")
    );

    let res = resolve_in(&config, DEV, &none, ("rec", "kibana-logs", "search_logs"), None);
    assert_eq!(
        res.describe(&config),
        "confirmation (notification) — règle « default » de la racine"
    );

    let config = config_with(PolicyTree::default(), PolicyTree::default());
    let res = resolve_in(&config, DEV, &none, SCALE, None);
    assert_eq!(
        res.describe(&config),
        "refusé — aucune règle applicable, refus par défaut"
    );
}

// Exigence : EX-004-10
#[test]
fn summary_of_the_embedded_config() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let none = PolicyTree::default();
    let support = &config.profiles[&ProfileId::new("support").unwrap()];
    let summary = PolicyEngine::new(&config, support, &none).summary(&env("prod"), &mcp("k8s-admin"));
    assert_eq!(summary.get(Category::Read).decision, ALLOW);
    assert_eq!(summary.get(Category::Write).decision, DENY);
    assert_eq!(summary.get(Category::Destructive).decision, DENY);
    assert_eq!(
        summary.get(Category::Destructive).describe(&config),
        "refusé — règle « destructive » de Kubernetes Admin en PROD, verrouillée"
    );
    for (i, category) in Category::ALL.into_iter().enumerate() {
        assert_eq!(summary.categories[i].category, category);
    }

    let dev = &config.profiles[&ProfileId::new("developpeur").unwrap()];
    let hardened = tree(&[("env.dev.mcp.k8s-admin.write", r(CE))]);
    let summary = PolicyEngine::new(&config, dev, &hardened).summary(&env("dev"), &mcp("k8s-admin"));
    assert_eq!(summary.get(Category::Read).decision, ALLOW);
    assert_eq!(summary.get(Category::Write).decision, CE);
    assert_eq!(summary.get(Category::Destructive).decision, CE);
}

/// One expected decision: (profile, env, MCP, tool, annotations, decision).
type Expected = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<ToolAnnotations>,
    Decision,
);

/// Decisions expected from the embedded configuration, without user hardening.
const EMBEDDED_DECISIONS: &[Expected] = &[
    // Developer in their environments: everything allowed, destructive confirmed in the chat, unknown by
    // notification.
    ("developpeur", "dev", "kibana-logs", "search_logs", None, ALLOW),
    ("developpeur", "dev", "k8s-admin", "scale_deployment", None, ALLOW),
    ("developpeur", "dev", "kibana-logs", "delete_index", None, CE),
    ("developpeur", "dev", "gitlab-ops", "get_merge_request", None, CE),
    (
        "developpeur",
        "dev",
        "gitlab-ops",
        "get_merge_request",
        ann(Some(true), None),
        ALLOW,
    ),
    ("developpeur", "dev", "kibana-logs", "unknown_tool", None, CN),
    ("developpeur", "int", "k8s-admin", "delete_pod", None, CE),
    ("developpeur", "int", "db-readonly", "run_select", None, ALLOW),
    ("developpeur", "rec", "jira", "add_comment", None, ALLOW),
    ("developpeur", "rec", "gitlab-ops", "delete_branch", None, CE),
    ("developpeur", "rec", "jira", "unknown_tool", None, CN),
    // Developer outside their environments: the base applies.
    ("developpeur", "qua", "kibana-logs", "delete_index", None, DENY),
    ("developpeur", "qua", "kibana-logs", "unknown_tool", None, DENY),
    ("developpeur", "perf", "k8s-admin", "scale_deployment", None, CN),
    ("developpeur", "pprod", "k8s-admin", "scale_deployment", None, CN),
    ("developpeur", "prod", "k8s-admin", "scale_deployment", None, CN),
    ("developpeur", "prod", "jira", "add_comment", None, CN),
    ("developpeur", "prod", "k8s-admin", "delete_pod", None, DENY),
    ("developpeur", "prod", "kibana-logs", "delete_index", None, DENY),
    ("developpeur", "prod", "kibana-logs", "search_logs", None, ALLOW),
    ("developpeur", "prod", "db-readonly", "run_select", None, CN),
    // Support.
    ("support", "prod", "k8s-admin", "scale_deployment", None, DENY),
    ("support", "prod", "gitlab-ops", "retry_job", None, CN),
    ("support", "rec", "kibana-logs", "unknown_tool", None, DENY),
    (
        "support",
        "rec",
        "gitlab-ops",
        "add_merge_request_note",
        ann(None, Some(false)),
        CN,
    ),
    // Read only.
    ("lecture-seule", "prod", "jira", "add_comment", None, DENY),
    ("lecture-seule", "dev", "k8s-admin", "scale_deployment", None, DENY),
    ("lecture-seule", "dev", "kibana-logs", "delete_index", None, DENY),
    ("lecture-seule", "prod", "kibana-logs", "search_logs", None, ALLOW),
    ("lecture-seule", "rec", "kibana-logs", "unknown_tool", None, DENY),
];

// Exigence : EX-004-13
#[test]
fn embedded_config_gives_the_expected_decisions() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let none = PolicyTree::default();
    for (profile, e, m, t, annotations, expected) in EMBEDDED_DECISIONS {
        let profile = &config.profiles[&ProfileId::new(*profile).unwrap()];
        let resolution =
            PolicyEngine::new(&config, profile, &none).resolve(&env(e), &mcp(m), &tool(t), annotations.as_ref());
        assert_eq!(
            resolution.decision,
            *expected,
            "{} / {e} / {m} / {t}: {}",
            profile.id.as_str(),
            resolution.describe(&config)
        );
    }
}

// Exigence : EX-004-10
#[test]
fn summary_ignores_tool_rules() {
    let config = config_with(
        tree(&[("default", r(CN)), ("env.prod.mcp.k8s-admin.tool.delete_pod", r(ALLOW))]),
        PolicyTree::default(),
    );
    let profile = &config.profiles[&ProfileId::new(DEV).unwrap()];
    let none = PolicyTree::default();
    let summary = PolicyEngine::new(&config, profile, &none).summary(&env("prod"), &mcp("k8s-admin"));
    assert_eq!(summary.get(Category::Destructive).decision, CN);
}

/// Every rule of the base and of `profile` that is a candidate for the call, found through
/// `RulePosition::applies_to` rather than the engine's walk.
fn locked_candidates(
    config: &Config,
    profile: &crate::config::Profile,
    call: (&EnvId, &McpId, &ToolName),
    category: Category,
) -> Vec<(Layer, RulePosition, Rule)> {
    let layers = [
        (Layer::Base, &config.policy),
        (Layer::Profile(profile.id.clone()), &profile.policy),
    ];
    let mut out = Vec::new();
    for (layer, tree) in layers {
        for (position, rule) in tree.rules() {
            if rule.locked && position.applies_to(call.0, call.1, Some(call.2), category) {
                out.push((layer.clone(), position, rule));
            }
        }
    }
    out
}

// Exigence : EX-004-09
/// The validation (I1.21) and the floor tell the same story on the embedded configuration: the final decision is
/// never softer than a locked candidate, and whenever the floor has to raise the winner, the validator could not
/// have rejected that winner (the lock does not cover it, or it is a profile lock over a base rule).
#[test]
fn floor_agrees_with_validation_on_the_embedded_config() {
    let config = Config::embedded(crate::config::TEST_MODULES).unwrap();
    let none = PolicyTree::default();
    let annotations = [
        None,
        ann(Some(true), None),
        ann(None, Some(false)),
        ann(None, Some(true)),
    ];
    let mut raised = 0;
    let mut checked = 0;
    for profile in config.profiles.values() {
        let engine = PolicyEngine::new(&config, profile, &none);
        for m in &config.mcp {
            for e in m.instances.keys() {
                for (name, tool_config) in &m.tools {
                    for a in &annotations {
                        let res = engine.resolve(e, &m.id, name, a.as_ref());
                        assert!(res.known());
                        checked += 1;
                        for (_, position, rule) in locked_candidates(&config, profile, (e, &m.id, name), res.category) {
                            assert!(
                                res.decision >= rule.decision,
                                "{}/{e}/{}/{name}: {:?} below lock {position:?}",
                                profile.id,
                                m.id,
                                res.decision
                            );
                        }
                        if let DecidedBy::Floor(floor) = res.provenance.decided_by() {
                            raised += 1;
                            let winner = res.provenance.winner.as_ref().unwrap();
                            let validator_sees_it = floor.position.covers(&winner.position, tool_config.category)
                                && !(matches!(floor.layer, Layer::Profile(_)) && winner.layer == Layer::Base);
                            assert!(
                                !validator_sees_it,
                                "the validator should have rejected {winner:?} under {floor:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 0);
    // `lecture-seule` locks `write = deny` at its root, above the base `env.prod.write` rule.
    assert!(
        raised > 0,
        "the floor never raised anything: the test would prove nothing"
    );
}

// Exigence : EX-004-09
/// A configuration whose floor would raise a covered rule of the same layer does not load.
#[test]
fn validation_rejects_what_only_the_floor_would_fix() {
    let toml = crate::config::EMBEDDED_TOML.replace(
        "[policy.env.prod.mcp.db-readonly.tool.run_select]",
        "[policy.env.prod.mcp.k8s-admin.tool.scale_deployment]\nrule = { decision = \"allow\" }\n\n\
         [policy.env.prod.mcp.db-readonly.tool.run_select]",
    );
    let errors = Config::from_toml(&toml, crate::config::TEST_MODULES).unwrap_err();
    assert!(
        errors
            .errors
            .iter()
            .any(|e| e.path == "policy.env.prod.mcp.k8s-admin.tool.scale_deployment.rule"),
        "{errors}"
    );
}

// --- Property tests on generated trees -------------------------------------------------------------------------

const ENVS: [&str; 2] = ["dev", "prod"];
/// MCPs with an instance in both environments, with tools of each static category and one without.
const MCPS: [(&str, &[&str]); 2] = [
    ("k8s-admin", &["list_pods", "scale_deployment", "delete_pod"]),
    ("gitlab-ops", &["get_merge_request", "delete_branch"]),
];
const UNKNOWN_TOOL: &str = "drop_everything";
const PROFILES: [&str; 2] = [DEV, RO];

/// Every position of the generated universe. `with_unknown` adds tool rules on the unknown tool (only the user
/// tree may hold them: validation forbids rules on undeclared tools).
fn positions(with_unknown: bool) -> Vec<RulePosition> {
    let selectors = ["default", "read", "write", "destructive"];
    let mut out = Vec::new();
    for s in selectors {
        out.push(position(s));
    }
    for e in ENVS {
        for s in selectors {
            out.push(position(&format!("env.{e}.{s}")));
        }
        for (m, tools) in MCPS {
            for s in selectors {
                out.push(position(&format!("env.{e}.mcp.{m}.{s}")));
            }
            for t in tools.iter().copied().chain(with_unknown.then_some(UNKNOWN_TOOL)) {
                out.push(position(&format!("env.{e}.mcp.{m}.tool.{t}")));
            }
        }
    }
    out
}

fn decision() -> impl Strategy<Value = Decision> {
    prop_oneof![Just(ALLOW), Just(CE), Just(CN), Just(DENY)]
}

fn rule() -> impl Strategy<Value = Rule> {
    (decision(), any::<bool>()).prop_map(|(decision, locked)| Rule { decision, locked })
}

/// A tree over the universe, each position holding a rule with probability `density`.
pub(crate) fn tree_strategy(with_unknown: bool, density: f64) -> impl Strategy<Value = PolicyTree> {
    let all = positions(with_unknown);
    proptest::collection::vec(proptest::option::weighted(density, rule()), all.len()).prop_map(move |rules| {
        let mut tree = PolicyTree::default();
        for (position, rule) in all.iter().zip(rules) {
            if let Some(rule) = rule {
                insert(&mut tree, position, rule);
            }
        }
        tree
    })
}

/// A config tree: validation always imposes a root `default`.
fn base_strategy() -> impl Strategy<Value = PolicyTree> {
    (tree_strategy(false, 0.3), rule()).prop_map(|(mut tree, root)| {
        tree.rules.default = Some(root);
        tree
    })
}

#[derive(Debug, Clone)]
struct Call {
    profile: &'static str,
    env: EnvId,
    mcp: McpId,
    tool: ToolName,
    annotations: Option<ToolAnnotations>,
}

fn call_strategy() -> impl Strategy<Value = Call> {
    let hint = proptest::option::of(any::<bool>());
    (
        proptest::sample::select(PROFILES.to_vec()),
        proptest::sample::select(ENVS.to_vec()),
        proptest::sample::select(MCPS.to_vec()),
        any::<proptest::sample::Index>(),
        proptest::option::of((hint.clone(), hint)),
    )
        .prop_map(|(profile, e, (m, tools), index, hints)| {
            let mut names: Vec<&str> = tools.to_vec();
            names.push(UNKNOWN_TOOL);
            Call {
                profile,
                env: env(e),
                mcp: mcp(m),
                tool: tool(names[index.index(names.len())]),
                annotations: hints.map(|(read_only_hint, destructive_hint)| ToolAnnotations {
                    read_only_hint,
                    destructive_hint,
                }),
            }
        })
}

fn resolve_call(config: &Config, user: &PolicyTree, call: &Call) -> Resolution {
    let profile = &config.profiles[&ProfileId::new(call.profile).unwrap()];
    PolicyEngine::new(config, profile, user).resolve(&call.env, &call.mcp, &call.tool, call.annotations.as_ref())
}

/// Brute-force reference: candidates through `PolicyTree::rules` and `RulePosition::applies_to`, then the spec's
/// steps written out without the engine's helpers.
fn oracle(config: &Config, user: &PolicyTree, call: &Call, category: Category, known: bool) -> Decision {
    let profile = &config.profiles[&ProfileId::new(call.profile).unwrap()];
    let candidates = |tree: &PolicyTree, layer: u8| {
        tree.rules()
            .into_iter()
            .filter(|(p, _)| p.applies_to(&call.env, &call.mcp, Some(&call.tool), category))
            .map(|(p, rule)| ((p.selector.rank(), p.level(), layer), rule))
            .collect::<Vec<_>>()
    };
    let mut config_candidates = candidates(&config.policy, 0);
    config_candidates.extend(candidates(&profile.policy, 1));
    config_candidates.sort_by_key(|(key, _)| *key);
    let winner = config_candidates.last().map_or(DENY, |(_, rule)| rule.decision);
    let floor = config_candidates
        .iter()
        .filter(|(_, rule)| rule.locked)
        .map(|(_, rule)| rule.decision)
        .max();
    let mut decision = floor.map_or(winner, |f| winner.max(f));
    if !known {
        decision = decision.max(profile.unknown_tools);
    }
    let mut user_candidates = candidates(user, 2);
    user_candidates.sort_by_key(|(key, _)| *key);
    if let Some((_, hardening)) = user_candidates.last() {
        decision = decision.max(hardening.decision);
    }
    decision
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // Exigence : EX-004-03
    #[test]
    fn user_rules_never_lower_the_severity(
        base in base_strategy(),
        profile in tree_strategy(false, 0.2),
        user in tree_strategy(true, 0.15),
        extra_position in any::<proptest::sample::Index>(),
        extra_rule in rule(),
        call in call_strategy(),
    ) {
        let config = config_with(base, profile);
        let none = PolicyTree::default();
        let without = resolve_call(&config, &none, &call);
        let with = resolve_call(&config, &user, &call);
        prop_assert!(with.decision >= without.decision);
        prop_assert_eq!(with.provenance.config_decision(), without.provenance.config_decision());

        let all = positions(true);
        let mut more = user.clone();
        insert(&mut more, &all[extra_position.index(all.len())], extra_rule);
        let with_more = resolve_call(&config, &more, &call);
        prop_assert!(with_more.decision >= without.decision);

        // A single user rule on an empty tree: never below the configuration.
        let mut single = PolicyTree::default();
        insert(&mut single, &all[extra_position.index(all.len())], extra_rule);
        prop_assert!(resolve_call(&config, &single, &call).decision >= without.decision);
    }

    // Exigence : EX-004-04
    #[test]
    fn a_locked_rule_is_a_lower_bound(
        base in base_strategy(),
        profile in tree_strategy(false, 0.3),
        user in tree_strategy(true, 0.1),
        call in call_strategy(),
    ) {
        let config = config_with(base, profile);
        let res = resolve_call(&config, &user, &call);
        let profile = &config.profiles[&ProfileId::new(call.profile).unwrap()];
        for (_, position, rule) in locked_candidates(&config, profile, (&call.env, &call.mcp, &call.tool), res.category) {
            prop_assert!(res.decision >= rule.decision, "{:?} below lock {:?}", res.decision, position);
            prop_assert!(res.provenance.config_decision() >= rule.decision);
        }
    }

    // Exigence : EX-004-05
    #[test]
    fn resolution_is_total_and_deterministic(
        base in base_strategy(),
        profile in tree_strategy(false, 0.3),
        user in tree_strategy(true, 0.15),
        call in call_strategy(),
    ) {
        let config = config_with(base, profile);
        let first = resolve_call(&config, &user, &call);
        let second = resolve_call(&config, &user, &call);
        prop_assert_eq!(&first, &second);
        prop_assert_eq!(first.decision, first.provenance.decision());
        prop_assert!(first.provenance.winner.is_some());
        let expected = oracle(&config, &user, &call, first.category, first.known());
        prop_assert_eq!(first.decision, expected);
        // A clone of the inputs gives the same answer, rendering included.
        let copy = config.clone();
        let again = resolve_call(&copy, &user.clone(), &call);
        prop_assert_eq!(first.describe(&config), again.describe(&copy));
    }

    // Exigence : EX-004-05
    #[test]
    fn resolution_is_total_without_root_default(
        base in tree_strategy(false, 0.2),
        profile in tree_strategy(false, 0.2),
        user in tree_strategy(true, 0.1),
        call in call_strategy(),
    ) {
        let config = config_with(base, profile);
        let res = resolve_call(&config, &user, &call);
        prop_assert_eq!(res.decision, oracle(&config, &user, &call, res.category, res.known()));
        if res.provenance.winner.is_none() {
            prop_assert_eq!(res.decision, DENY);
        }
    }

    // Exigence : EX-004-06
    #[test]
    fn unknown_tool_is_never_softer_than_destructive(
        base in base_strategy(),
        profile in tree_strategy(false, 0.3),
        user in tree_strategy(false, 0.15),
        call in call_strategy(),
    ) {
        let mut call = call;
        call.tool = tool(UNKNOWN_TOOL);
        let config = config_with(base, profile);
        let unknown = resolve_call(&config, &user, &call);
        prop_assert_eq!(unknown.category, Category::Destructive);
        prop_assert_eq!(unknown.category_source, CategorySource::Unknown);

        // The same path, declared as a destructive tool.
        let mut declared = config.clone();
        let m = declared.mcp.iter_mut().find(|m| m.id == call.mcp).unwrap();
        m.tools.insert(call.tool.clone(), ToolConfig { category: Some(Category::Destructive) });
        let known = resolve_call(&declared, &user, &call);
        prop_assert_eq!(known.category_source, CategorySource::Config);
        prop_assert!(unknown.decision >= known.decision);

        let profile = &config.profiles[&ProfileId::new(call.profile).unwrap()];
        let summary = PolicyEngine::new(&config, profile, &user).summary(&call.env, &call.mcp);
        prop_assert!(unknown.decision >= summary.get(Category::Destructive).decision);
        prop_assert!(unknown.decision >= profile.unknown_tools);
    }
}

// Exigence : EX-004-03
/// Within the user tree, the most precise rule wins (step 6 applies steps 1 and 2 to it): a more precise and softer
/// user rule replaces a broader one, but never goes below the configuration (EX-004-03).
#[test]
fn a_more_precise_user_rule_replaces_a_broader_one() {
    let config = config_with(tree(&[("default", r(ALLOW))]), PolicyTree::default());
    let broad = tree(&[("env.prod.default", r(DENY))]);
    assert_eq!(resolve_in(&config, DEV, &broad, SCALE, None).decision, DENY);
    let narrower = tree(&[("env.prod.default", r(DENY)), ("env.prod.mcp.k8s-admin.write", r(CE))]);
    assert_eq!(resolve_in(&config, DEV, &narrower, SCALE, None).decision, CE);
}
