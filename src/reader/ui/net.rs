//! Worker threads, request correlation, and the one rule that keeps a hung fetch from
//! freezing the reader.
//!
//! # Threading
//!
//! `reqwest::blocking::Client` is `Send + Sync` and pools internally, so one `Arc<Session>` is
//! shared by a fixed pool of [`WORKERS`] threads. One worker would queue an image click behind
//! a 15 s hanging navigation, which is the exact failure this exists to avoid.
//!
//! # Correlation, not cancellation
//!
//! `reqwest::blocking` has no cancel token: an abandoned request runs to completion or timeout
//! and its result is thrown away. Every navigation mints a fresh [`ReqId`] and the UI drops any
//! message whose id is not the current one. That is the entire stale-response defense, and it
//! covers "the user hit back twice during a slow load". Do **not** introduce async to make
//! cancellation real: it drags in tokio and rewrites `fetch.rs`, the file the charter guards
//! most closely.
//!
//! # Panic containment
//!
//! Image decoding is untrusted-input parsing. "Treat a dead thread as a failed load" is not a
//! mechanism here: each worker holds a *cloned* `Sender`, so an unwinding thread closes nothing
//! and the UI would wait forever on a reply that never comes. Every job therefore runs under a
//! [`ReplyGuard`] that emits a failure on unwind, which also covers a panic outside the decode
//! call, unlike wrapping the decode in `catch_unwind`. The unwind still ends the thread, so
//! [`RespawnOnPanic`] refills the pool slot; containing the reply and containing the pool are
//! two jobs. Both depend on `panic = "unwind"`; a future `panic = "abort"` in
//! `[profile.release]` silently disarms them, and this paragraph is what such a change has to
//! answer. Note that a *stack overflow* aborts rather than unwinding, which is why the
//! extractor caps its own recursion (`html::MAX_DEPTH`) instead of relying on this.

use crate::fetch::FetchError;
use crate::reader::image_decode::{self, DecodeLimits, Decoded};
use crate::session::{LoadError, LoadOptions, Loaded, Rewrite, Session};
use eframe::egui;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use url::Url;

/// Four, because transfers are I/O-bound and the pool exists so a slow one cannot block a
/// fast one. Decoding is serialized separately (see [`DECODE_LOCK`]).
const WORKERS: usize = 4;

/// Decode is CPU-bound and each decode may hold up to the alloc ceiling in transient buffer,
/// so four concurrent decodes would be a gigabyte. Serializing costs no wall-clock worth
/// having and caps peak decode memory at one buffer.
static DECODE_LOCK: Mutex<()> = Mutex::new(());

/// Correlates a reply with the navigation that asked for it. Monotonic for the process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReqId(u64);

