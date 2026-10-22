use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Browser {
    Chrome,
    Edge,
    Firefox,
    Safari,
    Other,
}

struct Platform {
    browser: Browser,
    browser_name: String,
    operating_system: &'static str,
    ios: bool,
    android: bool,
    windows: bool,
}

impl Platform {
    fn from_headers(headers: &HeaderMap) -> Self {
        let agent = platform_user_agent(headers);
        let browser =
            if agent.contains("Edg/") || agent.contains("EdgA/") || agent.contains("EdgiOS/") {
                Browser::Edge
            } else if agent.contains("Firefox/") || agent.contains("FxiOS/") {
                Browser::Firefox
            } else if agent.contains("Chrome/") || agent.contains("CriOS/") {
                Browser::Chrome
            } else if agent.contains("Safari/") {
                Browser::Safari
            } else {
                Browser::Other
            };
        let browser_name = match browser {
            Browser::Chrome => "Chrome".to_owned(),
            Browser::Edge => "Edge".to_owned(),
            Browser::Firefox => "Firefox".to_owned(),
            Browser::Safari => "Safari".to_owned(),
            Browser::Other => {
                let product = agent.split('/').next().unwrap_or("");
                let mut chars = product.chars();
                let name = chars
                    .next()
                    .map(|first| {
                        first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                    })
                    .unwrap_or_default();
                escape(&name)
            }
        };
        let ios = agent.contains("iPhone") || agent.contains("iPad");
        let android = agent.contains("Android");
        let windows = agent.contains("Windows");
        let operating_system = if android {
            "Android"
        } else if agent.contains("iPad") {
            "iPad"
        } else if agent.contains("iPhone") {
            "iPhone"
        } else if agent.contains("Macintosh") {
            "macOS"
        } else if windows {
            "Windows"
        } else if agent.contains("CrOS") {
            "ChromeOS"
        } else if agent.contains("Linux") {
            "Linux"
        } else {
            ""
        };
        Self {
            browser,
            browser_name,
            operating_system,
            ios,
            android,
            windows,
        }
    }

    fn desktop(&self) -> bool {
        !self.ios && !self.android
    }
}

fn icon(name: &str, alt: &str) -> String {
    let svg = match name {
        "lock" => include_str!("../assets/notification_help/lock.svg"),
        "switch" => include_str!("../assets/notification_help/switch.svg"),
        "sliders" => include_str!("../assets/notification_help/sliders.svg"),
        "menu-dots-vertical" => include_str!("../assets/notification_help/menu-dots-vertical.svg"),
        "bell-alert" => include_str!("../assets/notification_help/bell-alert.svg"),
        "gear" => include_str!("../assets/notification_help/gear.svg"),
        "share" => include_str!("../assets/notification_help/share.svg"),
        "install-edge" => include_str!("../assets/notification_help/install-edge.svg"),
        _ => "",
    };
    format!(
        "<span class=\"icon icon--{name}\" role=\"img\" aria-label=\"{}\">{svg}</span>",
        escape(alt)
    )
}

fn list(items: &[String]) -> String {
    let mut html = String::from("<ol>");
    for item in items {
        html.push_str(&format!("<li>{item}</li>"));
    }
    html.push_str("</ol>");
    html
}

fn heading(text: &str) -> String {
    format!("<h3>{text}</h3>")
}

