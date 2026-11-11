//! S3 has no directory rename: immutable objects plus one conditional manifest
//! PUT form its completion boundary. Every object is read back before commit.
use crate::review_store::{CurrentReview, NativePackReview};
use anyhow::{ensure, Context, Result};
use hmac::{Hmac, Mac};
use reqwest::{
    blocking::{Client, Response},
    Method,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read, path::Path};

#[derive(Clone)]
struct S3<'a> {
    client: Client,
    endpoint: &'a str,
    bucket: &'a str,
    region: &'a str,
    prefix: String,
    access: String,
    secret: String,
    session: Option<String>,
    cancel: tokio_util::sync::CancellationToken,
}

struct UploadBody {
    bytes: std::io::Cursor<Vec<u8>>,
    cancel: tokio_util::sync::CancellationToken,
}
impl Read for UploadBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "listing transfer cancelled",
            ));
        }
        let size = buffer.len().min(65536);
        self.bytes.read(&mut buffer[..size])
    }
}
impl S3<'_> {
    fn request(&self, method: Method, name: &str, body: &[u8]) -> Result<Response> {
        let mut url = url::Url::parse(self.endpoint)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "endpoint must be an HTTP(S) origin without credentials or query"
        );
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("endpoint cannot be a base"))?
            .pop_if_empty()
            .push(self.bucket)
            .extend(self.prefix.split('/').filter(|s| !s.is_empty()))
            .extend(name.split('/'));
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().context("endpoint host")?),
            None => url.host_str().context("endpoint host")?.into(),
        };
        let date = chrono::Utc::now();
        let day = date.format("%Y%m%d").to_string();
        let timestamp = date.format("%Y%m%dT%H%M%SZ").to_string();
        let hash = vz_core::pipeline::sha256(body);
        let mut headers = BTreeMap::from([
            ("host", host),
            ("x-amz-content-sha256", hash.clone()),
            ("x-amz-date", timestamp.clone()),
        ]);
        if method == Method::PUT {
            headers.insert("if-none-match", "*".into());
        }
        if let Some(token) = &self.session {
            headers.insert("x-amz-security-token", token.clone());
        }
        let signed = headers.keys().copied().collect::<Vec<_>>().join(";");
        let canonical_headers = headers
            .iter()
            .map(|(k, v)| format!("{k}:{}\n", v.trim()))
            .collect::<String>();
        let canonical = format!(
            "{method}\n{}\n\n{canonical_headers}\n{signed}\n{hash}",
            url.path()
        );
        let scope = format!("{day}/{}/s3/aws4_request", self.region);
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{:x}",
            Sha256::digest(canonical.as_bytes())
        );
        let sign = |key: &[u8], data: &[u8]| {
            let mut h = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts all key lengths");
            h.update(data);
            h.finalize().into_bytes().to_vec()
        };
        let key = sign(format!("AWS4{}", self.secret).as_bytes(), day.as_bytes());
        let key = sign(&key, self.region.as_bytes());
        let key = sign(&key, b"s3");
        let key = sign(&key, b"aws4_request");
        let signature = sign(&key, to_sign.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let is_put = method == Method::PUT;
        let mut request = self.client.request(method, url).header("authorization", format!("AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={signature}", self.access));
        if is_put {
            request = request.body(reqwest::blocking::Body::sized(
                UploadBody {
                    bytes: std::io::Cursor::new(body.to_vec()),
                    cancel: self.cancel.clone(),
                },
                body.len() as u64,
            ));
        }
        for (key, value) in headers {
            request = request.header(key, value);
        }
        Ok(request.send()?)
    }
    fn readback(&self, name: &str, expected: &[u8]) -> Result<()> {
        let response = self.request(Method::GET, name, &[])?;
        ensure!(
            response.status().is_success(),
            "S3 readback failed: HTTP {}",
            response.status()
        );
        let mut bytes = Vec::new();
        response
            .take(expected.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(bytes == expected, "S3 readback mismatch");
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn upload(
    guard: &NativePackReview<'_>,
    current: &CurrentReview,
    payload: &Path,
    endpoint: &str,
    bucket: &str,
    region: &str,
    prefix: &str,
) -> Result<()> {
    ensure!(
        prefix == format!("vz/{}", current.pack_id),
        "S3 prefix must be the reviewed pack UUID under vz/"
    );
    let s3 = S3 {
        client: Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(120))
            .build()?,
        endpoint,
        bucket,
        region,
        prefix: prefix.into(),
        access: std::env::var("AWS_ACCESS_KEY_ID").context("AWS_ACCESS_KEY_ID required")?,
        secret: std::env::var("AWS_SECRET_ACCESS_KEY").context("AWS_SECRET_ACCESS_KEY required")?,
        session: std::env::var("AWS_SESSION_TOKEN").ok(),
        cancel: guard.cancellation_token(),
    };
    let mut files = crate::export::inventory(payload, &current.pack_id.to_string())?;
    files.sort_by_key(|f| f.path == "manifest.json");
    let intent = guard.intent(current, &format!("{endpoint}/{bucket}/{prefix}"))?;
    let stage = S3 {
        prefix: format!("_vz-staging/{}", intent.job_id),
        ..s3.clone()
    };
    let mut staged = Vec::new();
    let mut promotion_attempted = false;
    let result = (|| {
        // A job-owned namespace never receives a manifest/completion marker.
        for file in files.iter().filter(|file| file.path != "manifest.json") {
            guard.check_accepted(current)?;
            let bytes = std::fs::read(payload.join(&file.path))?;
            ensure!(
                bytes.len() as u64 == file.size && vz_core::pipeline::sha256(&bytes) == file.sha256,
                "reviewed payload changed before S3 staging"
            );
            let response = stage.request(Method::PUT, &file.path, &bytes)?;
            ensure!(
                response.status().is_success(),
                "S3 staging write failed: HTTP {}",
                response.status()
            );
            staged.push(file.path.clone());
            guard.check_accepted(current)?;
            stage.readback(&file.path, &bytes)?;
        }
        for file in &files {
            guard.check_accepted(current)?;
            if file.path == "manifest.json" {
                for payload_file in files.iter().filter(|f| f.path != "manifest.json") {
                    guard.check_accepted(current)?;
                    s3.readback(
                        &payload_file.path,
                        &std::fs::read(payload.join(&payload_file.path))?,
                    )?;
                }
                guard.check_accepted(current)?;
            }
            let bytes = std::fs::read(payload.join(&file.path))?;
            ensure!(
                bytes.len() as u64 == file.size && vz_core::pipeline::sha256(&bytes) == file.sha256,
                "reviewed payload changed before S3 write"
            );
            // Existing objects are never overwritten, even on uncertain retries.
            if file.path == "manifest.json" {
                promotion_attempted = true;
            }
            let response = s3.request(Method::PUT, &file.path, &bytes)?;
            ensure!(
                response.status().is_success(),
                "S3 conditional write failed: HTTP {}",
                response.status()
            );
            guard.check_accepted(current)?;
            s3.readback(&file.path, &bytes)?;
        }
        // No remote deletion on an unknown PUT/promotion result. The durable intent
        // retains the exact destination; reconciliation must inspect those bytes.
        guard.check_accepted(current)?;
        guard.completed(&intent, &files)
    })();
    // Remove only acknowledged writes in our random, incomplete namespace.
    // A stage containing a completion marker, or an unreadable stage, is left
    // for reconciliation. Never DELETE final pack objects, including on cancel.
    if matches!(stage.request(Method::HEAD, "manifest.json", &[]), Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND)
    {
        for name in staged {
            let _ = stage.request(Method::DELETE, &name, &[]);
        }
    }
    result.map_err(|error| {
        if promotion_attempted {
            crate::review_transfer::uncertain(&format!("{endpoint}/{bucket}/{prefix}"), error)
        } else {
            error
        }
    })
}
