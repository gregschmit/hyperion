//! What the pages show.

use m8r::views::markdown::render;

#[test]
fn what_a_run_printed_shows_as_markdown() {
    let page = render(
        "# Review\n\nSome **bold**, `code` and ~~gone~~.\n\n- [x] done\n- open\n\n\
         | a | b |\n|---|---|\n| 1 | 2 |\n\n```rust\nlet a = \"<b>\";\n```\n",
    );
    assert_eq!(
        page,
        "<h1>Review</h1>\n\
         <p>Some <strong>bold</strong>, <code>code</code> and <del>gone</del>.</p>\n\
         <ul>\n<li><input disabled=\"\" type=\"checkbox\" checked=\"\"/>\ndone</li>\n\
         <li>open</li>\n</ul>\n\
         <table><thead><tr><th>a</th><th>b</th></tr></thead><tbody>\n\
         <tr><td>1</td><td>2</td></tr>\n</tbody></table>\n\
         <pre><code class=\"language-rust\">let a = \"&lt;b&gt;\";\n</code></pre>\n"
    );
    // What a command prints isn't always markdown: its lines stay lines.
    assert_eq!(
        render("step 1\nstep 2\n\ndone"),
        "<p>step 1<br />\nstep 2</p>\n<p>done</p>\n"
    );
    assert_eq!(render(""), "");
}

#[test]
fn html_in_what_a_run_printed_is_text() {
    // In a line, and as a block. An agent may have read a hostile issue before it wrote this.
    assert_eq!(
        render("Hello <b onclick=\"x()\">there</b> <script>alert(1)</script> <!-- c -->"),
        "<p>Hello &lt;b onclick=\"x()\"&gt;there&lt;/b&gt; \
         &lt;script&gt;alert(1)&lt;/script&gt; &lt;!-- c --&gt;</p>\n"
    );
    assert_eq!(
        render("<script>\nalert(1)\n</script>\n\nAfter."),
        "<pre><code>&lt;script&gt;\nalert(1)\n&lt;/script&gt;\n</code></pre>\n<p>After.</p>\n"
    );
    let page = render("<details><summary>More</summary>\n\nHidden *text*.\n\n</details>");
    assert!(
        !page.contains("<details") && !page.contains("<summary"),
        "{page}"
    );
    assert!(page.contains("<p>Hidden <em>text</em>.</p>"), "{page}");
}

#[test]
fn a_link_goes_to_a_web_or_a_mail_address_or_it_is_text() {
    assert_eq!(
        render(
            "[ok](https://example.com/a?b=1&c=2 \"T\") <https://example.com> <me@example.com> \
             [mail](mailto:a@b.c)"
        ),
        "<p><a href=\"https://example.com/a?b=1&amp;c=2\" title=\"T\">ok</a> \
         <a href=\"https://example.com\">https://example.com</a> \
         <a href=\"mailto:me@example.com\">me@example.com</a> \
         <a href=\"mailto:a@b.c\">mail</a></p>\n"
    );
    // Nothing that a browser would run, and no path on m8r itself: only the words stay.
    assert_eq!(
        render(
            "[bad](javascript:alert(1)) [JS](JaVaScRiPt:alert(1)) [rel](/schedules/1/remove) \
             [data](data:text/html,<script>alert(1)</script>) [ref]\n\n[ref]: vbscript:msgbox(1)"
        ),
        "<p>bad JS rel data ref</p>\n"
    );
    // A quote in an address can't end the attribute that holds it.
    assert_eq!(
        render("[x](https://e.example/\"onmouseover=\"alert(1))"),
        "<p><a href=\"https://e.example/%22onmouseover=%22alert(1)\">x</a></p>\n"
    );
}

#[test]
fn a_picture_is_a_link_so_the_page_loads_nothing_from_elsewhere() {
    let page = render(
        "![chart](https://evil.example/x.png?leak=1) ![](https://e.example/x.png) \
         ![x](javascript:alert(1)) [![Build](https://img.example/b.svg)](https://ci.example/run)",
    );
    assert!(!page.contains("<img"), "{page}");
    assert_eq!(
        page,
        // Its description is the link's text, or where it goes if it has none. One that goes
        // nowhere safe is its description alone, and one inside a link is that link's text.
        "<p><a href=\"https://evil.example/x.png?leak=1\">chart</a> \
         <a href=\"https://e.example/x.png\">https://e.example/x.png</a> \
         x <a href=\"https://ci.example/run\">Build</a></p>\n"
    );
}