fn browser_help(platform: &Platform, origin: &str, account: i64) -> String {
    if platform.ios && matches!(platform.browser, Browser::Safari | Browser::Chrome) {
        return String::new();
    }
    let mut html =
        String::from("<div class=\"notifications-help\" data-notifications-target=\"details\">");
    let name = platform.browser_name.as_str();
    match (platform.browser, platform.android, platform.desktop()) {
        (Browser::Firefox, true, _) => {
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&list(&[
                format!("Tap <em>{}</em> in the address bar.", icon("lock", "the View site information button")),
                "Tap <em>Notification</em> to change to <em>Allowed</em>.".into(),
            ]));
        }
        (Browser::Edge, _, true) => {
            html.push_str(&heading("Turn on notifications for this website."));
            html.push_str(&list(&[
                format!("Click <em>{}</em> left of the address bar.", icon("lock", "the View site information button")),
                "Under <em>Permissions for this site &gt; Notifications</em>, choose <em>Allow</em>.".into(),
            ]));
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&desktop_browser_system_steps(platform));
        }
        (Browser::Firefox, _, true) => {
            html.push_str(&heading("Turn on notifications for this website."));
            html.push_str(&list(&[
                format!("Click <em>{name}</em> in the top left."),
                "Click <em>Settings…</em>".into(),
                "Click <em>Privacy &amp; Security</em> in the sidebar.".into(),
                "Scroll down to <em>Permissions</em>.".into(),
                "Click <em>Settings</em> next to <em>Notifications</em>.".into(),
                format!("Select <em>Allow</em> next to <em>{}/{account}/</em>.", escape(origin)),
            ]));
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&desktop_browser_system_steps(platform));
        }
        (Browser::Chrome, _, true) => {
            html.push_str(&heading("Turn on notifications for this website."));
            html.push_str(&list(&[
                format!("Click the <em>{}</em> icon in the address bar.", icon("sliders", "View site information")),
                "Click <em>Site Settings</em>.".into(),
                "Ensure notifications are <em>Allowed</em>.".into(),
            ]));
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&desktop_browser_system_steps(platform));
        }
        (Browser::Chrome, true, _) => {
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&list(&[
                format!("Tap the <em>{}</em> menu button.", icon("menu-dots-vertical", "More options")),
                "Tap <em>Settings</em>.".into(),
                "Tap <em>Notifications</em>.".into(),
                format!("Tap <em>{}</em> to <em>Allow {name} notifications</em>.", icon("switch", "the switch")),
                format!("Tap <em>{}</em> next to <em>Web apps</em>.", icon("switch", "the switch")),
                format!("Tap <em>{}</em> and select <em>Allow</em>.", icon("bell-alert", "the notification bell")),
            ]));
        }
        (Browser::Safari, _, true) => {
            html.push_str(&heading("Turn on notifications for this website."));
            html.push_str(&list(&[
                "Click <em aria-label=\"the Apple menu\"></em> in the top left.".into(),
                "Click <em>System Settings…</em>".into(),
                "Click <em>Notifications</em>.".into(),
                format!("Click <em>{}</em> in the list.", escape(origin)),
                format!("Click <em>{}</em> to <em>Allow notifications</em>.", icon("switch", "the switch")),
            ]));
            html.push_str(&heading(&format!("Turn on notifications for {name}.")));
            html.push_str(&list(&[
                format!("Click <em>{name}</em> in the top left."),
                "Click <em>Settings…</em>".into(),
                "Click the <em>Websites</em> tab.".into(),
                "Click <em>Notifications</em> in the sidebar.".into(),
                format!("Click <em>{}</em> in the list.", escape(origin)),
                "Select <em>Allow</em>.".into(),
            ]));
        }
        _ => html.push_str(&format!("<p>Ensure notifications are enabled for <em>{}/{account}/</em> in your web browser settings.</p>", escape(origin))),
    }
    html.push_str("</div>");
    html
}

fn desktop_browser_system_steps(platform: &Platform) -> String {
    let name = if platform.browser == Browser::Chrome {
        "Google Chrome"
    } else {
        platform.browser_name.as_str()
    };
    if platform.windows {
        list(&[
            "Click <em>Start</em>, then <em>Settings</em>.".into(),
            "Go to <em>System &gt; Notification</em>.".into(),
            format!(
                "Click <em>{}</em> <em>ON</em> for {}.",
                icon("switch", "the switch"),
                platform.browser_name
            ),
        ])
    } else {
        list(&[
            "Click <em aria-label=\"the Apple menu\"></em> in the top left.".into(),
            "Click <em>System Settings…</em>".into(),
            "Click <em>Notifications</em>.".into(),
            format!("Click <em>{name}</em>."),
            format!(
                "Click <em>{}</em> to <em>Allow notifications</em>.",
                icon("switch", "the switch")
            ),
        ])
    }
}

