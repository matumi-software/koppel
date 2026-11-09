//! One JSON config file that is the source of truth for a value, as `JsonConfigFile` is in
//! Koppel 0.1.x. A change is written to disk first (temp file, flush, replace) and only a write
//! that landed reaches the subscribers. Edits made by hand are picked up while it runs.
//!
//! The rules, from docs/architecture.md:
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
//!   one, and the call returns without waiting for it, since it would wait for itself.
//!
//! Hand edits are found by looking at the file's size and time every quarter second rather than
//! through the operating system's change events: those do not arrive through every Docker bind
//! mount, and the poll costs one `stat` per file.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::json;

const READ_ATTEMPTS: u32 = 5;
const READ_RETRY_DELAY: Duration = Duration::from_millis(200);
const REPLACE_ATTEMPTS: u32 = 20;
const REPLACE_RETRY_DELAY: Duration = Duration::from_millis(100);
const TEMP_SUFFIX: &str = ".tmp";
const WATCH_INTERVAL: Duration = Duration::from_millis(250);

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

/// A config file exists but cannot be used, or a change could not be written. The file is left
/// exactly as it was. The wording is 0.1.x's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileError {
    /// The file.
    pub path: PathBuf,
    /// What is wrong with it, in one line.
    pub reason: String,
}

impl ConfigFileError {
    fn new(path: &Path, reason: impl Into<String>) -> Self {
        Self {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }
}

impl fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Koppel cannot use {}: {}. The file was not changed. If it is encrypted, the key ring that \
             encrypted it is missing or has changed: restore the keys directory. If it was edited by \
             hand, fix the edit or restore the file. Then start Koppel again.",
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
}

impl<T> Default for Options<T> {
    fn default() -> Self {
        Self {
            codec: Arc::new(PlainText),
            validate: None,
            start_with_defaults_when_unusable: false,
        }
    }
}

/// The size and time of the file, to see that it changed.
type Stamp = (u64, SystemTime);

