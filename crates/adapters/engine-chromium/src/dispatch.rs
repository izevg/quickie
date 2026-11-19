//! Delivery of engine events and key equivalents to the handlers the shell installed.
//!
//! Everything runs on the main thread (Chromium's UI thread), so the handlers live in a
//! thread-local. A handler may call back into the engine, and the engine may report an event
//! synchronously from that call; such an event is queued and delivered once the running handler
//! returns, so handlers never run re-entrantly.

use std::cell::RefCell;
use std::collections::VecDeque;

use crate::{Event, KeyEquivalent};

type EventHandler = Box<dyn FnMut(Event)>;
type KeyHandler = Box<dyn FnMut(KeyEquivalent) -> bool>;

#[derive(Default)]
struct Handlers {
    events: Option<EventHandler>,
    keys: Option<KeyHandler>,
    queue: VecDeque<Event>,
    delivering: bool,
    /// Bumped by every install and uninstall, so a running handler is not put back over them.
    generation: u64,
}

thread_local! {
    static HANDLERS: RefCell<Handlers> = RefCell::default();
}

/// Installs the handlers, replacing earlier ones.
pub(crate) fn install(events: EventHandler, keys: KeyHandler) {
    HANDLERS.with_borrow_mut(|handlers| {
        handlers.events = Some(events);
        handlers.keys = Some(keys);
        handlers.generation += 1;
    });
}

/// Removes the handlers; later events are dropped and keys go to the page.
pub(crate) fn uninstall() {
    HANDLERS.with_borrow_mut(|handlers| {
        handlers.events = None;
        handlers.keys = None;
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
    let Some((mut handler, generation)) = HANDLERS.with_borrow_mut(|handlers| {
        let handler = handlers.keys.take()?;
        Some((handler, handlers.generation))
    }) else {
        return false;
    };
    let consumed = handler(key);
    HANDLERS.with_borrow_mut(|handlers| {
        if handlers.generation == generation {
            handlers.keys = Some(handler);
        }
    });
    consumed
}
