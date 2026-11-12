//! One JSON config file that is the source of truth for a value, as `JsonConfigFile` is in
//! Koppel 0.1.x. A change is written to disk first (temp file, flush, replace) and only a write
//! that landed reaches the subscribers. Edits made by hand are picked up while it runs.
//!
//! The rules, from docs/config.md:
//!
//! - Start-up never replaces a file. Only a missing file is seeded with the defaults. A file that
//!   exists but cannot be used stops [`ConfigFile::start`] with a [`ConfigFileError`] and is left
//!   as it was, unless the file was opened to start with defaults (the plain, non-critical
//!   `devices.json` and `attached.json`): then it is left untouched, [`ConfigFile::problem`] says
//!   why, and the first change moves it aside as `name.unreadable-yyyyMMddHHmmssfff` (UTC).
//! - A hand edit is checked like any other value. A bad one is ignored and the last good value
//!   stays; a good one is loaded before the next change, so a change never undoes it.
//! - Changes reach subscribers in write order, one at a time. A subscriber may change the same file
//!   from its handler: that change is written at once and its delivery queued behind the running
//!   one, and the call returns without waiting for it, since it would wait for itself. A handler
//!   that changes the file from another task and waits for it is not seen as nested; the call then
//!   gives up waiting for the delivery after [`Options::delivery_timeout`] (the change is already
//!   on disk) instead of waiting forever.
//! - A write is durable when `update` returns: the temp file is flushed, moved over the file, and
//!   the directory is flushed too, so a power cut cannot bring the old file back.
//! - The file holds a `version` (1). A file with a newer one is refused, as a secret file with a
//!   newer format is. Names Koppel does not know in a hand-edited file are logged, and are gone
//!   after the next write.
//!
//! Hand edits are found by looking at the file's size and time every quarter second rather than
//! through the operating system's change events: those do not arrive through every Docker bind
//! mount, and the poll costs one `stat` per file. A changed file is told from our own write by a
//! SHA-256 of its bytes, and a file written in the last few seconds is hashed on every round, so
//! an edit that leaves the size and a coarse time as they were is still seen.

use std::fmt;
use std::fs;
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use ring::digest;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;

const READ_ATTEMPTS: u32 = 5;
const READ_RETRY_DELAY: Duration = Duration::from_millis(200);
const REPLACE_ATTEMPTS: u32 = 20;
const REPLACE_RETRY_DELAY: Duration = Duration::from_millis(100);
const TEMP_SUFFIX: &str = ".tmp";
const WATCH_INTERVAL: Duration = Duration::from_millis(250);
/// A file this young is hashed every round, since a coarse clock may give an edit its time.
const RECENT: Duration = Duration::from_secs(3);
/// The most a config file may hold. Real ones are a few kilobytes.
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// The `version` this code writes and the newest it reads.
const FORMAT_VERSION: u64 = 1;
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Turns a file's JSON into the bytes on disk and back, so a file can be encrypted.
pub trait FileCodec: Send + Sync {
    fn encode(&self, json: &[u8]) -> Result<Vec<u8>, CodecError>;
    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, CodecError>;
}

/// Why a codec could not encode or decode, in a few words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecError(pub String);

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CodecError {}

/// Plain UTF-8, for files people may read and edit.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlainText;

impl FileCodec for PlainText {
    fn encode(&self, json: &[u8]) -> Result<Vec<u8>, CodecError> {
        Ok(json.to_vec())
    }

    fn decode(&self, bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
        Ok(bytes.to_vec())
    }
}

/// What kind of trouble a config file had, so a caller can tell a full disk from a missing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigErrorKind {
    /// The file could not be read or written: permissions, a full disk, a lock that did not clear,
    /// a file over the size limit.
    Io,
    /// The encrypted file did not open: another key, or damaged.
    Decrypt,
    /// The text is not JSON for this file.
    Parse,
    /// It parsed, but the value is not acceptable.
    Invalid,
    /// The file says it is a newer version than this Koppel reads.
    Newer,
}

/// A config file exists but cannot be used, or a change could not be written. The wording of the
/// advice follows the kind; the `reason` is one line about this file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileError {
    /// The file.
    pub path: PathBuf,
    /// What went wrong.
    pub kind: ConfigErrorKind,
    /// What is wrong with it, in one line.
    pub reason: String,
}

impl ConfigFileError {
    pub(crate) fn new(path: &Path, kind: ConfigErrorKind, reason: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            kind,
            reason: reason.into(),
        }
    }

    pub(crate) fn io(path: &Path, reason: impl Into<String>) -> Self {
        Self::new(path, ConfigErrorKind::Io, reason)
    }
}

