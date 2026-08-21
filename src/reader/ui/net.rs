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
//! cancellation real — it drags in tokio and rewrites `fetch.rs`, the file the charter guards
//! most closely.
//!
//! # Panic containment
//!
//! Image decoding is untrusted-input parsing. "Treat a dead thread as a failed load" is not a
//! mechanism here: each worker holds a *cloned* `Sender`, so an unwinding thread closes nothing
//! and the UI would wait forever on a reply that never comes. Every job therefore runs under a
//! [`ReplyGuard`] that emits a failure on unwind — which also covers a panic outside the decode
//! call, unlike wrapping the decode in `catch_unwind`. This depends on `panic = "unwind"`; a
//! future `panic = "abort"` in `[profile.release]` silently disarms it, and this paragraph is
//! what such a change has to answer.

use crate::fetch::FetchError;
use crate::reader::image_decode::{self, DecodeLimits, Decoded};
use crate::session::{LoadError, LoadOptions, Loaded, Notice, Session};
use eframe::egui;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use url::Url;

/// Four, because transfers are I/O-bound and the pool exists so a slow one cannot block a
/// fast one. Decoding is serialized separately — see [`DECODE_LOCK`].
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
    /// reply. `referrer` is the page being read — see `fetch::Referer`.
    Image {
        req: ReqId,
        page: ReqId,
        url: Url,
        referrer: Url,
        max_width: u32,
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
    Notice { req: ReqId, notice: Notice },
    Loaded {
        req: ReqId,
        loaded: Box<Loaded>,
    },
    Failed { req: ReqId, error: LoadError },
    Image {
        req: ReqId,
        page: ReqId,
        url: Url,
        result: Result<Decoded, ImageError>,
    },
    /// A worker unwound. Carries the page so the UI can stop waiting on it.
    Panicked { req: ReqId, page: ReqId },
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
            let job_rx = Arc::clone(&job_rx);
            let msg_tx = msg_tx.clone();
            let session = Arc::clone(&session);
            let ctx = ctx.clone();
            std::thread::Builder::new()
                .name(format!("hww-net-{i}"))
                .spawn(move || {
                    loop {
                        // Held only across `recv`, never across the request, or the pool would
                        // be a single worker wearing four hats.
                        let job = {
                            let rx = job_rx.lock().unwrap_or_else(|e| e.into_inner());
                            rx.recv()
                        };
                        let Ok(job) = job else { return };
                        let mut guard = ReplyGuard {
                            req: job.req(),
                            page: job.page(),
                            tx: msg_tx.clone(),
                            ctx: ctx.clone(),
                            armed: true,
                        };
                        run_job(&session, job, &msg_tx, &ctx);
                        guard.armed = false;
                    }
                })
                .map_err(|e| LoadError::Fetch(FetchError::Io(e)))?;
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
        // A closed channel means every worker died, which cannot happen while `self` holds the
        // sender. Dropping the job is still better than a panic in a reader.
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
            let mut notify = |n: &Notice| {
                let _ = tx.send(Msg::Notice {
                    req,
                    notice: n.clone(),
                });
                // The notice travels the channel before the blocking fetch starts, so it is on
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
            url,
            referrer,
            max_width,
        } => {
            let result = load_image(session, &url, &referrer, max_width);
            let _ = tx.send(Msg::Image {
                req,
                page,
                url,
                result,
            });
        }
    }
    ctx.request_repaint();
}

fn load_image(
    session: &Session,
    url: &Url,
    referrer: &Url,
    max_width: u32,
) -> Result<Decoded, ImageError> {
    let fetched = session.fetch_image(url, referrer)?;
    let _serialized = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let limits = DecodeLimits {
        max_width,
        ..DecodeLimits::default()
    };
    Ok(image_decode::decode(&fetched.bytes, &fetched.partial, &limits)?)
}
