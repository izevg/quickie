//! Toolkit-thread diagnostic state managing active client attachments and event rings.

use std::collections::HashMap;

use quickie_domain_browsing::TabId;

use super::events::{DiagnosticEventBatch, DiagnosticEventData, DiagnosticEventFilter, EventRing};
use super::types::{
    AttachmentId, BoundedSnapshot, ClientId, DiagnosticError, DocumentBinding, EnabledAttachment,
    GrantScope, MAX_CLIENTS, NavigationKind, RevokeReason, RevokedAttachment, is_valid_http_origin,
};

#[derive(Debug)]
struct ActiveAttachment {
    id: AttachmentId,
    client: ClientId,
    binding: DocumentBinding,
    scope: GrantScope,
    ring: EventRing,
}

/// Pure application state for active `Profile::Main` read-only diagnostic attachments.
///
/// Instantiated once on the toolkit thread. Enforces single-attachment-per-client ownership,
/// fixed client capacity (`MAX_CLIENTS`), non-reusable `AttachmentId` issuance, navigation-based
/// revocation, stale-callback rejection, and bounded per-attachment event replay.
#[derive(Debug, Default)]
pub struct DiagnosticState {
    next_attachment_id: u64,
    attachments: HashMap<AttachmentId, ActiveAttachment>,
    by_client: HashMap<ClientId, AttachmentId>,
}