impl fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let advice = match self.kind {
            ConfigErrorKind::Io => {
                "Check the disk and the permissions of its directory. A change that was being                  saved may not have been kept."
            }
            ConfigErrorKind::Decrypt => {
                "The file was not changed. The key ring that encrypted it is missing or has                  changed: restore the keys directory. Then start Koppel again."
            }
            ConfigErrorKind::Parse | ConfigErrorKind::Invalid => {
                "The file was not changed. If it was edited by hand, fix the edit or restore                  the file. Then start Koppel again."
            }
            ConfigErrorKind::Newer => {
                "The file was not changed. Install the Koppel that wrote it, or restore an older                  copy of the file."
            }
        };
        write!(
            f,
            "Koppel cannot use {}: {}. {advice}",
            self.path.display(),
            self.reason
        )
    }
}

impl std::error::Error for ConfigFileError {}

/// Says why a value is not acceptable, or `None` when it is.
pub type Validator<T> = Arc<ValidateFn<T>>;

/// The check a [`Validator`] runs.
pub type ValidateFn<T> = dyn Fn(&T) -> Option<String> + Send + Sync;

type HandlerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
type Handler<T> = Arc<dyn Fn(Arc<T>) -> HandlerFuture + Send + Sync>;

/// How a file is opened.
pub struct Options<T> {
    pub codec: Arc<dyn FileCodec>,
    pub validate: Option<Validator<T>>,
    /// Start with the defaults, and the file untouched, when it exists but cannot be used. Only
    /// for plain files whose defaults are safe.
    pub start_with_defaults_when_unusable: bool,
    /// Log the names in the file that Koppel does not know. For the files people edit by hand.
    pub warn_unknown_keys: bool,
    /// How long `update` waits for the subscribers to finish with its change before it returns
    /// anyway. The change is on disk by then.
    pub delivery_timeout: Duration,
}

impl<T> Default for Options<T> {
    fn default() -> Self {
        Self {
            codec: Arc::new(PlainText),
            validate: None,
            start_with_defaults_when_unusable: false,
            warn_unknown_keys: false,
            delivery_timeout: DELIVERY_TIMEOUT,
        }
    }
}

/// The size and time of the file, to see that it changed.
type Stamp = (u64, SystemTime);

/// SHA-256 of a file's bytes.
type Hash = [u8; 32];

/// What was last on disk: the size and time, and the bytes' hash. The hash is what says a file is
/// ours, the stamp only says when it is worth looking.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Seen {
    stamp: Stamp,
    hash: Hash,
}

fn hash_of(bytes: &[u8]) -> Hash {
    digest::digest(&digest::SHA256, bytes)
        .as_ref()
        .try_into()
        .expect("SHA-256 is 32 bytes")
}

struct State<T> {
    current: Arc<T>,
    last: Option<Seen>,
    problem: Option<String>,
}

struct Delivery<T> {
    value: Arc<T>,
    context: Vec<usize>,
    done: Option<oneshot::Sender<()>>,
}

