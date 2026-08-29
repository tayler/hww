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
//! What *can* be cancelled is a job that has not started. The queue is one FIFO drained by
//! [`WORKERS`] threads with no priority lane, so a page whose images were loaded in bulk puts
//! however many of them ahead of the reader's next click, each with its own 30 s ceiling; the
//! click then waits on pictures nobody will ever see. [`Net::mint_page`] publishes the current
//! navigation to the pool, and a worker that pulls an *automatic* [`Job::Image`] stamped with
//! an older page answers [`Msg::ImageDropped`] instead of fetching. In-flight requests are
//! untouched — those are still correlation, not cancellation — but the backlog behind them
//! drains in microseconds. `ImageDropped` says "never requested" rather than "failed", so the
//! placeholder goes back to offering itself instead of showing an error for a fetch that never
//! happened, and the disclosure it had already recorded is taken back.
//!
//! Only the automatic ones. A picture the reader clicked, or `i`, or `Shift+I`, is a thing
//! somebody asked for on a page that is still on screen and still being read; the outgoing page
//! stays visible and stays clickable by design, and a navigation that fails leaves the reader
//! on it. Discarding those would be answering a request with silence. What the automatic policy
//! queues is speculative by construction — nobody asked for that particular picture — so it is
//! the half that can be thrown away.
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
use crate::reader::iconcache;
use crate::reader::image_decode::{self, DecodeLimits, Decoded};
use crate::session::{LoadError, LoadOptions, Loaded, Rewrite, Session};
use eframe::egui;
use std::path::PathBuf;
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
    /// An id without a [`Net`], which needs an `egui::Context` and therefore a window. Only the
    /// ordering and the equality matter to anything that takes one.
    #[cfg(test)]
    pub(crate) fn for_test(n: u64) -> Self {
        ReqId(n)
    }

    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        ReqId(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    /// The id as a plain integer, for the one place it has to live in an `AtomicU64`: the
    /// current page, shared with the pool. Ids start at 1, so the zero a fresh [`Net`] holds
    /// matches no page and is not mistaken for one.
    fn raw(self) -> u64 {
        self.0
    }
}

pub enum Job {
    Load {
        req: ReqId,
        url: Url,
        opts: LoadOptions,
    },
    /// An image subresource. Usually on an explicit click; also the masthead favicon.
    /// `page` is the navigation it belongs to,
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
        /// Queued by `ImagePolicy::Auto` rather than by a click, `i`, or `Shift+I`, and so
        /// discardable when the page it belongs to stops being current. See the module doc.
        automatic: bool,
        /// Where these bytes may be kept, or `None` — which is every article picture, and every
        /// site mark for an address hww's own lists do not name.
        ///
        /// **The decision is made on the UI thread and carried here, never taken here.** A
        /// worker sees a `Job::Image` and cannot tell an article picture from a favicon, and it
        /// cannot see the archive, so it cannot know whether an address is one a marked row
        /// names. A worker that decided for itself would cache page content, and would write a
        /// favicon for every host the masthead ever asked — a trail of everywhere the reader has
        /// been, outliving *forget history*. See `reader::iconcache`.
        cache_as: Option<PathBuf>,
    },
    /// A site mark read from `reader::iconcache` instead of from a host.
    ///
    /// Its own variant rather than a flag on [`Job::Image`], because it is the one image job
    /// that contacts nobody: no URL, no referrer, no cookie count, no `Referer`, and nothing for
    /// `ImageStore::record_request` to have recorded. It is on a worker at all only so the file
    /// read and the decode stay off the frame.
    KeptIcon {
        req: ReqId,
        page: ReqId,
        /// What `ImageStore` keys this mark by, so the reply lands on the widget that asked.
        src: String,
        /// The address the file is for, and what its header has to name.
        ///
        /// Not always [`Job::KeptIcon::src`], which is the difference that makes it a second
        /// field: a list row's `src` is already `archive::icon_address`'s output, while the
        /// masthead's is whatever `html::favicon_of` joined against the page — credentials and
        /// all. Checked in [`iconcache::read`], because the filename is a non-cryptographic
        /// hash and this is what stops one site's mark being drawn beside another's name.
        key: String,
        path: PathBuf,
        max_width: u32,
    },
}

