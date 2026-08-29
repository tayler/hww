//! Site-icon bytes on disk, for the addresses `archive.json` already names.
//!
//! # Why this is allowed at all
//!
//! `reader::archive` holds the rule and this module holds its one exception, so read that
//! doctrine first. The short form: the rule separates bytes that reveal **what you read** from
//! bytes that reveal **which sites you already wrote down**. Page content and article images are
//! the first kind and stay off disk. A site icon for an address the archive already names is the
//! second — the address is on disk already, written by the same feature, one file over — so
//! keeping its bytes adds no local disclosure, and it removes a repeated contact with a third
//! party, which is a privacy cost the other way. That trade is lopsided and this module is the
//! whole of it.
//!
//! Three consequences, each enforced here rather than trusted to a call site:
//!
//! **Only what a marked list still names.** Never the history. The history draws no column and
//! stores no icon, and a cached mark for a visited-but-unbookmarked host would outlive *forget
//! history* — a second trail, with no switch over it. [`Cache::open`] is handed the set of
//! addresses the marked lists name and deletes everything else it finds, so an entry for an
//! unmarked host cannot survive a launch even if some future call site writes one.
//!
//! **Reading is not writing.** Any icon may be drawn *from* this store, on any page and under
//! every [`ImagePolicy`], because a read contacts nobody and discloses nothing that is not
//! already in `archive.json`. Only a mark drawn for a row of a marked list may be written *to*
//! it. That asymmetry is what keeps the masthead favicon — which fires on every page hww shows,
//! marked or not — from turning this directory into a list of everywhere you have been.
//!
//! **A bad entry is deleted, not complained about.** The exact opposite of `archive::load`, and
//! the contrast is the argument: the archive holds what the reader cannot retype, so a bad parse
//! gets a complaint and a write seal; everything here can be fetched again, so a bad file is just
//! gone.
//!
//! [`ImagePolicy`]: crate::reader::opts::ImagePolicy
//!
//! # Shape on disk
//!
//! ```text
//! <config_dir>/icons/<16 hex>.icon
//!     hww-icon 1\n
//!     <the icon address>\n
//!     <the second it was fetched>\n
//!     <how many bytes follow>\n
//!     <the bytes exactly as fetched, or nothing at all>
//! ```
//!
//! **The name is a hash and never the address.** An icon address is page-chosen text, and
//! page-chosen text that becomes a path component is a `../` away from being a directory
//! traversal. [`key`] is an FNV-1a of the address and nothing else reaches the filesystem.
//!
//! **The file names the address it is for, and a read checks it.** A non-cryptographic hash is
//! the right size for a filename and the wrong thing to trust, so a collision is caught by the
//! header rather than drawn as another site's mark in a column whose whole job is identity. It is
//! also what [`Cache::open`] reads to decide what is still named and what is an orphan, which is
//! why there is no index file: the directory *is* the index.
//!
//! **The bytes are as fetched, not decoded pixels.** Smaller, and a read goes back through
//! `reader::image_decode`, the audited untrusted-input path, instead of inventing a second one.
//!
//! **An empty body is a real answer.** A site that serves no icon at the address hww asked for is
//! the common case on the reading list, where the address is `archive::well_known_icon`'s guess
//! rather than anything a page declared — and without a record of the refusal, every open of that
//! list re-asks every one of those hosts, which is the contact this module exists to remove. It
//! expires sooner than a hit ([`MISS_TTL`] against [`TTL`]): a site that has since added an icon
//! should not go unmarked for a month.
//!
//! # What is not here
//!
//! No `egui`, no network, and no `settings::config_dir` — the directory arrives as a parameter.
//! The first two are why the default CI job tests this; the third is why its tests run in
//! parallel instead of serializing on a process-wide environment variable the way `settings`'s
//! have to.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The first line of every file, so a stray file in the directory is recognised as not one of
/// ours and deleted rather than parsed.
const MAGIC: &str = "hww-icon 1";

/// The extension every entry carries. Anything else in the directory — a `.tmp` a crash left
/// behind, something a reader dropped there — is not read and not deleted.
const EXT: &str = "icon";

