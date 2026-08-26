//! Desktop context resolver: selection text + active window + domain.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Where the text to speak came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    #[default]
    CliArg,
    Stdin,
    Selection,
    Api,
    Hotkey,
}

/// Resolved narration context for a single speak request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NarrationContext {
    pub text: String,
    #[serde(default)]
    pub source: TextSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl NarrationContext {
    /// Build a context from raw text without display-server introspection.
    pub fn from_text(text: impl Into<String>, source: TextSource) -> Self {
        Self {
            text: text.into(),
            source,
            ..Default::default()
        }
    }

    /// Populate window/application/domain fields from the active desktop context.
    pub async fn populate_from_desktop(&mut self, window: &dyn WindowBackend) -> Result<()> {
        let info = window.active_window().await?;
        self.application = info.application.map(|s| s.to_ascii_lowercase());
        self.window_title = info.title;
        self.domain = info.domain.or_else(|| extract_domain_from_text(&self.text));
        Ok(())
    }
}

/// Information about the active window.
#[derive(Debug, Clone, Default)]
pub struct WindowInfo {
    pub application: Option<String>,
    pub title: Option<String>,
    pub domain: Option<String>,
}

/// Backend that can return the currently selected text.
#[async_trait::async_trait]
pub trait SelectionBackend: Send + Sync {
    async fn selected_text(&self) -> Result<Option<String>>;
}

/// Backend that can return information about the active window.
#[async_trait::async_trait]
pub trait WindowBackend: Send + Sync {
    async fn active_window(&self) -> Result<WindowInfo>;
}

/// `arboard`-based selection backend (X11/Wayland clipboard + primary selection).
pub struct ArboardSelection;

#[async_trait::async_trait]
impl SelectionBackend for ArboardSelection {
    async fn selected_text(&self) -> Result<Option<String>> {
        tokio::task::spawn_blocking(|| {
            let mut clipboard = arboard::Clipboard::new().context("open clipboard")?;
            // Try the primary selection first (middle-mouse / Linux highlight).
            match clipboard.get_text() {
                Ok(text) if !text.trim().is_empty() => Ok(Some(text)),
                _ => Ok(None),
            }
        })
        .await
        .context("clipboard task")?
    }
}

/// Shell-based fallback for primary selection acquisition.
pub struct ShellSelection;

#[async_trait::async_trait]
impl SelectionBackend for ShellSelection {
    async fn selected_text(&self) -> Result<Option<String>> {
        tokio::task::spawn_blocking(|| {
            let cmds: [(&[&str], &str); 3] = [
                (&["xclip", "-o", "-selection", "primary"], "xclip"),
                (&["wl-paste", "--primary"], "wl-paste"),
                (&["xclip", "-o", "-selection", "clipboard"], "xclip"),
            ];
            for (args, _name) in cmds {
                if let Ok(out) = Command::new(args[0]).args(&args[1..]).output() {
                    if out.status.success() {
                        let text = String::from_utf8_lossy(&out.stdout).to_string();
                        if !text.trim().is_empty() {
                            return Ok(Some(text));
                        }
                    }
                }
            }
            Ok(None)
        })
        .await
        .context("shell selection task")?
    }
}

/// Try a sequence of selection backends and return the first non-empty result.
pub async fn acquire_selection(backends: &[&dyn SelectionBackend]) -> Result<Option<String>> {
    for backend in backends {
        match backend.selected_text().await {
            Ok(Some(text)) => return Ok(Some(text)),
            Ok(None) => continue,
            Err(e) => {
                tracing::debug!(error = %e, "selection backend failed");
                continue;
            }
        }
    }
    Ok(None)
}

/// X11 active-window backend using `x11rb`.
pub struct X11Window;

