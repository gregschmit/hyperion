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
         <div class=\"scroll\"><table><thead><tr>\
         <th><span class=\"word\">a</span></th><th><span class=\"word\">b</span></th>\
         </tr></thead><tbody>\n\
         <tr><td><span class=\"word\">1</span></td><td><span class=\"word\">2</span></td></tr>\n\
         </tbody></table>\n</div>\n\
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
fn a_table_is_in_a_box_that_scrolls_and_its_words_are_each_in_a_box_that_stays_whole() {
    assert_eq!(
        render(
            "| MR | Why |\n|---|---|\n\
             | [!12](https://e.example/12) | DOM-only `pull --ff-only` a<b>c |\n\ntext-safe"
        ),
        // A word is what's between two spaces: in a link, in code, and when it came in parts,
        // as one with HTML in it does. Outside a table, a line breaks where the browser likes.
        "<div class=\"scroll\"><table><thead><tr>\
         <th><span class=\"word\">MR</span></th><th><span class=\"word\">Why</span></th>\
         </tr></thead><tbody>\n<tr>\
         <td><a href=\"https://e.example/12\"><span class=\"word\">!12</span></a></td>\
         <td><span class=\"word\">DOM-only</span> \
         <code><span class=\"word\">pull</span> <span class=\"word\">--ff-only</span></code> \
         <span class=\"word\">a&lt;b&gt;c</span></td>\
         </tr>\n</tbody></table>\n</div>\n\
         <p>text-safe</p>\n"
    );
}

#[test]
fn a_bare_web_address_is_a_link_to_itself() {
    // What ends the sentence or closes the brackets around it isn't part of it, and brackets
    // of its own are. Part of a word is no address, and nor is the start of one alone.
    assert_eq!(
        render(
            "See https://example.com/a_b?c=1&d=2. (Or http://e.example/x_(y)), and \
             [https://e.example/z]; not xhttp://no.example, https:// or https://<host>/."
        ),
        "<p>See <a href=\"https://example.com/a_b?c=1&amp;d=2\">\
         https://example.com/a_b?c=1&amp;d=2</a>. \
         (Or <a href=\"http://e.example/x_(y)\">http://e.example/x_(y)</a>), and \
         [<a href=\"https://e.example/z\">https://e.example/z</a>]; \
         not xhttp://no.example, https:// or https://&lt;host&gt;/.</p>\n"
    );
    // Not where it's a link's text, and not in code.
    assert_eq!(
        render(
            "[https://a.example](https://b.example) <https://c.example> `https://d.example`\n\n\
             ```\nhttps://e.example\n```\n"
        ),
        "<p><a href=\"https://b.example\">https://a.example</a> \
         <a href=\"https://c.example\">https://c.example</a> \
         <code>https://d.example</code></p>\n\
         <pre><code>https://e.example\n</code></pre>\n"
    );
    // In HTML, which shows as text, and in a table's cell, where it's a word.
    assert_eq!(
        render("<a href=\"https://f.example/?q=1\">f</a>"),
        "<p>&lt;a href=\"<a href=\"https://f.example/?q=1\">https://f.example/?q=1</a>\"&gt;\
         f&lt;/a&gt;</p>\n"
    );
    assert!(
        render("| a |\n|---|\n| at https://g.example/a-b |").contains(
            "<td><span class=\"word\">at</span> <a href=\"https://g.example/a-b\">\
             <span class=\"word\">https://g.example/a-b</span></a></td>"
        ),
        "{}",
        render("| a |\n|---|\n| at https://g.example/a-b |")
    );
}

#[test]
fn a_note_of_githubs_kind_has_its_kind_as_its_class() {
    assert_eq!(
        render("> [!WARNING]\n> Mind the gap.\n\n> [!NOTE]\n> One more.\n\n> A plain quote."),
        "<blockquote class=\"markdown-alert-warning\">\n<p>Mind the gap.</p>\n</blockquote>\n\
         <blockquote class=\"markdown-alert-note\">\n<p>One more.</p>\n</blockquote>\n\
         <blockquote>\n<p>A plain quote.</p>\n</blockquote>\n"
    );
}

#[test]
fn a_footnote_has_a_number_and_a_name_that_nothing_else_on_the_page_has() {
    assert_eq!(
        // `output-raw` is the name of a tab on the run's page.
        render(
            "A claim.[^1] One more.[^output-raw] None: [^none].\n\n\
             [^1]: See https://example.com/source.\n\n[^output-raw]: Because."
        ),
        "<p>A claim.<sup class=\"footnote-reference\"><a href=\"#footnote-1\">1</a></sup> \
         One more.<sup class=\"footnote-reference\"><a href=\"#footnote-output-raw\">2</a></sup> \
         None: [^none].</p>\n\
         <div class=\"footnote-definition\" id=\"footnote-1\">\
         <sup class=\"footnote-definition-label\">1</sup>\n\
         <p>See <a href=\"https://example.com/source\">https://example.com/source</a>.</p>\n\
         </div>\n\
         <div class=\"footnote-definition\" id=\"footnote-output-raw\">\
         <sup class=\"footnote-definition-label\">2</sup>\n\
         <p>Because.</p>\n\
         </div>\n"
    );
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
