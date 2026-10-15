//! Private, bounded diagnostics for interactive terminals.
use parking_lot::Mutex;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing_subscriber::fmt::MakeWriter;

pub const MAX_BYTES: usize = 1024 * 1024;
const KEEP_COMPLETED: usize = 10;
const HEADER: &[u8] = b"Coven Code diagnostic log v1\n";
const ROLLED: &[u8] = b"Earlier diagnostics discarded at the 1 MiB limit.\n";
const OVERSIZED: &[u8] = b"Diagnostic event omitted: exceeds the file size limit.\n";

struct State {
    file: File,
    bytes: usize,
    failed: bool,
    error: Option<String>,
}

struct Inner {
    path: PathBuf,
    state: Mutex<State>,
}

#[derive(Clone)]
pub struct DiagnosticLog {
    inner: Arc<Inner>,
}

impl DiagnosticLog {
    pub fn open(directory: &Path) -> io::Result<Self> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory)?;
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.file_type().is_dir() {
            return Err(io::Error::other(
                "diagnostic directory must not be a symlink",
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        prune_completed(directory, KEEP_COMPLETED)?;
        let path = directory.join(format!("session-{}.log", uuid::Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.write(true).read(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        // Publish the ownership header only after locking: pruning cannot mistake
        // an opening file for a completed session. The lock lives with the writer.
        if let Err(error) = file
            .try_lock()
            .map_err(io::Error::from)
            .and_then(|()| file.write_all(HEADER))
        {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                state: Mutex::new(State {
                    file,
                    bytes: HEADER.len(),
                    failed: false,
                    error: None,
                }),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    pub fn take_error(&self) -> Option<String> {
        self.inner.state.lock().error.take()
    }
}

fn prune_completed(directory: &Path, keep: usize) -> io::Result<()> {
    let mut completed = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(id) = name
            .to_str()
            .and_then(|name| name.strip_prefix("session-"))
            .and_then(|name| name.strip_suffix(".log"))
        else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_err() || !entry.file_type()?.is_file() {
            continue;
        }
        let mut file = match OpenOptions::new().read(true).write(true).open(entry.path()) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => continue,
            Err(fs::TryLockError::Error(error)) => return Err(error),
        }
        let mut header = [0; HEADER.len()];
        if file.read_exact(&mut header).is_err() || header != HEADER {
            continue;
        }
        completed.push((file.metadata()?.modified()?, entry.path(), file));
    }
    completed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    for (_, path, _locked_file) in completed.into_iter().skip(keep) {
        // Keep the lock until unlink completes, including against concurrent pruning.
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub struct DiagnosticWriter<'a> {
    log: &'a DiagnosticLog,
}

impl<'a> MakeWriter<'a> for DiagnosticLog {
    type Writer = DiagnosticWriter<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        DiagnosticWriter { log: self }
    }
}

impl Write for DiagnosticWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self.log.inner.state.lock();
        if state.failed {
            return Err(io::Error::other(
                "diagnostic logging stopped after a write failure",
            ));
        }
        let payload = if buffer.len() > MAX_BYTES - HEADER.len() - ROLLED.len() {
            OVERSIZED
        } else {
            buffer
        };
        let result = (|| {
            if state.bytes + payload.len() > MAX_BYTES {
                state.file.set_len(0)?;
                state.file.seek(SeekFrom::Start(0))?;
                state.file.write_all(HEADER)?;
                state.file.write_all(ROLLED)?;
                state.bytes = HEADER.len() + ROLLED.len();
            }
            state.file.write_all(payload)?;
            state.bytes += payload.len();
            Ok(())
        })();
        if let Err(error) = result {
            state.failed = true;
            state.error = Some(format!("{error}"));
            return Err(error);
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        // File writes are unbuffered; no userspace buffer needs flushing.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prune_until(directory: &Path, keep: usize, expected_files: usize, live: &[&Path]) {
        // Concurrent tests spawn subprocesses. On Unix a fork may briefly inherit
        // a dropped writer's lock until exec, so pruning must conservatively skip it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            prune_completed(directory, keep).unwrap();
            assert!(live.iter().all(|path| path.exists()));
            if fs::read_dir(directory).unwrap().count() == expected_files {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "pruning never completed"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn private_distinct_files_preserve_live_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let a = DiagnosticLog::open(dir.path()).unwrap();
        let b = DiagnosticLog::open(dir.path()).unwrap();
        assert_ne!(a.path(), b.path());
        a.make_writer().write_all(b"first\n").unwrap();
        b.make_writer().write_all(b"second\n").unwrap();
        prune_completed(dir.path(), 0).unwrap();
        assert!(a.path().exists() && b.path().exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(a.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let a_path = a.path().to_owned();
        drop(a);
        prune_until(dir.path(), 0, 1, &[b.path()]);
        assert!(!a_path.exists());
        assert!(b.path().exists());
    }

    #[test]
    fn retention_keeps_only_completed_owned_files() {
        let dir = tempfile::tempdir().unwrap();
        for _ in 0..4 {
            drop(DiagnosticLog::open(dir.path()).unwrap());
        }
        let unrelated = dir
            .path()
            .join(format!("session-{}.log", uuid::Uuid::new_v4()));
        fs::write(&unrelated, b"user content").unwrap();
        prune_until(dir.path(), 2, 3, &[]);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
        assert_eq!(fs::read(unrelated).unwrap(), b"user content");
    }

    #[test]
    fn size_limit_retains_new_output_and_marks_discarded_history() {
        let dir = tempfile::tempdir().unwrap();
        let log = DiagnosticLog::open(dir.path()).unwrap();
        let chunk = vec![b'x'; MAX_BYTES / 2];
        log.make_writer().write_all(&chunk).unwrap();
        log.make_writer().write_all(&chunk).unwrap();
        log.make_writer().write_all(b"latest\n").unwrap();
        let bytes = fs::read(log.path()).unwrap();
        assert!(bytes.len() <= MAX_BYTES);
        assert!(bytes.starts_with(&[HEADER, ROLLED].concat()));
        assert!(bytes.ends_with(b"latest\n"));
        log.make_writer()
            .write_all(&vec![b'y'; MAX_BYTES + 1])
            .unwrap();
        let bytes = fs::read(log.path()).unwrap();
        assert!(bytes.len() <= MAX_BYTES && bytes.ends_with(OVERSIZED));
    }

    #[test]
    fn write_failure_is_latched_once_and_stops_further_writes() {
        let dir = tempfile::tempdir().unwrap();
        let log = DiagnosticLog::open(dir.path()).unwrap();
        // Substitute a read-only handle to provoke a real OS write error.
        log.inner.state.lock().file = File::open(log.path()).unwrap();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .log_internal_errors(false)
            .with_writer(log.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!("failure");
        });
        assert!(log.take_error().is_some());
        assert!(log.take_error().is_none());
        assert!(log.make_writer().write_all(b"again").is_err());
        assert!(log.take_error().is_none());
    }

    #[test]
    fn refuses_invalid_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, b"preserved").unwrap();
        assert!(DiagnosticLog::open(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), b"preserved");
    }
}