/// How long a stored icon stands before it is fetched again.
///
/// A favicon changes at the rate a site rebrands, which is years, and the cost of being a month
/// late to one is a slightly stale 16-pixel square. The cost of a short TTL is the thing this
/// module exists to remove, paid over again every time it expires.
pub const TTL: u64 = 30 * 24 * 60 * 60;

/// How long "this address served no icon" stands.
///
/// Shorter than [`TTL`] on purpose, and the asymmetry is the point: a hit that goes stale costs
/// one refetch, while a *miss* that goes stale costs a mark the reader can see is missing. A site
/// that adds a favicon should get its column back in a week rather than in a month.
pub const MISS_TTL: u64 = 7 * 24 * 60 * 60;

/// The largest body worth keeping.
///
/// `fetch::MAX_IMAGE_BYTES` is ten megabytes because it is an *article picture* ceiling, settled
/// by a click. Nothing here was clicked, every entry is decoded to a 64-pixel square, and a
/// favicon this size is a bug or an attack. Refused at the write, so the fetch and the draw both
/// still work and only the keeping does not.
pub const MAX_BYTES: usize = 256 * 1024;

/// How many entries the directory may hold.
///
/// Only addresses a marked list names are ever written, so `archive::MAX_ITEMS` plus
/// `archive::MAX_NEXT` already bounds this at 2 500 and [`Cache::open`] sweeps the rest away
/// every launch. This is the belt on those braces: a directory that somehow grew past it stops
/// gaining entries rather than growing without limit, and the next launch cuts it back.
pub const MAX_ENTRIES: usize = 4_000;

/// Seconds since the epoch, or zero on a machine whose clock predates it.
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the store knows about one icon address.
///
/// Three answers rather than two, because "this host serves no icon" is a different instruction
/// to the caller than "hww has never asked": one says draw nothing and contact nobody, the other
/// says go and find out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Known {
    /// The bytes are at this path. Read them; contact nobody.
    Kept(PathBuf),
    /// This address was asked and served no usable icon. Draw nothing; contact nobody.
    Nothing,
    /// Nothing is known, or what was known has expired.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// The second the bytes were fetched, from the file's own header rather than its mtime: a
    /// restored backup or an unpacked archive carries whatever mtime the tool felt like, and a
    /// decade-old icon presented as fresh is the one failure a TTL exists to prevent.
    at: u64,
    /// The address answered, and served nothing usable. See [`MISS_TTL`].
    empty: bool,
}

impl Entry {
    fn fresh(&self, now: u64) -> bool {
        let ttl = if self.empty { MISS_TTL } else { TTL };
        // Saturating, so a clock that has gone backwards since the write reads as fresh rather
        // than as impossibly old. The alternative is a machine whose clock is wrong refetching
        // every mark on every launch.
        now.saturating_sub(self.at) < ttl
    }
}

/// The icon addresses kept on disk, and where each one's bytes are.
///
/// Built once at launch and kept in memory, because the question it answers has to be answered
/// *before* anything is recorded: `ImageStore::record_request` moves the host tally and the
/// Referer count ahead of the request leaving, so a hit that reached it would have page info
/// naming hosts nobody contacted. A synchronous answer is what lets the caller take the other
/// path first. The file reading itself still happens on a worker.
#[derive(Debug, Default)]
pub struct Cache {
    /// `None` when there is no configuration directory at all, which is the same condition
    /// `archive::save` errors on. Everything here then answers [`Known::Unknown`] and writes
    /// nothing, so hww runs exactly as it did before this module existed.
    dir: Option<PathBuf>,
    index: HashMap<String, Entry>,
}

