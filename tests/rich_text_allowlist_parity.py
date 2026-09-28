#!/usr/bin/env python3
"""Compare Action Text's broader HTML allowlist and plain-text conversion."""

import tempfile

from auth import stop
from search_html_parity import api, rust_target, source_target


CASES = {
    "table": "<table><thead><tr><th>Head</th></tr></thead><tbody><tr><td>Cell</td></tr></tbody></table>",
    "details": "<details open><summary>More</summary>Hidden</details>",
    "figure": "<figure><figcaption>Caption</figcaption><p>Body</p></figure>",
    "mark": "<p>This is <mark>important</mark></p>",
    "ins": "<p>Old <ins>new</ins> <del>removed</del></p>",
    "small": "<p><small>Small</small> normal</p>",
    "abbr": '<p><abbr title="Application">App</abbr></p>',
    "kbd": "<p>Press <kbd>Enter</kbd></p>",
    "subsup": "<p>H<sub>2</sub>O and x<sup>2</sup></p>",
    "time": '<p><time datetime="2026-01-01">today</time></p>',
    "quote": '<blockquote cite="https://example.com">Quote</blockquote>',
    "target": '<a href="https://example.com" target="_blank" title="Title">Link</a>',
    "id": '<p id="intro" class="note">Text</p>',
    "style": '<p style="color:red" class="note">Text</p>',
    "style_rgb": '<p style="color:rgb(255, 0, 0)">Text</p>',
    "style_filtered": '<p style="font-weight:bold;position:absolute">Text</p>',
    "style_url": '<p style="background-image:url(javascript:alert(1))">Text</p>',
    "style_expression": '<p style="color:expression(alert(1))">Text</p>',
    "style_margin": '<p style="margin-top:4px">Text</p>',
    "style_custom": '<p style="--custom:1">Text</p>',
    "style_important": '<p style="color:red!important">Text</p>',
    "style_variable": '<p style="color:var(--text-color)">Text</p>',
    "style_variable_fallback": '<p style="color:var(--text-color, red)">Text</p>',
    "style_border_radius": '<p style="border-radius:4px">Text</p>',
    "style_border_corner": '<p style="border-top-left-radius:4px">Text</p>',
    "data": '<p data-action="click->x#y" data-x="1">Text</p>',
    "aria": '<p aria-label="hello" role="note">Text</p>',
    "image": '<p>Photo <img src="https://example.com/a.png" alt="A"></p>',
    "image_data_png": '<img src="data:image/png;base64,iVBORw0KGgo=" alt="A">',
    "image_data_svg": '<img src="data:image/svg+xml;base64,aGVsbG8=" alt="A">',
    "link_data_text": '<a href="data:text/plain,hello">Text</a>',
    "link_data_html": '<a href="data:text/html,hello">HTML</a>',
    "video": '<video controls src="https://example.com/a.mp4">Fallback</video>',
    "iframe": '<iframe src="https://example.com">Inner</iframe>',
    "svg": '<svg><script>alert(1)</script><text>Visible</text></svg>',
    "math": '<math><mi>x</mi></math>',
    "script": '<script>alert(1)</script><p>After</p>',
    "style_tag": '<style>.x{color:red}</style><p>After</p>',
    "custom": '<fancy-tag class="x">Custom</fancy-tag>',
    "paragraph": '<p><span class="x">A</span><br><span>B</span></p>',
    "entity": '<p>&copy; &amp; &nbsp; &#169; &lt;code&gt;</p>',
    "nested_list": '<ol><li>One<ul><li>Sub</li></ul></li><li>Two</li></ol>',
    "code": '<pre><code class="language-rust">fn main() {}</code></pre>',
    "font": '<font color="red">Color</font>',
    "anchor": '<a href="mailto:hi@example.com" rel="external" download="x">Email</a>',
}


def run(target):
    board = api(target, "/boards", "POST", {"board": {"name": "Rich-text allowlist parity"}}, 201)
    try:
        results = {}
        for label, description in CASES.items():
            card = api(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": label, "description": description}}, 201)
            results[label] = (card["description"], card["description_html"])
        return results
    finally:
        api(target, f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    source = source_target()
    with tempfile.TemporaryDirectory(prefix="rizzy-rich-text-allowlist-") as temp:
        rust, process = rust_target(temp)
        try:
            expected = run(source)
            actual = run(rust)
            differences = {key: (expected[key], actual[key]) for key in CASES if expected[key] != actual[key]}
            assert not differences, differences
            print(f"Rich-text allowlist matched Fizzy in {len(CASES)} cases")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