impl DiagnosticState {
    /// Creates an empty diagnostic state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of currently active diagnostic attachments (at most `MAX_CLIENTS`).
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.attachments.len()
    }

    /// Whether any active attachment currently observes `tab`.
    #[must_use]
    pub fn has_attachments_for_tab(&self, tab: TabId) -> bool {
        self.attachments
            .values()
            .any(|entry| entry.binding.tab == tab)
    }

    /// Enables a diagnostic attachment after native UI confirms `approved` for `client`.
    ///
    /// Fails without mutating state if `live` does not exactly match `approved`, if `approved`
    /// is not a valid HTTP(S) tuple origin on a `Profile::Main` tab, or if a new client would
    /// exceed `MAX_CLIENTS`.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::InvalidApproval`] for stale or invalid approvals,
    /// [`DiagnosticError::ClientCapacityReached`] when `MAX_CLIENTS` distinct clients are active,
    /// or [`DiagnosticError::AttachmentIdExhausted`] if the monotonic attachment ID space is full.
    pub fn enable(
        &mut self,
        client: ClientId,
        approved: DocumentBinding,
        live: Option<&DocumentBinding>,
        scope: GrantScope,
    ) -> Result<EnabledAttachment, DiagnosticError> {
        if live != Some(&approved) || !approved.is_valid_main() {
            return Err(DiagnosticError::InvalidApproval);
        }

        let has_existing = self.by_client.contains_key(&client);
        if !has_existing && self.attachments.len() >= MAX_CLIENTS {
            return Err(DiagnosticError::ClientCapacityReached);
        }

        let next_id = self
            .next_attachment_id
            .checked_add(1)
            .ok_or(DiagnosticError::AttachmentIdExhausted)?;

        let replaced = self.remove_client_attachment(client, RevokeReason::Replaced);
        self.next_attachment_id = next_id;
        let id = AttachmentId::issue(next_id);

        self.attachments.insert(
            id,
            ActiveAttachment {
                id,
                client,
                binding: approved,
                scope,
                ring: EventRing::new(),
            },
        );
        self.by_client.insert(client, id);

        Ok(EnabledAttachment {
            attachment: id,
            replaced,
        })
    }

    /// Returns the live [`DocumentBinding`] for an attachment owned by `client`.
    ///
    /// Unissued, revoked, and wrong-owner IDs all return [`DiagnosticError::AttachmentUnavailable`].
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] if the attachment is not active for `client`.
    pub fn binding_for(
        &self,
        client: ClientId,
        attachment: AttachmentId,
    ) -> Result<&DocumentBinding, DiagnosticError> {
        self.owned_attachment(client, attachment)
            .map(|entry| &entry.binding)
    }

    /// Validates that a native callback's captured `(attachment, binding)` is still live.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] if the attachment was retired or its
    /// binding has changed.
    pub fn validate_callback(
        &self,
        attachment: AttachmentId,
        captured: &DocumentBinding,
    ) -> Result<(), DiagnosticError> {
        let entry = self
            .attachments
            .get(&attachment)
            .ok_or(DiagnosticError::AttachmentUnavailable)?;
        if entry.binding != *captured {
            return Err(DiagnosticError::AttachmentUnavailable);
        }
        Ok(())
    }

    /// Validates a native snapshot callback and wraps its payload in a [`BoundedSnapshot`].
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] if the attachment was retired or its
    /// binding no longer matches `captured`.
    pub fn complete_snapshot(
        &self,
        attachment: AttachmentId,
        captured: &DocumentBinding,
        data: &str,
        truncated: bool,
    ) -> Result<BoundedSnapshot, DiagnosticError> {
        self.validate_callback(attachment, captured)?;
        Ok(BoundedSnapshot::new(captured.epoch, data, truncated))
    }

    /// Records a diagnostic event from a native observer callback.
    ///
    /// Rejects callbacks whose `attachment` is inactive or whose `captured` binding no longer
    /// matches the live attachment binding.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] if the callback is stale.
    pub fn record_event(
        &mut self,
        attachment: AttachmentId,
        captured: &DocumentBinding,
        data: DiagnosticEventData,
    ) -> Result<u64, DiagnosticError> {
        let entry = self
            .attachments
            .get_mut(&attachment)
            .ok_or(DiagnosticError::AttachmentUnavailable)?;
        if entry.binding != *captured {
            return Err(DiagnosticError::AttachmentUnavailable);
        }
        Ok(entry.ring.push(captured.epoch, data))
    }

    /// Reads paginated, filtered diagnostic events for an attachment owned by `client`.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] for unissued, revoked, or wrong-owner
    /// attachments, or [`DiagnosticError::InvalidCursor`] if `cursor` is ahead of the stream.
    pub fn read_events(
        &self,
        client: ClientId,
        attachment: AttachmentId,
        cursor: u64,
        filter: &DiagnosticEventFilter,
        limit: usize,
    ) -> Result<DiagnosticEventBatch, DiagnosticError> {
        let entry = self.owned_attachment(client, attachment)?;
        entry.ring.read(cursor, filter, limit)
    }

    /// Applies a main-frame navigation observed on `tab`, returning any revoked attachments.
    pub fn observe_navigation(
        &mut self,
        tab: TabId,
        observed: &DocumentBinding,
        kind: NavigationKind,
    ) -> Vec<RevokedAttachment> {
        let mut to_revoke = Vec::new();

        for entry in self.attachments.values_mut() {
            if entry.binding.tab != tab {
                continue;
            }
            if observed.tab != tab {
                to_revoke.push((entry.id, RevokeReason::NavigatedAway));
                continue;
            }
            if !is_valid_http_origin(&observed.origin) || observed.origin != entry.binding.origin {
                to_revoke.push((entry.id, RevokeReason::CrossOriginNavigation));
                continue;
            }
            match kind {
                NavigationKind::SameDocument if observed.epoch == entry.binding.epoch => {}
                NavigationKind::Reload
                    if entry.scope == GrantScope::SameOriginReload
                        && observed.epoch > entry.binding.epoch =>
                {
                    entry.binding = observed.clone();
                }
                NavigationKind::SameDocument
                | NavigationKind::Reload
                | NavigationKind::NewDocument => {
                    to_revoke.push((entry.id, RevokeReason::NavigatedAway));
                }
            }
        }

        self.remove_many(to_revoke)
    }

    /// Revokes all attachments bound to a closed `tab`.
    pub fn on_tab_closed(&mut self, tab: TabId) -> Vec<RevokedAttachment> {
        self.revoke_tab(tab, RevokeReason::TabClosed)
    }

    /// Revokes all attachments bound to a `tab` whose web content process terminated.
    pub fn on_process_terminated(&mut self, tab: TabId) -> Vec<RevokedAttachment> {
        self.revoke_tab(tab, RevokeReason::ProcessCrashed)
    }

    /// Manually revokes an attachment (e.g. user clicked Revoke in native UI).
    pub fn revoke(&mut self, attachment: AttachmentId) -> Option<RevokedAttachment> {
        self.remove_attachment(attachment, RevokeReason::UserRevoked)
    }

    /// Voluntarily detaches an attachment on behalf of its owning `client`.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticError::AttachmentUnavailable`] if the attachment is not active for `client`.
    pub fn detach(
        &mut self,
        client: ClientId,
        attachment: AttachmentId,
    ) -> Result<RevokedAttachment, DiagnosticError> {
        self.owned_attachment(client, attachment)?;
        self.remove_attachment(attachment, RevokeReason::ClientDetached)
            .ok_or(DiagnosticError::AttachmentUnavailable)
    }

    /// Releases any active attachment owned by a disconnected `client`.
    pub fn on_client_disconnected(&mut self, client: ClientId) -> Option<RevokedAttachment> {
        self.remove_client_attachment(client, RevokeReason::ClientDisconnected)
    }

    /// Releases all active attachments when the browser shuts down.
    pub fn revoke_all_for_shutdown(&mut self) -> Vec<RevokedAttachment> {
        let targets = self
            .attachments
            .keys()
            .copied()
            .map(|id| (id, RevokeReason::BrowserShuttingDown))
            .collect();
        self.remove_many(targets)
    }

    #[cfg(test)]
    pub(super) const fn set_next_attachment_id(&mut self, next: u64) {
        self.next_attachment_id = next;
    }

    fn owned_attachment(
        &self,
        client: ClientId,
        attachment: AttachmentId,
    ) -> Result<&ActiveAttachment, DiagnosticError> {
        self.attachments
            .get(&attachment)
            .filter(|entry| entry.client == client)
            .ok_or(DiagnosticError::AttachmentUnavailable)
    }

    fn revoke_tab(&mut self, tab: TabId, reason: RevokeReason) -> Vec<RevokedAttachment> {
        let targets = self
            .attachments
            .values()
            .filter(|entry| entry.binding.tab == tab)
            .map(|entry| (entry.id, reason))
            .collect();
        self.remove_many(targets)
    }

    fn remove_client_attachment(
        &mut self,
        client: ClientId,
        reason: RevokeReason,
    ) -> Option<RevokedAttachment> {
        let id = self.by_client.get(&client).copied()?;
        self.remove_attachment(id, reason)
    }

    fn remove_attachment(
        &mut self,
        attachment: AttachmentId,
        reason: RevokeReason,
    ) -> Option<RevokedAttachment> {
        let entry = self.attachments.remove(&attachment)?;
        self.by_client.remove(&entry.client);
        Some(RevokedAttachment {
            attachment: entry.id,
            client: entry.client,
            tab: entry.binding.tab,
            reason,
        })
    }

    fn remove_many(
        &mut self,
        targets: Vec<(AttachmentId, RevokeReason)>,
    ) -> Vec<RevokedAttachment> {
        let mut revoked: Vec<RevokedAttachment> = targets
            .into_iter()
            .filter_map(|(id, reason)| self.remove_attachment(id, reason))
            .collect();
        revoked.sort_by_key(|entry| entry.attachment);
        revoked
    }
}