tokio::task_local! {
    /// The files whose deliveries the running handler is part of. It travels with a change
    /// into the next file's delivery, so a handler that changes a file further up the chain is
    /// seen as nested there too, as .NET's AsyncLocal sees it.
    static DELIVERING: Vec<usize>;
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

struct Inner<T> {
    id: usize,
    path: PathBuf,
    defaults: T,
    codec: Arc<dyn FileCodec>,
    validate: Option<Validator<T>>,
    lenient: bool,
    warn_unknown_keys: bool,
    delivery_timeout: Duration,
    gate: tokio::sync::Mutex<()>,
    state: Mutex<State<T>>,
    handlers: Mutex<Vec<(u64, Handler<T>)>>,
    next_handler: AtomicUsize,
    deliveries: mpsc::UnboundedSender<Delivery<T>>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<Delivery<T>>>>,
    watcher: Mutex<Option<JoinHandle<()>>>,
}

impl<T> Drop for Inner<T> {
    fn drop(&mut self) {
        if let Some(watcher) = lock(&self.watcher).take() {
            watcher.abort();
        }
    }
}

fn lock<V>(mutex: &Mutex<V>) -> std::sync::MutexGuard<'_, V> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One JSON config file. Cheap to clone: clones share the file.
pub struct ConfigFile<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for ConfigFile<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Lets a subscriber stop listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Subscription(u64);

impl<T> ConfigFile<T>
where
    T: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + 'static,
{
    /// A file at `path` holding `defaults` until it is started.
    pub fn new(path: impl Into<PathBuf>, defaults: T, options: Options<T>) -> Self {
        let (deliveries, receiver) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                path: path.into(),
                defaults: defaults.clone(),
                codec: options.codec,
                validate: options.validate,
                lenient: options.start_with_defaults_when_unusable,
                warn_unknown_keys: options.warn_unknown_keys,
                delivery_timeout: options.delivery_timeout,
                gate: tokio::sync::Mutex::new(()),
                state: Mutex::new(State {
                    current: Arc::new(defaults),
                    last: None,
                    problem: None,
                }),
                handlers: Mutex::new(Vec::new()),
                next_handler: AtomicUsize::new(1),
                deliveries,
                receiver: Mutex::new(Some(receiver)),
                watcher: Mutex::new(None),
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The value as last written or loaded.
    pub fn current(&self) -> Arc<T> {
        lock(&self.inner.state).current.clone()
    }

    /// Why the file on disk is not being applied, or `None`. [`Self::current`] is then still the
    /// last good value, or the defaults when the file was bad at start.
    pub fn problem(&self) -> Option<String> {
        lock(&self.inner.state).problem.clone()
    }

    /// Calls `handler` with every change once it is on disk, in write order, one at a time.
    pub fn subscribe<F, Fut>(&self, handler: F) -> Subscription
    where
        F: Fn(Arc<T>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let id = self.inner.next_handler.fetch_add(1, Ordering::Relaxed) as u64;
        let handler: Handler<T> = Arc::new(move |value| Box::pin(handler(value)));
        lock(&self.inner.handlers).push((id, handler));
        Subscription(id)
    }

    pub fn unsubscribe(&self, subscription: Subscription) {
        lock(&self.inner.handlers).retain(|(id, _)| *id != subscription.0);
    }

    /// Loads the file, writing the defaults if it does not exist yet, then starts watching it.
    /// Stale temp files from a crash are swept first.
    pub async fn start(&self) -> Result<(), ConfigFileError> {
        let inner = &self.inner;
        let directory = inner
            .path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        fs::create_dir_all(&directory).map_err(|e| {
            ConfigFileError::io(
                &inner.path,
                format!("its directory could not be made ({e})"),
            )
        })?;
        delete_stale_temp_files(&inner.path);
        if inner.path.exists() {
            match inner.load().await {
                Ok(Loaded::New(value, seen)) => inner.set_current(value, seen),
                Ok(Loaded::Same(_)) => {}
                Err(e) if inner.lenient => {
                    lock(&inner.state).problem = Some(e.reason.clone());
                    tracing::warn!(
                        "{} cannot be used ({}). Starting with defaults and leaving the file as it is until something changes it",
                        inner.path.display(),
                        e.reason
                    );
                }
                Err(e) => return Err(e),
            }
        } else {
            let seen = inner.write(&inner.defaults).await?;
            lock(&inner.state).last = Some(seen);
            tracing::info!("Wrote defaults to {}", inner.path.display());
        }

        let weak = Arc::downgrade(inner);
        let watcher = tokio::spawn(watch(weak));
        if let Some(old) = lock(&inner.watcher).replace(watcher) {
            old.abort();
        }
        Ok(())
    }

    /// Stops watching for hand edits. Safe to call more than once.
    pub fn stop(&self) {
        if let Some(watcher) = lock(&self.inner.watcher).take() {
            watcher.abort();
        }
    }

    /// Writes the changed value, then delivers it to the subscribers. A change that leaves the
    /// value as it was skips the write but is still delivered once, because whoever asked waits
    /// for it. A hand edit made since the last read or write is loaded first.
    ///
    /// Returns once the subscribers have seen the change, except when called from inside one of
    /// this file's deliveries, and except after [`Options::delivery_timeout`]: the change is on
    /// disk by then, and a handler that is stuck cannot hold the caller for ever.
    pub async fn update(&self, change: impl FnOnce(&T) -> T) -> Result<Arc<T>, ConfigFileError> {
        let inner = &self.inner;
        let context = DELIVERING.try_with(Clone::clone).unwrap_or_default();
        let nested = context.contains(&inner.id);
        let (next, done) = {
            let _gate = inner.gate.lock().await;
            inner.pick_up_hand_edit().await?;
            let current = self.current();
            let next = change(&current);
            let unchanged = *current == next;
            if !unchanged || !inner.path.exists() {
                let seen = inner.write(&next).await?;
                lock(&inner.state).last = Some(seen);
            }
            let next = Arc::new(next);
            {
                let mut state = lock(&inner.state);
                state.current = next.clone();
                state.problem = None;
            }
            let done = inner.enqueue(next.clone(), context);
            (next, done)
        };
        if !nested
            && tokio::time::timeout(inner.delivery_timeout, done)
                .await
                .is_err()
        {
            tracing::warn!(
                "The subscribers of {} were still busy {} seconds after a change, which is on disk; not waiting for them",
                inner.path.display(),
                inner.delivery_timeout.as_secs()
            );
        }
        Ok(next)
    }
}

/// What reading the file found.
enum Loaded<T> {
    /// The bytes are the ones we already know, so nothing was decoded.
    Same(Seen),
    New(T, Seen),
}

impl<T> Inner<T>
where
    T: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + 'static,
{
    fn set_current(&self, value: T, seen: Seen) {
        let mut state = lock(&self.state);
        state.current = Arc::new(value);
        state.problem = None;
        state.last = Some(seen);
    }

    /// Queues a delivery. Called under the gate, so deliveries keep the order of the writes.
    fn enqueue(self: &Arc<Self>, value: Arc<T>, context: Vec<usize>) -> oneshot::Receiver<()> {
        if let Some(receiver) = lock(&self.receiver).take() {
            tokio::spawn(deliver(Arc::downgrade(self), receiver));
        }
        let (done, waiter) = oneshot::channel();
        let _ = self.deliveries.send(Delivery {
            value,
            context,
            done: Some(done),
        });
        waiter
    }

    /// Reads, decodes, parses and checks the file. Retries a file that is briefly locked or being
    /// written. Bytes that hash as the last ones seen are not decoded again.
    async fn load(&self) -> Result<Loaded<T>, ConfigFileError> {
        let path = self.path.clone();
        let codec = self.codec.clone();
        let validate = self.validate.clone();
        let warn_unknown_keys = self.warn_unknown_keys;
        let known = lock(&self.state).last.map(|seen| seen.hash);
        tokio::task::spawn_blocking(move || {
            load_blocking(
                &path,
                codec.as_ref(),
                validate.as_deref(),
                warn_unknown_keys,
                known,
            )
        })
        .await
        .map_err(|e| ConfigFileError::io(&self.path, format!("it could not be read ({e})")))?
    }

    async fn write(&self, value: &T) -> Result<Seen, ConfigFileError> {
        let json = Zeroizing::new(versioned_json(value));
        let path = self.path.clone();
        let codec = self.codec.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = codec.encode(&json).map_err(|e| {
                ConfigFileError::io(&path, format!("it could not be encrypted ({e})"))
            })?;
            let hash = hash_of(&bytes);
            let stamp = write_atomic(&path, &bytes).map_err(|e| {
                ConfigFileError::io(&path, format!("it could not be written ({e})"))
            })?;
            Ok(Seen { stamp, hash })
        })
        .await
        .map_err(|e| ConfigFileError::io(&self.path, format!("it could not be written ({e})")))?
    }

    /// Under the gate: loads the file when it changed behind our back and is usable.
    async fn pick_up_hand_edit(&self) -> Result<(), ConfigFileError> {
        let Some(stamp) = stamp(&self.path) else {
            return Ok(());
        };
        let last = lock(&self.state).last;
        if Some(stamp) == last.map(|seen| seen.stamp) && !recent(stamp.1) {
            return Ok(());
        }
        match self.load().await {
            Ok(Loaded::New(value, seen)) => self.set_current(value, seen),
            Ok(Loaded::Same(seen)) => lock(&self.state).last = Some(seen),
            Err(e) if self.lenient => self.move_aside(&e.reason)?,
            Err(e) => tracing::warn!(
                "{} was edited by hand but is not usable ({}); this update replaces it",
                self.path.display(),
                e.reason
            ),
        }
        Ok(())
    }

    /// Keeps what the user had instead of overwriting it. The caller then writes a fresh file.
    fn move_aside(&self, reason: &str) -> Result<(), ConfigFileError> {
        let mut aside = self.path.clone().into_os_string();
        aside.push(format!(
            ".unreadable-{}",
            utc_stamp(OffsetDateTime::now_utc())
        ));
        let aside = PathBuf::from(aside);
        fs::rename(&self.path, &aside).map_err(|e| {
            ConfigFileError::io(
                &self.path,
                format!("it is not usable ({reason}) and could not be moved aside ({e})"),
            )
        })?;
        tracing::warn!(
            "{} cannot be used ({reason}); it was moved to {} and this change writes a fresh file",
            self.path.display(),
            aside.display()
        );
        Ok(())
    }

    /// Loads a hand edit the watcher saw. False when the file is not usable.
    async fn reload(self: &Arc<Self>) -> bool {
        let done = {
            let _gate = self.gate.lock().await;
            match self.load().await {
                Ok(Loaded::New(value, seen)) => {
                    let changed = *lock(&self.state).current != value;
                    if changed {
                        self.set_current(value, seen);
                        let current = lock(&self.state).current.clone();
                        Some(self.enqueue(current, Vec::new()))
                    } else {
                        // An edit that leaves the value as it was, such as another layout.
                        let mut state = lock(&self.state);
                        state.problem = None;
                        state.last = Some(seen);
                        None
                    }
                }
                Ok(Loaded::Same(seen)) => {
                    // Our own write, or the bad file put back as it was.
                    let mut state = lock(&self.state);
                    state.problem = None;
                    state.last = Some(seen);
                    None
                }
                Err(e) => {
                    lock(&self.state).problem = Some(e.reason.clone());
                    tracing::warn!(
                        "{} was not applied: {}. Keeping the last good value",
                        self.path.display(),
                        e.reason
                    );
                    return false;
                }
            }
        };
        if let Some(done) = done {
            tracing::info!("Reloaded {} after an edit", self.path.display());
            let _ = done.await;
        }
        true
    }

    /// Whether the bytes on disk are not the ones last seen. A file that cannot be read says no.
    async fn bytes_differ(&self) -> bool {
        let path = self.path.clone();
        let Some(known) = lock(&self.state).last.map(|seen| seen.hash) else {
            return true;
        };
        tokio::task::spawn_blocking(move || match read_once(&path) {
            Ok(Read::Stable(bytes, _)) => hash_of(&bytes) != known,
            Ok(Read::TooLarge | Read::Changed) => true,
            Err(_) => false,
        })
        .await
        .unwrap_or(false)
    }
}