struct State<T> {
    current: Arc<T>,
    current_json: Vec<u8>,
    last_stamp: Option<Stamp>,
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
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    /// A file at `path` holding `defaults` until it is started.
    pub fn new(path: impl Into<PathBuf>, defaults: T, options: Options<T>) -> Self {
        let current_json = json::to_vec(&defaults).expect("a config value always serializes");
        let (deliveries, receiver) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                path: path.into(),
                defaults: defaults.clone(),
                codec: options.codec,
                validate: options.validate,
                lenient: options.start_with_defaults_when_unusable,
                gate: tokio::sync::Mutex::new(()),
                state: Mutex::new(State {
                    current: Arc::new(defaults),
                    current_json,
                    last_stamp: None,
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
            ConfigFileError::new(
                &inner.path,
                format!("its directory could not be made ({e})"),
            )
        })?;
        delete_stale_temp_files(&inner.path);
        if inner.path.exists() {
            match inner.load().await {
                Ok((value, stamp)) => inner.set_current(value, Some(stamp)),
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
            let stamp = inner.write(&inner.defaults).await?;
            lock(&inner.state).last_stamp = Some(stamp);
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
    /// Waits until the subscribers have seen the change, except when called from inside one of
    /// this file's deliveries.
    pub async fn update(&self, change: impl FnOnce(&T) -> T) -> Result<Arc<T>, ConfigFileError> {
        let inner = &self.inner;
        let context = DELIVERING.try_with(Clone::clone).unwrap_or_default();
        let nested = context.contains(&inner.id);
        let (next, done) = {
            let _gate = inner.gate.lock().await;
            inner.pick_up_hand_edit().await?;
            let current = self.current();
            let next = change(&current);
            let next_json = json::to_vec(&next).expect("a config value always serializes");
            let unchanged = next_json == lock(&inner.state).current_json;
            if !unchanged || !inner.path.exists() {
                let stamp = inner.write(&next).await?;
                lock(&inner.state).last_stamp = Some(stamp);
            }
            let next = Arc::new(next);
            {
                let mut state = lock(&inner.state);
                state.current = next.clone();
                state.current_json = next_json;
                state.problem = None;
            }
            let done = inner.enqueue(next.clone(), context);
            (next, done)
        };
        if !nested {
            let _ = done.await;
        }
        Ok(next)
    }
}

impl<T> Inner<T>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    fn set_current(&self, value: T, stamp: Option<Stamp>) {
        let current_json = json::to_vec(&value).expect("a config value always serializes");
        let mut state = lock(&self.state);
        state.current = Arc::new(value);
        state.current_json = current_json;
        state.problem = None;
        if stamp.is_some() {
            state.last_stamp = stamp;
        }
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

    /// Reads, decodes, parses and checks the file. Retries a file that is briefly locked.
    async fn load(&self) -> Result<(T, Stamp), ConfigFileError> {
        let path = self.path.clone();
        let codec = self.codec.clone();
        let validate = self.validate.clone();
        tokio::task::spawn_blocking(move || {
            load_blocking(&path, codec.as_ref(), validate.as_deref())
        })
        .await
        .map_err(|e| ConfigFileError::new(&self.path, format!("it could not be read ({e})")))?
    }

    async fn write(&self, value: &T) -> Result<Stamp, ConfigFileError> {
        let json = json::to_vec(value).expect("a config value always serializes");
        let path = self.path.clone();
        let codec = self.codec.clone();
        tokio::task::spawn_blocking(move || {
            let bytes = codec.encode(&json).map_err(|e| {
                ConfigFileError::new(&path, format!("it could not be encrypted ({e})"))
            })?;
            write_atomic(&path, &bytes)
                .map_err(|e| ConfigFileError::new(&path, format!("it could not be written ({e})")))
        })
        .await
        .map_err(|e| ConfigFileError::new(&self.path, format!("it could not be written ({e})")))?
    }

    /// Under the gate: loads the file when it changed behind our back and is usable.
    async fn pick_up_hand_edit(&self) -> Result<(), ConfigFileError> {
        let Some(stamp) = stamp(&self.path) else {
            return Ok(());
        };
        if Some(stamp) == lock(&self.state).last_stamp {
            return Ok(());
        }
        match self.load().await {
            Ok((value, stamp)) => self.set_current(value, Some(stamp)),
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
            ConfigFileError::new(
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
                Ok((value, stamp)) => {
                    let json = json::to_vec(&value).expect("a config value always serializes");
                    let changed = json != lock(&self.state).current_json;
                    if changed {
                        self.set_current(value, Some(stamp));
                        let current = lock(&self.state).current.clone();
                        Some(self.enqueue(current, Vec::new()))
                    } else {
                        // Our own writes land here too; they read back the same and are ignored.
                        let mut state = lock(&self.state);
                        state.problem = None;
                        state.last_stamp = Some(stamp);
                        None
                    }
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
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    let mut seen: Option<Stamp> = None;
    let mut refused: Option<Stamp> = None;
    loop {
        tokio::time::sleep(WATCH_INTERVAL).await;
        let Some(inner) = file.upgrade() else {
            return;
        };
        let now = stamp(&inner.path);
        let last = lock(&inner.state).last_stamp;
        let Some(now) = now.filter(|s| Some(*s) != last && Some(*s) != refused) else {
            seen = None;
            continue;
        };
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

fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = fs::metadata(path).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

fn load_blocking<T: DeserializeOwned>(
    path: &Path,
    codec: &dyn FileCodec,
    validate: Option<&ValidateFn<T>>,
) -> Result<(T, Stamp), ConfigFileError> {
    let mut attempt = 1;
    let (bytes, stamp) = loop {
        let before = stamp(path);
        match fs::read(path) {
            Ok(bytes) => break (bytes, before.unwrap_or((0, SystemTime::UNIX_EPOCH))),
            Err(_) if attempt < READ_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(READ_RETRY_DELAY);
            }
            Err(e) => {
                return Err(ConfigFileError::new(
                    path,
                    format!("it could not be read ({e})"),
                ));
            }
        }
    };
    let text = codec
        .decode(&bytes)
        .map_err(|e| ConfigFileError::new(path, format!("it could not be decrypted ({e})")))?;
    let not_json = |e: serde_json::Error| {
        ConfigFileError::new(path, format!("it is not valid JSON for this file ({e})"))
    };
    let value = json::parse(&text).map_err(not_json)?;
    if value.is_null() {
        return Err(ConfigFileError::new(path, "it holds no value"));
    }
    let value: T = json::from_value(value).map_err(not_json)?;
    if let Some(invalid) = validate.and_then(|validate| validate(&value)) {
        return Err(ConfigFileError::new(
            path,
            format!("it is not valid ({invalid})"),
        ));
    }
    Ok((value, stamp))
}

/// Writes `bytes` to a random temp name beside `path`, flushes it to disk, and moves it over
/// `path`. The move is retried for a while, because on Windows a reader (an editor, antivirus) can
/// hold the target open for a moment. A failed write removes its temp file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<Stamp> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}{TEMP_SUFFIX}", crate::random_hex(16)));
    let temp = PathBuf::from(temp);
    let result = (|| {
        let mut file = fs::File::create_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let mut attempt = 1;
        loop {
            match fs::rename(&temp, path) {
                Ok(()) => return Ok(()),
                Err(_) if attempt < REPLACE_ATTEMPTS => {
                    attempt += 1;
                    std::thread::sleep(REPLACE_RETRY_DELAY);
                }
                Err(e) => return Err(e),
            }
        }
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e);
    }
    Ok(stamp(path).unwrap_or((0, SystemTime::UNIX_EPOCH)))
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
pub(crate) fn utc_stamp(now: OffsetDateTime) -> String {
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
