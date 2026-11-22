//! Policy engine: decides, for each tool call, whether it is allowed, needs a confirmation or is denied.
//!
//! Three trees of the same shape take part: the base (`[policy]`), the active profile (`[profiles.<id>.policy]`)
//! and the user hardening tree (local settings). The algorithm is the one of spec 004:
//!
//! 1. candidates: every rule of the base and the profile whose node is on the call path and whose selector matches;
//! 2. winner: the most precise candidate (selector, then depth, then layer);
//! 3. floor: the most severe locked candidate, all layers and levels together;
//! 4. config decision = `max(winner, floor)`;
//! 5. unknown tool: category `Destructive`, then `max(decision, unknown_tools of the profile)`;
//! 6. user hardening: steps 1 and 2 on the user tree alone, then `max(decision, result)`.
//!
//! Every resolution carries its provenance, rendered in French by [`Resolution::describe`].

mod explain;

#[cfg(test)]
pub(crate) mod tests;

use crate::config::{Config, EnvId, McpId, Profile, ProfileId, RuleSet, ToolName};

pub use crate::config::{Category, Channel, Decision, Level, PolicyTree, Rule, RulePosition, Selector};

/// The part of the MCP tool annotations the category is derived from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolAnnotations {
    pub read_only_hint: Option<bool>,
    pub destructive_hint: Option<bool>,
}

impl ToolAnnotations {
    /// `None` when no hint is present: the annotations then say nothing.
    pub fn category(&self) -> Option<Category> {
        if self.read_only_hint.is_none() && self.destructive_hint.is_none() {
            return None;
        }
        Some(if self.read_only_hint == Some(true) {
            Category::Read
        } else if self.destructive_hint == Some(false) {
            Category::Write
        } else {
            Category::Destructive
        })
    }
}

/// Where the category of a tool comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CategorySource {
    /// `category` declared in `[mcp.tools]`.
    Config,
    /// Derived from the server annotations of a declared tool without a category.
    Annotation,
    /// Declared tool without a category nor usable annotations: `Destructive`.
    Default,
    /// Tool unknown to the configuration: always `Destructive`, whatever its annotations.
    Unknown,
}

/// The tree a rule comes from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Layer {
    Base,
    Profile(ProfileId),
    User,
}

impl Layer {
    fn rank(&self) -> u8 {
        match self {
            Self::Base => 0,
            Self::Profile(_) => 1,
            Self::User => 2,
        }
    }
}

/// A rule together with where it was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOrigin {
    pub layer: Layer,
    pub position: RulePosition,
    pub rule: Rule,
}

impl RuleOrigin {
    pub fn level(&self) -> Level {
        self.position.level()
    }

    /// Lexicographic precision: selector, then depth, then layer. Unique among the candidates of one call, since a
    /// node holds at most one rule per selector.
    fn precision(&self) -> (u8, Level, u8) {
        (self.position.selector.rank(), self.level(), self.layer.rank())
    }
}

/// The `unknown_tools` setting of the profile, applied to a tool unknown to the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTools {
    pub profile: ProfileId,
    pub decision: Decision,
}

/// Why a decision was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// Most precise candidate of the base and the profile. `None` only for a tree without a root `default`, which
    /// validation rejects: the decision is then `Deny`.
    pub winner: Option<RuleOrigin>,
    /// Most severe locked candidate, whether or not it raised the winner.
    pub floor: Option<RuleOrigin>,
    pub unknown_tools: Option<UnknownTools>,
    /// Most precise candidate of the user hardening tree.
    pub hardening: Option<RuleOrigin>,
}

