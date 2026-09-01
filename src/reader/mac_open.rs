//! The link macOS hands hww when a reader chooses it for one.
//!
//! `packaging/build-dmg.sh` puts `CFBundleURLTypes` in the bundle, which is what places hww in
//! Finder's **Open With** and in the default-browser list, and README tells a reader to use it.
//! It did not work. macOS does not put the address in `argv` the way a `.desktop` file's `%u`
//! does: Launch Services starts the application and then **sends it an Apple Event** —
//! `kInternetEventClass`/`kAEGetURL`, both spelled `'GURL'` — and an application that reads
//! only `argv` opens on its home screen with the link dropped and nothing said. winit has no
//! hook for it, so this is the hook.
//!
//! What it does is the smallest thing that can work: register one class method as the handler,
//! pull the direct object out of the event, and leave the string where the reader can pick it
//! up. It makes no decision about the address. [`ReaderApp`](crate::reader::ui) hands what it
//! takes to `follow_link`, so an address arriving from the desktop goes through
//! `session::classify_link` — the same door, the same refusals, the same toast — as an `href`
//! on a page. The bundle claims `http` and `https`, but an Apple Event carries any string at all,
//! and this module is the one part of the path a test on Linux never sees: the less it decides,
//! the less rides on that.
//!
//! # Why a class and not an object
//!
//! `setEventHandler:andSelector:forEventClass:andEventID:` takes a target and a selector and
//! does **not** retain the target, so an object would have to be leaked on purpose to outlive
//! the call. A class *is* an object, it is registered for the life of the process already, and
//! `add_class_method` puts the selector on it — so there is nothing to allocate, nothing to
//! retain, and nothing to keep alive by hand.
//!
//! # Two calls, deliberately
//!
//! [`install`] runs before `eframe::run_native`, because the launch event is dispatched as soon
//! as the run loop turns and a handler registered after that is a handler registered too late.
//! [`wake_with`] runs when the window exists, and covers the other arrival: a link chosen while
//! hww is already open is not a window event, so the event loop has no reason to draw a frame
//! and the address would sit here until the reader happened to move the mouse.

use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, NSObject, Sel};
use objc2::{ClassType, class, msg_send, sel};
use std::ffi::{CStr, c_char};
use std::sync::{Mutex, Once};

/// `'GURL'`, which is `kInternetEventClass` and `kAEGetURL` both: Apple spells the class and the
/// id of this event with the same four characters.
const GURL: u32 = u32::from_be_bytes(*b"GURL");

/// `'----'`, `keyDirectObject`: the parameter a `GURL` event carries its address in.
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

/// The address last handed over, until the reader takes it.
///
/// One slot and not a queue. A second link arriving before the first is read replaces it, which
/// is the same thing a second link would do a frame later anyway: the reader has one window and
/// goes to the page it was last sent to.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// What to call after an address lands, so a window that is drawing nothing draws a frame.
///
/// A boxed closure rather than an `egui::Context`, for the reason `reader::desktop` takes one:
/// this module compiles in a build with no egui in it.
type Wake = Box<dyn Fn() + Send>;
static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

/// The address the desktop last sent, if there is one and it has not been read yet.
pub fn take() -> Option<String> {
    PENDING.lock().ok().and_then(|mut slot| slot.take())
}

/// Register the handler. Idempotent, and safe to call before there is a window.
///
/// Silent on every failure. A reader who chose hww for a link and got the home screen is no
/// worse off than they were before this module existed, and there is nowhere to report it to:
/// this runs before the window, and `src/bin/hww.rs` is a GUI-subsystem binary whose stdout
/// reaches nobody on the platform that inherited that rule.
pub fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Some(mut builder) = ClassBuilder::new(c"HwwGetUrlHandler", NSObject::class()) else {
            return;
        };
        // SAFETY: the signature matches what Apple Events invokes the selector with — the class
        // itself as the receiver, the event, and the reply event, all object pointers, and no
        // return value.
        unsafe {
            builder.add_class_method(
                sel!(handleGetURLEvent:withReplyEvent:),
                handle_get_url as extern "C" fn(_, _, _, _),
            );
        }
        let class: &'static AnyClass = builder.register();
        let handler: &AnyObject = unsafe { &*(class as *const AnyClass).cast::<AnyObject>() };
        // SAFETY: `sharedAppleEventManager` is a singleton with no arguments, and the four
        // arguments below are the types its selector declares: an object, a selector, and two
        // `FourCharCode`s.
        unsafe {
            let manager: *mut AnyObject =
                msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
            if manager.is_null() {
                return;
            }
            let _: () = msg_send![
                manager,
                setEventHandler: handler,
                andSelector: sel!(handleGetURLEvent:withReplyEvent:),
                forEventClass: GURL,
                andEventID: GURL,
            ];
        }
    });
}

/// Say how to wake the window once there is one. See the module doc for why this is separate
/// from [`install`].
pub fn wake_with(wake: impl Fn() + Send + 'static) {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = Some(Box::new(wake));
    }
}

/// `-[HwwGetUrlHandler handleGetURLEvent:withReplyEvent:]`, called on the main thread by the
/// Apple Event manager.
///
/// Every pointer is checked. Messaging `nil` is legal in Objective-C and returns zero, so a
/// missing parameter would otherwise walk one selector further and read a string out of it.
extern "C" fn handle_get_url(
    _class: &AnyClass,
    _cmd: Sel,
    event: *mut AnyObject,
    _reply: *mut AnyObject,
) {
    if event.is_null() {
        return;
    }
    // SAFETY: `event` is the `NSAppleEventDescriptor` the manager passed, each message is one
    // its class declares, and each reply is checked for `nil` before it is used as a receiver.
    let address = unsafe {
        let direct: *mut AnyObject = msg_send![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT];
        if direct.is_null() {
            return;
        }
        let text: *mut AnyObject = msg_send![direct, stringValue];
        if text.is_null() {
            return;
        }
        let utf8: *const c_char = msg_send![text, UTF8String];
        if utf8.is_null() {
            return;
        }
        CStr::from_ptr(utf8).to_string_lossy().into_owned()
    };
    if let Ok(mut slot) = PENDING.lock() {
        *slot = Some(address);
    }
    if let Ok(wake) = WAKE.lock()
        && let Some(wake) = wake.as_ref()
    {
        wake();
    }
}
