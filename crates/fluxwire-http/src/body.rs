//! Response body sink: memory up to a limit, then a temp file; hard cap.

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use fluxwire_protocol::Body;
use tokio::io::AsyncWriteExt;

pub enum SinkError {
    Cap(u64),
    Io(std::io::Error),
}

impl From<std::io::Error> for SinkError {
    fn from(e: std::io::Error) -> Self {
        SinkError::Io(e)
    }
}

pub struct Sink {
    mem: Vec<u8>,
    file: Option<(tokio::fs::File, tempfile::TempPath)>,
    len: u64,
    mem_limit: u64,
    max: u64,
    temp_dir: Option<PathBuf>,
}

impl Sink {
    pub fn new(mem_limit: u64, max: u64, temp_dir: Option<PathBuf>) -> Self {
        Self {
            mem: Vec::new(),
            file: None,
            len: 0,
            mem_limit,
            max,
            temp_dir,
        }
    }

    pub async fn write(&mut self, chunk: &[u8]) -> Result<(), SinkError> {
        if self.len + chunk.len() as u64 > self.max {
            return Err(SinkError::Cap(self.max));
        }
        if self.file.is_none() && self.len + chunk.len() as u64 > self.mem_limit {
            let mut b = tempfile::Builder::new();
            b.prefix("fluxwire-body-");
            let named = match &self.temp_dir {
                Some(d) => b.tempfile_in(d)?,
                None => b.tempfile()?,
            };
            let (std_file, path) = named.into_parts();
            let mut f = tokio::fs::File::from_std(std_file);
            f.write_all(&self.mem).await?;
            self.mem = Vec::new();
            self.file = Some((f, path));
        }
        match &mut self.file {
            Some((f, _)) => f.write_all(chunk).await?,
            None => self.mem.extend_from_slice(chunk),
        }
        self.len += chunk.len() as u64;
        Ok(())
    }

    pub async fn finish(self) -> std::io::Result<Body> {
        match self.file {
            Some((mut f, path)) => {
                f.flush().await?;
                Ok(Body::File {
                    path: Arc::new(path),
                    len: self.len,
                })
            }
            None => Ok(Body::Memory(Bytes::from(self.mem))),
        }
    }
}
