//! Resolve GitLab Markdown upload paths without exposing credentials in image URLs.
use anyhow::{Context, bail};
use percent_encoding::percent_decode_str;
use reqwest::{Client, Response};
use url::Url;

const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
fn decoded(value: &str) -> Option<String> {
    percent_decode_str(value)
        .decode_utf8()
        .ok()
        .map(|s| s.into_owned())
}
fn clean_url(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.username().is_empty()
        && url.password().is_none())
    .then_some(url)
}
fn upload_url(base: &Url, project: &str, tail: &str) -> Option<String> {
    let (secret, filename) = tail.split_once('/')?;
    if secret.len() != 32 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let filename = decoded(filename)?;
    if filename.is_empty() || filename.contains('/') || matches!(filename.as_str(), "." | "..") {
        return None;
    }
    let mut url = base.clone();
    url.set_query(None);
    url.set_fragment(None);
    url.path_segments_mut().ok()?.pop_if_empty().extend([
        "api",
        "v4",
        "projects",
        &decoded(project)?,
        "uploads",
        secret,
        &filename,
    ]);
    Some(url.into())
}

/// Resolve /uploads relative to the document's project (which may be a related issue
/// in another project). Absolute external images remain anonymous HTTP resources.
pub fn resolve_image_url(instance: &str, document: &str, source: &str) -> Option<String> {
    let base = clean_url(instance)?;
    let document = clean_url(document)?;
    let project = || {
        if document.origin() != base.origin() {
            return None;
        }
        let prefix = format!("{}/", base.path().trim_end_matches('/'));
        let path = document.path().strip_prefix(&prefix)?;
        let project = path.split_once("/-/")?.0;
        (!project.is_empty()).then_some(project)
    };
    if let Some(tail) = source
        .strip_prefix("/uploads/")
        .or_else(|| source.strip_prefix("uploads/"))
    {
        return upload_url(&base, project()?, tail);
    }
    // Resolve other relative HTTP resources against the document, never against disk.
    let resolved = document.join(source).ok()?;
    let resolved = clean_url(resolved.as_str())?;
    if resolved.origin() == base.origin() {
        let prefix = format!("{}/", base.path().trim_end_matches('/'));
        if let Some(path) = resolved.path().strip_prefix(&prefix) {
            if let Some(path) = path.strip_prefix("-/project/") {
                let (project, tail) = path.split_once("/uploads/")?;
                return upload_url(&base, project, tail);
            }
            if !path.starts_with("api/v4/")
                && let Some((project, tail)) = path.split_once("/uploads/")
            {
                return upload_url(&base, project, tail);
            }
        }
    }
    Some(resolved.into())
}

/// Authentication may only be attached to the configured instance's precise download route.
pub fn is_upload_url(instance: &str, source: &str) -> bool {
    let Some(base) = clean_url(instance) else {
        return false;
    };
    let Some(url) = clean_url(source) else {
        return false;
    };
    if base.origin() != url.origin() || url.query().is_some() || url.fragment().is_some() {
        return false;
    }
    let prefix = format!("{}/api/v4/projects/", base.path().trim_end_matches('/'));
    let Some(path) = url.path().strip_prefix(&prefix) else {
        return false;
    };
    let parts: Vec<_> = path.split('/').collect();
    parts.len() == 4
        && !parts[0].is_empty()
        && parts[1] == "uploads"
        && parts[2].len() == 32
        && parts[2].bytes().all(|b| b.is_ascii_hexdigit())
        && decoded(parts[3]).is_some_and(|name| {
            !name.is_empty() && !name.contains('/') && name != "." && name != ".."
        })
}

pub(crate) async fn image_bytes(mut response: Response) -> anyhow::Result<Vec<u8>> {
    if !response.status().is_success() {
        bail!("Image download failed: HTTP {}", response.status());
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !(content_type.starts_with("image/") || content_type.starts_with("application/octet-stream"))
    {
        bail!("Image download returned an unsupported content type");
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_IMAGE_BYTES as u64)
    {
        bail!("Image exceeds the 10 MiB limit");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| e.without_url())
        .context("Cannot read image response")?
    {
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            bail!("Image exceeds the 10 MiB limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Public images never receive GitLab or Cloudflare headers, including redirects.
pub async fn download_public_image(url: &str) -> anyhow::Result<Vec<u8>> {
    let url = clean_url(url).context("Unsupported image URL")?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    image_bytes(
        client
            .get(url)
            .send()
            .await
            .map_err(|e| e.without_url())
            .context("Cannot download image")?,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    const SECRET: &str = "5f4dcc3b5aa765d61d8327deb882cf99";
    #[test]
    fn uploads_use_the_documents_project_and_preserve_instance_subpaths() {
        let relative = format!("/uploads/{SECRET}/01-schedule-sheet.png");
        let url = resolve_image_url(
            "https://gitlab.example.com/gitlab",
            "https://gitlab.example.com/gitlab/team/app/-/merge_requests/12",
            &relative,
        )
        .unwrap();
        assert_eq!(
            url,
            format!(
                "https://gitlab.example.com/gitlab/api/v4/projects/team%2Fapp/uploads/{SECRET}/01-schedule-sheet.png"
            )
        );
        assert!(is_upload_url("https://gitlab.example.com/gitlab", &url));
        let issue = resolve_image_url(
            "https://gitlab.example.com",
            "https://gitlab.example.com/other/project/-/issues/42",
            &relative,
        )
        .unwrap();
        assert!(issue.contains("projects/other%2Fproject/uploads/"));
        let rendered =
            format!("https://gitlab.example.com/-/project/123/uploads/{SECRET}/file%20name.png");
        assert_eq!(
            resolve_image_url(
                "https://gitlab.example.com",
                "https://gitlab.example.com/team/app/-/merge_requests/12",
                &rendered
            )
            .unwrap(),
            format!(
                "https://gitlab.example.com/api/v4/projects/123/uploads/{SECRET}/file%20name.png"
            )
        );
    }
    #[test]
    fn authenticated_route_rejects_other_origins_and_non_upload_endpoints() {
        let base = "https://gitlab.example.com";
        for url in [
            format!("https://elsewhere.invalid/api/v4/projects/1/uploads/{SECRET}/a.png"),
            format!("http://gitlab.example.com/api/v4/projects/1/uploads/{SECRET}/a.png"),
            format!("{base}/api/v4/projects/1/uploads/{SECRET}/a%2Fb.png"),
            format!("{base}/api/v4/projects/1/uploads/{SECRET}/a.png?token=secret"),
            format!("{base}/api/v4/user"),
        ] {
            assert!(!is_upload_url(base, &url), "{url}");
        }
        assert!(
            resolve_image_url(
                base,
                "https://gitlab.example.com/team/app/-/merge_requests/12",
                "file:///etc/passwd"
            )
            .is_none()
        );
    }
}
