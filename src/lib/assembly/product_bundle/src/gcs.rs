// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Google Cloud Storage (`gs://`) URI utilities for loading Product Bundles.
#![allow(dead_code)]

use camino::{Utf8Component, Utf8Path, Utf8PathBuf};
use std::io::{self, Read, Seek, SeekFrom};

pub(crate) type ByteRange = Option<(Option<u64>, u64)>;
const CHUNK_SIZE: u64 = 256 * 1024;

/// Error communicating with Google Cloud Storage (GCS) or parsing a `gs://` URI.
#[derive(Debug, thiserror::Error)]
#[error("GCS error for '{0}': {1}")]
pub struct GcsError(pub String, pub String);

/// Returns `true` if `path` is a `gs://` URI.
pub fn is_gcs_uri(path: impl AsRef<Utf8Path>) -> bool {
    path.as_ref().as_str().starts_with("gs://")
}

pub(crate) fn join_gcs_uri(base: impl AsRef<Utf8Path>, rel: impl AsRef<Utf8Path>) -> Utf8PathBuf {
    let r = rel.as_ref();
    if is_gcs_uri(r) {
        return r.to_path_buf();
    }
    let Ok(loc) = GcsLocation::parse(base.as_ref()) else {
        return base.as_ref().join(r.as_str().trim_start_matches("./"));
    };
    let mut v: Vec<_> = loc.object.split('/').filter(|s| !s.is_empty()).collect();
    for c in r.components() {
        match c {
            Utf8Component::Normal(s) => v.push(s),
            Utf8Component::ParentDir => _ = v.pop(),
            _ => {}
        }
    }
    Utf8PathBuf::from(format!("gs://{}/{}", loc.bucket, v.join("/")).trim_end_matches('/'))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GcsLocation {
    pub bucket: String,
    pub object: String,
}

impl GcsLocation {
    pub fn parse(uri: impl AsRef<Utf8Path>) -> Result<Self, GcsError> {
        let raw = uri.as_ref().as_str();
        let err = |m: &str| GcsError(raw.into(), m.into());
        let s = raw
            .strip_prefix("gs://")
            .ok_or_else(|| err("URI must start with gs://"))?
            .trim_end_matches('/');
        let (b, o) = s.split_once('/').unwrap_or((s, ""));
        if b.is_empty() || b.chars().any(char::is_whitespace) {
            return Err(err("invalid bucket name"));
        }
        let object = o.split('/').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("/");
        Ok(Self { bucket: b.into(), object })
    }

    pub fn is_zip(&self) -> bool {
        self.object.to_ascii_lowercase().ends_with(".zip")
    }

    pub fn directory_manifest(&self) -> (Utf8PathBuf, String) {
        let s = self.object.strip_suffix("product_bundle.json").unwrap_or(&self.object);
        let dir = join_gcs_uri(format!("gs://{}", self.bucket), s);
        let obj = Self::parse(join_gcs_uri(&dir, "product_bundle.json")).unwrap().object;
        (dir, obj)
    }
}

pub(crate) fn parse_content_range_total_size(h: &str) -> Option<u64> {
    h.trim().strip_prefix("bytes ")?.split_once('/')?.1.parse().ok()
}

pub(crate) trait GcsFetcher: Send + Sync {
    fn fetch(&self, b: &str, o: &str, r: ByteRange) -> Result<(u64, Vec<u8>), GcsError>;
}

pub(crate) struct GcsRangeReader<'a, F>(&'a F, GcsLocation, u64, u64, Vec<u8>, u64);

impl<'a, F: GcsFetcher> GcsRangeReader<'a, F> {
    pub fn new(f: &'a F, loc: GcsLocation) -> Result<Self, GcsError> {
        let (len, buf) = f.fetch(&loc.bucket, &loc.object, Some((None, CHUNK_SIZE)))?;
        let start = len
            .checked_sub(buf.len() as u64)
            .ok_or_else(|| GcsError(loc.object.clone(), "tail > total".into()))?;
        Ok(Self(f, loc, len, start, buf, 0))
    }
}

impl<F: GcsFetcher> Seek for GcsRangeReader<'_, F> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let next = match pos {
            SeekFrom::Start(o) => o as i64,
            SeekFrom::End(d) => self.2 as i64 + d,
            SeekFrom::Current(d) => self.5 as i64 + d,
        };
        self.5 = u64::try_from(next).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
        Ok(self.5)
    }
}

impl<F: GcsFetcher> Read for GcsRangeReader<'_, F> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.5 >= self.2 || out.is_empty() {
            return Ok(0);
        }
        if self.5 < self.3 || self.5 >= self.3 + self.4.len() as u64 {
            let end = (self.5 + (out.len() as u64).max(CHUNK_SIZE)).min(self.2) - 1;
            self.4 = self
                .0
                .fetch(&self.1.bucket, &self.1.object, Some((Some(self.5), end)))
                .map_err(io::Error::other)?
                .1;
            if self.4.is_empty() {
                return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
            }
            self.3 = self.5;
        }
        let idx = (self.5 - self.3) as usize;
        let n = out.len().min(self.4.len() - idx);
        out[..n].copy_from_slice(&self.4[idx..idx + n]);
        self.5 += n as u64;
        Ok(n)
    }
}

#[cfg(test)]
impl GcsFetcher for std::collections::HashMap<&str, Vec<u8>> {
    fn fetch(&self, _: &str, o: &str, r: ByteRange) -> Result<(u64, Vec<u8>), GcsError> {
        assert!(!(o.ends_with(".zip") && matches!(r, None | Some((Some(0), _)))));
        let d = &self[o];
        let slice = match r {
            None => &d[..],
            Some((None, m)) => &d[d.len().saturating_sub(m as usize)..],
            Some((Some(s), e)) => &d[s as usize..=(e as usize).min(d.len() - 1)],
        };
        Ok((d.len() as u64, slice.to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Cursor, Write};
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipArchive, ZipWriter};

    #[test]
    fn test_gcs_location_and_range_reader() {
        let loc = GcsLocation::parse("gs://b/p//pb/").unwrap();
        assert_eq!(loc.directory_manifest().1, "p/pb/product_bundle.json");
        let direct = GcsLocation::parse("gs://b/p/pb/product_bundle.json").unwrap();
        assert_eq!(direct.directory_manifest(), loc.directory_manifest());
        assert!(GcsLocation::parse("gs://b/pb.ZIP").unwrap().is_zip());
        assert!(GcsLocation::parse("http://x/y").is_err());
        assert_eq!(join_gcs_uri("gs://b/p/", "./a/../c"), Utf8Path::new("gs://b/p/c"));
        assert_eq!(parse_content_range_total_size("bytes 0-9/10"), Some(10));

        let mut cur = Cursor::new(Vec::new());
        let mut z = ZipWriter::new(&mut cur);
        let st = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (k, n) in [("a", 520_000), ("pb.json", 2), ("p", 300_000)] {
            z.start_file(k, st).unwrap();
            z.write_all(&vec![b'x'; n]).unwrap();
        }
        z.finish().unwrap();
        let f = HashMap::from([("pb.zip", cur.into_inner())]);
        let rdr = GcsRangeReader::new(&f, GcsLocation::parse("gs://b/pb.zip").unwrap()).unwrap();
        let mut s = String::new();
        ZipArchive::new(rdr).unwrap().by_name("pb.json").unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "xx");
    }
}
