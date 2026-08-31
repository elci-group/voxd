//! Voice-routing policy engine.
//!
//! Routes a `NarrationContext` to a voice profile using hierarchical rules:
//! explicit override > window > domain > application > default.

use crate::config::{Config, RuleScope, VoiceProfileCfg};
use crate::context::NarrationContext;
use crate::Settings;
use serde::{Deserialize, Serialize};

/// A persisted routing rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingRule {
    pub id: String,
    pub scope: RuleScope,
    pub pattern: String,
    pub voice: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

impl RoutingRule {
    /// Build a rule with a deterministic id from its content.
    pub fn new(scope: RuleScope, pattern: impl Into<String>, voice: impl Into<String>) -> Self {
        let pattern = pattern.into();
        let voice = voice.into();
        let id = rule_id(&scope, &pattern, &voice);
        Self {
            id,
            scope,
            pattern,
            voice,
            priority: 0,
            enabled: true,
        }
    }

    /// Set a non-zero priority to break ties within the same scope.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

fn rule_id(scope: &RuleScope, pattern: &str, voice: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(scope.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(pattern.as_bytes());
    hasher.update(b"|");
    hasher.update(voice.as_bytes());
    hex::encode(&hasher.finalize()[..8])
}

/// Resolved voice selection for a piece of text.
#[derive(Debug, Clone)]
pub struct VoiceSelection {
    pub voice_id: String,
    pub label: String,
    pub provider: Option<crate::config::SpeechProvider>,
    pub settings: Settings,
    pub rule_id: Option<String>,
    pub rule_scope: Option<RuleScope>,
}

/// Policy engine that merges config profiles, config rules, and persisted DB rules.
pub struct Router {
    default_voice: String,
    profiles: std::collections::HashMap<String, VoiceProfileCfg>,
    rules: Vec<RoutingRule>,
    defaults: Settings,
}

impl Router {
    pub fn new(cfg: &Config, db_rules: Vec<RoutingRule>) -> Self {
        // Config rules are seeded first; DB rules override on matching id.
        let mut rules: Vec<RoutingRule> = cfg
            .routing
            .rules
            .iter()
            .map(|r| RoutingRule {
                id: rule_id(&r.scope, &r.pattern, &r.voice),
                scope: r.scope,
                pattern: r.pattern.clone(),
                voice: r.voice.clone(),
                priority: r.priority,
                enabled: r.enabled,
            })
            .collect();
        let mut seen: std::collections::HashSet<String> =
            rules.iter().map(|r| r.id.clone()).collect();
        for r in db_rules {
            if seen.insert(r.id.clone()) {
                rules.push(r);
            } else {
                // DB rule replaces config rule with same id.
                if let Some(pos) = rules.iter().position(|x| x.id == r.id) {
                    rules[pos] = r;
                }
            }
        }
        // Higher priority first, then stable by scope precedence.
        rules.sort_by(|a, b| {
            scope_rank(&b.scope)
                .cmp(&scope_rank(&a.scope))
                .then_with(|| b.priority.cmp(&a.priority))
        });
        Self {
            default_voice: cfg.routing.default_voice.clone(),
            profiles: cfg.voices.clone(),
            rules,
            defaults: cfg.defaults,
        }
    }

    /// Resolve the voice for a context. An explicit `voice_override` always wins.
    pub fn resolve(&self, ctx: &NarrationContext, voice_override: Option<&str>) -> VoiceSelection {
        if let Some(name) = voice_override {
            if let Some(sel) = self.resolve_profile(name) {
                return VoiceSelection {
                    rule_id: None,
                    rule_scope: Some(RuleScope::Explicit),
                    ..sel
                };
            }
            // Treat unknown override as a raw voice id for the active provider.
            return VoiceSelection {
                voice_id: name.to_string(),
                label: "explicit".into(),
                provider: None,
                settings: self.defaults,
                rule_id: None,
                rule_scope: Some(RuleScope::Explicit),
            };
        }

        for rule in &self.rules {
            if !rule.enabled {
                continue;
            }
            if self.rule_matches(rule, ctx) {
                if let Some(sel) = self.resolve_profile(&rule.voice) {
                    return VoiceSelection {
                        rule_id: Some(rule.id.clone()),
                        rule_scope: Some(rule.scope),
                        ..sel
                    };
                }
                // If the profile doesn't exist, fall back to treating the name as a raw id.
                return VoiceSelection {
                    voice_id: rule.voice.clone(),
                    label: format!("{:?}", rule.scope).to_lowercase(),
                    provider: None,
                    settings: self.defaults,
                    rule_id: Some(rule.id.clone()),
                    rule_scope: Some(rule.scope),
                };
            }
        }

        // Final fallback: default voice profile, then system voice raw id.
        if let Some(sel) = self.resolve_profile(&self.default_voice) {
            return VoiceSelection {
                rule_id: None,
                rule_scope: Some(RuleScope::Default),
                ..sel
            };
        }
        VoiceSelection {
            voice_id: self.default_voice.clone(),
            label: "default".into(),
            provider: None,
            settings: self.defaults,
            rule_id: None,
            rule_scope: Some(RuleScope::Default),
        }
    }

    /// Resolve a named profile. Unknown names are treated as raw voice ids.
    pub fn resolve_profile_name(&self, name: &str) -> VoiceSelection {
        self.resolve_profile(name).unwrap_or_else(|| VoiceSelection {
            voice_id: name.to_string(),
            label: "unknown".into(),
            provider: None,
            settings: self.defaults,
            rule_id: None,
            rule_scope: None,
        })
    }

    fn resolve_profile(&self, name: &str) -> Option<VoiceSelection> {
        self.profiles.get(name).map(|p| VoiceSelection {
            voice_id: p.voice_id.clone(),
            label: if p.label.is_empty() {
                name.to_string()
            } else {
                p.label.clone()
            },
            provider: p.provider,
            settings: self.defaults.apply(&p.settings),
            rule_id: None,
            rule_scope: None,
        })
    }

    fn rule_matches(&self, rule: &RoutingRule, ctx: &NarrationContext) -> bool {
        let pat = rule.pattern.to_ascii_lowercase();
        match rule.scope {
            RuleScope::Application => ctx
                .application
                .as_ref()
                .map(|s| glob_match(&pat, &s.to_ascii_lowercase()))
                .unwrap_or(false),
            RuleScope::Domain => ctx
                .domain
                .as_ref()
                .map(|s| glob_match(&pat, &s.to_ascii_lowercase()))
                .unwrap_or(false),
            RuleScope::Window => ctx
                .window_title
                .as_ref()
                .map(|s| glob_match(&pat, &s.to_ascii_lowercase()))
                .unwrap_or(false),
            RuleScope::Explicit | RuleScope::Default => false,
        }
    }
}

/// Scope precedence rank: higher number wins.
fn scope_rank(scope: &RuleScope) -> u8 {
    match scope {
        RuleScope::Explicit => 5,
        RuleScope::Window => 4,
        RuleScope::Domain => 3,
        RuleScope::Application => 2,
        RuleScope::Default => 1,
    }
}

/// Minimal glob matcher: `*` matches any substring, otherwise exact equality.
fn glob_match(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern.is_empty() {
        return true;
    }
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        if parts.is_empty() {
            return true;
        }
        let mut rest = value;
        for (i, part) in parts.iter().enumerate() {
            if part.is_empty() {
                continue;
            }
            if i == 0 && !rest.starts_with(part) {
                return false;
            }
            if let Some(pos) = rest.find(part) {
                rest = &rest[pos + part.len()..];
            } else {
                return false;
            }
        }
        // Trailing empty part means match anywhere; otherwise we consumed everything.
        pattern.ends_with('*') || rest.is_empty()
    } else {
        pattern == value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx(app: &str, domain: Option<&str>, title: &str) -> NarrationContext {
        NarrationContext {
            text: "hello".into(),
            source: crate::context::TextSource::CliArg,
            application: Some(app.into()),
            window_title: Some(title.into()),
            domain: domain.map(|s| s.into()),
            project_id: None,
        }
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("firefox", "firefox"));
        assert!(!glob_match("firefox", "firefox-beta"));
        assert!(glob_match("*firefox*", "org.mozilla.firefox"));
        assert!(glob_match("*steam*", "steam"));
        assert!(glob_match("*.wikipedia.org", "en.wikipedia.org"));
        assert!(!glob_match("*.wikipedia.org", "wikipedia.org"));
        assert!(glob_match("*", "anything"));
    }

    #[test]
    fn routing_precedence_domain_over_application() {
        let mut cfg = Config::default();
        cfg.voices.insert(
            "amanda".into(),
            VoiceProfileCfg {
                voice_id: "vid-amanda".into(),
                ..Default::default()
            },
        );
        cfg.voices.insert(
            "jeff".into(),
            VoiceProfileCfg {
                voice_id: "vid-jeff".into(),
                ..Default::default()
            },
        );
        cfg.routing.rules.push(crate::config::RoutingRuleCfg {
            scope: RuleScope::Application,
            pattern: "firefox".into(),
            voice: "amanda".into(),
            priority: 0,
            enabled: true,
        });
        cfg.routing.rules.push(crate::config::RoutingRuleCfg {
            scope: RuleScope::Domain,
            pattern: "github.com".into(),
            voice: "jeff".into(),
            priority: 0,
            enabled: true,
        });
        let router = Router::new(&cfg, Vec::new());
        let ctx = test_ctx("firefox", Some("github.com"), "GitHub");
        let sel = router.resolve(&ctx, None);
        assert_eq!(sel.voice_id, "vid-jeff");
        assert_eq!(sel.rule_scope, Some(RuleScope::Domain));
    }

    #[test]
    fn explicit_override_wins() {
        let cfg = Config::default();
        let router = Router::new(&cfg, Vec::new());
        let ctx = test_ctx("firefox", Some("github.com"), "GitHub");
        let sel = router.resolve(&ctx, Some("override-id"));
        assert_eq!(sel.voice_id, "override-id");
        assert_eq!(sel.rule_scope, Some(RuleScope::Explicit));
    }
}