impl Job {
    fn req(&self) -> ReqId {
        match self {
            Job::Load { req, .. } | Job::Image { req, .. } | Job::KeptIcon { req, .. } => *req,
        }
    }

    /// The page a job may be thrown away for, or `None` if it may not be.
    ///
    /// A picture the reader clicked, or `i`, or `Shift+I`, is a thing somebody asked for and is
    /// fetched whatever page it belongs to; see the module doc. What is discardable is what
    /// nobody asked for: the automatic policy's pictures, the site marks, and a mark being read
    /// off the disk.
    fn discardable_page(&self) -> Option<ReqId> {
        match self {
            Job::Image {
                page,
                automatic: true,
                ..
            }
            | Job::KeptIcon { page, .. } => Some(*page),
            _ => None,
        }
    }

    /// The `src` of a job [`Job::discardable_page`] answered for, so the reply can name it.
    fn discardable_src(&self) -> Option<String> {
        match self {
            Job::Image {
                src,
                automatic: true,
                ..
            }
            | Job::KeptIcon { src, .. } => Some(src.clone()),
            _ => None,
        }
    }

    /// The page a reply belongs to. For a navigation that is itself; for an image it is the
    /// page it was clicked on.
    fn page(&self) -> ReqId {
        match self {
            Job::Load { req, .. } => *req,
            Job::Image { page, .. } | Job::KeptIcon { page, .. } => *page,
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
        /// What the icon cache did for this reply. See [`Kept`].
        kept: Kept,
    },
    /// A [`Job::KeptIcon`] whose file was not there, not readable, or not for this address.
    ///
    /// **Never a fetch from the worker.** Falling through to the network here would put a
    /// third-party request on the wire that `ImageStore::record_request` never counted, because
    /// the UI took the cache path precisely to avoid touching it — the panel would then name
    /// hosts it had reported contacting nobody at. So the miss comes back, the UI drops the
    /// index entry, and the ordinary door goes out and discloses it.
    IconMissed {
        page: ReqId,
        src: String,
    },
    /// An image job that was still queued when its page stopped being the current one, and so
    /// was never dispatched.
    ///
    /// Distinct from a failed fetch on purpose. Nothing was requested, nothing was disclosed,
    /// and no host was contacted, so the counters must not move and the placeholder must not
    /// say that loading was tried. The UI forgets the entry, which puts it back to offering
    /// itself if the page it belongs to is still the one on screen.
    ImageDropped {
        page: ReqId,
        src: String,
    },
    /// A worker unwound. Carries the page so the UI can stop waiting on it.
    Panicked {
        req: ReqId,
        page: ReqId,
    },
}

/// What `reader::iconcache` did for one image reply.
///
/// One field on the reply rather than two messages, because both halves are things only the
/// worker knows and both have to reach the UI's in-memory index: without [`Kept::Written`] that
/// index would only ever describe the directory as it was at launch, and the second open of a
/// list in one session would refetch every mark the first open had just written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// Nothing: an article picture, or a mark for an address hww's lists do not name.
    No,
    /// These bytes came off this machine. No host was contacted, so no counter moved.
    Read,
    /// These bytes were fetched and written, stamped at this second. An empty body was written
    /// when the address answered and served nothing usable; see `iconcache`'s module doc.
    Written(u64),
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("{0}")]
    Fetch(#[from] FetchError),
    /// The request arrived and the server said no. Its own arm rather than a `FetchError`,
    /// because the document path shows a 4xx body on purpose (`--why` and the "shown anyway"
    /// caution both depend on it) and only an image is entitled to refuse one.
    #[error("the server answered {0}")]
    Status(u16),
    #[error("{0}")]
    Decode(#[from] image_decode::DecodeError),
}

impl ImageError {
    /// Whether asking again could plausibly end differently.
    ///
    /// Not every fetch failure is about the attempt. A downgrade refusal is *hww's own policy*
    /// and answers the same way forever; a redirect chain that loops, or lands on a `Location`
    /// the server cannot spell, is a fault in what the server serves rather than in reaching
    /// it. Offering a retry for those draws the button this change exists to remove, one arm
    /// further out.
    ///
    /// What is left is genuinely about the attempt: a timeout, a reset, a body that stopped
    /// early. `LikelyBlocked` counts as retryable — a rate-based block clears, and by its own
    /// doc it is a document-fetch diagnosis that an image should not be inheriting anyway.
    pub fn offers_retry(&self) -> bool {
        match self {
            Self::Fetch(e) => match e {
                FetchError::SchemeDowngrade(_)
                | FetchError::TooManyRedirects(_)
                | FetchError::BadLocation(_)
                | FetchError::RedirectWithoutLocation(_) => false,
                FetchError::LikelyBlocked
                | FetchError::Timeout(_)
                | FetchError::Transport(_)
                | FetchError::Io(_) => true,
            },
            // Asking again can only end differently if the server was answering about *now*.
            // A 429 lifts and a 5xx passes; every other refusal is about the address, and the
            // address is not going to change while the page is open.
            Self::Status(s) => *s == 429 || (500..600).contains(s),
            Self::Decode(e) => e.offers_retry(),
        }
    }
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
    page: Arc<AtomicU64>,
) -> Result<(), LoadError> {
    let respawn = RespawnOnPanic {
        i,
        job_rx: Arc::clone(&job_rx),
        msg_tx: msg_tx.clone(),
        session: Arc::clone(&session),
        ctx: ctx.clone(),
        page: Arc::clone(&page),
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
                // Before the fetch and before the guard: a picture the automatic policy
                // queued on a page that is no longer current is answered, not fetched. A
                // picture somebody asked for is fetched whatever page it belongs to. See the
                // module doc for why those differ.
                // A kept icon joins the automatic ones: nobody asked for it either, and a
                // read for a page the reader has left is a texture uploaded for a document that
                // is not on screen. `Msg::ImageDropped` is the right answer for both — nothing
                // was requested and nothing was disclosed, which is true of a disk read twice
                // over.
                if let (Some(page), Some(src)) = (job.discardable_page(), job.discardable_src())
                    && page.raw() != respawn.page.load(Ordering::Relaxed)
                {
                    let _ = respawn.msg_tx.send(Msg::ImageDropped { page, src });
                    respawn.ctx.request_repaint();
                    continue;
                }
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
    /// The current page, shared with every worker and with [`Net`]. Read before an image job
    /// is dispatched; see the module doc.
    page: Arc<AtomicU64>,
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
            Arc::clone(&self.page),
        );
    }
}

