//! JSON shape of the settings file. The hardening tree has the shape of the configuration trees
//! (`default`, `read`, `write`, `destructive`, `env.<id>.mcp.<id>.tool.<name>.rule`), without `locked`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{SETTINGS_SCHEMA_VERSION, UserSettings};
use crate::config::{
    Channel, Decision, EnvId, EnvPolicy, IdError, McpId, McpPolicy, PolicyTree, ProfileId, Rule, RuleSet, ToolName,
};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SettingsFile {
    schema_version: u32,
    profile: Option<String>,
    exposed_environments: Vec<String>,
    disabled_mcp: Vec<String>,
    hardening: TreeFile,
    autostart: bool,
    beta_channel: bool,
    gateway_suspended: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DecisionFile {
    Allow,
    Confirm,
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ChannelFile {
    Elicitation,
    Notification,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    decision: DecisionFile,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    channel: Option<ChannelFile>,
}

/// The four selectors of a node. Spelled out in every node: `#[serde(flatten)]` does not combine with
/// `deny_unknown_fields`.
macro_rules! node_file {
    ($name:ident { $($child:ident: $child_ty:ty),* }) => {
        #[derive(Debug, Default, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct $name {
            #[serde(default, skip_serializing_if = "Option::is_none")]
            default: Option<RuleFile>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            read: Option<RuleFile>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            write: Option<RuleFile>,
            #[serde(default, skip_serializing_if = "Option::is_none")]
            destructive: Option<RuleFile>,
            $(
                #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
                $child: $child_ty,
            )*
        }

        impl $name {
            fn rule_set(&self) -> RuleSetFile {
                RuleSetFile {
                    default: self.default,
                    read: self.read,
                    write: self.write,
                    destructive: self.destructive,
                }
            }
        }
    };
}

#[derive(Debug, Clone, Copy, Default)]
struct RuleSetFile {
    default: Option<RuleFile>,
    read: Option<RuleFile>,
    write: Option<RuleFile>,
    destructive: Option<RuleFile>,
}

node_file!(TreeFile { env: BTreeMap<String, EnvFile> });
node_file!(EnvFile { mcp: BTreeMap<String, McpFile> });
node_file!(McpFile { tool: BTreeMap<String, ToolFile> });

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolFile {
    rule: RuleFile,
}

/// Why a well-formed JSON document is not valid settings. French; quotes no value of the file except identifiers
/// already validated.
pub(super) type Invalid = String;

fn rule_to_file(rule: Rule) -> RuleFile {
    let (decision, channel) = match rule.decision {
        Decision::Allow => (DecisionFile::Allow, None),
        Decision::Confirm(Channel::Elicitation) => (DecisionFile::Confirm, Some(ChannelFile::Elicitation)),
        Decision::Confirm(Channel::Notification) => (DecisionFile::Confirm, Some(ChannelFile::Notification)),
        Decision::Deny => (DecisionFile::Deny, None),
    };
    RuleFile { decision, channel }
}

fn rule_from_file(path: &str, rule: RuleFile) -> Result<Rule, Invalid> {
    let decision = match (rule.decision, rule.channel) {
        (DecisionFile::Allow, None) => Decision::Allow,
        (DecisionFile::Deny, None) => Decision::Deny,
        (DecisionFile::Confirm, Some(ChannelFile::Elicitation)) => Decision::Confirm(Channel::Elicitation),
        (DecisionFile::Confirm, Some(ChannelFile::Notification)) => Decision::Confirm(Channel::Notification),
        (DecisionFile::Confirm, None) => return Err(format!("{path} : canal obligatoire avec « confirm »")),
        (DecisionFile::Allow | DecisionFile::Deny, Some(_)) => {
            return Err(format!("{path} : canal admis seulement avec « confirm »"));
        }
    };
    Ok(Rule {
        decision,
        locked: false,
    })
}

fn set_to_file(set: &RuleSet) -> RuleSetFile {
    RuleSetFile {
        default: set.default.map(rule_to_file),
        read: set.read.map(rule_to_file),
        write: set.write.map(rule_to_file),
        destructive: set.destructive.map(rule_to_file),
    }
}

fn set_from_file(path: &str, set: RuleSetFile) -> Result<RuleSet, Invalid> {
    let one =
        |name: &str, rule: Option<RuleFile>| rule.map(|r| rule_from_file(&format!("{path}.{name}"), r)).transpose();
    Ok(RuleSet {
        default: one("default", set.default)?,
        read: one("read", set.read)?,
        write: one("write", set.write)?,
        destructive: one("destructive", set.destructive)?,
    })
}

fn id<T>(path: &str, value: String, make: fn(String) -> Result<T, IdError>) -> Result<T, Invalid> {
    make(value).map_err(|e| format!("{path} : identifiant invalide ({e})"))
}

fn tree_to_file(tree: &PolicyTree) -> TreeFile {
    let r = set_to_file(&tree.rules);
    let env = tree
        .env
        .iter()
        .map(|(env_id, env)| {
            let r = set_to_file(&env.rules);
            let mcp = env
                .mcp
                .iter()
                .map(|(mcp_id, mcp)| {
                    let r = set_to_file(&mcp.rules);
                    let tool = mcp
                        .tools
                        .iter()
                        .map(|(name, rule)| {
                            (
                                name.as_str().to_owned(),
                                ToolFile {
                                    rule: rule_to_file(*rule),
                                },
                            )
                        })
                        .collect();
                    let file = McpFile {
                        default: r.default,
                        read: r.read,
                        write: r.write,
                        destructive: r.destructive,
                        tool,
                    };
                    (mcp_id.as_str().to_owned(), file)
                })
                .collect();
            let file = EnvFile {
                default: r.default,
                read: r.read,
                write: r.write,
                destructive: r.destructive,
                mcp,
            };
            (env_id.as_str().to_owned(), file)
        })
        .collect();
    TreeFile {
        default: r.default,
        read: r.read,
        write: r.write,
        destructive: r.destructive,
        env,
    }
}

fn tree_from_file(tree: TreeFile) -> Result<PolicyTree, Invalid> {
    let mut out = PolicyTree {
        rules: set_from_file("hardening", tree.rule_set())?,
        env: BTreeMap::new(),
    };
    for (env_name, env) in tree.env {
        let env_id = id("hardening.env", env_name, EnvId::new)?;
        let env_path = format!("hardening.env.{env_id}");
        let mut env_node = EnvPolicy {
            rules: set_from_file(&env_path, env.rule_set())?,
            mcp: BTreeMap::new(),
        };
        for (mcp_name, mcp) in env.mcp {
            let mcp_id = id(&format!("{env_path}.mcp"), mcp_name, McpId::new)?;
            let mcp_path = format!("{env_path}.mcp.{mcp_id}");
            let mut mcp_node = McpPolicy {
                rules: set_from_file(&mcp_path, mcp.rule_set())?,
                tools: BTreeMap::new(),
            };
            for (tool_name, tool) in mcp.tool {
                let name = id(&format!("{mcp_path}.tool"), tool_name, ToolName::new)?;
                let rule = rule_from_file(&format!("{mcp_path}.tool.{name}.rule"), tool.rule)?;
                mcp_node.tools.insert(name, rule);
            }
            env_node.mcp.insert(mcp_id, mcp_node);
        }
        out.env.insert(env_id, env_node);
    }
    Ok(out)
}

impl SettingsFile {
    pub(super) fn from_settings(settings: &UserSettings) -> Self {
        Self {
            schema_version: settings.schema_version,
            profile: settings.profile.as_ref().map(|p| p.as_str().to_owned()),
            exposed_environments: settings
                .exposed_environments
                .iter()
                .map(|e| e.as_str().to_owned())
                .collect(),
            disabled_mcp: settings.disabled_mcp.iter().map(|m| m.as_str().to_owned()).collect(),
            hardening: tree_to_file(&settings.hardening),
            autostart: settings.autostart,
            beta_channel: settings.beta_channel,
            gateway_suspended: settings.gateway_suspended,
        }
    }

    pub(super) fn into_settings(self) -> Result<UserSettings, Invalid> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "version de schéma {} non prise en charge (attendue : {SETTINGS_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        let profile = self.profile.map(|p| id("profile", p, ProfileId::new)).transpose()?;
        let exposed_environments = self
            .exposed_environments
            .into_iter()
            .map(|e| id("exposed_environments", e, EnvId::new))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let disabled_mcp = self
            .disabled_mcp
            .into_iter()
            .map(|m| id("disabled_mcp", m, McpId::new))
            .collect::<Result<BTreeSet<_>, _>>()?;
        Ok(UserSettings {
            schema_version: self.schema_version,
            profile,
            exposed_environments,
            disabled_mcp,
            hardening: tree_from_file(self.hardening)?,
            autostart: self.autostart,
            beta_channel: self.beta_channel,
            gateway_suspended: self.gateway_suspended,
        })
    }
}