impl Cache {
    /// Read the directory, drop what no marked row still names, and index the rest.
    ///
    /// The sweep is here rather than only at the three call sites that forget things, because
    /// those cannot cover every way an entry becomes an orphan: re-bookmarking a page whose
    /// `<link rel=icon>` changed leaves the old one behind, a failed write leaves a forget
    /// half-done, and a hand-edited `archive.json` orphans whatever it likes. One pass over a
    /// directory bounded by [`MAX_ENTRIES`] at launch is what makes those all the same case.
    pub fn open(dir: Option<PathBuf>, keep: &HashSet<String>) -> Self {
        let mut cache = Cache {
            dir,
            index: HashMap::new(),
        };
        let Some(dir) = cache.dir.clone() else {
            return cache;
        };
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // No directory yet is the ordinary first-run case and not a complaint.
            return cache;
        };
        let now = now();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
                continue;
            }
            match header(&path) {
                // Named, fresh, and where its own address says it should be. The last of those
                // three is what stops a file copied to another name being served for the address
                // it was copied over.
                Some((url, at, empty))
                    if keep.contains(&url)
                        && Entry { at, empty }.fresh(now)
                        && path.file_name().and_then(|n| n.to_str())
                            == Some(&format!("{}.{EXT}", key(&url))) =>
                {
                    cache.index.insert(url, Entry { at, empty });
                }
                // Unreadable, expired, or for an address nothing names any more. Deleted without
                // a word: see the module doc for why this is the opposite of `archive::load`.
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        cache
    }

    /// Where the store lives, for the settings panel to print. `None` when there is nowhere.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// How many icons are kept, for a test and for a caller that wants to say so.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// What is known about one icon address, without touching the disk.
    pub fn known(&self, url: &str) -> Known {
        let Some(dir) = &self.dir else {
            return Known::Unknown;
        };
        match self.index.get(url) {
            Some(e) if !e.fresh(now()) => Known::Unknown,
            Some(e) if e.empty => Known::Nothing,
            Some(_) => Known::Kept(dir.join(format!("{}.{EXT}", key(url)))),
            None => Known::Unknown,
        }
    }

    /// Where a fetch of `url` may put its bytes, or `None` if it may not keep them.
    ///
    /// Answered here rather than at the fetch, so "is there anywhere to write" and "is the store
    /// full" are one question asked in one place. The caller carries the answer to the worker; a
    /// worker never decides for itself whether an icon may be kept, because it cannot see the
    /// archive and so cannot know whether the address is one a marked row names.
    pub fn place_for(&self, url: &str) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        if !self.index.contains_key(url) && self.index.len() >= MAX_ENTRIES {
            return None;
        }
        Some(dir.join(format!("{}.{EXT}", key(url))))
    }

    /// Record a write a worker has already made, so the rest of the session can see it.
    ///
    /// Without this the index would only ever describe the directory as it was at launch, and
    /// the second open of a list in one session would refetch every mark the first open had just
    /// written — which is the case this whole module exists for.
    pub fn note(&mut self, url: &str, at: u64, empty: bool) {
        if self.dir.is_some() {
            self.index.insert(url.to_owned(), Entry { at, empty });
        }
    }

    /// Drop one entry and its file: what a worker found unreadable, or what a read raced.
    pub fn forget(&mut self, url: &str) {
        if self.index.remove(url).is_some()
            && let Some(dir) = &self.dir
        {
            let _ = std::fs::remove_file(dir.join(format!("{}.{EXT}", key(url))));
        }
    }

    /// Drop every entry no address in `keep` names. Returns how many went.
    ///
    /// The index rather than the directory, because this runs on a keypress — un-bookmarking a
    /// page — and a reader who presses `Ctrl+D` should not pay for a stat of every file in the
    /// store. [`Cache::open`]'s sweep is the one that reads the directory, and it is what makes
    /// anything this misses a launch-shaped problem rather than a permanent one.
    pub fn prune(&mut self, keep: &HashSet<String>) -> usize {
        let gone: Vec<String> = self
            .index
            .keys()
            .filter(|u| !keep.contains(*u))
            .cloned()
            .collect();
        for url in &gone {
            self.forget(url);
        }
        gone.len()
    }

    /// Delete every entry and the directory itself: the reader emptied both marked lists.
    ///
    /// `prune` with an empty set does the same to the index; this also takes the directory, so
    /// "forget everything" leaves nothing behind for a reader to find and wonder about.
    pub fn forget_all(&mut self) -> usize {
        let n = self.index.len();
        self.index.clear();
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        n
    }
}

