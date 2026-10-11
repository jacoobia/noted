use ego_tree::NodeId;
use scraper::node::Text;
use scraper::{Html, Node, Selector};

const CONTENT_ROOTS: [&str; 5] = [
    "#mw-content-text > .mw-parser-output",
    ".mw-parser-output",
    "main",
    "article",
    "body",
];
const NOISE: &str =
    "script, style, nav, footer, noscript, img, .mw-editsection, #toc, .toc, .navbox";

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
    let links: Vec<_> = document
        .select(&selector("a"))
        .map(|link| {
            let has_text = link.text().any(|text| !text.trim().is_empty());
            let title = link.value().attr("title").map(str::trim).map(str::to_owned);
            (link.id(), has_text, title.filter(|title| !title.is_empty()))
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
        if let Some(mut node) = document.tree.get_mut(id)
            && let Node::Element(element) = node.value()
        {
            element.name.local = tag.into();
        }
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
        let cells: Vec<&str> = cells
            .into_iter()
            .map(|cell| if is_rule(cell) { "---" } else { cell })
            .collect();
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

struct Heading<'a> {
    line: usize,
    level: usize,
    title: &'a str,
}

pub fn section(text: &str, heading: &str) -> Result<String, Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let headings = headings(&lines);
    let wanted = heading.trim().to_lowercase();
    let Some(index) = headings
        .iter()
        .position(|h| h.title.to_lowercase() == wanted)
    else {
        return Err(headings.iter().map(|h| h.title.to_owned()).collect());
    };
    let start = &headings[index];
    let end = headings[index + 1..]
        .iter()
        .find(|h| h.level <= start.level)
        .map_or(lines.len(), |h| h.line);
    Ok(lines[start.line..end].join("\n").trim_end().to_owned())
}

fn headings<'a>(lines: &[&'a str]) -> Vec<Heading<'a>> {
    let mut in_fence = false;
    let mut found = Vec::new();
    for (line, text) in lines.iter().enumerate() {
        if is_fence(text) {
            in_fence = !in_fence;
        } else if !in_fence && let Some((level, title)) = parse_heading(text) {
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