/// Delivers changes one at a time, each handler in turn. A handler that panics is logged and the
/// ones after it still run.
async fn deliver<T: Send + Sync + 'static>(
    file: Weak<Inner<T>>,
    mut receiver: mpsc::UnboundedReceiver<Delivery<T>>,
) {
    while let Some(delivery) = receiver.recv().await {
        let Some(inner) = file.upgrade() else {
            return;
        };
        let handlers: Vec<Handler<T>> = lock(&inner.handlers)
            .iter()
            .map(|(_, h)| h.clone())
            .collect();
        let path = inner.path.clone();
        let mut context = delivery.context;
        context.push(inner.id);
        drop(inner);
        for handler in handlers {
            let run = DELIVERING.scope(context.clone(), handler(delivery.value.clone()));
            if let Err(e) = tokio::spawn(run).await {
                tracing::error!(
                    "A handler failed while handling a change to {}: {e}",
                    path.display()
                );
            }
        }
        if let Some(done) = delivery.done {
            let _ = done.send(());
        }
    }
}

/// Looks for hand edits. A change is loaded once the file has stayed the same for one round, so
/// an editor that writes in pieces is read when it is done. A file that was not usable is not
/// tried again until it changes.
async fn watch<T>(file: Weak<Inner<T>>)
where
    T: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + 'static,
{
    let mut seen: Option<Stamp> = None;
    let mut refused: Option<Stamp> = None;
    loop {
        tokio::time::sleep(WATCH_INTERVAL).await;
        let Some(inner) = file.upgrade() else {
            return;
        };
        let Some(now) = stamp(&inner.path).filter(|s| Some(*s) != refused) else {
            seen = None;
            continue;
        };
        let last = lock(&inner.state).last.map(|seen| seen.stamp);
        // The size and time say when to look. A file written a moment ago is looked at every round,
        // because a coarse clock can give a second edit the time of the first.
        let changed = Some(now) != last || (recent(now.1) && inner.bytes_differ().await);
        if !changed {
            seen = None;
            continue;
        }
        if seen != Some(now) {
            seen = Some(now);
            continue;
        }
        seen = None;
        if !inner.reload().await {
            refused = Some(now);
        }
    }
}

