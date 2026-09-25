use kuchiki::{NodeRef, traits::TendrilSink};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

fn allowed_html(input: &str) -> String {
    let mut sanitizer = ammonia::Builder::new();
    sanitizer
        .tags(HashSet::from([
            "a",
            "b",
            "blockquote",
            "br",
            "code",
            "del",
            "div",
            "em",
            "figcaption",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "hr",
            "i",
            "li",
            "ol",
            "p",
            "pre",
            "s",
            "span",
            "strike",
            "strong",
            "u",
            "ul",
        ]))
        .tag_attributes(HashMap::from([("a", HashSet::from(["href", "title"]))]))
        .generic_attributes(HashSet::from(["class"]))
        .clean_content_tags(HashSet::new())
        .link_rel(None)
        .url_relative(ammonia::UrlRelative::PassThrough);
    sanitizer.clean(input).to_string()
}

fn linkify_text(input: &str) -> String {
    if input.len() > 10_000 {
        return input.to_owned();
    }
    static PATTERN: OnceLock<regex::Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        regex::Regex::new(r#"https?://[^\s<>"']+|[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?"#)
            .expect("valid link pattern")
    });
    let mut out = String::new();
    let mut offset = 0;
    for found in pattern.find_iter(input) {
        out.push_str(&input[offset..found.start()]);
        let text = found
            .as_str()
            .trim_end_matches(['.', '?', ',', ':', '!', ';', '"', '\'', '<', '>']);
        if text.is_empty() {
            out.push_str(found.as_str());
        } else {
            let href = if text.starts_with("http://") || text.starts_with("https://") {
                text.to_owned()
            } else {
                format!("mailto:{text}")
            };
            out.push_str(&format!(
                "<a href=\"{href}\" rel=\"noopener noreferrer\">{text}</a>"
            ));
            out.push_str(&found.as_str()[text.len()..]);
        }
        offset = found.end();
    }
    out.push_str(&input[offset..]);
    out
}

fn linkify_html(input: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    let mut excluded = Vec::<String>::new();
    while let Some(relative) = input[cursor..].find('<') {
        let start = cursor + relative;
        let text = &input[cursor..start];
        if excluded.is_empty() {
            out.push_str(&linkify_text(text));
        } else {
            out.push_str(text);
        }
        let Some(relative_end) = input[start..].find('>') else {
            out.push_str(&input[start..]);
            return out;
        };
        let end = start + relative_end + 1;
        let tag = &input[start..end];
        out.push_str(tag);
        let name = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|ch: char| ch.is_ascii_whitespace() || ch == '>' || ch == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if tag.starts_with("</") {
            if excluded.last().is_some_and(|current| current == &name) {
                excluded.pop();
            }
        } else if matches!(name.as_str(), "a" | "figcaption" | "pre" | "code") {
            excluded.push(name);
        }
        cursor = end;
    }
    let text = &input[cursor..];
    if excluded.is_empty() {
        out.push_str(&linkify_text(text));
    } else {
        out.push_str(text);
    }
    out
}

pub(super) fn html(input: &str) -> String {
    if !input.contains(['<', '>', '&']) && !input.contains("http") && !input.contains('@') {
        return format!("<div class=\"action-text-content\">\n  {input}\n</div>\n");
    }
    format!(
        "<div class=\"action-text-content\">\n  {}\n</div>\n",
        linkify_html(&allowed_html(input))
    )
}

fn line_break(out: &mut String, count: usize) {
    if out.is_empty() {
        return;
    }
    let existing = out.chars().rev().take_while(|c| *c == '\n').count();
    if existing < count {
        out.extend(std::iter::repeat_n('\n', count - existing));
    }
}

fn walk(node: &NodeRef, out: &mut String) {
    if let Some(text) = node.as_text() {
        out.push_str(&text.borrow());
        return;
    }
    let tag = node
        .as_element()
        .map(|element| element.name.local.to_string())
        .unwrap_or_default();
    if matches!(tag.as_str(), "script" | "style" | "noscript") {
        return;
    }
    if tag == "br" {
        line_break(out, 1);
        return;
    }
    if tag == "ul" || tag == "ol" {
        if !out.is_empty() && !out.ends_with('\n') {
            line_break(out, 2);
        }
        let mut index = 1;
        for child in node.children() {
            if child
                .as_element()
                .is_some_and(|element| element.name.local.as_ref() == "li")
            {
                if !out.is_empty() && !out.ends_with('\n') {
                    line_break(out, 1);
                }
                if tag == "ol" {
                    out.push_str(&format!("{index}. "));
                } else {
                    out.push_str("• ");
                }
                for item in child.children() {
                    walk(&item, out);
                }
                line_break(out, 1);
                index += 1;
            } else {
                walk(&child, out);
            }
        }
        line_break(out, 2);
        return;
    }
    for child in node.children() {
        walk(&child, out);
    }
    match tag.as_str() {
        "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "pre" => line_break(out, 2),
        "div" | "li" => line_break(out, 1),
        _ => {}
    }
}

pub(super) fn plain_text(input: &str) -> String {
    if !input.contains(['<', '&']) {
        return input.to_owned();
    }
    let document = kuchiki::parse_html().one(format!("<body>{input}</body>"));
    let Ok(body) = document.select_first("body") else {
        return input.to_owned();
    };
    let mut text = String::new();
    for child in body.as_node().children() {
        walk(&child, &mut text);
    }
    text.trim_end_matches('\n').to_owned()
}

pub(super) fn attachment_sgids(input: &str) -> Vec<String> {
    if !input.contains("action-text-attachment") {
        return Vec::new();
    }
    let document = kuchiki::parse_html().one(input);
    let Ok(attachments) = document.select("action-text-attachment[sgid]") else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for attachment in attachments {
        if let Some(sgid) = attachment.attributes.borrow().get("sgid") {
            if !values.iter().any(|value| value == sgid) {
                values.push(sgid.to_owned());
            }
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_common_action_text() {
        let input =
            "<p>Hello <strong>world</strong> &amp; <a href=\"https://example.com\">link</a></p>";
        assert_eq!(plain_text(input), "Hello world & link");
        assert_eq!(
            html(input),
            format!("<div class=\"action-text-content\">\n  {input}\n</div>\n")
        );
        assert_eq!(
            plain_text("<p>Next<br>line</p><ul><li>One</li><li>Two</li></ul>"),
            "Next\nline\n\n• One\n• Two"
        );
    }

    #[test]
    fn strips_active_attributes_and_protocols() {
        let input = "<p><a href=\"javascript:alert(1)\" data-action=\"x\">bad</a></p>";
        assert_eq!(
            html(input),
            "<div class=\"action-text-content\">\n  <p><a>bad</a></p>\n</div>\n"
        );
    }

    #[test]
    fn links_text_without_rewriting_existing_links_or_code() {
        assert_eq!(
            html("See https://example.com now"),
            "<div class=\"action-text-content\">\n  See <a href=\"https://example.com\" rel=\"noopener noreferrer\">https://example.com</a> now\n</div>\n"
        );
        assert_eq!(
            html(
                "<p><a href=\"https://example.com\">https://example.com</a> <code>https://example.com</code></p>"
            ),
            "<div class=\"action-text-content\">\n  <p><a href=\"https://example.com\">https://example.com</a> <code>https://example.com</code></p>\n</div>\n"
        );
    }
}