impl ReqId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ReqId(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub enum Job {
    Load {
        req: ReqId,
        url: Url,
        opts: LoadOptions,
    },
    /// An image subresource, on an explicit click only. `page` is the navigation it belongs to,
    /// so a result that arrives after the reader moved on is discarded like any other stale
    /// reply. `referrer` is the page being read (see `fetch::Referer`).
    Image {
        req: ReqId,
        page: ReqId,
        /// The `src` exactly as `ir::Image` carries it. It travels with the job rather than
        /// being recovered from the response, because a redirect makes the fetched URL a
        /// different string from the one the placeholder is keyed by, and then the picture
        /// arrives and nothing on screen changes.
        src: String,
        url: Url,
        referrer: Url,
        max_width: u32,
        /// The settings toggle, carried to the fetch rather than applied to the report.
        send_referer: bool,
    },
}

impl Job {
    fn req(&self) -> ReqId {
        match self {
            Job::Load { req, .. } | Job::Image { req, .. } => *req,
        }
    }

    /// The page a reply belongs to. For a navigation that is itself; for an image it is the
    /// page it was clicked on.
    fn page(&self) -> ReqId {
        match self {
            Job::Load { req, .. } => *req,
            Job::Image { page, .. } => *page,
        }
    }
}

pub enum Msg {
    /// Emitted before the request is dispatched. Charter point 4.
    Rewrote {
        req: ReqId,
        rewrite: Rewrite,
    },
    Loaded {
        req: ReqId,
        loaded: Box<Loaded>,
    },
    Failed {
        req: ReqId,
        error: LoadError,
    },
    Image {
        req: ReqId,
        page: ReqId,
        src: String,
        /// Cookies the subresource tried to set, counted whether or not it decoded.
        ///
        /// Its own field rather than a member of the `Ok`: the fetch is what sees a cookie and
        /// the decode is what fails, so pairing the count with the picture loses it on exactly
        /// the untrusted-input path most likely to fail.
        cookies: usize,
        result: Result<Decoded, ImageError>,
    },
    /// A worker unwound. Carries the page so the UI can stop waiting on it.
    Panicked {
        req: ReqId,
        page: ReqId,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("{0}")]
    Fetch(#[from] FetchError),
    #[error("{0}")]
    Decode(#[from] image_decode::DecodeError),
}

/// Sends a failure if the job's thread unwinds. Disarmed on the normal path.
struct ReplyGuard {
    req: ReqId,
    page: ReqId,
    tx: mpsc::Sender<Msg>,
    ctx: egui::Context,
    armed: bool,
}

impl Drop for ReplyGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.tx.send(Msg::Panicked {
                req: self.req,
                page: self.page,
            });
            self.ctx.request_repaint();
        }
    }
}

/// One pool worker, which replaces itself if it dies.
///
/// [`ReplyGuard`] turns a panic into a failed tab, but the unwind continues out of the
/// closure and takes the thread with it, and the receive loop lives *inside* that closure.
/// Nothing used to respawn, so each panic permanently cost a worker; after [`WORKERS`] of
/// them the pool was empty, the last `Arc` on the job receiver dropped, and every subsequent
/// `submit` failed silently and left the tab on `Page::Loading` forever. Containing the reply
/// is not the same as containing the pool.
///
/// The replacement is spawned from the dying thread's own guard, so it costs nothing on the
/// normal path and does not need the pool to be watched from outside.
fn spawn_worker(
    i: usize,
    job_rx: Arc<Mutex<mpsc::Receiver<Job>>>,
    msg_tx: mpsc::Sender<Msg>,
    session: Arc<Session>,
    ctx: egui::Context,
) -> Result<(), LoadError> {
    let respawn = RespawnOnPanic {
        i,
        job_rx: Arc::clone(&job_rx),
        msg_tx: msg_tx.clone(),
        session: Arc::clone(&session),
        ctx: ctx.clone(),
        armed: true,
    };
    std::thread::Builder::new()
        .name(format!("hww-net-{i}"))
        .spawn(move || {
            let mut respawn = respawn;
            loop {
                // Held only across `recv`, never across the request, or the pool would
                // be a single worker wearing four hats.
                let job = {
                    let rx = respawn.job_rx.lock().unwrap_or_else(|e| e.into_inner());
                    rx.recv()
                };
                let Ok(job) = job else {
                    // The sender is gone: the reader is shutting down, not failing.
                    respawn.armed = false;
                    return;
                };
                let mut guard = ReplyGuard {
                    req: job.req(),
                    page: job.page(),
                    tx: respawn.msg_tx.clone(),
                    ctx: respawn.ctx.clone(),
                    armed: true,
                };
                run_job(&respawn.session, job, &respawn.msg_tx, &respawn.ctx);
                guard.armed = false;
            }
        })
        .map_err(|e| LoadError::Fetch(FetchError::Io(e)))?;
    Ok(())
}

/// Refills the pool slot when its worker unwinds out of the receive loop.
struct RespawnOnPanic {
    i: usize,
    job_rx: Arc<Mutex<mpsc::Receiver<Job>>>,
    msg_tx: mpsc::Sender<Msg>,
    session: Arc<Session>,
    ctx: egui::Context,
    armed: bool,
}

