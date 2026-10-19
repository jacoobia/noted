use serde_json::{Value, json};

use crate::get::cache_key;

const SAFE_LONG_FLAGS: [&str; 4] = ["silent", "show-error", "location", "fail"];
const SAFE_SHORT_FLAGS: &str = "sSLf";

pub fn respond(input: &str, allow_webfetch: bool) -> Option<String> {
    let event: Value = serde_json::from_str(input).ok()?;
    let tool_input = event.get("tool_input")?;
    let decision = match event.get("tool_name")?.as_str()? {
        "Bash" => {
            let command = rewrite_curl(tool_input.get("command")?.as_str()?)?;
            let mut updated = tool_input.clone();
            updated["command"] = Value::String(command);
            json!({ "hookEventName": "PreToolUse", "updatedInput": updated })
        }
        "WebFetch" if !allow_webfetch => {
            let url = tool_input
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("<url>");
            json!({
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": format!(
                    "Use `noted get {url}` instead of WebFetch: it caches the page and returns it \
                     as markdown. Add --section \"<heading>\" to read one part."
                ),
            })
        }
        _ => return None,
    };
    Some(json!({ "hookSpecificOutput": decision }).to_string())
}

pub fn rewrite_curl(command: &str) -> Option<String> {
    let (head, rest) = split_first_pipe(command)?;
    let words = words(head);
    let (program, args) = words.split_first()?;
    if program.value != "curl" {
        return None;
    }
    let mut url = None;
    for word in args {
        if word.value.starts_with('-') {
            if !is_safe_flag(&word.value) {
                return None;
            }
        } else if url.replace(word).is_some() {
            return None;
        }
    }
    let url = url.filter(|url| cache_key(&url.value).is_ok())?;
    Some(match rest {
        None => format!("noted get {}", url.raw),
        Some(rest) => format!(
            "noted get --raw --quiet {} | {}",
            url.raw,
            rest.trim_start()
        ),
    })
}

fn split_first_pipe(command: &str) -> Option<(&str, Option<&str>)> {
    let bytes = command.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    for (i, &byte) in bytes.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some(b'\'') {
            if byte == b'\'' {
                quote = None;
            }
            continue;
        }
        match (quote, byte) {
            (Some(_), b'"') => quote = None,
            (_, b'\\') => escaped = true,
            (None, b'\'' | b'"') => quote = Some(byte),
            (None, b'|') if bytes.get(i + 1) != Some(&b'|') => {
                return Some((&command[..i], Some(&command[i + 1..])));
            }
            (Some(_), b'$' | b'`')
            | (None, b'|' | b';' | b'&' | b'>' | b'<' | b'`' | b'$' | b'(' | b')' | b'\n') => {
                return None;
            }
            _ => {}
        }
    }
    quote.is_none().then_some((command, None))
}

struct Word<'a> {
    raw: &'a str,
    value: String,
}

fn words(text: &str) -> Vec<Word<'_>> {
    let mut words = Vec::new();
    let mut start = None;
    let mut value = String::new();
    let mut quote = None;
    let mut escaped = false;
    for (i, ch) in text.char_indices() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        match (quote, ch) {
            (Some('\''), '\'') | (Some('"'), '"') => quote = None,
            (Some('\''), _) => value.push(ch),
            (_, '\\') => {
                escaped = true;
                start.get_or_insert(i);
            }
            (Some(_), _) => value.push(ch),
            (None, '\'' | '"') => {
                quote = Some(ch);
                start.get_or_insert(i);
            }
            (None, ch) if ch.is_whitespace() => {
                if let Some(begin) = start.take() {
                    let value = std::mem::take(&mut value);
                    words.push(Word {
                        raw: &text[begin..i],
                        value,
                    });
                }
            }
            (None, _) => {
                start.get_or_insert(i);
                value.push(ch);
            }
        }
    }
    if let Some(begin) = start {
        words.push(Word {
            raw: &text[begin..],
            value,
        });
    }
    words
}

fn is_safe_flag(flag: &str) -> bool {
    match flag.strip_prefix("--") {
        Some(long) => SAFE_LONG_FLAGS.contains(&long),
        None => flag.len() > 1 && flag[1..].chars().all(|c| SAFE_SHORT_FLAGS.contains(c)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_simple_curl() {
        let cases = [
            ("curl https://a.io/x", "noted get https://a.io/x"),
            ("curl -fsSL https://a.io/x", "noted get https://a.io/x"),
            (
                "curl --silent --location https://a.io/x",
                "noted get https://a.io/x",
            ),
            (
                "curl -s https://a.io/x | jq .name",
                "noted get --raw --quiet https://a.io/x | jq .name",
            ),
            (
                "curl 'https://a.io/x?q=1&r=2'",
                "noted get 'https://a.io/x?q=1&r=2'",
            ),
        ];
        for (command, expected) in cases {
            assert_eq!(
                rewrite_curl(command).as_deref(),
                Some(expected),
                "{command}"
            );
        }
    }

    #[test]
    fn leaves_anything_else_alone() {
        let cases = [
            "curl -H \"Authorization: Bearer t\" https://a.io",
            "curl -X POST https://a.io",
            "curl -o out.html https://a.io",
            "curl -d x=1 https://a.io",
            "curl https://a.io && rm -rf x",
            "curl https://a.io; echo done",
            "curl https://a.io > f",
            "curl https://a.io || true",
            "curl https://a.io https://b.io",
            "curl ftp://a.io",
            "curl $URL",
            "curl \"$(cat url.txt)\"",
            "echo curl https://a.io",
            "curl",
            "curl 'https://a.io",
        ];
        for command in cases {
            assert_eq!(rewrite_curl(command), None, "{command}");
        }
    }

    fn bash(command: &str) -> String {
        json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": {"command": command, "description": "Fetch it", "timeout": 120_000}
        })
        .to_string()
    }

    #[test]
    fn bash_curl_gets_updated_input() {
        let out: Value =
            serde_json::from_str(&respond(&bash("curl -s https://a.io/x"), false).unwrap())
                .unwrap();
        let hook = &out["hookSpecificOutput"];
        assert_eq!(hook["hookEventName"], "PreToolUse");
        assert_eq!(hook["updatedInput"]["command"], "noted get https://a.io/x");
        assert_eq!(hook["updatedInput"]["description"], "Fetch it");
        assert_eq!(hook["updatedInput"]["timeout"], 120_000);
        assert!(hook.get("permissionDecision").is_none());
    }

    #[test]
    fn other_bash_commands_pass_through() {
        assert_eq!(respond(&bash("ls -la"), false), None);
    }

    #[test]
    fn webfetch_is_denied_with_a_pointer() {
        let input = json!({
            "tool_name": "WebFetch",
            "tool_input": {"url": "https://a.io/x", "prompt": "summarise"}
        })
        .to_string();
        let out: Value = serde_json::from_str(&respond(&input, false).unwrap()).unwrap();
        let hook = &out["hookSpecificOutput"];
        assert_eq!(hook["permissionDecision"], "deny");
        assert!(
            hook["permissionDecisionReason"]
                .as_str()
                .unwrap()
                .contains("noted get https://a.io/x")
        );
        assert_eq!(respond(&input, true), None);
    }

    #[test]
    fn unrelated_or_broken_input_is_ignored() {
        let read = json!({"tool_name": "Read", "tool_input": {"file_path": "x"}}).to_string();
        assert_eq!(respond(&read, false), None);
        assert_eq!(respond("{not json", false), None);
        assert_eq!(respond("{\"tool_name\": \"Bash\"}", false), None);
    }
}