fn stamp_of(metadata: &fs::Metadata) -> Stamp {
    (
        metadata.len(),
        metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    )
}

fn stamp(path: &Path) -> Option<Stamp> {
    fs::metadata(path).ok().map(|m| stamp_of(&m))
}

/// A file touched this recently may share its time with the next edit.
fn recent(modified: SystemTime) -> bool {
    let apart = match SystemTime::now().duration_since(modified) {
        Ok(age) => age,
        Err(ahead) => ahead.duration(),
    };
    apart < RECENT
}

/// What one pass at reading a file found.
enum Read {
    /// The bytes, and the size and time of the file they came from.
    Stable(Vec<u8>, Stamp),
    /// The file changed while it was read.
    Changed,
    TooLarge,
}

/// Reads the whole file through one handle, so the bytes and the stamp belong together, and not
/// more than [`MAX_FILE_BYTES`].
fn read_once(path: &Path) -> io::Result<Read> {
    let file = fs::File::open(path)?;
    let before = stamp_of(&file.metadata()?);
    if before.0 > MAX_FILE_BYTES {
        return Ok(Read::TooLarge);
    }
    let mut bytes = Vec::with_capacity(before.0 as usize);
    (&file).take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Ok(Read::TooLarge);
    }
    let after = stamp_of(&file.metadata()?);
    if before != after || bytes.len() as u64 != after.0 {
        return Ok(Read::Changed);
    }
    Ok(Read::Stable(bytes, after))
}