pub struct Net {
    jobs: mpsc::Sender<Job>,
    msgs: mpsc::Receiver<Msg>,
    session: Arc<Session>,
    /// The navigation the reader is on, published to the pool by [`Net::mint_page`].
    page: Arc<AtomicU64>,
}

impl Net {
    pub fn new(ctx: egui::Context) -> Result<Self, LoadError> {
        let session = Arc::new(Session::new()?);
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (msg_tx, msgs) = mpsc::channel::<Msg>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let page = Arc::new(AtomicU64::new(0));

        for i in 0..WORKERS {
            spawn_worker(
                i,
                Arc::clone(&job_rx),
                msg_tx.clone(),
                Arc::clone(&session),
                ctx.clone(),
                Arc::clone(&page),
            )?;
        }
        Ok(Self {
            jobs,
            msgs,
            session,
            page,
        })
    }

    /// A fresh id for a subresource of the page being read.
    pub fn mint(&self) -> ReqId {
        ReqId::next()
    }

    /// A fresh id for a *navigation*, published to the pool as it is minted.
    ///
    /// Minting and publishing are one call rather than two so that a second place that starts a
    /// navigation cannot forget the second half and leave the pool discarding every image on
    /// the page it just put up. `ReaderApp::present`, the screenshot tool's injection point, is
    /// exactly that second place.
    pub fn mint_page(&self) -> ReqId {
        let req = ReqId::next();
        self.page.store(req.raw(), Ordering::Relaxed);
        req
    }

