//! French rendering of a provenance, shown as is in the journal and in the "politique" tooltip of the MCP tab.

use super::{CategoryPolicy, CategorySource, DecidedBy, Layer, Provenance, Resolution, RuleOrigin, UnknownTools};
use crate::config::{Config, Level, Selector};

impl CategorySource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Config => "déclarée dans la config",
            Self::Annotation => "déduite des annotations du serveur",
            Self::Default => "par défaut, faute d'annotations",
            Self::Unknown => "outil inconnu de la config",
        }
    }
}

fn env_label(config: &Config, id: &crate::config::EnvId) -> String {
    config
        .environment(id)
        .map_or_else(|| id.as_str().to_owned(), |e| e.label.clone())
}

fn mcp_label(config: &Config, id: &crate::config::McpId) -> String {
    config
        .mcp(id)
        .map_or_else(|| id.as_str().to_owned(), |m| m.label.clone())
}

fn profile_label(config: &Config, id: &crate::config::ProfileId) -> String {
    config
        .profiles
        .get(id)
        .map_or_else(|| id.as_str().to_owned(), |p| p.label.clone())
}

impl RuleOrigin {
    /// For example `règle « write » de PROD, verrouillée` or
    /// `durcissement utilisateur « default » de Kibana Logs en PROD`.
    pub fn describe(&self, config: &Config) -> String {
        let kind = match self.layer {
            Layer::User => "durcissement utilisateur",
            Layer::Base | Layer::Profile(_) => "règle",
        };
        let selector = match &self.position.selector {
            Selector::Default => format!("{kind} « default »"),
            Selector::Category(c) => format!("{kind} « {} »", c.key()),
            Selector::Tool(tool) => format!("{kind} de l'outil « {tool} »"),
        };
        let place = match (&self.position.env, &self.position.mcp, self.level()) {
            (_, _, Level::Root) | (None, _, _) => "de la racine".to_owned(),
            (Some(env), None, _) => format!("de {}", env_label(config, env)),
            (Some(env), Some(mcp), _) => format!("de {} en {}", mcp_label(config, mcp), env_label(config, env)),
        };
        let mut text = format!("{selector} {place}");
        if let Layer::Profile(id) = &self.layer {
            text.push_str(&format!(", profil « {} »", profile_label(config, id)));
        }
        if self.rule.locked && self.layer != Layer::User {
            text.push_str(", verrouillée");
        }
        text
    }
}

impl UnknownTools {
    pub fn describe(&self, config: &Config) -> String {
        format!(
            "outil inconnu de la config, profil « {} »",
            profile_label(config, &self.profile)
        )
    }
}

impl Provenance {
    /// `<decision> — <what decided>`, for example
    /// `confirmation (notification) — règle « write » de PROD, verrouillée`.
    pub fn describe(&self, config: &Config) -> String {
        let decider = match self.decided_by() {
            DecidedBy::NoRule => "aucune règle applicable, refus par défaut".to_owned(),
            DecidedBy::Winner(rule) | DecidedBy::Floor(rule) | DecidedBy::Hardening(rule) => rule.describe(config),
            DecidedBy::UnknownTools(unknown) => unknown.describe(config),
        };
        format!("{} — {decider}", self.decision().label())
    }

    /// The short text followed by the rules the decision overrode.
    pub fn describe_detailed(&self, config: &Config) -> String {
        let mut text = self.describe(config);
        let decided_by = self.decided_by();
        if let Some(winner) = &self.winner
            && !matches!(decided_by, DecidedBy::Winner(_))
        {
            text.push_str(&format!(
                " ; règle la plus précise : {} ({})",
                winner.describe(config),
                winner.rule.decision.label()
            ));
        }
        if let Some(floor) = &self.floor
            && !matches!(decided_by, DecidedBy::Floor(_))
            && Some(floor) != self.winner.as_ref()
        {
            text.push_str(&format!(
                " ; plancher : {} ({})",
                floor.describe(config),
                floor.rule.decision.label()
            ));
        }
        text
    }
}

impl Resolution {
    /// Short French explanation, see [`Provenance::describe`].
    pub fn describe(&self, config: &Config) -> String {
        self.provenance.describe(config)
    }

    /// Detailed French explanation: decision, overridden rules, category and its source.
    pub fn describe_detailed(&self, config: &Config) -> String {
        format!(
            "{} ; catégorie « {} » ({})",
            self.provenance.describe_detailed(config),
            self.category.key(),
            self.category_source.label()
        )
    }
}

impl CategoryPolicy {
    pub fn describe(&self, config: &Config) -> String {
        self.provenance.describe(config)
    }
}
