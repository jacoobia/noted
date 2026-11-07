use url::Url;

const MAX_ROBOTS_BYTES: usize = 500 * 1024;

pub fn product_token(user_agent: &str) -> &str {
    user_agent
        .split(['/', ' ', '('])
        .next()
        .unwrap_or(user_agent)
}

pub fn robots_url(page: &Url) -> String {
    let mut robots = page.clone();
    robots.set_path("/robots.txt");
    robots.set_query(None);
    robots.set_fragment(None);
    robots.into()
}

pub fn path_and_query(page: &Url) -> String {
    match page.query() {
        Some(query) => format!("{}?{query}", page.path()),
        None => page.path().to_owned(),
    }
}

pub fn is_allowed(robots_txt: &str, token: &str, path: &str) -> bool {
    if path == "/robots.txt" {
        return true;
    }
    let path = normalise(path);
    let groups = parse(robots_txt);
    let named: Vec<&Group> = groups
        .iter()
        .filter(|group| group.agents.iter().any(|a| a.eq_ignore_ascii_case(token)))
        .collect();
    let chosen = if named.is_empty() {
        groups
            .iter()
            .filter(|group| group.agents.iter().any(|a| a == "*"))
            .collect()
    } else {
        named
    };
    chosen
        .iter()
        .flat_map(|group| &group.rules)
        .filter(|rule| !rule.pattern.is_empty() && pattern_matches(&rule.pattern, &path))
        .max_by_key(|rule| (rule.pattern.len(), rule.allow))
        .is_none_or(|rule| rule.allow)
}

struct Group {
    agents: Vec<String>,
    rules: Vec<Rule>,
}

struct Rule {
    allow: bool,
    pattern: String,
}

fn parse(robots_txt: &str) -> Vec<Group> {
    let text = robots_txt.strip_prefix('\u{feff}').unwrap_or(robots_txt);
    let text = &text[..floor_char_boundary(text, MAX_ROBOTS_BYTES)];
    let mut groups: Vec<Group> = Vec::new();
    let mut collecting_agents = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match key.as_str() {
            "user-agent" => {
                if !collecting_agents {
                    groups.push(Group {
                        agents: Vec::new(),
                        rules: Vec::new(),
                    });
                }
                if let Some(group) = groups.last_mut() {
                    group.agents.push(value.to_owned());
                }
                collecting_agents = true;
            }
            "allow" | "disallow" => {
                if let Some(group) = groups.last_mut() {
                    group.rules.push(Rule {
                        allow: key == "allow",
                        pattern: normalise(value),
                    });
                }
                collecting_agents = false;
            }
            _ => {}
        }
    }
    groups
}

/// RFC 9309 compares paths and rules after percent-encoding both the same way, so non-ASCII
/// is encoded, escape hex is uppercased and escaped unreserved characters are decoded.
fn normalise(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let escaped = (byte == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(value) = escaped {
            if value.is_ascii_alphanumeric() || b"-._~".contains(&value) {
                out.push(char::from(value));
            } else {
                push_escape(&mut out, value);
            }
            i += 3;
        } else {
            if byte.is_ascii() {
                out.push(char::from(byte));
            } else {
                push_escape(&mut out, byte);
            }
            i += 1;
        }
    }
    out
}

fn push_escape(out: &mut String, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.push('%');
    out.push(char::from(HEX[usize::from(byte >> 4)]));
    out.push(char::from(HEX[usize::from(byte & 0x0f)]));
}

fn pattern_matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(stripped) => (stripped, true),
        None => (pattern, false),
    };
    let mut parts = pattern.split('*');
    let Some(mut rest) = path.strip_prefix(parts.next().unwrap_or("")) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return !anchored || rest.is_empty();
    };
    for part in middle {
        let Some(at) = rest.find(part) else {
            return false;
        };
        rest = &rest[at + part.len()..];
    }
    if anchored {
        rest.ends_with(last)
    } else {
        rest.contains(last)
    }
}

