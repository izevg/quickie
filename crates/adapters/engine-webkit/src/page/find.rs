//! Find in page. `WKWebView`'s own find (macOS 13) selects the next match and scrolls to it,
//! but tells only whether there was one; a script in an isolated content world then counts
//! the matches, marks them with CSS highlights, and says which one is selected
//! ([`find.js`](find.js)).

use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, Message};
use objc2_foundation::{NSDictionary, NSError, NSNumber, NSString};
use objc2_web_kit::{WKContentWorld, WKFindConfiguration, WKFindResult, WKWebView};
use quickie_application::FindStep;
use quickie_domain_browsing::FindMatches;

/// What hears the matches of a find.
type Done = Rc<dyn Fn(FindMatches)>;

/// The counting script: the body of an async function of `query` and `found`.
const SCRIPT: &str = include_str!("find.js");

/// The content world Quickie's scripts run in, apart from the page's own.
const WORLD: &str = "quickie";

/// Selects a match of `query` in `view` as `step` says, wrapping at the ends, then calls `done`
/// with the matches. An empty query clears the marks and finds nothing.
pub(crate) fn find(
    view: &WKWebView,
    query: &str,
    step: FindStep,
    done: impl Fn(FindMatches) + 'static,
) {
    let done: Done = Rc::new(done);
    if query.is_empty() {
        count(view, query, false, done);
        return;
    }
    if step != FindStep::Refine {
        select(view, query, step == FindStep::Previous, done);
        return;
    }
    let weak = Weak::from_retained(&view.retain());
    let query_text = query.to_owned();
    let block = RcBlock::new(move |_value: *mut AnyObject, _error: *mut NSError| {
        if let Some(view) = weak.load() {
            select(&view, &query_text, false, Rc::clone(&done));
        }
    });
    // SAFETY: a plain script in Quickie's world, the main frame, and WebKit calls the block
    // once on the main thread. Its value, `0`, is ignored.
    unsafe {
        view.evaluateJavaScript_inFrame_inContentWorld_completionHandler(
            &NSString::from_str("getSelection()?.collapseToStart(); 0"),
            None,
            &world(view.mtm()),
            Some(&block),
        );
    }
}

/// `WebKit`'s own find: selects the match after the selection, before it when `backwards`.
fn select(view: &WKWebView, query: &str, backwards: bool, done: Done) {
    let mtm = view.mtm();
    // SAFETY: a fresh configuration has no preconditions.
    let configuration = unsafe { WKFindConfiguration::new(mtm) };
    // SAFETY: plain boolean setters.
    unsafe {
        configuration.setBackwards(backwards);
    }
    // SAFETY: as above.
    unsafe { configuration.setWraps(true) };
    // SAFETY: as above.
    unsafe { configuration.setCaseSensitive(false) };
    let weak = Weak::from_retained(&view.retain());
    let query_text = query.to_owned();
    let block = RcBlock::new(move |result: std::ptr::NonNull<WKFindResult>| {
        // SAFETY: WebKit passes a valid result that lives for the call.
        let result = unsafe { result.as_ref() };
        // SAFETY: a getter with no preconditions.
        let found = unsafe { result.matchFound() };
        if let Some(view) = weak.load() {
            count(&view, &query_text, found, Rc::clone(&done));
        }
    });
    // SAFETY: the configuration is fully set up, and WebKit calls the block once on the main
    // thread, where the view and what the block captures live.
    unsafe {
        view.findString_withConfiguration_completionHandler(
            &NSString::from_str(query),
            Some(&configuration),
            &block,
        );
    }
}

/// Runs the counting script; `found` says whether `WebKit`'s find selected a match.
fn count(view: &WKWebView, query: &str, found: bool, done: Done) {
    let mtm = view.mtm();
    let query_value = NSString::from_str(query);
    let found_value = NSNumber::new_bool(found);
    let values: [&AnyObject; 2] = [&query_value, &found_value];
    let arguments: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[&*NSString::from_str("query"), &*NSString::from_str("found")],
        &values,
    );
    let block = RcBlock::new(move |value: *mut AnyObject, _error: *mut NSError| {
        // SAFETY: WebKit passes the script's result or null, valid for the call.
        let value = unsafe { value.as_ref() };
        let text = value
            .and_then(AnyObject::downcast_ref::<NSString>)
            .map(NSString::to_string);
        done(text.as_deref().map_or(FindMatches::NONE, matches));
    });
    // SAFETY: the script is a function body using only its two arguments, the main frame is
    // `None`, and WebKit calls the block once on the main thread.
    unsafe {
        view.callAsyncJavaScript_arguments_inFrame_inContentWorld_completionHandler(
            &NSString::from_str(SCRIPT),
            Some(&arguments),
            None,
            &world(mtm),
            Some(&block),
        );
    }
}

fn world(mtm: MainThreadMarker) -> Retained<WKContentWorld> {
    // SAFETY: a named world is created on first use and shared by later calls.
    unsafe { WKContentWorld::worldWithName(&NSString::from_str(WORLD), mtm) }
}

/// The script's answer, `current/total`; anything else is no match.
fn matches(answer: &str) -> FindMatches {
    let parsed = answer
        .split_once('/')
        .and_then(|(current, total)| Some((current.parse().ok()?, total.parse().ok()?)));
    parsed.map_or(FindMatches::NONE, |(current, total)| {
        FindMatches::new(Some(current), total)
    })
}

#[cfg(test)]
mod tests {
    use quickie_domain_browsing::FindMatches;

    use super::matches;

    #[test]
    fn the_answer_is_current_of_total() {
        assert_eq!(matches("3/12"), FindMatches::new(Some(3), 12));
        assert_eq!(matches("0/12"), FindMatches::new(None, 12));
        assert_eq!(matches("0/0"), FindMatches::NONE);
    }

    #[test]
    fn a_garbled_answer_is_no_match() {
        for answer in ["", "3", "a/b", "3/", "-1/2"] {
            assert_eq!(matches(answer), FindMatches::NONE, "{answer}");
        }
    }
}