    /// Publish a navigation id the reader already holds, without minting one.
    ///
    /// For `ReaderApp::keep_shown`, where the navigation in flight is abandoned in favour of
    /// the page still in the column. That page's automatic images are queued against *its* id,
    /// so a fresh one here would have the pool discard every one of them for a page nobody
    /// left. Its manual requests are fetched whatever the current page is, but their replies
    /// are matched against the same id at the other end; see `Ready::req`.
    pub fn resume_page(&self, req: ReqId) {
        self.page.store(req.raw(), Ordering::Relaxed);
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
            automatic: _,
            cache_as,
        } => {
            let keep = cache_as.as_deref().map(|path| Keep { path, src: &src });
            let (cookies, result, kept) =
                load_image(session, &url, &referrer, max_width, send_referer, keep);
            let _ = tx.send(Msg::Image {
                req,
                page,
                src,
                cookies,
                result,
                kept,
            });
        }
        Job::KeptIcon {
            req,
            page,
            src,
            key,
            path,
            max_width,
        } => {
            let msg = match read_kept_icon(&path, &key, max_width) {
                Some(result) => Msg::Image {
                    req,
                    page,
                    src,
                    // A file on this machine set no cookie and answered no request.
                    cookies: 0,
                    result,
                    kept: Kept::Read,
                },
                None => Msg::IconMissed { page, src },
            };
            let _ = tx.send(msg);
        }
    }
    ctx.request_repaint();
}

/// Decode the bytes kept for `src`, or `None` when this file is not them.
///
/// `None` covers the file having been deleted, truncated, or written for another address between
/// the UI consulting its index and this worker opening it — and an empty body, which records that
/// the address served nothing and is a state the UI answers without ever queuing a job. All three
/// are the same instruction to the caller: the index is wrong, go and ask properly.
fn read_kept_icon(
    path: &std::path::Path,
    src: &str,
    max_width: u32,
) -> Option<Result<Decoded, ImageError>> {
    let bytes = iconcache::read(path, src)?;
    if bytes.is_empty() {
        return None;
    }
    let _serialized = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let limits = DecodeLimits {
        max_width,
        ..DecodeLimits::default()
    };
    // The same decoder the fetch path uses, on the same untrusted bytes. A store is not a
    // source of trust: these bytes came off a disk a reader can write to, and the last program
    // to write them was a network fetch.
    Some(
        image_decode::decode(&bytes, &crate::fetch::Truncation::Complete, &limits)
            .map_err(ImageError::from),
    )
}

/// Permission to keep one fetched mark, and where.
///
/// Carried into [`load_image`] rather than applied to its result, because the bytes worth keeping
/// are the ones that were fetched and `Decoded` does not hold them: it holds pixels, already
/// scaled to sixty-four across. Keeping those instead would be a second, unaudited decode path
/// on the way back out, which `reader::image_decode`'s whole placement exists to prevent.
struct Keep<'a> {
    path: &'a std::path::Path,
    src: &'a str,
}