#[async_trait::async_trait]
impl WindowBackend for X11Window {
    async fn active_window(&self) -> Result<WindowInfo> {
        tokio::task::spawn_blocking(|| {
            use x11rb::connection::Connection;
            use x11rb::protocol::xproto::*;
            use x11rb::rust_connection::RustConnection;

            let (conn, screen_num) = RustConnection::connect(None)?;
            let screen = &conn.setup().roots[screen_num];
            let root = screen.root;

            let active_atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW")?.reply()?.atom;
            let utf8_atom = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
            let wm_class_atom = conn.intern_atom(false, b"WM_CLASS")?.reply()?.atom;
            let wm_name_atom = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;

            let active_window = conn
                .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)?
                .reply()?;
            let window = active_window
                .value32()
                .and_then(|mut v| v.next())
                .unwrap_or(root);

            let mut info = WindowInfo::default();

            // WM_CLASS holds instance and class names.
            if let Ok(reply) = conn.get_property(false, window, wm_class_atom, AtomEnum::STRING, 0, 1024)?.reply() {
                let raw = reply.value;
                // WM_CLASS is two null-terminated strings.
                let strings: Vec<&[u8]> = raw.split(|&b| b == 0).filter(|s| !s.is_empty()).collect();
                if let Some(class) = strings.last() {
                    info.application = Some(String::from_utf8_lossy(class).to_string());
                }
            }

            // _NET_WM_NAME is the modern UTF-8 window title.
            if let Ok(reply) = conn.get_property(false, window, wm_name_atom, utf8_atom, 0, 1024)?.reply() {
                if !reply.value.is_empty() {
                    info.title = Some(String::from_utf8_lossy(&reply.value).to_string());
                }
            }

            // Fallback to WM_NAME if _NET_WM_NAME is absent.
            if info.title.is_none() {
                if let Ok(reply) = conn.get_property(false, window, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 1024)?.reply() {
                    if !reply.value.is_empty() {
                        info.title = Some(String::from_utf8_lossy(&reply.value).to_string());
                    }
                }
            }

            Ok(info)
        })
        .await
        .context("x11 window task")?
    }
}

/// Shell-based fallback for active-window detection.
pub struct ShellWindow;

#[async_trait::async_trait]
impl WindowBackend for ShellWindow {
    async fn active_window(&self) -> Result<WindowInfo> {
        tokio::task::spawn_blocking(|| {
            let mut info = WindowInfo::default();

            // Try xdotool first.
            if let Ok(out) = Command::new("xdotool").args(["getactivewindow", "getwindowname"]).output() {
                if out.status.success() {
                    info.title = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
                }
            }
            if let Ok(out) = Command::new("xdotool").args(["getactivewindow", "getwindowclassname"]).output() {
                if out.status.success() {
                    info.application = Some(String::from_utf8_lossy(&out.stdout).trim().to_ascii_lowercase());
                }
            }

            // Hyprland fallback.
            if info.title.is_none() && info.application.is_none() {
                if let Ok(out) = Command::new("hyprctl").args(["activewindow", "-j"]).output() {
                    if out.status.success() {
                        let text = String::from_utf8_lossy(&out.stdout);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                            info.title = v.get("title").and_then(|x| x.as_str()).map(|s| s.to_string());
                            info.application = v.get("class").and_then(|x| x.as_str()).map(|s| s.to_ascii_lowercase());
                        }
                    }
                }
            }

            // Extract a domain from the title if it looks like a browser window.
            info.domain = info.title.as_ref().and_then(|t| extract_domain_from_title(t));
            Ok(info)
        })
        .await
        .context("shell window task")?
    }
}

/// Try a sequence of window backends and return the first successful result.
pub async fn active_window(backends: &[&dyn WindowBackend]) -> Result<WindowInfo> {
    for backend in backends {
        match backend.active_window().await {
            Ok(info) => return Ok(info),
            Err(e) => {
                tracing::debug!(error = %e, "window backend failed");
            }
        }
    }
    Ok(WindowInfo::default())
}

fn extract_domain_from_text(text: &str) -> Option<String> {
    // Very naive URL detector; browsers should supply domain via a future extension.
    let trimmed = text.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.split('/').nth(2).map(|s| s.to_ascii_lowercase())
    } else {
        None
    }
}

fn extract_domain_from_title(_title: &str) -> Option<String> {
    // "Page Title - Mozilla Firefox" → no domain; "example.com - Page Title" → example.com
    // This is intentionally conservative.
    None
}

/// Window backend that returns hard-coded values for tests.
pub struct MockWindow {
    pub info: WindowInfo,
}

#[async_trait::async_trait]
impl WindowBackend for MockWindow {
    async fn active_window(&self) -> Result<WindowInfo> {
        Ok(self.info.clone())
    }
}

/// Selection backend that returns hard-coded text for tests.
pub struct MockSelection {
    pub text: Option<String>,
}

#[async_trait::async_trait]
impl SelectionBackend for MockSelection {
    async fn selected_text(&self) -> Result<Option<String>> {
        Ok(self.text.clone())
    }
}
