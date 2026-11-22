//! Typed policy trees as declared in the configuration (`[policy]` and `[profiles.<id>.policy]`) and in the user
//! settings (hardening tree), with the vocabulary shared by the validator and the policy engine.
//!
//! Resolution of a decision belongs to `crate::policy`. Where a rule sits and which calls it applies to
//! ([`RulePosition`]) is defined once here, so that the locked-rule coverage check of the validator and the floor
//! applied by the engine agree.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::ids::{EnvId, McpId, ToolName};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    Elicitation,
    Notification,
}

/// A policy decision. Ordered by severity: `Allow < Confirm(Elicitation) < Confirm(Notification) < Deny`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Decision {
    Allow,
    Confirm(Channel),
    Deny,
}

impl Decision {
    /// Short French label, shown in the journal and the interface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Allow => "autorisé",
            Self::Confirm(Channel::Elicitation) => "confirmation (elicitation)",
            Self::Confirm(Channel::Notification) => "confirmation (notification)",
            Self::Deny => "refusé",
        }
    }

    pub fn severity(self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Confirm(Channel::Elicitation) => 1,
            Self::Confirm(Channel::Notification) => 2,
            Self::Deny => 3,
        }
    }
}

impl Ord for Decision {
    fn cmp(&self, other: &Self) -> Ordering {
        self.severity().cmp(&other.severity())
    }
}

impl PartialOrd for Decision {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Read,
    Write,
    Destructive,
}

impl Category {
    pub const ALL: [Self; 3] = [Self::Read, Self::Write, Self::Destructive];

    pub fn key(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    pub decision: Decision,
    pub locked: bool,
}

/// Rules attached to one node of a tree: the `default` selector and one optional rule per category.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleSet {
    pub default: Option<Rule>,
    pub read: Option<Rule>,
    pub write: Option<Rule>,
    pub destructive: Option<Rule>,
}

impl RuleSet {
    pub fn category(&self, category: Category) -> Option<Rule> {
        match category {
            Category::Read => self.read,
            Category::Write => self.write,
            Category::Destructive => self.destructive,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpPolicy {
    pub rules: RuleSet,
    /// `tool.<name>.rule`: applies to that tool whatever its category.
    pub tools: BTreeMap<ToolName, Rule>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvPolicy {
    pub rules: RuleSet,
    pub mcp: BTreeMap<McpId, McpPolicy>,
}

/// A policy tree: root → environment → MCP → tool.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyTree {
    pub rules: RuleSet,
    pub env: BTreeMap<EnvId, EnvPolicy>,
}

/// Depth of a node, from the least to the most precise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Root,
    Env,
    Mcp,
    Tool,
}

/// What a rule selects within its node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Selector {
    Default,
    Category(Category),
    /// `tool.<name>.rule`, whatever the tool's category.
    Tool(ToolName),
}

impl Selector {
    /// Precision rank: tool rule > category > default.
    pub fn rank(&self) -> u8 {
        match self {
            Self::Default => 0,
            Self::Category(_) => 1,
            Self::Tool(_) => 2,
        }
    }
}

/// Where a rule sits in a tree. A tool rule always has both `env` and `mcp`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RulePosition {
    pub env: Option<EnvId>,
    pub mcp: Option<McpId>,
    pub selector: Selector,
}

impl RulePosition {
    pub fn level(&self) -> Level {
        match (&self.selector, &self.env, &self.mcp) {
            (Selector::Tool(_), _, _) => Level::Tool,
            (_, _, Some(_)) => Level::Mcp,
            (_, Some(_), None) => Level::Env,
            (_, None, None) => Level::Root,
        }
    }

    /// Whether this rule is a candidate for a call on `tool` (`None` for the MCP-wide summary) of `category`.
    pub fn applies_to(&self, env: &EnvId, mcp: &McpId, tool: Option<&ToolName>, category: Category) -> bool {
        let node = self.env.as_ref().is_none_or(|e| e == env) && self.mcp.as_ref().is_none_or(|m| m == mcp);
        let selector = match &self.selector {
            Selector::Default => true,
            Selector::Category(c) => *c == category,
            Selector::Tool(t) => tool == Some(t),
        };
        node && selector
    }

    /// Whether `self` is a candidate every time `other` is, `other_category` being the static category of the tool
    /// when `other` is a tool rule. Without a static category, whether a category rule applies to that tool is only
    /// known at run time, so a category rule does not cover it.
    pub fn covers(&self, other: &RulePosition, other_category: Option<Category>) -> bool {
        let node = (self.env.is_none() || self.env == other.env) && (self.mcp.is_none() || self.mcp == other.mcp);
        let selector = match (&self.selector, &other.selector) {
            (Selector::Default, _) => true,
            (Selector::Category(a), Selector::Category(b)) => a == b,
            (Selector::Category(a), Selector::Tool(_)) => other_category == Some(*a),
            (Selector::Tool(a), Selector::Tool(b)) => a == b,
            _ => false,
        };
        node && selector
    }
}

impl PolicyTree {
    /// Every rule of the tree with its position, in a stable order (root first, then environments, MCPs and tools
    /// in identifier order).
    pub fn rules(&self) -> Vec<(RulePosition, Rule)> {
        fn push_set(out: &mut Vec<(RulePosition, Rule)>, set: &RuleSet, env: Option<&EnvId>, mcp: Option<&McpId>) {
            let entries = [
                (Selector::Default, set.default),
                (Selector::Category(Category::Read), set.read),
                (Selector::Category(Category::Write), set.write),
                (Selector::Category(Category::Destructive), set.destructive),
            ];
            for (selector, rule) in entries {
                if let Some(rule) = rule {
                    let position = RulePosition {
                        env: env.cloned(),
                        mcp: mcp.cloned(),
                        selector,
                    };
                    out.push((position, rule));
                }
            }
        }

        let mut out = Vec::new();
        push_set(&mut out, &self.rules, None, None);
        for (env_id, env) in &self.env {
            push_set(&mut out, &env.rules, Some(env_id), None);
            for (mcp_id, mcp) in &env.mcp {
                push_set(&mut out, &mcp.rules, Some(env_id), Some(mcp_id));
                for (tool, rule) in &mcp.tools {
                    let position = RulePosition {
                        env: Some(env_id.clone()),
                        mcp: Some(mcp_id.clone()),
                        selector: Selector::Tool(tool.clone()),
                    };
                    out.push((position, *rule));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_order() {
        let ordered = [
            Decision::Allow,
            Decision::Confirm(Channel::Elicitation),
            Decision::Confirm(Channel::Notification),
            Decision::Deny,
        ];
        assert!(ordered.windows(2).all(|w| w[0] < w[1]));
    }
}