/// Keep what came back, and say when. A failure to write is silent: the mark still draws.
///
/// Two things are worth keeping. The obvious one is the bytes. The other is a **refusal**: an
/// address that answered and served nothing usable is the ordinary case on the reading list,
/// where the mark is `archive::well_known_icon`'s guess rather than anything a page declared,
/// and without a record of it every open of that list re-asks every one of those hosts. Recorded
/// as an empty body with its own shorter expiry.
///
/// Only a refusal that is *about the address*. A timeout, a reset, a 429 or a 5xx are about the
/// attempt, and writing those down would turn one bad minute into a week of a blank column —
/// which is the question `ImageError::offers_retry` already answers for the reader's own retry
/// control, asked here for the store instead.
fn keep_icon(keep: Keep<'_>, fetched: &[u8], result: &Result<Decoded, ImageError>) -> Kept {
    let at = iconcache::now();
    let wrote = match result {
        // The bytes as they arrived, never an error body: this arm is the one that decoded.
        Ok(_) => iconcache::write(keep.path, keep.src, at, fetched),
        Err(e) if !e.offers_retry() => iconcache::write(keep.path, keep.src, at, b""),
        Err(_) => return Kept::No,
    };
    if wrote.is_ok() {
        Kept::Written(at)
    } else {
        Kept::No
    }
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
    keep: Option<Keep<'_>>,
) -> (usize, Result<Decoded, ImageError>, Kept) {
    let fetched = match session.fetch_image(url, referrer, send_referer) {
        // Nothing arrived, so there is nothing to write down — not even a refusal. See
        // [`keep_icon`] for why an attempt that failed is not an answer about the address.
        Err(e) => return (0, Err(e.into()), Kept::No),
        Ok(f) => f,
    };
    let cookies = fetched.cookie_attempts;
    let result = if !(200..300).contains(&fetched.status) {
        // Before the decoder, and before the lock it would queue behind. An error body is not a
        // picture in any format, so letting it through would spend a decode slot to arrive at
        // `UnknownFormat` — the one classification that promises a retry is worth pressing.
        Err(ImageError::Status(fetched.status))
    } else {
        let _serialized = DECODE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let limits = DecodeLimits {
            max_width,
            ..DecodeLimits::default()
        };
        image_decode::decode(&fetched.bytes, &fetched.partial, &limits).map_err(ImageError::from)
    };
    // After the decode rather than beside the fetch, so nothing is kept that could not be drawn:
    // a body that arrives with a 200 and is not an image in any format hww reads is not worth a
    // file, and a reader who opens the store later should find pictures in it.
    let kept = keep.map_or(Kept::No, |k| keep_icon(k, &fetched.bytes, &result));
    (cookies, result, kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real PNG, so the store's round trip goes through the real decoder.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
        });
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
            .expect("a PNG encodes");
        buf
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hww-net-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MARK: &str = "https://kept.example/mark.png";

    /// The worker half of the read: bytes on this machine become a texture, and anything that is
    /// not those bytes becomes a miss rather than a fetch.
    ///
    /// The miss is the half that matters. A worker that fell through to the network here would
    /// put a third-party request on the wire that `ImageStore::record_request` never counted,
    /// because the UI took this path precisely to avoid touching it — the panel would then be
    /// naming no host for a request that went out. `Msg::IconMissed` exists for that, and this is
    /// the function that has to answer `None` for it to be sent.
    #[test]
    fn a_kept_mark_decodes_and_anything_else_is_a_miss() {
        let dir = scratch("read");
        let path = dir.join("mark.icon");
        iconcache::write(&path, MARK, iconcache::now(), &png(32, 32)).unwrap();

        let decoded = read_kept_icon(&path, MARK, 64)
            .expect("the file is there")
            .expect("and it decodes");
        assert_eq!((decoded.source_width, decoded.source_height), (32, 32));

        // Written for another address: the filename is a non-cryptographic hash, so this is what
        // stops one site's mark being drawn beside another's name.
        assert!(read_kept_icon(&path, "https://other.example/mark.png", 64).is_none());
        // The refusal record. The UI answers this from its own index and never queues a job for
        // it, so reaching here means the index is stale — which is a miss like any other.
        iconcache::write(&path, MARK, iconcache::now(), b"").unwrap();
        assert!(read_kept_icon(&path, MARK, 64).is_none());
        // And gone entirely, which is the race the whole message exists for.
        std::fs::remove_file(&path).unwrap();
        assert!(read_kept_icon(&path, MARK, 64).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What may be written down, and — sharply — what may not.
    ///
    /// A refusal that is about the *address* is worth keeping: it is the ordinary answer on the
    /// reading list, where the mark is a guess, and without it every open re-asks every one of
    /// those hosts. A failure that is about the *attempt* is not: writing a timeout down would
    /// turn one bad minute into a week of blank column, and `ImageError::offers_retry` is
    /// already the program's answer to that question for the reader's own retry control.
    #[test]
    fn only_a_refusal_about_the_address_is_written_down() {
        let dir = scratch("write");
        let bytes = png(16, 16);
        let decoded = image_decode::decode(
            &bytes,
            &crate::fetch::Truncation::Complete,
            &DecodeLimits::default(),
        )
        .unwrap();

        let kept = dir.join("kept.icon");
        assert!(matches!(
            keep_icon(
                Keep {
                    path: &kept,
                    src: MARK
                },
                &bytes,
                &Ok(decoded)
            ),
            Kept::Written(_)
        ));
        assert_eq!(
            iconcache::read(&kept, MARK).as_deref(),
            Some(&bytes[..]),
            "the bytes as fetched, not the pixels"
        );

        // A 404 is about the address, and is kept as an empty body.
        let missing = dir.join("missing.icon");
        assert!(matches!(
            keep_icon(
                Keep {
                    path: &missing,
                    src: MARK
                },
                b"<html>not found</html>",
                &Err(ImageError::Status(404))
            ),
            Kept::Written(_)
        ));
        assert_eq!(
            iconcache::read(&missing, MARK).as_deref(),
            Some(&b""[..]),
            "the error body is never what is kept"
        );

        // A 503 and a timeout are about the attempt, and are not kept at all.
        for transient in [
            ImageError::Status(503),
            ImageError::Status(429),
            ImageError::Fetch(FetchError::Timeout(std::time::Duration::from_secs(30))),
        ] {
            let path = dir.join("transient.icon");
            assert_eq!(
                keep_icon(
                    Keep {
                        path: &path,
                        src: MARK
                    },
                    b"",
                    &Err(transient)
                ),
                Kept::No
            );
            assert!(!path.exists(), "nothing was written");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reader draws its retry control on this, so an arm that answers wrongly is a button
    /// that re-issues a third-party request and re-fails for as long as the page is open.
    ///
    /// Exhaustive on purpose: the match in `offers_retry` names every variant rather than
    /// falling through, so a new `FetchError` fails the build there, and this says which way
    /// the existing ones were decided.
    #[test]
    fn only_a_failure_about_the_attempt_offers_a_retry() {
        let url = Url::parse("http://cdn.example.net/a.png").unwrap();
        let refuses = [
            FetchError::SchemeDowngrade(url.clone()),
            FetchError::TooManyRedirects(vec![url]),
            FetchError::BadLocation("://".to_owned()),
            FetchError::RedirectWithoutLocation(302),
        ];
        for e in refuses {
            assert!(
                !ImageError::Fetch(e).offers_retry(),
                "hww's own policy and a server's own fault answer the same way every time"
            );
        }
        let offers = [
            FetchError::Timeout(std::time::Duration::from_secs(30)),
            FetchError::LikelyBlocked,
            FetchError::Io(std::io::Error::other("connection reset")),
        ];
        for e in offers {
            assert!(ImageError::Fetch(e).offers_retry());
        }
        // And the decode side still answers for itself.
        assert!(!ImageError::Decode(image_decode::DecodeError::Unsupported("SVG")).offers_retry());
    }

    /// A dead image URL is the commonest permanent failure there is, and before `Status` had
    /// an arm the error body reached the decoder and came back `UnknownFormat`, which offers a
    /// retry. The button re-fetched and re-failed for as long as the page was open.
    #[test]
    fn a_refused_status_offers_no_retry_and_a_busy_one_does() {
        for s in [400, 401, 403, 404, 410, 451] {
            assert!(
                !ImageError::Status(s).offers_retry(),
                "{s} is about the address, and the address is not going to change"
            );
        }
        for s in [429, 500, 502, 503] {
            assert!(
                ImageError::Status(s).offers_retry(),
                "{s} is about right now"
            );
        }
    }
}