impl Drop for RespawnOnPanic {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // A failed spawn here is the one case with nowhere to report: the pool is down a
        // worker and the reader keeps running on the rest. Dropping is better than a panic
        // inside a panic, which aborts.
        let _ = spawn_worker(
            self.i,
            Arc::clone(&self.job_rx),
            self.msg_tx.clone(),
            Arc::clone(&self.session),
            self.ctx.clone(),
        );
    }
}

pub struct Net {
    jobs: mpsc::Sender<Job>,
    msgs: mpsc::Receiver<Msg>,
    session: Arc<Session>,
}

impl Net {
    pub fn new(ctx: egui::Context) -> Result<Self, LoadError> {
        let session = Arc::new(Session::new()?);
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (msg_tx, msgs) = mpsc::channel::<Msg>();
        let job_rx = Arc::new(Mutex::new(job_rx));

        for i in 0..WORKERS {
            spawn_worker(
                i,
                Arc::clone(&job_rx),
                msg_tx.clone(),
                Arc::clone(&session),
                ctx.clone(),
            )?;
        }
        Ok(Self {
            jobs,
            msgs,
            session,
        })
    }

    /// A fresh id. Every navigation gets one; everything older is stale by definition.
    pub fn mint(&self) -> ReqId {
        ReqId::next()
    }

    pub fn submit(&self, job: Job) {
        // A closed channel means every worker died. The *receiver* lives in the pool, not
        // here, so holding the sender does not keep it open. `spawn_worker` refills the pool
        // on panic, so this should now be unreachable; dropping the job is still better than
        // a panic in a reader.
        let _ = self.jobs.send(job);
    }

    pub fn try_recv(&self) -> Option<Msg> {
        self.msgs.try_recv().ok()
    }

    pub fn session(&self) -> &Session {
        &self.session
    }
}

fn run_job(session: &Session, job: Job, tx: &mpsc::Sender<Msg>, ctx: &egui::Context) {
    match job {
        Job::Load { req, url, opts } => {
            let mut notify = |n: &Rewrite| {
                let _ = tx.send(Msg::Rewrote {
                    req,
                    rewrite: n.clone(),
                });
                // It travels the channel before the blocking fetch starts, so it is on
                // screen for the whole of a 15 s hang instead of arriving with the corpse.
                ctx.request_repaint();
            };
            let msg = match session.load(&url, &opts, &mut notify) {
                Ok(loaded) => Msg::Loaded {
                    req,
                    loaded: Box::new(loaded),
                },
                Err(error) => Msg::Failed { req, error },
            };
            let _ = tx.send(msg);
        }
        Job::Image {
            req,
            page,
            src,
            url,
            referrer,
            max_width,
            send_referer,
        } => {
            let (cookies, result) = load_image(session, &url, &referrer, max_width, send_referer);
            let _ = tx.send(Msg::Image {
                req,
                page,
                src,
                cookies,
                result,
            });
        }
    }
    ctx.request_repaint();
}

/// Returns the cookie attempts alongside the result rather than inside it.
///
/// A cookie attempt is a fact about the *request*, and it is discovered before the bytes are
/// ever decoded. Folding it into the `Ok` arm would discard it whenever a decode failed, which
/// is the one path where untrusted bytes get a say in whether the count survives.
fn load_image(
    session: &Session,
    url: &Url,
    referrer: &Url,
    max_width: u32,
    send_referer: bool,
) -> (usize, Result<Decoded, ImageError>) {
    let fetched = match session.fetch_image(url, referrer, send_referer) {
        Ok(f) => f,
        Err(e) => return (0, Err(e.into())),
    };
    let cookies = fetched.cookie_attempts;
    let _serialized = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let limits = DecodeLimits {
        max_width,
        ..DecodeLimits::default()
    };
    let decoded = image_decode::decode(&fetched.bytes, &fetched.partial, &limits);
    (cookies, decoded.map_err(ImageError::from))
}
