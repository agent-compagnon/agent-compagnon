//! Typed policy trees as declared in the configuration (`[policy]` and `[profiles.<id>.policy]`).
//!
//! Resolution of a decision belongs to the policy engine (step 2c); this module only holds validated data and the
//! severity order the validator needs.

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
