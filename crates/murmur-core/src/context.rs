//! What app is the user dictating into, and what kind of writing is that?

use serde::{Deserialize, Serialize};

/// Facts the OS layer could gather about the dictation target. Every field is
/// optional because each platform exposes a different subset.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppContext {
    /// Display name, e.g. "Slack".
    pub app_name: Option<String>,
    /// macOS bundle id ("com.tinyspeck.slackmacgap") or Windows exe ("slack.exe").
    pub app_id: Option<String>,
    pub window_title: Option<String>,
    /// Host of the focused browser tab, when the platform can read it.
    pub url_host: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Personal,
    Work,
    Email,
    /// IDEs, terminals, AI coding tools: keep identifiers verbatim.
    Code,
    /// Docs and notes apps: full sentences, paragraphs allowed.
    Document,
    Other,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Personal,
        Category::Work,
        Category::Email,
        Category::Code,
        Category::Document,
        Category::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Category::Personal => "Personal messages",
            Category::Work => "Work chat",
            Category::Email => "Email",
            Category::Code => "Code & terminals",
            Category::Document => "Docs & notes",
            Category::Other => "Everything else",
        }
    }
}

const BROWSERS: &[&str] = &[
    "chrome", "chromium", "safari", "firefox", "msedge", "edge", "arc", "brave", "opera",
    "vivaldi", "zen", "orion", "dia", "comet", "thebrowser", "librewolf",
];

/// Whole-token app signatures. Tokens come from the app name and id split on
/// non-alphanumerics, so "com.tinyspeck.slackmacgap" yields "slackmacgap".
const APP_SIGNATURES: &[(Category, &[&str])] = &[
    (
        Category::Code,
        &[
            "vscode", "code", "cursor", "windsurf", "zed", "xcode", "jetbrains", "intellij",
            "pycharm", "webstorm", "goland", "rider", "clion", "android studio", "sublime",
            "nova", "neovim", "nvim", "terminal", "iterm2", "warp", "ghostty", "kitty",
            "alacritty", "wezterm", "hyper", "windowsterminal", "powershell", "cmd", "conhost",
            "claude code", "todesktop",
        ],
    ),
    (
        Category::Email,
        &[
            "mail", "outlook", "olk", "superhuman", "spark", "airmail", "thunderbird",
            "mimestream", "hey", "protonmail", "canarymail", "missive", "shortwave",
        ],
    ),
    (
        Category::Work,
        &[
            "slack", "slackmacgap", "teams", "msteams", "linear", "jira", "zoom", "webex",
            "mattermost", "basecamp", "asana", "clickup",
        ],
    ),
    (
        Category::Personal,
        &[
            "whatsapp", "telegram", "messages", "mobilesms", "signal", "messenger", "imessage",
            "discord", "wechat", "viber", "beeper", "instagram",
        ],
    ),
    (
        Category::Document,
        &[
            "notion", "obsidian", "word", "winword", "pages", "bear", "craft", "notes",
            "evernote", "onenote", "ulysses", "ia writer", "typora", "logseq", "coda",
        ],
    ),
];

/// Hosts match exactly or as a parent domain.
const SITE_HOSTS: &[(Category, &[&str])] = &[
    (Category::Email, &["mail.google.com", "outlook.live.com", "outlook.office.com", "app.hey.com", "mail.proton.me", "app.fastmail.com", "mail.superhuman.com"]),
    (Category::Work, &["slack.com", "teams.microsoft.com", "linear.app", "atlassian.net", "app.asana.com", "app.clickup.com", "chat.google.com"]),
    (Category::Personal, &["web.whatsapp.com", "web.telegram.org", "messenger.com", "discord.com", "instagram.com", "x.com", "twitter.com"]),
    (Category::Code, &["github.com", "gitlab.com", "claude.ai", "chatgpt.com", "replit.com", "codesandbox.io", "stackblitz.com", "v0.dev", "lovable.dev", "bolt.new"]),
    (Category::Document, &["docs.google.com", "notion.so", "notion.site", "coda.io", "quip.com", "dropbox.com/paper", "office.com"]),
];

/// Browser tab titles end with the site name ("Inbox (3) - Gmail").
const SITE_TITLES: &[(Category, &[&str])] = &[
    (Category::Email, &["gmail", "outlook", "superhuman", "proton mail", "fastmail"]),
    (Category::Work, &["slack", "microsoft teams", "linear", "jira", "asana", "google chat"]),
    (Category::Personal, &["whatsapp", "telegram", "messenger", "discord", "instagram"]),
    (Category::Code, &["github", "gitlab", "claude", "chatgpt"]),
    (Category::Document, &["google docs", "notion", "coda"]),
];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

