//! The service's log file: `%ProgramData%\Koppel\logs\koppel.log`, the lines `tracing` writes at
//! Info and above (or as `RUST_LOG` says), for the support request that has only the Event Log's
//! warnings to go on.
//!
//! [`SizeRotatingFile`] is the writer: when the file would pass its size cap it becomes
//! `koppel.1.log`, that one `koppel.2.log`, and so on, and the oldest goes, so the folder never
//! holds more than `files * max_bytes`. `tracing-appender`'s own rolling appender rotates by time
//! only, so it supplies just the non-blocking worker thread ([`start`]). A rotation that fails (a
//! reader holds the file open on Windows) keeps writing to the current file and tries again on a
//! later write, so a line is never dropped for it.
//!
//! This module has no Windows calls, so its tests run on Linux.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The file the service writes.
pub const FILE_NAME: &str = "koppel";
/// Each file is cut at about this size.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// The current file and this many minus one older ones.
pub const FILES: usize = 5;

/// A log file that rotates by size. See the module's description.
pub struct SizeRotatingFile {
    folder: PathBuf,
    stem: String,
    max_bytes: u64,
    files: usize,
    file: File,
    size: u64,
}

impl SizeRotatingFile {
    /// Opens (appending) `<folder>/<stem>.log`, which must exist as a folder. `files` is at least
    /// two (the current file and one older), and `max_bytes` at least one byte.
    pub fn open(folder: &Path, stem: &str, max_bytes: u64, files: usize) -> io::Result<Self> {
        let path = folder.join(format!("{stem}.log"));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            folder: folder.to_owned(),
            stem: stem.to_owned(),
            max_bytes: max_bytes.max(1),
            files: files.max(2),
            file,
            size,
        })
    }

    fn path(&self, index: usize) -> PathBuf {
        if index == 0 {
            self.folder.join(format!("{}.log", self.stem))
        } else {
            self.folder.join(format!("{}.{index}.log", self.stem))
        }
    }

    /// Shifts every file one place up and starts a new current file. The oldest is deleted. On a
    /// failure nothing is lost: the current file stays the one written to.
    fn rotate(&mut self) -> io::Result<()> {
        let oldest = self.path(self.files - 1);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for index in (0..self.files - 1).rev() {
            let from = self.path(index);
            if from.exists() {
                fs::rename(&from, self.path(index + 1))?;
            }
        }
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path(0))?;
        self.size = 0;
        Ok(())
    }
}

impl Write for SizeRotatingFile {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        // A file that already holds something is rotated before a write that would pass the cap; a
        // single line larger than the cap goes into a file of its own.
        if self.size > 0 && self.size + buffer.len() as u64 > self.max_bytes {
            // Not fatal: the next write tries again.
            let _ = self.rotate();
        }
        let written = self.file.write(buffer)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Keeps the log file's writer thread alive; dropping it (when the service ends) flushes what is
/// queued.
#[cfg(windows)]
pub struct LogGuard(#[allow(dead_code)] tracing_appender::non_blocking::WorkerGuard);

/// The log file layer, filtered as `filter` says, and its guard. An
/// error when the file cannot be opened.
#[cfg(windows)]
pub fn start<S>(
    folder: &Path,
    filter: tracing_subscriber::filter::Targets,
) -> Result<(impl tracing_subscriber::Layer<S> + use<S>, LogGuard), io::Error>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use tracing_subscriber::Layer;

    let file = SizeRotatingFile::open(folder, FILE_NAME, MAX_BYTES, FILES)?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(writer)
        .with_filter(filter);
    Ok((layer, LogGuard(guard)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "koppel-log-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn it_appends_to_what_is_there() {
        let dir = temp("append");
        fs::write(dir.join("koppel.log"), "old\n").unwrap();
        let mut log = SizeRotatingFile::open(&dir, "koppel", 1000, 3).unwrap();
        log.write_all(b"new\n").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("koppel.log")).unwrap(),
            "old\nnew\n"
        );
    }

    #[test]
    fn it_rotates_at_the_cap_and_keeps_the_newest_files() {
        let dir = temp("rotate");
        let mut log = SizeRotatingFile::open(&dir, "koppel", 10, 3).unwrap();
        for line in ["aaaa\n", "bbbb\n", "cccc\n", "dddd\n", "eeee\n", "ffff\n"] {
            log.write_all(line.as_bytes()).unwrap();
        }
        assert_eq!(
            names(&dir),
            ["koppel.1.log", "koppel.2.log", "koppel.log"],
            "only three files stay"
        );
        // 10 bytes hold two lines.
        assert_eq!(
            fs::read_to_string(dir.join("koppel.log")).unwrap(),
            "eeee\nffff\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("koppel.1.log")).unwrap(),
            "cccc\ndddd\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("koppel.2.log")).unwrap(),
            "aaaa\nbbbb\n"
        );
    }

    #[test]
    fn the_oldest_file_goes() {
        let dir = temp("oldest");
        let mut log = SizeRotatingFile::open(&dir, "koppel", 5, 2).unwrap();
        for line in ["1111\n", "2222\n", "3333\n"] {
            log.write_all(line.as_bytes()).unwrap();
        }
        assert_eq!(names(&dir), ["koppel.1.log", "koppel.log"]);
        assert_eq!(
            fs::read_to_string(dir.join("koppel.1.log")).unwrap(),
            "2222\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("koppel.log")).unwrap(),
            "3333\n"
        );
    }

    #[test]
    fn a_line_over_the_cap_is_kept_whole() {
        let dir = temp("long");
        let mut log = SizeRotatingFile::open(&dir, "koppel", 4, 2).unwrap();
        log.write_all(b"a long line\n").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("koppel.log")).unwrap(),
            "a long line\n"
        );
    }

    #[test]
    fn a_reopened_file_counts_what_it_holds() {
        let dir = temp("reopen");
        fs::write(dir.join("koppel.log"), "123456789\n").unwrap();
        let mut log = SizeRotatingFile::open(&dir, "koppel", 12, 2).unwrap();
        log.write_all(b"abcd\n").unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("koppel.1.log")).unwrap(),
            "123456789\n"
        );
        assert_eq!(
            fs::read_to_string(dir.join("koppel.log")).unwrap(),
            "abcd\n"
        );
    }
}
