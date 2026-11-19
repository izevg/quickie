//! Delivery of what the C++ side reports (engine events, key equivalents, wakes) to the handlers
//! the engine installed.
//!
//! Everything runs on the main thread (Chromium's UI thread), so the handlers live in a
//! thread-local. A handler may call back into the engine, and the engine may report an event
//! synchronously from that call; such an event is queued and delivered once the running handler
//! returns, so handlers never run re-entrantly.

use std::cell::RefCell;
use std::collections::VecDeque;

use super::{Event, KeyEquivalent};

pub(crate) type EventHandler = Box<dyn FnMut(Event)>;
pub(crate) type KeyHandler = Box<dyn FnMut(KeyEquivalent) -> bool>;
pub(crate) type WakeHandler = Box<dyn FnMut()>;

#[derive(Default)]
struct Handlers {
    events: Option<EventHandler>,
    keys: Option<KeyHandler>,
    wake: Option<WakeHandler>,
    queue: VecDeque<Event>,
    delivering: bool,
    /// Bumped by every install and uninstall, so a running handler is not put back over them.
    generation: u64,
}

thread_local! {
    static HANDLERS: RefCell<Handlers> = RefCell::default();
}

/// Installs the handlers, replacing earlier ones.
pub(crate) fn install(events: EventHandler, keys: KeyHandler, wake: WakeHandler) {
    HANDLERS.with_borrow_mut(|handlers| {
        handlers.events = Some(events);
        handlers.keys = Some(keys);
        handlers.wake = Some(wake);
        handlers.generation += 1;
    });
}

/// Removes the handlers; later events and wakes are dropped and keys go to the page.
pub(crate) fn uninstall() {
    HANDLERS.with_borrow_mut(|handlers| {
        handlers.events = None;
        handlers.keys = None;
        handlers.wake = None;
        handlers.queue.clear();
        handlers.generation += 1;
    });
}

/// Queues an event and, unless a handler is already running, delivers the queue.
pub(crate) fn deliver_event(event: Event) {
    let start = HANDLERS.with_borrow_mut(|handlers| {
        // While a handler runs it is taken out, but `delivering` is set: queue for it.
        if handlers.events.is_none() && !handlers.delivering {
            return false;
        }
        handlers.queue.push_back(event);
        !std::mem::replace(&mut handlers.delivering, true)
    });
    if !start {
        return;
    }
    loop {
        let next = HANDLERS.with_borrow_mut(|handlers| {
            let event = handlers.queue.pop_front();
            let handler = handlers.events.take();
            match (event, handler) {
                (Some(event), Some(handler)) => Some((event, handler, handlers.generation)),
                (_, handler) => {
                    if handler.is_none() {
                        handlers.queue.clear();
                    }
                    handlers.events = handler;
                    handlers.delivering = false;
                    None
                }
            }
        });
        let Some((event, mut handler, generation)) = next else {
            return;
        };
        handler(event);
        HANDLERS.with_borrow_mut(|handlers| {
            // An install or uninstall made by the handler itself wins.
            if handlers.generation == generation {
                handlers.events = Some(handler);
            }
        });
    }
}

/// Asks the key handler whether it consumes a key equivalent; without one, or while it is
/// running, the key goes to the page.
pub(crate) fn deliver_key(key: KeyEquivalent) -> bool {
    run_alone(|handlers| &mut handlers.keys, |handler| handler(key)).unwrap_or(false)
}

/// Runs the wake handler; without one, or while it is running, the wake is dropped.
pub(crate) fn deliver_wake() {
    run_alone(|handlers| &mut handlers.wake, |handler| handler());
}

/// Takes a handler out of its slot while it runs, so it cannot be entered again, and puts it
/// back unless it was replaced or removed meanwhile. `None` when there was no handler to run.
fn run_alone<H, R>(
    slot: fn(&mut Handlers) -> &mut Option<H>,
    run: impl FnOnce(&mut H) -> R,
) -> Option<R> {
    let (mut handler, generation) = HANDLERS.with_borrow_mut(|handlers| {
        let handler = slot(handlers).take()?;
        Some((handler, handlers.generation))
    })?;
    let result = run(&mut handler);
    HANDLERS.with_borrow_mut(|handlers| {
        if handlers.generation == generation {
            *slot(handlers) = Some(handler);
        }
    });
    Some(result)
}
