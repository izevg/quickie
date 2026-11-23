use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::NSString;

/// `WebKit`'s CTAP PIN completion. Dropping an unanswered request sends nil.
pub(super) struct PinReply(Option<RcBlock<dyn Fn(*mut NSString)>>);

impl PinReply {
    pub(super) fn new(block: RcBlock<dyn Fn(*mut NSString)>) -> Self {
        Self(Some(block))
    }

    pub(super) fn send(mut self, pin: &Retained<NSString>) {
        if let Some(reply) = self.0.take() {
            reply.call((Retained::as_ptr(pin).cast_mut(),));
        }
    }
}

impl Drop for PinReply {
    fn drop(&mut self) {
        if let Some(reply) = self.0.take() {
            reply.call((std::ptr::null_mut(),));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::{NSString, PinReply, RcBlock};

    #[test]
    fn delivers_the_response_once_and_answers_nil_on_drop() {
        let heard = Rc::new(RefCell::new(Vec::new()));
        let copy = Rc::clone(&heard);
        let callback =
            RcBlock::new(move |value: *mut NSString| copy.borrow_mut().push(value.is_null()));
        PinReply::new(callback.clone()).send(&NSString::from_str("test"));
        drop(PinReply::new(callback));
        assert_eq!(&*heard.borrow(), &[false, true]);
    }
}
