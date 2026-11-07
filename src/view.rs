use ego_tree::NodeId;
use scraper::node::Text;
use scraper::{ElementRef, Html, Node, Selector};

const CONTENT_ROOTS: [&str; 5] = [
    "#mw-content-text > .mw-parser-output",
    ".mw-parser-output",
    "main",
    "article",
    "body",
];
const NOISE: &str = "script, style, nav, footer, noscript, img, .mw-editsection, #toc, .toc, .navbox, \
                     .infobox-switch-resources, .infobox-switch-buttons-caption";

pub fn is_html(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|ct| ct.split(';').next())
        .map(str::trim)
        .is_some_and(|mime| {
            mime.eq_ignore_ascii_case("text/html")
                || mime.eq_ignore_ascii_case("application/xhtml+xml")
        })
}

pub fn html_to_markdown(html: &str) -> String {
    let mut document = Html::parse_document(html);
    let noise: Vec<_> = document.select(&selector(NOISE)).map(|e| e.id()).collect();
    detach(&mut document, noise);
    tidy_links(&mut document);
    drop_empty_rows(&mut document);
    lift_caption_rows(&mut document);
    normalise_table_cells(&mut document);
    let content = CONTENT_ROOTS
        .iter()
        .find_map(|root| document.select(&selector(root)).next())
        .map(|root| root.inner_html())
        .unwrap_or_default();
    compact_tables(htmd::convert(&content).unwrap_or_default().trim())
}

fn detach(document: &mut Html, ids: Vec<NodeId>) {
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }
}

fn tidy_links(document: &mut Html) {
    let anchors = selector("a");
    let links: Vec<_> = document
        .select(&anchors)
        .map(|link| {
            let has_text = has_text(link);
            let title = link.value().attr("title").map(str::trim).map(str::to_owned);
            let repeated = !has_text && repeats_a_text_link_in_its_row(link, &anchors);
            (
                link.id(),
                has_text,
                title.filter(|title| !title.is_empty() && !repeated),
            )
        })
        .collect();
    for (id, has_text, title) in links {
        let Some(mut node) = document.tree.get_mut(id) else {
            continue;
        };
        match (has_text, title) {
            (true, _) => {}
            (false, Some(title)) => {
                node.append(Node::Text(Text { text: title.into() }));
            }
            (false, None) => {
                node.detach();
                continue;
            }
        }
        if let Node::Element(element) = node.value() {
            element.attrs = std::mem::take(&mut element.attrs)
                .into_iter()
                .filter(|(name, _)| &*name.local != "title")
                .collect();
        }
    }
}

fn has_text(element: ElementRef) -> bool {
    element.text().any(|text| !text.trim().is_empty())
}

fn repeats_a_text_link_in_its_row(link: ElementRef, anchors: &Selector) -> bool {
    let Some(href) = link.value().attr("href") else {
        return false;
    };
    link.ancestors()
        .filter_map(ElementRef::wrap)
        .find(|ancestor| ancestor.value().name() == "tr")
        .is_some_and(|row| {
            row.select(anchors).any(|other| {
                other.id() != link.id()
                    && other.value().attr("href") == Some(href)
                    && has_text(other)
            })
        })
}

fn drop_empty_rows(document: &mut Html) {
    let empty: Vec<_> = document
        .select(&selector("tr"))
        .filter(|row| !has_text(*row))
        .map(|row| row.id())
        .collect();
    detach(document, empty);
}

/// A first row holding a single caption cell (an infobox title spanning the table) would
/// otherwise become a one-column header over a wider table.
fn lift_caption_rows(document: &mut Html) {
    let (rows, cells) = (selector("tr"), selector("th, td"));
    let mut lifts = Vec::new();
    for table in document.select(&selector("table")) {
        let mut table_rows = table.select(&rows);
        let Some(first) = table_rows.next() else {
            continue;
        };
        let filled: Vec<_> = first
            .select(&cells)
            .filter(|cell| has_text(*cell))
            .collect();
        let widest_other = table_rows.map(|row| row.select(&cells).count()).max();
        if let ([caption], Some(2..)) = (filled.as_slice(), widest_other) {
            lifts.push((table.id(), first.id(), caption.id()));
        }
    }
    for (table, row, caption) in lifts {
        rename(document, row, "p");
        rename(document, caption, "strong");
        if let Some(mut row_node) = document.tree.get_mut(row) {
            row_node.detach();
        }
        if let Some(mut table_node) = document.tree.get_mut(table) {
            table_node.insert_id_before(row);
        }
    }
}

