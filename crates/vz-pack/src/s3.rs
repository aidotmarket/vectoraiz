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
        Self::verify_readback(response, expected)
    }
    fn verify_readback(response: Response, expected: &[u8]) -> Result<()> {
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
    let mut intent = None;
    let mut promotion_attempted = false;
    let result = (|| {
        // A repeat of a completed upload has no new promotion boundary or intent.
        // Refuse conflicting manifests before writing any objects. A matching
        // manifest is complete only after all reviewed payload bytes read back.
        guard.check_accepted(current)?;
        let manifest = files
            .iter()
            .find(|f| f.path == "manifest.json")
            .context("missing manifest")?;
        let bytes = std::fs::read(payload.join(&manifest.path))?;
        ensure!(
            bytes.len() as u64 == manifest.size
                && vz_core::pipeline::sha256(&bytes) == manifest.sha256,
            "reviewed payload changed before S3 write"
        );
        let existing = s3.request(Method::GET, "manifest.json", &[])?;
        // Without ListBucket, AWS also returns 403 for a missing key. Treat
        // that as unknown existence; conditional PUTs and readback still fence
        // every object, including the manifest completion boundary.
        if !matches!(
            existing.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::FORBIDDEN
        ) {
            ensure!(
                existing.status().is_success(),
                "S3 manifest check failed: HTTP {}",
                existing.status()
            );
            S3::verify_readback(existing, &bytes)?;
            for file in files.iter().filter(|f| f.path != "manifest.json") {
                guard.check_accepted(current)?;
                let bytes = std::fs::read(payload.join(&file.path))?;
                ensure!(
                    bytes.len() as u64 == file.size
                        && vz_core::pipeline::sha256(&bytes) == file.sha256,
                    "reviewed payload changed before S3 readback"
                );
                s3.readback(&file.path, &bytes)?;
            }
            guard.check_accepted(current)?;
            return guard.resolve_verified_transfers(
                current,
                &format!("{endpoint}/{bucket}/{prefix}"),
                &files,
            );
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
            // Resume only byte-identical immutable objects after a partial upload.
            // A manifest 412 also requires byte-identical readback, since a
            // forbidden preflight cannot establish whether it already exists.
            if file.path == "manifest.json" {
                intent = Some(guard.intent(current, &format!("{endpoint}/{bucket}/{prefix}"))?);
                promotion_attempted = true;
            }
            let response = s3.request(Method::PUT, &file.path, &bytes)?;
            if response.status() != reqwest::StatusCode::PRECONDITION_FAILED {
                ensure!(
                    response.status().is_success(),
                    "S3 conditional write failed: HTTP {}",
                    response.status()
                );
            }
            guard.check_accepted(current)?;
            s3.readback(&file.path, &bytes)?;
        }
        // No remote deletion on an unknown PUT/promotion result. The durable intent
        // retains the exact destination; reconciliation must inspect those bytes.
        guard.check_accepted(current)?;
        guard.completed(&intent.context("missing S3 intent")?, &files)?;
        guard.resolve_verified_transfers(current, &format!("{endpoint}/{bucket}/{prefix}"), &files)
    })();
    result.map_err(|error| {
        if promotion_attempted {
            crate::review_transfer::uncertain(&format!("{endpoint}/{bucket}/{prefix}"), error)
        } else {
            error
        }
    })
}