fn system_help(platform: &Platform) -> String {
    let mut html = format!(
        "<div class=\"notifications-help hide-in-browser\" data-notifications-target=\"details\"><h3>Check your {} settings</h3>",
        platform.operating_system
    );
    let items: Vec<String> = match (
        platform.browser,
        platform.android,
        platform.ios,
        platform.desktop(),
    ) {
        (Browser::Firefox, true, _, _) => vec![
            format!(
                "Tap the <em>{}</em> menu button.",
                icon("menu-dots-vertical", "More options")
            ),
            "Tap <em>Settings</em>.".into(),
            "Tap <em>Notifications</em>.".into(),
            format!(
                "Tap <em>{}</em> to <em>Allow Firefox notifications</em>.",
                icon("switch", "the toggle button")
            ),
        ],
        (Browser::Edge, _, _, true) => vec![
            "Click <em>Start</em>, then <em>Settings</em>.".into(),
            "Go to <em>System &gt; Notification</em>.".into(),
            format!(
                "Click <em>{}</em> <em>ON</em> for Rizzy.",
                icon("switch", "the toggle button")
            ),
        ],
        (Browser::Firefox | Browser::Chrome, _, _, true) if platform.windows => vec![
            "Click <em>Start</em>, then <em>Settings</em>.".into(),
            "Go to <em>System &gt; Notification</em>.".into(),
            format!(
                "Click <em>{}</em> <em>ON</em> for Rizzy.",
                icon("switch", "the toggle button")
            ),
        ],
        (Browser::Firefox | Browser::Chrome | Browser::Safari, _, _, true) => vec![
            "Click <em aria-label=\"the Apple menu\"></em> in the top left.".into(),
            "Click <em>System Settings…</em>".into(),
            "Click <em>Notifications</em>.".into(),
            "Click <em>Rizzy</em>.".into(),
            format!(
                "Click <em>{}</em> to <em>Allow notifications</em>.",
                icon("switch", "the allow notifications switch")
            ),
        ],
        (Browser::Safari | Browser::Chrome, _, true, _) => vec![
            format!(
                "Open the <em>{}</em> Settings app.",
                icon("gear", "the Settings app")
            ),
            "Scroll to and tap <em>Rizzy</em>.".into(),
            "Tap <em>Notifications</em>.".into(),
            format!(
                "Tap <em>{}</em> to <em>Allow Notifications</em>.",
                icon("switch", "the allow notifications switch button")
            ),
        ],
        (Browser::Chrome, true, _, _) => vec![
            format!(
                "Open the <em>{}</em> Settings app.",
                icon("gear", "the Settings app")
            ),
            "Tap <em>Notifications</em>.".into(),
            "Tap <em>App notifications</em>.".into(),
            "Scroll to <em>Rizzy</em>.".into(),
            format!(
                "Tap <em>{}</em> to <em>Allow Notifications</em>.",
                icon("switch", "the switch")
            ),
        ],
        _ => Vec::new(),
    };
    if items.is_empty() {
        html.push_str(&format!(
            "<p>Ensure notifications are allowed for {} in your system settings.</p>",
            platform.browser_name
        ));
    } else {
        html.push_str(&list(&items));
    }
    html.push_str("</div>");
    html
}

fn install_help(platform: &Platform) -> String {
    if (platform.browser == Browser::Chrome && !platform.ios)
        || (platform.browser == Browser::Firefox && !platform.android)
    {
        return String::new();
    }
    let mut html = String::from("<div class=\"notifications-help pwa__instructions hide-in-pwa\">");
    let title = if platform.browser == Browser::Safari && platform.desktop() {
        "…or install Rizzy as a web app."
    } else {
        "Install Rizzy as a web app."
    };
    html.push_str(&heading(title));
    match (platform.browser, platform.android, platform.ios, platform.desktop()) {
        (Browser::Edge, _, _, _) => html.push_str(&list(&[
            format!("Click <em>{}</em> in the address bar.", icon("install-edge", "the app available - install Rizzy button")),
            "Click <em>Install</em>.".into(),
        ])),
        (Browser::Chrome, true, _, _) => html.push_str(&list(&[
            format!("Tap the <em>{}</em> menu button.", icon("menu-dots-vertical", "More options")),
            "Tap <em>Install app</em> in the menu.".into(),
        ])),
        (Browser::Firefox, true, _, _) => html.push_str(&list(&[
            format!("Tap the <em>{}</em> menu button.", icon("menu-dots-vertical", "More options")),
            "Tap <em>Install</em> in the menu.".into(),
        ])),
        (Browser::Safari, _, _, true) => html.push_str(&list(&[
            "Click <em>File</em> in the top left.".into(),
            "Click <em>Add to Dock…</em>".into(),
        ])),
        (Browser::Safari | Browser::Chrome, _, true, _) => {
            html.push_str(&format!("<p>To receive push notifications in {} for {}, you must first install Rizzy as a web app.</p>", platform.browser_name, platform.operating_system));
            html.push_str(&list(&[
                format!("Tap <em>{}</em>", icon("share", "the share button")),
                "Tap <em>Add to Home Screen</em>.".into(),
            ]));
        }
        _ => html.push_str("<p>Some platforms require you to install Rizzy as a web app to receive push notifications.</p>"),
    }
    html.push_str("</div>");
    html
}

pub(super) fn render(headers: &HeaderMap, account: i64) -> String {
    let platform = Platform::from_headers(headers);
    let origin = base(headers);
    format!(
        "<p class=\"margin-none-block-start notifications__on-message\">When push notifications aren’t working, this can usually be fixed by checking your notification settings to make sure they’re allowed.</p>{}{}{}",
        browser_help(&platform, &origin, account),
        system_help(&platform),
        install_help(&platform)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_unknown_browser_product_in_help_html() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, HeaderValue::from_static("<script>/1.0"));
        let html = render(&headers, 1);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>"));
    }
}