/// The bytes kept for `url` at `path`, or `None` if this file is not them.
///
/// The address check is the whole reason the header exists: [`key`] is not a cryptographic hash,
/// so a collision — or a file copied over another — must end as a miss rather than as one site's
/// mark drawn beside another site's name, in a column whose only job is saying which site a row
/// leads to.
///
/// An empty `Some` is the negative entry and a real answer; see the module doc.
pub fn read(path: &Path, url: &str) -> Option<Vec<u8>> {
    // Asked before the read, not after: this file is on a disk a reader can write to, and
    // `read` on a path someone has pointed at a huge file should not become the allocation.
    let len = std::fs::metadata(path).ok()?.len();
    if len > (MAX_BYTES + 4 * 1024) as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let (head, body) = split_header(&bytes)?;
    (head.0 == url).then(|| body.to_vec())
}

/// Put `bytes` at `path` as the kept icon for `url`, stamped `at`.
///
/// Through `settings::write_atomically`, which is what `archive::save` uses, so a crash or a
/// second hww window mid-write leaves either the old file or the new one and never half of
/// either. A truncated file would be handled — [`read`] would refuse it and [`Cache::open`]
/// would delete it — but not having to is cheaper than handling it well.
pub fn write(path: &Path, url: &str, at: u64, bytes: &[u8]) -> std::io::Result<()> {
    if bytes.len() > MAX_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{} bytes is past the icon cache ceiling", bytes.len()),
        ));
    }
    // An address with a newline in it would make the header ambiguous. `Url::to_string` never
    // produces one — a newline percent-encodes — but this file is only safe to parse if that is
    // checked here rather than assumed from a type two modules away.
    if url.contains('\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "an icon address cannot contain a newline",
        ));
    }
    let mut out = format!("{MAGIC}\n{url}\n{at}\n{}\n", bytes.len()).into_bytes();
    out.extend_from_slice(bytes);
    crate::reader::settings::write_bytes_atomically(path, &out)
}

/// The header of the file at `path`: its address, when it was fetched, and whether it is empty.
fn header(path: &Path) -> Option<(String, u64, bool)> {
    let len = std::fs::metadata(path).ok()?.len();
    if len > (MAX_BYTES + 4 * 1024) as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let ((url, at), body) = split_header(&bytes)?;
    Some((url.to_owned(), at, body.is_empty()))
}

/// Split a file into `(address, fetched-at)` and the bytes, or `None` if it is not one of ours.
///
/// The declared length is checked against what actually follows. A file whose header promises
/// more than it carries is a torn write, and serving its first half as a picture would hand
/// truncated bytes to the decoder for no reason — `image_decode` would survive it, and a store
/// that knowingly feeds it garbage is still a store doing the wrong thing.
fn split_header(bytes: &[u8]) -> Option<((&str, u64), &[u8])> {
    fn take<'a>(rest: &mut &'a [u8]) -> Option<&'a str> {
        let at = rest.iter().position(|b| *b == b'\n')?;
        let (head, tail) = rest.split_at(at);
        *rest = &tail[1..];
        std::str::from_utf8(head).ok()
    }
    let mut rest = bytes;
    if take(&mut rest)? != MAGIC {
        return None;
    }
    let url = take(&mut rest)?;
    let at: u64 = take(&mut rest)?.parse().ok()?;
    let len: usize = take(&mut rest)?.parse().ok()?;
    (rest.len() == len).then_some(((url, at), rest))
}

