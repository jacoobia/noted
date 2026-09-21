use ego_tree::NodeId;
use scraper::{Html, Node, Selector};

const CONTENT_ROOTS: [&str; 4] = [".mw-parser-output", "main", "article", "body"];
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
    demote_row_headers(&mut document);
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
    let (empty, labelled): (Vec<_>, Vec<_>) = document
        .select(&selector("a"))
        .partition(|link| link.text().all(|text| text.trim().is_empty()));
    let labelled: Vec<_> = labelled.into_iter().map(|link| link.id()).collect();
    detach(document, empty.into_iter().map(|link| link.id()).collect());
    for id in labelled {
        if let Some(mut node) = document.tree.get_mut(id)
            && let Node::Element(element) = node.value()
        {
            element.attrs = std::mem::take(&mut element.attrs)
                .into_iter()
                .filter(|(name, _)| &*name.local != "title")
                .collect();
        }
    }
}

/// htmd only reads `td` cells below a table's header row, so a `th` used as a row label
/// would vanish along with its row.
fn demote_row_headers(document: &mut Html) {
    let mut row_headers = Vec::new();
    for table in document.select(&selector("table")) {
        for (index, row) in table.select(&selector("tr")).enumerate() {
            if index > 0 || row.select(&selector("td")).next().is_some() {
                row_headers.extend(row.select(&selector("th")).map(|cell| cell.id()));
            }
        }
    }
    for id in row_headers {
        if let Some(mut node) = document.tree.get_mut(id)
            && let Node::Element(element) = node.value()
        {
            element.name.local = "td".into();
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
    let mut cells = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (i, ch) in inner.char_indices() {
        match ch {
            '\\' => escaped = !escaped,
            '|' if !escaped => {
                cells.push(&inner[start..i]);
                start = i + 1;
            }
            _ => escaped = false,
        }
    }
    cells.push(&inner[start..]);
    cells
}

fn is_rule(cell: &str) -> bool {
    !cell.is_empty() && cell.chars().all(|ch| ch == '-')
}

fn selector(css: &str) -> Selector {
    Selector::parse(css).expect("selectors are compile-time constants")
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIKI: &str = include_str!("../tests/fixtures/wiki.html");

    #[test]
    fn picks_mw_parser_output() {
        let md = html_to_markdown(WIKI);
        assert!(md.contains("Abyssal demons are demons"), "{md}");
        assert!(!md.contains("Random page"), "{md}");
        assert!(!md.contains("CC BY-NC-SA"), "{md}");
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
    fn strips_link_titles_and_empty_links() {
        let md = html_to_markdown(
            "<body><p><a href=\"/w/Icon\" title=\"Icon\"><img src=\"/i.png\"></a>\
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

    #[test]
    fn is_html_ignores_charset() {
        assert!(is_html(Some("text/html; charset=utf-8")));
        assert!(is_html(Some("application/xhtml+xml")));
        assert!(!is_html(Some("application/json")));
        assert!(!is_html(None));
    }
}