fn load_blocking<T: Serialize + DeserializeOwned>(
    path: &Path,
    codec: &dyn FileCodec,
    validate: Option<&ValidateFn<T>>,
    warn_unknown_keys: bool,
    known: Option<Hash>,
) -> Result<Loaded<T>, ConfigFileError> {
    let mut attempt = 1;
    let (bytes, stamp) = loop {
        let trouble = match read_once(path) {
            Ok(Read::Stable(bytes, stamp)) => break (bytes, stamp),
            Ok(Read::TooLarge) => {
                return Err(ConfigFileError::io(
                    path,
                    format!("it is larger than {} MiB", MAX_FILE_BYTES / (1024 * 1024)),
                ));
            }
            Ok(Read::Changed) => "it kept changing while it was read".to_string(),
            Err(e) => format!("it could not be read ({e})"),
        };
        if attempt >= READ_ATTEMPTS {
            return Err(ConfigFileError::io(path, trouble));
        }
        attempt += 1;
        std::thread::sleep(READ_RETRY_DELAY);
    };
    let seen = Seen {
        stamp,
        hash: hash_of(&bytes),
    };
    if known == Some(seen.hash) {
        return Ok(Loaded::Same(seen));
    }
    // The plaintext of an encrypted file is wiped when this function is done with it.
    let text = Zeroizing::new(codec.decode(&bytes).map_err(|e| {
        ConfigFileError::new(
            path,
            ConfigErrorKind::Decrypt,
            format!("it could not be decrypted ({e})"),
        )
    })?);
    let not_json = |e: serde_json::Error| {
        ConfigFileError::new(
            path,
            ConfigErrorKind::Parse,
            format!("it is not valid JSON for this file ({e})"),
        )
    };
    let mut value = parse_json(&text).map_err(not_json)?;
    if value.is_null() {
        return Err(ConfigFileError::new(
            path,
            ConfigErrorKind::Invalid,
            "it holds no value",
        ));
    }
    take_version(path, &mut value)?;
    let names: Vec<String> = match (&value, warn_unknown_keys) {
        (Value::Object(map), true) => map.keys().cloned().collect(),
        _ => Vec::new(),
    };
    let value: T = serde_json::from_value(value).map_err(not_json)?;
    if let Some(invalid) = validate.and_then(|validate| validate(&value)) {
        return Err(ConfigFileError::new(
            path,
            ConfigErrorKind::Invalid,
            format!("it is not valid ({invalid})"),
        ));
    }
    let unknown = unknown_names(&names, &value);
    if !unknown.is_empty() {
        tracing::warn!(
            "{} has names Koppel does not know ({}). They are ignored, and gone after the next change Koppel writes",
            path.display(),
            unknown.join(", ")
        );
    }
    Ok(Loaded::New(value, seen))
}

/// Takes `version` out of the file's top level. A file from a newer Koppel is refused, since
/// writing it back would drop what that Koppel put there. A file without one is version 1.
fn take_version(path: &Path, value: &mut Value) -> Result<(), ConfigFileError> {
    let Value::Object(map) = value else {
        return Ok(());
    };
    let Some(key) = map
        .keys()
        .find(|key| key.eq_ignore_ascii_case("version"))
        .cloned()
    else {
        return Ok(());
    };
    match map.shift_remove(&key).and_then(|v| v.as_u64()) {
        Some(version) if version > FORMAT_VERSION => Err(ConfigFileError::new(
            path,
            ConfigErrorKind::Newer,
            format!(
                "it is version {version}, written by a newer Koppel; this one reads version {FORMAT_VERSION}"
            ),
        )),
        Some(_) => Ok(()),
        None => Err(ConfigFileError::new(
            path,
            ConfigErrorKind::Invalid,
            "its version is not a whole number",
        )),
    }
}

/// The top level names in `input` that `value` has no field for, matched without regard to case
/// as reading does.
fn unknown_names<T: Serialize>(input: &[String], value: &T) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }
    let Ok(Value::Object(known)) = serde_json::to_value(value) else {
        return Vec::new();
    };
    input
        .iter()
        .filter(|name| !known.keys().any(|k| k.eq_ignore_ascii_case(name)))
        .cloned()
        .collect()
}

/// What goes into the file: the value's fields, then `version`.
fn versioned_json<T: Serialize>(value: &T) -> Vec<u8> {
    #[derive(Serialize)]
    struct Versioned<'a, T> {
        #[serde(flatten)]
        value: &'a T,
        version: u64,
    }
    serde_json::to_vec_pretty(&Versioned {
        value,
        version: FORMAT_VERSION,
    })
    .expect("a config value always serializes")
}

/// The JSON of `value` as a Koppel file holds it, indented, with the `version`. For the upgrade,
/// which writes a file the same way [`ConfigFile`] does.
pub fn encode_json<T: Serialize>(value: &T) -> Vec<u8> {
    versioned_json(value)
}