/// The filename an icon address gets: sixteen hex digits of FNV-1a and nothing else.
///
/// FNV rather than a hash from a crate, because what this needs is a short, stable, portable
/// name for a string, and the only property it must not have is being derived from the address's
/// own characters — a page-chosen path component is a traversal waiting to happen. Collision
/// resistance is not asked of it: [`read`] checks the address in the header, so a collision costs
/// a refetch. Adding a cryptographic hash crate to buy what a four-line function and one
/// comparison already provide is five dependencies for nothing.
fn key(url: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in url.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this process's own, without an environment variable to serialize on.
    ///
    /// `settings`'s tests have to take a lock because `HWW_CONFIG_DIR` is process-wide; nothing
    /// here reads it, which is the whole reason [`Cache::open`] takes the directory as an
    /// argument. These run in parallel.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hww-iconcache-{}-{name}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn keep(urls: &[&str]) -> HashSet<String> {
        urls.iter().map(|u| (*u).to_owned()).collect()
    }

    const A: &str = "https://example.org/favicon.ico";
    const B: &str = "https://other.example/icon.png";

    #[test]
    fn bytes_written_come_back_and_an_unknown_address_does_not() {
        let dir = scratch("roundtrip");
        let mut c = Cache::open(Some(dir.clone()), &keep(&[A]));
        let at = c.place_for(A).expect("a directory means a place");
        write(&at, A, now(), b"PNGish").unwrap();
        c.note(A, now(), false);

        let Known::Kept(path) = c.known(A) else {
            panic!("the address just written is kept");
        };
        assert_eq!(read(&path, A).as_deref(), Some(&b"PNGish"[..]));
        assert_eq!(c.known(B), Known::Unknown);

        // And across a launch, which is the case the feature is for.
        let reopened = Cache::open(Some(dir.clone()), &keep(&[A]));
        assert!(matches!(reopened.known(A), Known::Kept(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The reading list's mark is a guess, so most of what it asks for 404s. Without this the
    /// refusal is re-asked of every one of those hosts on every open, which is the third-party
    /// contact this module exists to remove.
    #[test]
    fn an_address_that_served_nothing_is_remembered_as_nothing() {
        let dir = scratch("negative");
        let mut c = Cache::open(Some(dir.clone()), &keep(&[A]));
        let at = c.place_for(A).unwrap();
        write(&at, A, now(), b"").unwrap();
        c.note(A, now(), true);
        assert_eq!(c.known(A), Known::Nothing);
        assert_eq!(
            Cache::open(Some(dir.clone()), &keep(&[A])).known(A),
            Known::Nothing
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_entry_is_a_miss_and_a_stale_refusal_expires_sooner() {
        let dir = scratch("stale");
        let mut c = Cache::open(Some(dir.clone()), &keep(&[A, B]));
        let long_ago = now() - (TTL + 10);
        let a_while = now() - (MISS_TTL + 10);
        write(&c.place_for(A).unwrap(), A, long_ago, b"old").unwrap();
        c.note(A, long_ago, false);
        write(&c.place_for(B).unwrap(), B, a_while, b"").unwrap();
        c.note(B, a_while, true);

        assert_eq!(c.known(A), Known::Unknown, "past the TTL");
        assert_eq!(c.known(B), Known::Unknown, "past the shorter one");
        // A hit is still fresh at the age a refusal has expired at.
        let mut d = Cache::open(Some(dir.clone()), &keep(&[A]));
        d.note(A, a_while, false);
        assert!(matches!(d.known(A), Known::Kept(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_or_truncated_file_is_a_miss_and_is_deleted() {
        let dir = scratch("corrupt");
        let junk = dir.join(format!("{}.{EXT}", key(A)));
        std::fs::write(&junk, b"not an icon file at all").unwrap();
        assert_eq!(read(&junk, A), None);

        // A header promising more than it carries.
        let torn = dir.join(format!("{}.{EXT}", key(B)));
        std::fs::write(&torn, format!("{MAGIC}\n{B}\n{}\n99\nshort", now())).unwrap();
        assert_eq!(read(&torn, B), None);

        let c = Cache::open(Some(dir.clone()), &keep(&[A, B]));
        assert!(c.is_empty(), "neither was indexed");
        assert!(
            !junk.exists() && !torn.exists(),
            "and neither was left behind"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The hash is not cryptographic, so the header is what stops one site's mark being drawn
    /// beside another site's name in a column whose only job is identity.
    #[test]
    fn a_file_is_never_served_for_an_address_it_does_not_name() {
        let dir = scratch("mismatch");
        let path = dir.join(format!("{}.{EXT}", key(A)));
        write(&path, B, now(), b"B's bytes").unwrap();
        assert_eq!(read(&path, A), None, "the header names another address");
        assert_eq!(read(&path, B).as_deref(), Some(&b"B's bytes"[..]));
        // And `open` refuses it too: it is not at the name its own address hashes to.
        assert!(Cache::open(Some(dir.clone()), &keep(&[A, B])).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A page chooses the icon address, so it chooses the input to the filename. Nothing it can
    /// write may become a path component.
    #[test]
    fn a_page_chosen_address_never_becomes_a_path() {
        let dir = scratch("traversal");
        let nasty = [
            "https://example.org/../../../etc/passwd",
            "https://example.org/a%00b",
            "https://example.org/\u{202e}drawn-backwards",
            "https://example.org/CON",
        ];
        let c = Cache::open(Some(dir.clone()), &HashSet::new());
        for url in nasty {
            let path = c.place_for(url).unwrap();
            assert_eq!(
                path.parent(),
                Some(dir.as_path()),
                "{url} escaped the store"
            );
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(name.len(), 16 + 1 + EXT.len());
            assert!(
                name.trim_end_matches(&format!(".{EXT}"))
                    .chars()
                    .all(|c| c.is_ascii_hexdigit()),
                "{name} is not sixteen hex digits"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Both lists at once: an icon can serve a bookmark and a reading-list row, so what survives
    /// is decided by "no marked row still names this" and never per tenant.
    #[test]
    fn pruning_keeps_what_a_row_still_names_and_drops_the_rest() {
        let dir = scratch("prune");
        let mut c = Cache::open(Some(dir.clone()), &keep(&[A, B]));
        for url in [A, B] {
            write(&c.place_for(url).unwrap(), url, now(), b"x").unwrap();
            c.note(url, now(), false);
        }
        assert_eq!(c.prune(&keep(&[A])), 1);
        assert!(matches!(c.known(A), Known::Kept(_)));
        assert_eq!(c.known(B), Known::Unknown);
        assert!(!dir.join(format!("{}.{EXT}", key(B))).exists());

        // And an orphan the call sites missed goes at the next launch.
        let swept = Cache::open(Some(dir.clone()), &HashSet::new());
        assert!(swept.is_empty());
        assert!(!dir.join(format!("{}.{EXT}", key(A))).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn forget_all_takes_the_directory_with_it() {
        let dir = scratch("forget-all");
        let mut c = Cache::open(Some(dir.clone()), &keep(&[A]));
        write(&c.place_for(A).unwrap(), A, now(), b"x").unwrap();
        c.note(A, now(), false);
        assert_eq!(c.forget_all(), 1);
        assert!(!dir.exists());
        assert_eq!(c.known(A), Known::Unknown);
    }

    #[test]
    fn an_oversized_body_is_refused_and_the_count_is_bounded() {
        let dir = scratch("caps");
        let c = Cache::open(Some(dir.clone()), &keep(&[A]));
        let path = c.place_for(A).unwrap();
        assert!(write(&path, A, now(), &vec![0u8; MAX_BYTES + 1]).is_err());
        assert!(!path.exists(), "and nothing was left behind");
        assert!(write(&path, A, now(), &vec![0u8; MAX_BYTES]).is_ok());

        let mut full = Cache::open(Some(dir.clone()), &HashSet::new());
        for i in 0..MAX_ENTRIES {
            full.note(&format!("https://example.org/{i}.ico"), now(), false);
        }
        assert!(full.place_for("https://example.org/one-more.ico").is_none());
        // An address already kept may still be refreshed when the store is full.
        assert!(full.place_for("https://example.org/0.ico").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no configuration directory there is nowhere to write, which is the condition
    /// `archive::save` errors on. Here it is silent: every answer is `Unknown`, nothing is
    /// written, and hww behaves as it did before this module existed.
    #[test]
    fn no_directory_means_no_store_and_no_complaint() {
        let mut c = Cache::open(None, &keep(&[A]));
        assert_eq!(c.known(A), Known::Unknown);
        assert_eq!(c.place_for(A), None);
        c.note(A, now(), false);
        assert!(c.is_empty(), "there is nowhere for a note to describe");
        assert_eq!(c.dir(), None);
    }
}
