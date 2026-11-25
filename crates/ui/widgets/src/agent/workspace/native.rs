use std::cell::RefCell;
use std::rc::Weak;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSBezelStyle, NSButton, NSColor, NSLineBreakMode, NSTextField, NSView};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use quickie_ui_shared::tokens::TextRole;

use super::{AgentWorkspacePanel, WorkspacePanelRequest};

pub(super) struct TargetIvars {
    owner: RefCell<Weak<AgentWorkspacePanel>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and Target implements no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAgentWorkspaceTarget"]
    #[ivars = TargetIvars]
    pub(super) struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(submit:))]
        fn submit(&self, _sender: Option<&AnyObject>) {
            if let Some(panel) = self.ivars().owner.borrow().upgrade() { panel.submit(); }
        }

        #[unsafe(method(stop:))]
        fn stop(&self, _sender: Option<&NSButton>) {
            if let Some(panel) = self.ivars().owner.borrow().upgrade()
                && panel.conversation.borrow().running {
                panel.request(WorkspacePanelRequest::Stop);
            }
        }

        #[unsafe(method(closeAgent:))]
        fn close_agent(&self, _sender: Option<&NSButton>) {
            if let Some(panel) = self.ivars().owner.borrow().upgrade() {
                panel.request(WorkspacePanelRequest::Close);
            }
        }

        #[unsafe(method(togglePreview:))]
        fn toggle_preview(&self, _sender: Option<&NSButton>) {
            if let Some(panel) = self.ivars().owner.borrow().upgrade() {
                panel.request(WorkspacePanelRequest::TogglePreview);
            }
        }

        #[unsafe(method(settings:))]
        fn settings(&self, _sender: Option<&NSButton>) {
            if let Some(panel) = self.ivars().owner.borrow().upgrade() {
                panel.request(WorkspacePanelRequest::Settings);
            }
        }

        #[unsafe(method(details:))]
        fn details(&self, sender: Option<&NSButton>) {
            let index = sender.and_then(|button| usize::try_from(button.tag()).ok());
            if let (Some(panel), Some(index)) = (self.ivars().owner.borrow().upgrade(), index) {
                panel.toggle_detail(index);
            }
        }

        #[unsafe(method(cite:))]
        fn cite(&self, sender: Option<&NSButton>) {
            let id = sender.and_then(|button| u32::try_from(button.tag()).ok());
            if let (Some(panel), Some(id)) = (self.ivars().owner.borrow().upgrade(), id) {
                panel.cite(id);
            }
        }
    }
);

impl Target {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars {
            owner: RefCell::new(Weak::new()),
        });
        // SAFETY: NSObject's designated initializer; ivars are initialized.
        unsafe { msg_send![super(this), init] }
    }

    pub fn set_owner(&self, owner: Weak<AgentWorkspacePanel>) {
        *self.ivars().owner.borrow_mut() = owner;
    }

    pub fn open(&self, url: url::Url) {
        if let Some(panel) = self.ivars().owner.borrow().upgrade() {
            panel.request(WorkspacePanelRequest::Open(url));
        }
    }
}

pub(super) struct ViewIvars {
    owner: RefCell<Weak<AgentWorkspacePanel>>,
}

define_class!(
    // SAFETY: NSView has no additional subclassing requirements; the public size and
    // coordinate methods call their superclass where required. No Drop is implemented.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "QuickieAgentWorkspaceView"]
    #[ivars = ViewIvars]
    pub(super) struct WorkspaceView;

    impl WorkspaceView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            // SAFETY: hitTest: accepts a point in the superview's coordinates and returns its view.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            // The overlay root fills the window. Leave empty space to its underlying siblings.
            if self.ivars().owner.borrow().strong_count() > 0
                && hit.as_deref().is_some_and(|view| view == &**self)
            {
                None
            } else {
                hit
            }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: NSView's method accepts the new frame size.
            let () = unsafe { msg_send![super(self), setFrameSize: size] };
            if let Some(panel) = self.ivars().owner.borrow().upgrade() {
                panel.resize();
            }
        }
    }
);

impl WorkspaceView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            owner: RefCell::new(Weak::new()),
        });
        // SAFETY: NSView's designated initializer; ivars are initialized.
        unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] }
    }

    pub fn set_owner(&self, owner: Weak<AgentWorkspacePanel>) {
        *self.ivars().owner.borrow_mut() = owner;
    }
}

pub(super) fn button(
    title: &str,
    action: Sel,
    target: &Target,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::new(mtm);
    button.setTitle(&NSString::from_str(title));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setFont(Some(&TextRole::RowDetail.font()));
    // SAFETY: the panel retains the target for as long as the button.
    unsafe { button.setTarget(Some(target)) };
    // SAFETY: Target implements the supplied action with an optional sender.
    unsafe { button.setAction(Some(action)) };
    button.sizeToFit();
    button
}

pub(super) fn label(
    role: TextRole,
    color: &NSColor,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(" "), mtm);
    label.setFont(Some(&role.font()));
    label.setTextColor(Some(color));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

pub(super) fn place(view: &NSView, x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat) {
    view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
}