fn rename(document: &mut Html, id: NodeId, tag: &str) {
    if let Some(mut node) = document.tree.get_mut(id)
        && let Node::Element(element) = node.value()
    {
        element.name.local = tag.into();
    }
}

/// htmd builds the header from the first row's `th` cells and reads only `td` cells after it,
/// so any other mix of cell types loses data.
fn normalise_table_cells(document: &mut Html) {
    let (rows, th, td) = (selector("tr"), selector("th"), selector("td"));
    let mut renames = Vec::new();
    for table in document.select(&selector("table")) {
        for (index, row) in table.select(&rows).enumerate() {
            let (from, to) = if index == 0 { (&td, "th") } else { (&th, "td") };
            renames.extend(row.select(from).map(|cell| (cell.id(), to)));
        }
    }
    for (id, tag) in renames {
        rename(document, id, tag);
    }
}

fn compact_tables(markdown: &str) -> String {
    let mut in_fence = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence || !line.starts_with('|') {
            lines.push(line.to_owned());
            continue;
        }
        let cells: Vec<&str> = split_cells(line).into_iter().map(str::trim).collect();
        if cells.iter().all(|cell| cell.is_empty()) {
            continue;
        }
        let separator = cells.iter().any(|cell| is_rule(cell))
            && cells.iter().all(|cell| cell.is_empty() || is_rule(cell));
        let cells: Vec<&str> = if separator {
            vec!["---"; cells.len()]
        } else {
            cells
        };
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    lines.join("\n")
}

fn split_cells(row: &str) -> Vec<&str> {
    let inner = row.trim().strip_prefix('|').unwrap_or(row);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    inner.split('|').collect()
}

fn is_rule(cell: &str) -> bool {
    !cell.is_empty() && cell.chars().all(|ch| ch == '-')
}

struct Heading {
    line: usize,
    level: usize,
    title: String,
}

pub fn plain(markdown: &str) -> String {
    let mut in_fence = false;
    let mut lines = Vec::new();
    for line in markdown.lines() {
        let fence = is_fence(line);
        if fence {
            in_fence = !in_fence;
        }
        if in_fence || fence {
            lines.push(line.to_owned());
        } else {
            lines.push(link_text(line));
        }
    }
    lines.join("\n")
}

pub fn outline(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    headings(&lines)
        .iter()
        .map(|heading| format!("{} {}", "#".repeat(heading.level), heading.title))
        .collect()
}

pub fn section(text: &str, heading: &str) -> Result<String, Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let headings = headings(&lines);
    let wanted = heading.trim().to_lowercase();
    let Some(index) = headings
        .iter()
        .position(|h| h.title.to_lowercase() == wanted)
    else {
        return Err(headings.into_iter().map(|h| h.title).collect());
    };
    let start = &headings[index];
    let end = headings[index + 1..]
        .iter()
        .find(|h| h.level <= start.level)
        .map_or(lines.len(), |h| h.line);
    Ok(lines[start.line..end].join("\n").trim_end().to_owned())
}

fn headings(lines: &[&str]) -> Vec<Heading> {
    let mut in_fence = false;
    let mut found = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        if is_fence(text) {
            in_fence = !in_fence;
        } else if !in_fence && let Some((level, title)) = parse_heading(text) {
            let title = link_text(title);
            found.push(Heading { line, level, title });
        }
    }
    found
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    let rest = &line[level..];
    if !(1..=6).contains(&level) || !(rest.is_empty() || rest.starts_with(' ')) {
        return None;
    }
    let title = rest.trim();
    let unclosed = title.trim_end_matches('#');
    if unclosed.is_empty() || unclosed.ends_with(' ') {
        Some((level, unclosed.trim_end()))
    } else {
        Some((level, title))
    }
}

