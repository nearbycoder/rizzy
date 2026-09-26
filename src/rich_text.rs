use kuchiki::{NodeRef, traits::TendrilSink};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

fn allowed_html(input: &str) -> String {
    static SANITIZER: OnceLock<ammonia::Builder<'static>> = OnceLock::new();
    let sanitizer = SANITIZER.get_or_init(|| {
        let mut sanitizer = ammonia::Builder::new();
        let style_properties = HashSet::from([
            "azimuth",
            "align-content",
            "align-items",
            "align-self",
            "aspect-ratio",
            "background",
            "background-color",
            "border",
            "border-bottom",
            "border-bottom-color",
            "border-collapse",
            "border-color",
            "border-left",
            "border-left-color",
            "border-right",
            "border-right-color",
            "border-top",
            "border-top-color",
            "clear",
            "color",
            "cursor",
            "direction",
            "display",
            "elevation",
            "flex",
            "flex-basis",
            "flex-direction",
            "flex-flow",
            "flex-grow",
            "flex-shrink",
            "flex-wrap",
            "float",
            "font",
            "font-family",
            "font-size",
            "font-style",
            "font-variant",
            "font-weight",
            "height",
            "justify-content",
            "letter-spacing",
            "line-height",
            "list-style",
            "list-style-type",
            "margin",
            "margin-bottom",
            "margin-left",
            "margin-right",
            "margin-top",
            "max-height",
            "max-width",
            "min-height",
            "min-width",
            "order",
            "overflow",
            "overflow-x",
            "overflow-y",
            "padding",
            "padding-bottom",
            "padding-left",
            "padding-right",
            "padding-top",
            "page-break-after",
            "page-break-before",
            "page-break-inside",
            "pause",
            "pause-after",
            "pause-before",
            "pitch",
            "pitch-range",
            "richness",
            "speak",
            "speak-header",
            "speak-numeral",
            "speak-punctuation",
            "speech-rate",
            "stress",
            "text-align",
            "text-decoration",
            "text-indent",
            "unicode-bidi",
            "vertical-align",
            "voice-family",
            "volume",
            "white-space",
            "width",
            "fill",
            "fill-opacity",
            "fill-rule",
            "stroke",
            "stroke-width",
            "stroke-linecap",
            "stroke-linejoin",
            "stroke-opacity",
        ]);
        sanitizer
            .tags(HashSet::from([
                "a",
                "abbr",
                "acronym",
                "action-text-attachment",
                "address",
                "audio",
                "b",
                "big",
                "blockquote",
                "br",
                "cite",
                "code",
                "dd",
                "del",
                "details",
                "dfn",
                "div",
                "dl",
                "dt",
                "em",
                "embed",
                "figcaption",
                "figure",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "hr",
                "i",
                "img",
                "ins",
                "kbd",
                "li",
                "mark",
                "ol",
                "p",
                "pre",
                "s",
                "samp",
                "small",
                "source",
                "span",
                "strong",
                "sub",
                "summary",
                "sup",
                "table",
                "tbody",
                "td",
                "th",
                "thead",
                "time",
                "tr",
                "tt",
                "ul",
                "var",
                "video",
            ]))
            .tag_attributes(HashMap::from([("a", HashSet::from(["href", "title"]))]))
            .generic_attributes(HashSet::from([
                "abbr",
                "alt",
                "caption",
                "cite",
                "class",
                "content",
                "content-type",
                "controls",
                "data-language",
                "data-lightbox-caption-value",
                "data-lightbox-target",
                "data-turbo-frame",
                "datetime",
                "filename",
                "filesize",
                "height",
                "href",
                "lang",
                "name",
                "poster",
                "presentation",
                "previewable",
                "sgid",
                "src",
                "start",
                "style",
                "title",
                "type",
                "url",
                "value",
                "width",
                "xml:lang",
            ]))
            .filter_style_properties(style_properties)
            .clean_content_tags(HashSet::new())
            .link_rel(None)
            .url_relative(ammonia::UrlRelative::PassThrough);
        sanitizer
    });
    normalize_styles(sanitizer.clean(input).to_string())
}

