use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use std::io::Read;

pub(super) struct VerifiedReader<R> {
    reader: R,
    hash: Sha256,
    size: u64,
    limit: u64,
}
impl<R: Read> VerifiedReader<R> {
    pub(super) fn new(reader: R, limit: u64) -> Self {
        Self {
            reader,
            hash: Sha256::new(),
            size: 0,
            limit,
        }
    }

    pub(super) fn finish(mut self, diffid: &str) -> Result<u64> {
        let start = self.size;
        let mut tail = [0u8; 64 * 1024];
        loop {
            let n = self.read(&mut tail)?;
            if n == 0 {
                break;
            }
            ensure!(
                tail[..n].iter().all(|b| *b == 0),
                "nonzero data after tar terminator"
            );
        }
        ensure!(
            self.size.saturating_sub(start) >= 512,
            "missing tar end block"
        );
        ensure!(
            format!("sha256:{}", hex::encode(self.hash.finalize())) == diffid,
            "DiffID mismatch"
        );
        Ok(self.size)
    }
}
impl<R: Read> Read for VerifiedReader<R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        let n = self.reader.read(bytes)?;
        self.size = self
            .size
            .checked_add(n as u64)
            .ok_or_else(|| std::io::Error::other("decompressed size overflow"))?;
        if self.size > self.limit {
            return Err(std::io::Error::other("decompression budget exceeded"));
        }
        self.hash.update(&bytes[..n]);
        Ok(n)
    }
}