fn floor_char_boundary(text: &str, max: usize) -> usize {
    (0..=max.min(text.len()))
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_token_stops_at_version_space_or_comment() {
        assert_eq!(
            product_token("Claude-User (claude-code/2.1; +x)"),
            "Claude-User"
        );
        assert_eq!(
            product_token("Claude-Code/2.1 (+mailto:a@b.c)"),
            "Claude-Code"
        );
        assert_eq!(product_token("MyBot"), "MyBot");
    }

    #[test]
    fn matching_group_beats_star() {
        let robots = "User-agent: *\nDisallow: /\n\nUser-agent: Claude-User\nAllow: /\n";
        assert!(is_allowed(robots, "Claude-User", "/docs"));
        assert!(!is_allowed(robots, "OtherBot", "/docs"));
    }

    #[test]
    fn group_names_match_case_insensitively_and_combine() {
        let robots =
            "User-agent: claude-user\nDisallow: /a\n\nUser-agent: CLAUDE-USER\nDisallow: /b\n";
        assert!(!is_allowed(robots, "Claude-User", "/a/1"));
        assert!(!is_allowed(robots, "Claude-User", "/b/1"));
        assert!(is_allowed(robots, "Claude-User", "/c"));
    }

    #[test]
    fn consecutive_user_agent_lines_share_a_group() {
        let robots = "User-agent: A\nUser-agent: B\nDisallow: /x\n";
        assert!(!is_allowed(robots, "A", "/x"));
        assert!(!is_allowed(robots, "B", "/x"));
    }

    #[test]
    fn longest_match_wins_and_allow_wins_ties() {
        let robots =
            "User-agent: *\nDisallow: /docs\nAllow: /docs/public\nDisallow: /same\nAllow: /same\n";
        assert!(!is_allowed(robots, "Bot", "/docs/private"));
        assert!(is_allowed(robots, "Bot", "/docs/public/page"));
        assert!(is_allowed(robots, "Bot", "/same"));
    }

    #[test]
    fn wildcards_and_end_anchors() {
        let robots = "User-agent: *\nDisallow: /*.pdf$\nDisallow: /tmp*/cache\n";
        assert!(!is_allowed(robots, "Bot", "/files/report.pdf"));
        assert!(is_allowed(robots, "Bot", "/files/report.pdf?download=1"));
        assert!(!is_allowed(robots, "Bot", "/tmp-1/cache/x"));
        assert!(is_allowed(robots, "Bot", "/tmp-1/other"));
    }

    #[test]
    fn empty_disallow_and_missing_groups_allow_everything() {
        assert!(is_allowed("User-agent: *\nDisallow:\n", "Bot", "/anything"));
        assert!(is_allowed(
            "User-agent: OtherBot\nDisallow: /\n",
            "Bot",
            "/anything"
        ));
        assert!(is_allowed("", "Bot", "/anything"));
    }

    #[test]
    fn comments_bom_crlf_and_unknown_lines_are_tolerated() {
        let robots = "\u{feff}# site rules\r\nUser-agent: * # everyone\r\nCrawl-delay: 5\r\nSitemap: https://a.io/s.xml\r\nDisallow: /secret # keep out\r\n";
        assert!(!is_allowed(robots, "Bot", "/secret/1"));
        assert!(is_allowed(robots, "Bot", "/public"));
    }

    #[test]
    fn rules_and_paths_compare_in_one_encoding() {
        let robots = "User-agent: *\nDisallow: /café\nDisallow: /a%2fb\nDisallow: /%7Euser\n";
        assert!(!is_allowed(robots, "Bot", "/caf%C3%A9/menu"));
        assert!(!is_allowed(robots, "Bot", "/a%2Fb"));
        assert!(!is_allowed(robots, "Bot", "/~user/home"));
        assert!(is_allowed(robots, "Bot", "/a/b"));
    }

    #[test]
    fn robots_txt_itself_is_always_allowed() {
        assert!(is_allowed(
            "User-agent: *\nDisallow: /\n",
            "Bot",
            "/robots.txt"
        ));
    }

    #[test]
    fn robots_url_keeps_scheme_host_and_port() {
        let page = url::Url::parse("https://a.io:8443/w/page?q=1#frag").unwrap();
        assert_eq!(robots_url(&page), "https://a.io:8443/robots.txt");
        let v6 = url::Url::parse("http://[::1]:8080/x").unwrap();
        assert_eq!(robots_url(&v6), "http://[::1]:8080/robots.txt");
    }

    #[test]
    fn path_to_check_includes_the_query() {
        let page = url::Url::parse("https://a.io/search?q=x").unwrap();
        assert_eq!(path_and_query(&page), "/search?q=x");
    }
}
