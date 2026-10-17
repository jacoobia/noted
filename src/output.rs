use std::io::{ErrorKind, Write};

use anyhow::Result;

pub fn emit(bytes: &[u8]) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    match stdout.write_all(bytes).and_then(|()| stdout.flush()) {
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        result => Ok(result?),
    }
}

pub fn short_duration(secs: i64) -> String {
    const UNITS: [(i64, &str); 4] = [(86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")];
    UNITS.iter().find(|(size, _)| secs >= *size).map_or_else(
        || "0s".to_owned(),
        |(size, unit)| format!("{}{unit}", secs / size),
    )
}

pub fn short_size(bytes: i64) -> String {
    const UNITS: [(i64, &str); 3] = [(1 << 20, "MB"), (1 << 10, "KB"), (1, "B")];
    UNITS.iter().find(|(size, _)| bytes >= *size).map_or_else(
        || "0B".to_owned(),
        |(size, unit)| format!("{}{unit}", bytes / size),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_truncate_to_the_largest_unit() {
        let cases = [
            (0, "0B"),
            (1023, "1023B"),
            (1024, "1KB"),
            ((1 << 20) - 1, "1023KB"),
            (1 << 20, "1MB"),
        ];
        for (bytes, expected) in cases {
            assert_eq!(short_size(bytes), expected, "{bytes}");
        }
    }

    #[test]
    fn durations_truncate_to_the_largest_unit() {
        let cases = [
            (-5, "0s"),
            (0, "0s"),
            (59, "59s"),
            (90 * 60, "1h"),
            (7 * 86_400, "7d"),
        ];
        for (secs, expected) in cases {
            assert_eq!(short_duration(secs), expected, "{secs}");
        }
    }
}