/// The step that set the final decision. A later step only counts when it is strictly more severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy<'a> {
    /// No candidate at all: fail closed.
    NoRule,
    Winner(&'a RuleOrigin),
    Floor(&'a RuleOrigin),
    UnknownTools(&'a UnknownTools),
    Hardening(&'a RuleOrigin),
}

impl Provenance {
    /// `max(winner, floor)`, `Deny` without a winner.
    pub fn config_decision(&self) -> Decision {
        let winner = self.winner.as_ref().map_or(Decision::Deny, |w| w.rule.decision);
        self.floor.as_ref().map_or(winner, |f| winner.max(f.rule.decision))
    }

    pub fn decision(&self) -> Decision {
        self.steps().0
    }

    pub fn decided_by(&self) -> DecidedBy<'_> {
        self.steps().1
    }

    fn steps(&self) -> (Decision, DecidedBy<'_>) {
        let mut current = match &self.winner {
            Some(w) => (w.rule.decision, DecidedBy::Winner(w)),
            None => (Decision::Deny, DecidedBy::NoRule),
        };
        if let Some(floor) = &self.floor
            && floor.rule.decision > current.0
        {
            current = (floor.rule.decision, DecidedBy::Floor(floor));
        }
        if let Some(unknown) = &self.unknown_tools
            && unknown.decision > current.0
        {
            current = (unknown.decision, DecidedBy::UnknownTools(unknown));
        }
        if let Some(hardening) = &self.hardening
            && hardening.rule.decision > current.0
        {
            current = (hardening.rule.decision, DecidedBy::Hardening(hardening));
        }
        current
    }
}

/// The decision for one tool call and its explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub decision: Decision,
    pub category: Category,
    pub category_source: CategorySource,
    pub provenance: Provenance,
}

impl Resolution {
    /// Whether the tool is declared in the configuration for this environment.
    pub fn known(&self) -> bool {
        self.category_source != CategorySource::Unknown
    }
}

/// Decision of one category in [`PolicySummary`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryPolicy {
    pub category: Category,
    pub decision: Decision,
    pub provenance: Provenance,
}

/// Default policy of an MCP in an environment: the decision for a declared tool of each category that has no rule
/// of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySummary {
    pub env: EnvId,
    pub mcp: McpId,
    /// In the order of [`Category::ALL`].
    pub categories: [CategoryPolicy; 3],
}

impl PolicySummary {
    pub fn get(&self, category: Category) -> &CategoryPolicy {
        match category {
            Category::Read => &self.categories[0],
            Category::Write => &self.categories[1],
            Category::Destructive => &self.categories[2],
        }
    }
}

/// Resolves decisions for one profile and one user hardening tree.
#[derive(Debug, Clone, Copy)]
pub struct PolicyEngine<'a> {
    config: &'a Config,
    profile: &'a Profile,
    hardening: &'a PolicyTree,
}

impl<'a> PolicyEngine<'a> {
    pub fn new(config: &'a Config, profile: &'a Profile, hardening: &'a PolicyTree) -> Self {
        Self {
            config,
            profile,
            hardening,
        }
    }

