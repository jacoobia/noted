use anyhow::Result;

const MAX_BODY: u64 = 20 * 1024 * 1024;

pub struct Response {
    pub status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: Vec<u8>,
}

pub fn fetch(url: &str, etag: Option<&str>, last_modified: Option<&str>) -> Result<Response> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.get(url).header("User-Agent", user_agent());
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    if let Some(last_modified) = last_modified {
        request = request.header("If-Modified-Since", last_modified);
    }
    let mut response = request.call()?;
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let (content_type, etag, last_modified) = (
        header("Content-Type"),
        header("ETag"),
        header("Last-Modified"),
    );
    let body = response
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()?;
    Ok(Response {
        status: response.status().as_u16(),
        content_type,
        etag,
        last_modified,
        body,
    })
}

pub fn user_agent() -> String {
    let base = concat!("noted/", env!("CARGO_PKG_VERSION"));
    match std::env::var("NOTED_CONTACT") {
        Ok(contact) if !contact.is_empty() => format!("{base} ({contact})"),
        _ => base.to_owned(),
    }
}
