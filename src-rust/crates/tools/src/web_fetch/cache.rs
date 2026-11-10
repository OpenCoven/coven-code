//! Extraction reuse is conditional on a fresh bounded response's bytes and MIME.
use super::{url_hash, Extraction, MAX_OUTPUT_BYTES};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Extracted {
    pub text: String,
    pub method: Extraction,
    pub original_text_bytes: usize,
    pub text_truncated: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    schema_version: u8,
    body_sha256: String,
    content_type: String,
    extracted: Extracted,
}

pub(super) fn load(
    directory: &Path,
    url: &str,
    body_sha256: &str,
    content_type: &str,
) -> Option<Extracted> {
    let path = directory.join(format!("{}.json", url_hash(url)));
    // JSON escaping can expand one output byte to six bytes. Never load an
    // unbounded local file simply because its name matches a cache key.
    let limit = MAX_OUTPUT_BYTES * 6 + 4096;
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > limit {
        return None;
    }
    let entry: Entry = serde_json::from_slice(&bytes).ok()?;
    if entry.schema_version != 1
        || entry.body_sha256 != body_sha256
        || entry.content_type != content_type
        || entry.extracted.text.len() > MAX_OUTPUT_BYTES
        || entry.extracted.original_text_bytes < entry.extracted.text.len()
        || (!entry.extracted.text_truncated
            && entry.extracted.original_text_bytes != entry.extracted.text.len())
    {
        return None;
    }
    Some(entry.extracted)
}

pub(super) fn save(
    directory: &Path,
    url: &str,
    body_sha256: &str,
    content_type: &str,
    extracted: &Extracted,
) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    // Serialize borrowed data; do not duplicate the extracted page in memory.
    #[derive(Serialize)]
    struct BorrowedEntry<'a> {
        schema_version: u8,
        body_sha256: &'a str,
        content_type: &'a str,
        extracted: &'a Extracted,
    }
    let entry = BorrowedEntry {
        schema_version: 1,
        body_sha256,
        content_type,
        extracted,
    };
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut temporary, &entry)?;
    temporary.flush()?;
    temporary.persist(directory.join(format!("{}.json", url_hash(url))))?;
    Ok(())
}