/// The JSON in `bytes`, after a UTF-8 byte order mark if there is one: Notepad adds one.
fn parse_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
}

/// Replaces `path` with `bytes` the way [`ConfigFile`] writes: a temp file, flushed, moved over the
/// file, and the directory flushed. For the upgrade.
pub fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic(path, bytes).map(|_| ())
}

/// The points in writing a file where a test can make it fail, to see what a crash leaves behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    TempCreated,
    TempWritten,
    TempFlushed,
    BeforeReplace,
    Replaced,
    DirectoryFlushed,
}

/// Writes `bytes` to a random temp name beside `path`, flushes it to disk, moves it over `path`
/// and flushes the directory, so the new file survives a power cut. The temp file is `0600`. The
/// move is retried for a while, because on Windows a reader (an editor, antivirus) can hold the
/// target open for a moment. A failed write removes its temp file. Returns the new file's size and
/// time, taken from the file before it was moved, so an edit that lands after the move is not
/// mistaken for this write.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<Stamp> {
    write_atomic_with(path, bytes, &|_| Ok(()))
}

pub(crate) fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    at: &dyn Fn(Step) -> io::Result<()>,
) -> io::Result<Stamp> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}{TEMP_SUFFIX}", crate::random_hex(16)));
    let temp = PathBuf::from(temp);
    let result = (|| {
        let mut file = create_private(&temp)?;
        at(Step::TempCreated)?;
        file.write_all(bytes)?;
        at(Step::TempWritten)?;
        file.sync_all()?;
        at(Step::TempFlushed)?;
        let stamp = stamp_of(&file.metadata()?);
        drop(file);
        at(Step::BeforeReplace)?;
        let mut attempt = 1;
        loop {
            match fs::rename(&temp, path) {
                Ok(()) => break,
                Err(_) if attempt < REPLACE_ATTEMPTS => {
                    attempt += 1;
                    std::thread::sleep(REPLACE_RETRY_DELAY);
                }
                Err(e) => return Err(e),
            }
        }
        at(Step::Replaced)?;
        sync_directory(path)?;
        at(Step::DirectoryFlushed)?;
        Ok(stamp)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// A new file that only its owner can read, where the platform has modes. On Windows the
/// directory's access list decides, and the config directories are restricted already.
pub(crate) fn create_private(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Flushes the directory `path` is in, so a file moved or made there is not undone by a power cut.
/// Windows keeps its file system's metadata in a journal and has no call for this. A file system
/// that cannot flush a directory (some network and FUSE mounts) says so with an error that is not
/// worth failing a write over.
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let directory = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        match fs::File::open(directory).and_then(|d| d.sync_all()) {
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::InvalidInput | io::ErrorKind::Unsupported
                ) =>
            {
                tracing::debug!(
                    "{} cannot be flushed ({e}); relying on the file system",
                    directory.display()
                );
                Ok(())
            }
            other => other,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Leftovers of a crash between creating a temp file and moving it: `name.*.tmp`. Nothing else
/// uses that pattern. Best effort; the next start tries again.
fn delete_stale_temp_files(path: &Path) {
    let (Some(directory), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return;
    };
    let prefix = format!("{name}.");
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if file_name.starts_with(&prefix)
            && file_name.ends_with(TEMP_SUFFIX)
            && file_name.len() > prefix.len() + TEMP_SUFFIX.len()
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// `yyyyMMddHHmmssfff`, as 0.1.x names a file it moved aside.
pub fn utc_stamp(now: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}{:03}",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.millisecond()
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("koppel-file-{name}-{}", crate::random_hex(6)));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(TEMP_SUFFIX))
            .collect()
    }

    const ALL_STEPS: [Step; 6] = [
        Step::TempCreated,
        Step::TempWritten,
        Step::TempFlushed,
        Step::BeforeReplace,
        Step::Replaced,
        Step::DirectoryFlushed,
    ];

    #[test]
    fn a_write_goes_through_every_step_in_order_and_flushes_the_directory_last() {
        let dir = temp_dir("steps");
        let seen = RefCell::new(Vec::new());
        write_atomic_with(&dir.join("a.json"), b"new", &|step| {
            seen.borrow_mut().push(step);
            Ok(())
        })
        .unwrap();
        assert_eq!(*seen.borrow(), ALL_STEPS);
        assert_eq!(fs::read(dir.join("a.json")).unwrap(), b"new");
        let _ = fs::remove_dir_all(dir);
    }

    /// A failure at each point stands for the process dying there. Before the replace the old file
    /// is whole and no temp file is left behind; from the replace on, the new file is whole.
    #[test]
    fn a_failure_at_any_step_leaves_the_old_file_or_the_whole_new_one() {
        for failing in ALL_STEPS {
            let dir = temp_dir("crash");
            let target = dir.join("a.json");
            fs::write(&target, b"old contents").unwrap();

            let result = write_atomic_with(&target, b"new contents, longer", &|step| {
                if step == failing {
                    Err(io::Error::other("injected"))
                } else {
                    Ok(())
                }
            });

            assert!(result.is_err(), "{failing:?}");
            let on_disk = fs::read(&target).unwrap();
            let replaced = matches!(failing, Step::Replaced | Step::DirectoryFlushed);
            let expected: &[u8] = if replaced {
                b"new contents, longer"
            } else {
                b"old contents"
            };
            assert_eq!(on_disk, expected, "{failing:?}");
            assert!(leftovers(&dir).is_empty(), "{failing:?}");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_failure_to_make_the_file_leaves_no_file_at_all() {
        let dir = temp_dir("fresh");
        let target = dir.join("a.json");
        let result = write_atomic_with(&target, b"x", &|step| {
            if step == Step::TempWritten {
                Err(io::Error::other("disk full"))
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert!(!target.exists());
        assert!(leftovers(&dir).is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_temp_file_and_so_the_file_are_for_their_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let seen = RefCell::new(None);
        write_atomic_with(&dir.join("a.json"), b"x", &|step| {
            if step == Step::TempCreated {
                // The temp file is the only entry with the suffix at this point.
                let temp = leftovers(&dir).pop().unwrap();
                *seen.borrow_mut() =
                    Some(fs::metadata(dir.join(temp)).unwrap().permissions().mode() & 0o777);
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(*seen.borrow(), Some(0o600));
        let mode = fs::metadata(dir.join("a.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_stamp_is_the_files_own_size_and_time_so_nothing_slips_in_after_the_move() {
        let dir = temp_dir("stamp");
        let target = dir.join("a.json");
        let stamp = write_atomic(&target, b"{}").unwrap();
        assert_eq!(stamp, super::stamp(&target).unwrap());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_files_version_is_read_and_a_newer_one_refused() {
        let path = Path::new("x.json");
        let mut plain: Value = serde_json::from_str(r#"{"role":"client"}"#).unwrap();
        take_version(path, &mut plain).unwrap();

        let mut current: Value = serde_json::from_str(r#"{"Version":1,"role":"client"}"#).unwrap();
        take_version(path, &mut current).unwrap();
        assert!(current.get("Version").is_none(), "taken out of the value");

        let mut newer: Value = serde_json::from_str(r#"{"version":2}"#).unwrap();
        let refused = take_version(path, &mut newer).unwrap_err();
        assert_eq!(refused.kind, ConfigErrorKind::Newer);

        for odd in [
            r#"{"version":"1"}"#,
            r#"{"version":-1}"#,
            r#"{"version":1.5}"#,
        ] {
            let mut value: Value = serde_json::from_str(odd).unwrap();
            assert_eq!(
                take_version(path, &mut value).unwrap_err().kind,
                ConfigErrorKind::Invalid,
                "{odd}"
            );
        }
    }

    #[test]
    fn names_the_type_has_no_field_for_are_found_in_any_case() {
        let settings = crate::files::KoppelSettings::default();
        let input: Vec<String> = ["Role", "shareOnStartup", "colour", "NOTIFYLINK", "x"]
            .map(String::from)
            .to_vec();
        assert_eq!(unknown_names(&input, &settings), ["colour", "x"]);
        assert!(unknown_names(&[], &settings).is_empty());
    }

    #[test]
    fn what_is_written_ends_with_the_version_after_the_fields_as_before() {
        let text = String::from_utf8(versioned_json(&crate::files::KoppelSettings::default()))
            .unwrap()
            .replace("\r\n", "\n");
        assert!(text.starts_with("{\n  \"role\": \"server\",\n"), "{text}");
        assert!(
            text.ends_with("\"notifyAttachFailures\": true,\n  \"version\": 1\n}"),
            "{text}"
        );
    }

    #[test]
    fn each_kind_of_error_gives_its_own_advice() {
        let path = Path::new("secrets.json");
        let advice = |kind| ConfigFileError::new(path, kind, "why").to_string();
        assert!(advice(ConfigErrorKind::Decrypt).contains("restore the keys directory"));
        assert!(advice(ConfigErrorKind::Parse).contains("fix the edit"));
        assert!(advice(ConfigErrorKind::Invalid).contains("fix the edit"));
        assert!(advice(ConfigErrorKind::Newer).contains("Install the Koppel that wrote it"));
        let io = advice(ConfigErrorKind::Io);
        assert!(!io.contains("restore the keys directory"), "{io}");
        assert!(io.starts_with("Koppel cannot use secrets.json: why. "));
    }
}