fn normalize_styles(input: String) -> String {
    static STYLE: OnceLock<regex::Regex> = OnceLock::new();
    static FUNCTION: OnceLock<regex::Regex> = OnceLock::new();
    static COMMA: OnceLock<regex::Regex> = OnceLock::new();
    static IMPORTANT: OnceLock<regex::Regex> = OnceLock::new();
    let style = STYLE.get_or_init(|| regex::Regex::new(r#"style="([^"]*)""#).unwrap());
    let function = FUNCTION.get_or_init(|| regex::Regex::new(r"(?i)([a-z-]+)\s*\(").unwrap());
    let comma = COMMA.get_or_init(|| regex::Regex::new(r"\s*,\s*").unwrap());
    let important = IMPORTANT.get_or_init(|| regex::Regex::new(r"(?i)!\s+important").unwrap());
    style
        .replace_all(&input, |captures: &regex::Captures<'_>| {
            let declarations = captures[1]
                .split(';')
                .filter_map(|declaration| {
                    let (property, value) = declaration.split_once(':')?;
                    let property = property.trim().to_ascii_lowercase();
                    let value = value.trim();
                    let lowered = value.to_ascii_lowercase();
                    if value.is_empty()
                        || lowered.contains(['\\', '@'])
                        || lowered.contains("javascript:")
                        || lowered.contains("vbscript:")
                        || lowered.contains("data:")
                        || function.captures_iter(value).any(|match_| {
                            !matches!(
                                &match_[1].to_ascii_lowercase()[..],
                                "attr"
                                    | "blur"
                                    | "brightness"
                                    | "calc"
                                    | "circle"
                                    | "contrast"
                                    | "counter"
                                    | "counters"
                                    | "cubic-bezier"
                                    | "drop-shadow"
                                    | "ellipse"
                                    | "grayscale"
                                    | "hsl"
                                    | "hsla"
                                    | "hue-rotate"
                                    | "hwb"
                                    | "inset"
                                    | "invert"
                                    | "linear-gradient"
                                    | "matrix"
                                    | "matrix3d"
                                    | "opacity"
                                    | "perspective"
                                    | "polygon"
                                    | "radial-gradient"
                                    | "repeating-linear-gradient"
                                    | "repeating-radial-gradient"
                                    | "rgb"
                                    | "rgba"
                                    | "rotate"
                                    | "rotate3d"
                                    | "rotatex"
                                    | "rotatey"
                                    | "rotatez"
                                    | "saturate"
                                    | "sepia"
                                    | "scale"
                                    | "scale3d"
                                    | "scalex"
                                    | "scaley"
                                    | "scalez"
                                    | "skew"
                                    | "skewx"
                                    | "skewy"
                                    | "symbols"
                                    | "translate"
                                    | "translate3d"
                                    | "translatex"
                                    | "translatey"
                                    | "translatez"
                            )
                        })
                    {
                        return None;
                    }
                    let value = comma.replace_all(value, ", ");
                    let value = important.replace_all(&value, "!important");
                    Some(format!("{property}:{value};"))
                })
                .collect::<String>();
            format!("style=\"{declarations}\"")
        })
        .into_owned()
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

fn without_trailing_newlines(value: &str) -> &str {
    value.trim_end_matches(['\n', '\r'])
}

fn plain_node(node: &NodeRef) -> String {
    if let Some(text) = node.as_text() {
        return without_trailing_newlines(&text.borrow()).to_owned();
    }
    let tag = node
        .as_element()
        .map(|element| element.name.local.to_string())
        .unwrap_or_default();
    if matches!(tag.as_str(), "script" | "style" | "noscript") {
        return String::new();
    }
    if tag == "br" {
        return "\n".to_owned();
    }
    let children: String = node.children().map(|child| plain_node(&child)).collect();
    match tag.as_str() {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "p" => {
            format!("{}\n\n", without_trailing_newlines(&children))
        }
        "ul" | "ol" => {
            let nested = node.ancestors().any(|ancestor| {
                ancestor
                    .as_element()
                    .is_some_and(|element| matches!(element.name.local.as_ref(), "ul" | "ol"))
            });
            format!(
                "{}{}\n\n",
                if nested { "\n" } else { "" },
                without_trailing_newlines(&children)
            )
        }
        "div" => format!("{}\n", without_trailing_newlines(&children)),
        "figcaption" => format!("[{}]", without_trailing_newlines(&children)),
        "blockquote" => {
            let mut value = format!("{}\n\n", without_trailing_newlines(&children));
            let bounds = value
                .char_indices()
                .filter(|(_, ch)| !ch.is_whitespace())
                .map(|(index, ch)| (index, index + ch.len_utf8()))
                .collect::<Vec<_>>();
            if let (Some(first), Some(last)) = (bounds.first(), bounds.last()) {
                value.insert_str(last.1, "”");
                value.insert_str(first.0, "“");
                value
            } else {
                "“”".to_owned()
            }
        }
        "li" => {
            let lists = node
                .ancestors()
                .filter_map(|ancestor| {
                    ancestor
                        .as_element()
                        .map(|element| element.name.local.to_string())
                })
                .filter(|name| matches!(name.as_str(), "ul" | "ol"))
                .collect::<Vec<_>>();
            let bullet = if lists.first().is_some_and(|name| name == "ol") {
                let mut index = 1;
                let mut previous = node.previous_sibling();
                while let Some(sibling) = previous {
                    if sibling.as_element().is_some() {
                        index += 1;
                    }
                    previous = sibling.previous_sibling();
                }
                format!("{index}.")
            } else {
                "•".to_owned()
            };
            format!(
                "{}{} {}\n",
                "  ".repeat(lists.len().saturating_sub(1)),
                bullet,
                without_trailing_newlines(&children)
            )
        }
        _ => children,
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
    let text: String = body
        .as_node()
        .children()
        .map(|child| plain_node(&child))
        .collect();
    without_trailing_newlines(&text).to_owned()
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