    pub fn config(&self) -> &'a Config {
        self.config
    }

    pub fn profile(&self) -> &'a Profile {
        self.profile
    }

    /// Category of a tool and its source. A tool counts as known only if its MCP is declared with an instance in
    /// this environment and lists it in `[mcp.tools]`.
    pub fn category(
        &self,
        env: &EnvId,
        mcp: &McpId,
        tool: &ToolName,
        annotations: Option<&ToolAnnotations>,
    ) -> (Category, CategorySource) {
        let declared = self
            .config
            .mcp(mcp)
            .filter(|m| m.instances.contains_key(env))
            .and_then(|m| m.tools.get(tool));
        match declared {
            None => (Category::Destructive, CategorySource::Unknown),
            Some(config) => match config.category {
                Some(category) => (category, CategorySource::Config),
                None => match annotations.and_then(ToolAnnotations::category) {
                    Some(category) => (category, CategorySource::Annotation),
                    None => (Category::Destructive, CategorySource::Default),
                },
            },
        }
    }

    /// Decision for a call to `tool` of `mcp` in `env`. `annotations` are the server's, used only for a declared
    /// tool without a category. Total: every input gives a decision.
    pub fn resolve(
        &self,
        env: &EnvId,
        mcp: &McpId,
        tool: &ToolName,
        annotations: Option<&ToolAnnotations>,
    ) -> Resolution {
        let (category, category_source) = self.category(env, mcp, tool, annotations);
        let unknown = (category_source == CategorySource::Unknown).then(|| UnknownTools {
            profile: self.profile.id.clone(),
            decision: self.profile.unknown_tools,
        });
        let provenance = self.provenance(env, mcp, Some(tool), category, unknown);
        Resolution {
            decision: provenance.decision(),
            category,
            category_source,
            provenance,
        }
    }

    /// Default policy of `mcp` in `env`, one decision per category, user hardening included.
    pub fn summary(&self, env: &EnvId, mcp: &McpId) -> PolicySummary {
        let entry = |category: Category| {
            let provenance = self.provenance(env, mcp, None, category, None);
            CategoryPolicy {
                category,
                decision: provenance.decision(),
                provenance,
            }
        };
        PolicySummary {
            env: env.clone(),
            mcp: mcp.clone(),
            categories: Category::ALL.map(entry),
        }
    }

    fn provenance(
        &self,
        env: &EnvId,
        mcp: &McpId,
        tool: Option<&ToolName>,
        category: Category,
        unknown_tools: Option<UnknownTools>,
    ) -> Provenance {
        let mut config_candidates = Vec::new();
        candidates(
            &self.config.policy,
            &Layer::Base,
            env,
            mcp,
            tool,
            category,
            &mut config_candidates,
        );
        let profile_layer = Layer::Profile(self.profile.id.clone());
        candidates(
            &self.profile.policy,
            &profile_layer,
            env,
            mcp,
            tool,
            category,
            &mut config_candidates,
        );
        let mut user_candidates = Vec::new();
        candidates(
            self.hardening,
            &Layer::User,
            env,
            mcp,
            tool,
            category,
            &mut user_candidates,
        );

        let floor = config_candidates
            .iter()
            .filter(|c| c.rule.locked)
            .max_by_key(|c| (c.rule.decision, c.precision()))
            .cloned();
        let winner = config_candidates.into_iter().max_by_key(RuleOrigin::precision);
        let hardening = user_candidates.into_iter().max_by_key(RuleOrigin::precision);
        Provenance {
            winner,
            floor,
            unknown_tools,
            hardening,
        }
    }
}

/// Pushes the candidates of `tree` along the path root → `env` → `mcp` → `tool`.
fn candidates(
    tree: &PolicyTree,
    layer: &Layer,
    env: &EnvId,
    mcp: &McpId,
    tool: Option<&ToolName>,
    category: Category,
    out: &mut Vec<RuleOrigin>,
) {
    let mut push = |rule: Option<Rule>, position: RulePosition| {
        if let Some(rule) = rule {
            out.push(RuleOrigin {
                layer: layer.clone(),
                position,
                rule,
            });
        }
    };
    let mut push_set = |set: &RuleSet, env: Option<&EnvId>, mcp: Option<&McpId>| {
        let at = |selector| RulePosition {
            env: env.cloned(),
            mcp: mcp.cloned(),
            selector,
        };
        push(set.default, at(Selector::Default));
        push(set.category(category), at(Selector::Category(category)));
    };

    push_set(&tree.rules, None, None);
    let Some(env_node) = tree.env.get(env) else { return };
    push_set(&env_node.rules, Some(env), None);
    let Some(mcp_node) = env_node.mcp.get(mcp) else { return };
    push_set(&mcp_node.rules, Some(env), Some(mcp));
    if let Some((tool, rule)) = tool.and_then(|t| mcp_node.tools.get_key_value(t)) {
        out.push(RuleOrigin {
            layer: layer.clone(),
            position: RulePosition {
                env: Some(env.clone()),
                mcp: Some(mcp.clone()),
                selector: Selector::Tool(tool.clone()),
            },
            rule: *rule,
        });
    }
}
