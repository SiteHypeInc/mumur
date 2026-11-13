//! Per-app behavior: a writing style per category, plus user-defined modes that
//! override it for specific apps or sites.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::context::{self, AppContext, Category};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Style {
    /// Clean up, but keep the speaker's own tone.
    #[default]
    Natural,
    Formal,
    Casual,
    /// All lowercase, light punctuation, texting shorthand only where it stands for the exact words.
    VeryCasual,
    Excited,
}

impl Style {
    pub const ALL: [Style; 5] =
        [Style::Natural, Style::Formal, Style::Casual, Style::VeryCasual, Style::Excited];
}

/// Key pressed after pasting, for chat boxes where you want "say it and send it".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKey {
    Enter,
    ShiftEnter,
    CmdEnter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Mode {
    pub name: String,
    /// App names or ids (bundle id / exe), matched case-insensitively.
    pub apps: Vec<String>,
    /// Site hosts, matched exactly or as a parent domain.
    pub hosts: Vec<String>,
    /// Extra instructions appended to the cleanup prompt.
    pub instructions: String,
    pub style: Option<Style>,
    /// Override whether the LLM step runs (e.g. off for a terminal).
    pub polish: Option<bool>,
    pub send_key: Option<SendKey>,
}

impl Mode {
    fn matches(&self, ctx: &AppContext) -> bool {
        let app_hit = self.apps.iter().any(|want| {
            let want = want.to_lowercase();
            [&ctx.app_name, &ctx.app_id]
                .into_iter()
                .flatten()
                .any(|have| have.to_lowercase() == want)
        });
        let host_hit = ctx.url_host.as_deref().is_some_and(|host| {
            let host = host.trim_start_matches("www.").to_lowercase();
            self.hosts.iter().any(|h| {
                let h = h.to_lowercase();
                host == h || host.ends_with(&format!(".{h}"))
            })
        });
        app_hit || host_hit
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub category: Category,
    pub mode_name: Option<String>,
    pub style: Style,
    pub instructions: Option<String>,
    pub polish: bool,
    pub send_key: Option<SendKey>,
}

pub fn default_styles() -> HashMap<Category, Style> {
    HashMap::from([
        (Category::Personal, Style::Natural),
        (Category::Work, Style::Natural),
        (Category::Email, Style::Natural),
        (Category::Code, Style::Natural),
        (Category::Document, Style::Natural),
        (Category::Other, Style::Natural),
    ])
}

/// Site-specific modes beat app-specific ones (a browser tab is more specific
/// than "Chrome"), and both beat the category default.
pub fn resolve(
    ctx: &AppContext,
    modes: &[Mode],
    styles: &HashMap<Category, Style>,
    polish_default: bool,
) -> Resolved {
    let category = context::classify(ctx);
    let by_host = modes.iter().find(|m| {
        ctx.url_host.is_some() && !m.hosts.is_empty() && m.matches(&AppContext { app_name: None, app_id: None, ..ctx.clone() })
    });
    let mode = by_host.or_else(|| modes.iter().find(|m| m.matches(ctx)));

    let base_style = styles.get(&category).copied().unwrap_or_default();
    match mode {
        Some(m) => Resolved {
            category,
            mode_name: Some(m.name.clone()),
            style: m.style.unwrap_or(base_style),
            instructions: (!m.instructions.trim().is_empty()).then(|| m.instructions.trim().to_owned()),
            polish: m.polish.unwrap_or(polish_default),
            send_key: m.send_key,
        },
        None => Resolved {
            category,
            mode_name: None,
            style: base_style,
            instructions: None,
            polish: polish_default,
            send_key: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chrome(host: &str) -> AppContext {
        AppContext {
            app_name: Some("Google Chrome".into()),
            app_id: Some("com.google.Chrome".into()),
            url_host: Some(host.into()),
            window_title: None,
        }
    }

    #[test]
    fn falls_back_to_category_style() {
        let ctx = AppContext { app_name: Some("Mail".into()), ..Default::default() };
        let r = resolve(&ctx, &[], &default_styles(), true);
        assert_eq!(r.category, Category::Email);
        // Out of the box nothing is restyled; tones are opt-in.
        assert_eq!(r.style, Style::Natural);
        assert!(r.polish);
        assert_eq!(r.mode_name, None);
    }

    #[test]
    fn host_mode_beats_app_mode() {
        let modes = vec![
            Mode { name: "Chrome".into(), apps: vec!["google chrome".into()], ..Default::default() },
            Mode {
                name: "Linear".into(),
                hosts: vec!["linear.app".into()],
                instructions: "Write as an issue description.".into(),
                style: Some(Style::Formal),
                ..Default::default()
            },
        ];
        let r = resolve(&chrome("linear.app"), &modes, &default_styles(), true);
        assert_eq!(r.mode_name.as_deref(), Some("Linear"));
        assert_eq!(r.style, Style::Formal);
        let r = resolve(&chrome("example.com"), &modes, &default_styles(), true);
        assert_eq!(r.mode_name.as_deref(), Some("Chrome"));
    }

    #[test]
    fn mode_can_disable_polish_and_set_send_key() {
        let modes = vec![Mode {
            name: "Terminal".into(),
            apps: vec!["com.apple.Terminal".into()],
            polish: Some(false),
            send_key: Some(SendKey::Enter),
            ..Default::default()
        }];
        let ctx = AppContext { app_id: Some("com.apple.Terminal".into()), ..Default::default() };
        let r = resolve(&ctx, &modes, &default_styles(), true);
        assert!(!r.polish);
        assert_eq!(r.send_key, Some(SendKey::Enter));
        assert_eq!(r.category, Category::Code);
    }
}
