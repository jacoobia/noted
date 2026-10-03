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