/// True when `sig` (one or more words) appears as consecutive whole tokens.
fn contains_phrase(toks: &[String], sig: &str) -> bool {
    let sig_toks = tokens(sig);
    if sig_toks.is_empty() || sig_toks.len() > toks.len() {
        return false;
    }
    toks.windows(sig_toks.len()).any(|w| w == sig_toks.as_slice())
}

fn host_matches(host: &str, sig: &str) -> bool {
    let host = host.trim_start_matches("www.").to_lowercase();
    host == sig || host.ends_with(&format!(".{sig}"))
}

fn app_tokens(ctx: &AppContext) -> Vec<String> {
    let mut t = Vec::new();
    for s in [&ctx.app_name, &ctx.app_id].into_iter().flatten() {
        let s = s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s);
        t.extend(tokens(s));
    }
    t
}

pub fn is_browser(ctx: &AppContext) -> bool {
    let toks = app_tokens(ctx);
    BROWSERS.iter().any(|b| contains_phrase(&toks, b))
}

/// The site name a tab title ends with, skipping a trailing browser name:
/// "Inbox (3) - me@x.com - Gmail - Google Chrome" -> "gmail".
fn site_from_title(title: &str) -> Option<String> {
    let lowered = title.to_lowercase();
    lowered
        .split([' ', '\u{a0}'])
        .collect::<Vec<_>>()
        .join(" ")
        .split(" - ")
        .flat_map(|s| s.split(" | "))
        .flat_map(|s| s.split(" · "))
        .flat_map(|s| s.split(" — "))
        .flat_map(|s| s.split(" – "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| {
            let toks = tokens(s);
            !BROWSERS.iter().any(|b| contains_phrase(&toks, b))
        })
        .last()
        .map(str::to_owned)
}

pub fn classify(ctx: &AppContext) -> Category {
    if is_browser(ctx) {
        if let Some(host) = &ctx.url_host {
            for (cat, hosts) in SITE_HOSTS {
                if hosts.iter().any(|h| host_matches(host, h)) {
                    return *cat;
                }
            }
        }
        if let Some(title) = &ctx.window_title {
            if let Some(site) = site_from_title(title) {
                for (cat, names) in SITE_TITLES {
                    if names.iter().any(|n| site == *n) {
                        return *cat;
                    }
                }
            }
        }
        return Category::Other;
    }

    let toks = app_tokens(ctx);
    for (cat, sigs) in APP_SIGNATURES {
        if sigs.iter().any(|s| contains_phrase(&toks, s)) {
            return *cat;
        }
    }
    Category::Other
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str, id: &str) -> AppContext {
        AppContext { app_name: Some(name.into()), app_id: Some(id.into()), ..Default::default() }
    }

    fn browser(title: &str, host: Option<&str>) -> AppContext {
        AppContext {
            app_name: Some("Google Chrome".into()),
            app_id: Some("com.google.Chrome".into()),
            window_title: Some(title.into()),
            url_host: host.map(Into::into),
        }
    }

    #[test]
    fn native_apps() {
        assert_eq!(classify(&app("Slack", "com.tinyspeck.slackmacgap")), Category::Work);
        assert_eq!(classify(&app("Messages", "com.apple.MobileSMS")), Category::Personal);
        assert_eq!(classify(&app("Mail", "com.apple.mail")), Category::Email);
        assert_eq!(classify(&app("Cursor", "com.todesktop.230313mzl4w4u92")), Category::Code);
        assert_eq!(classify(&app("Code", "Code.exe")), Category::Code);
        assert_eq!(classify(&app("OUTLOOK", "OUTLOOK.EXE")), Category::Email);
        assert_eq!(classify(&app("Notion", "notion.id")), Category::Document);
        assert_eq!(classify(&app("Finder", "com.apple.finder")), Category::Other);
    }

    #[test]
    fn substrings_do_not_match() {
        // "mail" must not match "Mailchimp", "code" must not match "Barcode Scanner Pro".
        assert_eq!(classify(&app("Mailchimp", "com.mailchimp.app")), Category::Other);
        assert_eq!(classify(&app("Barcodes", "com.example.barcodes")), Category::Other);
    }

    #[test]
    fn browser_by_host_then_title() {
        assert_eq!(classify(&browser("Inbox", Some("mail.google.com"))), Category::Email);
        assert_eq!(classify(&browser("x", Some("app.slack.com"))), Category::Work);
        assert_eq!(classify(&browser("Inbox (3) - me@x.com - Gmail - Google Chrome", None)), Category::Email);
        assert_eq!(classify(&browser("Fix bug · Pull Request #12 · acme/app - GitHub", None)), Category::Code);
        assert_eq!(classify(&browser("Hey there - Some Blog", None)), Category::Other);
    }
}