fn link_text(markdown: &str) -> String {
    let chars: Vec<char> = markdown.chars().collect();
    let mut out = String::with_capacity(markdown.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                out.extend(&chars[i..i + 2]);
                i += 2;
            }
            '`' => {
                let end = code_span_end(&chars, i).unwrap_or(i + backtick_run(&chars, i));
                out.extend(&chars[i..end]);
                i = end;
            }
            '[' => {
                if let Some((label, end)) = inline_link(&chars, i) {
                    out.push_str(&link_text(&label));
                    i = end;
                } else {
                    out.push('[');
                    i += 1;
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

fn backtick_run(chars: &[char], start: usize) -> usize {
    chars[start..].iter().take_while(|&&c| c == '`').count()
}

fn code_span_end(chars: &[char], start: usize) -> Option<usize> {
    let run = backtick_run(chars, start);
    let mut i = start + run;
    while i < chars.len() {
        if chars[i] == '`' {
            let closing = backtick_run(chars, i);
            if closing == run {
                return Some(i + closing);
            }
            i += closing;
        } else {
            i += 1;
        }
    }
    None
}

/// Returns the label and the index just past `[label](target)` when one starts at `start`.
fn inline_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    let label_end = closing(chars, start, '[', ']')?;
    if chars.get(label_end + 1) != Some(&'(') {
        return None;
    }
    let target_end = closing(chars, label_end + 1, '(', ')')?;
    Some((chars[start + 1..label_end].iter().collect(), target_end + 1))
}

fn closing(chars: &[char], open_at: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    let mut i = open_at;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 1,
            '`' if open == '[' => {
                if let Some(end) = code_span_end(chars, i) {
                    i = end;
                    continue;
                }
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn is_fence(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with("```") || line.starts_with("~~~")
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("selectors are compile-time constants")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKI: &str = include_str!("../tests/fixtures/wiki.html");

    #[test]
    fn picks_article_parser_output() {
        let md = html_to_markdown(WIKI);
        assert!(md.starts_with("Abyssal demons are demons"), "{md}");
        assert!(!md.contains("From the Old School"), "{md}");
        assert!(!md.contains("Protected"), "{md}");
    }

    #[test]
    fn first_row_label_value_pairs_survive() {
        let md = html_to_markdown(
            "<body><table><tr><th>Members</th><td>Yes</td></tr>\
             <tr><th>Weight</th><td>0.453 kg</td></tr></table></body>",
        );
        assert!(md.contains("| Members | Yes |"), "{md}");
        assert!(md.contains("| Weight | 0.453 kg |"), "{md}");
    }

    #[test]
    fn compaction_skips_code_and_drops_empty_rows() {
        let md = html_to_markdown(
            "<body><pre><code>|  keep   spacing  |</code></pre>\
             <table><tr><th>A</th></tr><tr><td></td></tr><tr><td>1</td></tr></table></body>",
        );
        assert!(md.contains("|  keep   spacing  |"), "{md}");
        assert!(md.ends_with("| A |\n| --- |\n| 1 |"), "{md}");
    }

    #[test]
    fn drops_script_style_and_editsection() {
        let md = html_to_markdown(WIKI);
        assert!(!md.contains("[edit]"), "{md}");
        assert!(!md.contains("trackingNoise"), "{md}");
        assert!(!md.contains("color: red"), "{md}");
        assert!(!md.contains("Enable JavaScript"), "{md}");
    }

    #[test]
    fn atx_headings() {
        let md = html_to_markdown(WIKI);
        assert!(md.contains("## Drops"), "{md}");
        assert!(md.contains("### 100%"), "{md}");
    }

    #[test]
    fn falls_back_to_body() {
        assert_eq!(
            html_to_markdown("<html><body><p>hi</p></body></html>"),
            "hi"
        );
    }

    #[test]
    fn prefers_main_over_body() {
        let md = html_to_markdown("<body><header>menu</header><main><p>core</p></main></body>");
        assert_eq!(md, "core");
    }

    #[test]
    fn compacts_table_padding() {
        let md = html_to_markdown(
            "<body><table><tr><th>A very long header cell</th><th>B</th></tr>\
             <tr><td>1</td><td>2</td></tr></table></body>",
        );
        assert_eq!(
            md,
            "| A very long header cell | B |\n| --- | --- |\n| 1 | 2 |"
        );
    }

    #[test]
    fn keeps_row_header_cells() {
        let md = html_to_markdown(
            "<body><table><tr><th>Item</th><th>Weight</th></tr>\
             <tr><th>Members</th><td>Yes</td></tr>\
             <tr><th colspan=\"2\">Combat</th></tr>\
             <tr><th>Attack</th><td>+82</td></tr></table></body>",
        );
        assert!(md.contains("| Members | Yes |"), "{md}");
        assert!(md.contains("| Combat |"), "{md}");
        assert!(md.contains("| Attack | +82 |"), "{md}");
    }

    #[test]
    fn keeps_header_row_that_mixes_cell_types() {
        let md = html_to_markdown(
            "<body><table><tr><th colspan=\"2\">Attack bonuses</th><td>img</td></tr>\
             <tr><th>Stab</th><th>Slash</th></tr>\
             <tr><td>+0</td><td>+82</td></tr></table></body>",
        );
        assert!(md.starts_with("| Attack bonuses |"), "{md}");
        assert!(md.contains("| Stab | Slash |"), "{md}");
        assert!(md.contains("| +0 | +82 |"), "{md}");
        assert!(md.lines().next().unwrap().contains("img"), "{md}");
    }

    #[test]
    fn icon_link_next_to_the_same_text_link_is_dropped() {
        let md = html_to_markdown(
            "<body><table><tr><th>Item</th><th>Name</th></tr>\
             <tr><td><a href=\"/w/Death_rune\" title=\"Death rune\"><img src=\"/d.png\"></a></td>\
             <td><a href=\"/w/Death_rune\" title=\"Death rune\">Death rune</a></td></tr></table></body>",
        );
        assert_eq!(md.matches("[Death rune]").count(), 1, "{md}");
    }

    #[test]
    fn duplicate_rule_matches_on_href_and_spares_icon_headers() {
        let md = html_to_markdown(
            "<body><table><tr><th><a href=\"/w/Stab\" title=\"Stab\"><img src=\"/s.png\"></a></th>\
             <th><a href=\"/w/Slash\" title=\"Slash\"><img src=\"/l.png\"></a></th></tr>\
             <tr><td><a href=\"/w/Death_rune\" title=\"Death rune\"><img src=\"/d.png\"></a>\
             <a href=\"/w/Death_rune\">Death rune</a></td>\
             <td><a href=\"/w/Blood_rune\" title=\"Blood rune\"><img src=\"/b.png\"></a>\
             <a href=\"/w/Blood_rune\">Blood rune</a></td></tr></table></body>",
        );
        assert!(
            md.contains("| [Stab](/w/Stab) | [Slash](/w/Slash) |"),
            "{md}"
        );
        assert_eq!(md.matches("[Death rune]").count(), 1, "{md}");
        assert_eq!(md.matches("[Blood rune]").count(), 1, "{md}");
    }

    #[test]
    fn separator_rows_stay_valid_when_a_column_is_empty() {
        let md = html_to_markdown(
            "<body><table><tr><th></th><th>Item</th></tr><tr><td></td><td>Ashes</td></tr></table></body>",
        );
        assert!(md.contains("| --- | --- |"), "{md}");
    }

    #[test]
    fn infobox_switch_data_is_dropped() {
        let md = html_to_markdown(
            "<div class=\"mw-parser-output\"><table class=\"infobox\"><tr><th>Attack</th><td>250</td></tr></table>\
             <div class=\"infobox-switch-resources hidden\"><span data-attr-param=\"att\">250</span>\
             <span data-attr-param=\"str\">140</span></div></div>",
        );
        assert!(!md.contains("140"), "{md}");
        let buttons = html_to_markdown(
            "<div class=\"mw-parser-output\"><div class=\"infobox-switch-buttons-caption\">Normal</div>\
             <p>Stats</p></div>",
        );
        assert_eq!(buttons, "Stats");
    }

    #[test]
    fn caption_only_first_row_becomes_a_caption() {
        let md = html_to_markdown(
            "<body><table><tr><th colspan=\"2\">Cache of runes</th></tr>\
             <tr><td colspan=\"2\" class=\"infobox-padding\"></td></tr>\
             <tr><td colspan=\"2\"><img src=\"/i.png\"></td></tr>\
             <tr><th>Released</th><td>2024</td></tr><tr><th>Members</th><td>Yes</td></tr></table></body>",
        );
        assert!(md.starts_with("**Cache of runes**\n\n| Released |"), "{md}");
        assert!(!md.contains("| Cache of runes |"), "{md}");
        assert!(md.contains("| Released | 2024 |\n| --- | --- |"), "{md}");
        assert!(md.contains("| Members | Yes |"), "{md}");
    }

    #[test]
    fn plain_handles_the_link_shapes_htmd_produces() {
        let cases = [
            (r"see [Foo](/w/Foo_\(bar\)) now", "see Foo now"),
            ("use [`at()`](/at) here", "use `at()` here"),
            (r"\[sic\] see [x](/u)", r"\[sic\] see x"),
            ("a ` b [x](/u)", "a ` b x"),
            ("keep `[a](b)` and [c](/d)", "keep `[a](b)` and c"),
            ("[outer [inner] text](/u)", "outer [inner] text"),
            ("no links here", "no links here"),
        ];
        for (markdown, expected) in cases {
            assert_eq!(plain(markdown), expected, "{markdown}");
        }
        assert_eq!(plain("```\n[a](b)\n```\n[c](d)"), "```\n[a](b)\n```\nc");
    }

    #[test]
    fn icon_links_become_their_title() {
        let md = html_to_markdown(
            "<body><p><a href=\"/w/Stab\" title=\"Stab\"><img src=\"/s.png\"></a></p></body>",
        );
        assert_eq!(md, "[Stab](/w/Stab)");
    }

    #[test]
    fn strips_link_titles_and_empty_links() {
        let md = html_to_markdown(
            "<body><p><a href=\"/w/File:Icon.png\"><img src=\"/i.png\"></a>\
             <a href=\"/w/Members\" title=\"Members\">Members</a></p></body>",
        );
        assert_eq!(md, "[Members](/w/Members)");
    }

    #[test]
    fn drops_images_toc_and_navboxes() {
        let md = html_to_markdown(
            "<div class=\"mw-parser-output\"><p><img src=\"/icon.png\">text</p>\
             <div id=\"toc\" class=\"toc\">Contents</div>\
             <table class=\"navbox\"><tr><td>Everything else</td></tr></table></div>",
        );
        assert_eq!(md, "text");
    }

    const OUTLINE: &str = "# A\nx\n## B\ny\n### C\nz\n## D\nw";

    #[test]
    fn section_until_same_level() {
        assert_eq!(section(OUTLINE, "b").unwrap(), "## B\ny\n### C\nz");
    }

    #[test]
    fn section_last_runs_to_end() {
        assert_eq!(section(OUTLINE, "D").unwrap(), "## D\nw");
    }

    #[test]
    fn section_case_insensitive_trimmed() {
        assert_eq!(
            section("## Drops\nash", "  drops ").unwrap(),
            "## Drops\nash"
        );
    }

    #[test]
    fn section_ignores_hash_in_code_fence() {
        let text = "## Setup\n```sh\n# comment\n```\nafter\n## Next\nn";
        assert_eq!(
            section(text, "setup").unwrap(),
            "## Setup\n```sh\n# comment\n```\nafter"
        );
        assert!(section(text, "comment").is_err());
    }

    #[test]
    fn section_keeps_hash_inside_title() {
        let text = "## C#\nsharp\n## Drops ##\nash\n#hashtag\n## Über\nu";
        assert_eq!(section(text, "c#").unwrap(), "## C#\nsharp");
        assert_eq!(
            section(text, "drops").unwrap(),
            "## Drops ##\nash\n#hashtag"
        );
        assert_eq!(section(text, "über").unwrap(), "## Über\nu");
    }

    #[test]
    fn section_matches_linked_headings_by_their_text() {
        let text = "# Array.prototype.at()\n## [Description](#description)\nbody\n## [Examples](#examples)\nx";
        assert_eq!(
            section(text, "description").unwrap(),
            "## [Description](#description)\nbody"
        );
        assert_eq!(
            section(text, "nope").unwrap_err(),
            ["Array.prototype.at()", "Description", "Examples"]
        );
    }

    #[test]
    fn section_missing_lists_headings() {
        assert_eq!(section(OUTLINE, "nope").unwrap_err(), ["A", "B", "C", "D"]);
    }

    #[test]
    fn is_html_ignores_charset() {
        assert!(is_html(Some("text/html; charset=utf-8")));
        assert!(is_html(Some("application/xhtml+xml")));
        assert!(!is_html(Some("application/json")));
        assert!(!is_html(None));
    }
}
