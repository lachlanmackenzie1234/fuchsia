// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Session lifecycle: identity staleness, allowlist validation, and the
//! exclusive mutation lease.
//!
//! Opening a session validates the caller's identity expectations against
//! the proxy instance and the entire allowlist against the resources and
//! ceiling. A stale expectation or rejected allowlist creates no session
//! and performs no hardware operation.

use crate::access_policy::{
    AccessPolicy, AccessRule, Denial, MmioResource, ResourceCeiling, ResourceId,
};
use std::collections::BTreeMap;

/// Host-supplied run context recorded with each session. Values are
/// diagnostic provenance, not authorization.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunContext {
    /// Host run identifier.
    pub run_id: String,
    /// Host case identifier.
    pub case_id: String,
    /// Canonical plan digest.
    pub plan_digest: String,
    /// Host tool version.
    pub host_tool_version: String,
}

/// The proxy instance identity that session expectations are validated
/// against. Also the shape of those expectations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyIdentity {
    /// Boot identity of the target.
    pub boot_id: String,
    /// Generation of this proxy instance; changes on driver restart.
    pub proxy_generation: u64,
    /// Whole-instance resource-description digest.
    pub resource_digest: String,
    /// Target-policy digest.
    pub policy_digest: String,
}

pub use crate::access_policy::SessionMode;

/// Why a session could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The proxy is stopping and accepts no new sessions.
    NotAccepting,
    /// The expected boot identity does not match.
    StaleBootId,
    /// The expected proxy generation does not match.
    StaleProxyGeneration,
    /// The expected resource digest does not match.
    StaleResourceDigest,
    /// The expected policy digest does not match.
    StalePolicyDigest,
    /// An allowlist rule failed narrowing validation; no session exists.
    RejectedAllowlist {
        /// The first offending rule.
        rule: AccessRule,
        /// Why it was rejected.
        denial: Denial,
    },
    /// Another mutating session already holds the mutation lease.
    MutationLeaseHeld,
    /// Mutating sessions are not permitted by the target ceiling policy.
    MutationNotPermitted,
    /// The request set a reserved (phase 2) expectation field. Fail
    /// closed rather than silently ignoring the field.
    UnsupportedExpectation,
}

/// One open session.
#[derive(Debug)]
pub struct Session {
    /// Session identifier, unique for the proxy instance's lifetime.
    pub id: u64,
    /// Session mode.
    pub mode: SessionMode,
    /// Host-supplied run context.
    pub context: RunContext,
    /// The validated per-session policy engine.
    pub policy: AccessPolicy,
}

/// Tracks open sessions, identity staleness, and the mutation lease.
#[derive(Debug)]
pub struct SessionManager {
    identity: ProxyIdentity,
    resources: BTreeMap<ResourceId, MmioResource>,
    ceiling: BTreeMap<ResourceId, ResourceCeiling>,
    allow_mutating_sessions: bool,
    next_id: u64,
    sessions: BTreeMap<u64, Session>,
    mutating: Option<u64>,
    accepting: bool,
}

impl SessionManager {
    /// Creates a manager for one proxy instance.
    pub fn new(
        identity: ProxyIdentity,
        resources: BTreeMap<ResourceId, MmioResource>,
        ceiling: BTreeMap<ResourceId, ResourceCeiling>,
        allow_mutating_sessions: bool,
    ) -> Self {
        Self {
            identity,
            resources,
            ceiling,
            allow_mutating_sessions,
            next_id: 1,
            sessions: BTreeMap::new(),
            mutating: None,
            accepting: true,
        }
    }

    /// The identity `Describe` reports and expectations are checked
    /// against.
    pub fn identity(&self) -> &ProxyIdentity {
        &self.identity
    }

    /// The resource descriptions offered by this instance.
    pub fn resources(&self) -> &BTreeMap<ResourceId, MmioResource> {
        &self.resources
    }

    /// Opens a session. Stale identity is rejected before allowlist
    /// validation; a rejected allowlist creates no session.
    pub fn open_session(
        &mut self,
        context: RunContext,
        mode: SessionMode,
        expectations: &ProxyIdentity,
        allowlist: impl IntoIterator<Item = AccessRule>,
    ) -> Result<u64, OpenError> {
        if !self.accepting {
            return Err(OpenError::NotAccepting);
        }
        if expectations.boot_id != self.identity.boot_id {
            return Err(OpenError::StaleBootId);
        }
        if expectations.proxy_generation != self.identity.proxy_generation {
            return Err(OpenError::StaleProxyGeneration);
        }
        if expectations.resource_digest != self.identity.resource_digest {
            return Err(OpenError::StaleResourceDigest);
        }
        if expectations.policy_digest != self.identity.policy_digest {
            return Err(OpenError::StalePolicyDigest);
        }
        if mode == SessionMode::Mutating {
            if !self.allow_mutating_sessions {
                return Err(OpenError::MutationNotPermitted);
            }
            if self.mutating.is_some() {
                return Err(OpenError::MutationLeaseHeld);
            }
        }
        let policy =
            AccessPolicy::new(mode, self.resources.clone(), self.ceiling.clone(), allowlist)
                .map_err(|(rule, denial)| OpenError::RejectedAllowlist { rule, denial })?;
        let id = self.next_id;
        self.next_id += 1;
        if mode == SessionMode::Mutating {
            self.mutating = Some(id);
        }
        self.sessions.insert(id, Session { id, mode, context, policy });
        Ok(id)
    }

    /// The session with `id`, if open.
    pub fn session(&self, id: u64) -> Option<&Session> {
        self.sessions.get(&id)
    }

    /// Closes a session, releasing the mutation lease if it held it.
    /// Returns whether the session existed.
    pub fn close_session(&mut self, id: u64) -> bool {
        let existed = self.sessions.remove(&id).is_some();
        if existed && self.mutating == Some(id) {
            self.mutating = None;
        }
        existed
    }

    /// Stop path: reject all new sessions. Existing sessions are closed by
    /// their channels.
    pub fn reject_new_sessions(&mut self) {
        self.accepting = false;
    }

    /// Whether new sessions are being accepted.
    pub fn is_accepting(&self) -> bool {
        self.accepting
    }

    /// Number of open sessions.
    pub fn open_count(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_policy::{AccessClass, WIDTH32};

    const CTRL: ResourceId = 1;

    fn identity() -> ProxyIdentity {
        ProxyIdentity {
            boot_id: "boot-1".to_string(),
            proxy_generation: 7,
            resource_digest: "sha256:aa".to_string(),
            policy_digest: "sha256:bb".to_string(),
        }
    }

    fn manager() -> SessionManager {
        let resources = BTreeMap::from([(
            CTRL,
            MmioResource {
                name: "ctrl".to_string(),
                kind: crate::access_policy::ResourceKind::Mmio,
                logical_size: 0x100,
                mapped_size: 0x100,
            },
        )]);
        let ceiling = BTreeMap::from([(
            CTRL,
            ResourceCeiling {
                hard_denied: vec![],
                allow_unknown_reads: true,
                allow_poll: false,
                writable_registers: vec![],
                protocol: None,
            },
        )]);
        SessionManager::new(identity(), resources, ceiling, true)
    }

    fn open(manager: &mut SessionManager, mode: SessionMode) -> Result<u64, OpenError> {
        manager.open_session(RunContext::default(), mode, &identity(), [])
    }

    #[test]
    fn open_and_close() {
        let mut manager = manager();
        let id = open(&mut manager, SessionMode::ReadOnly).unwrap();
        assert_eq!(manager.open_count(), 1);
        assert!(manager.session(id).is_some());
        assert!(manager.close_session(id));
        assert_eq!(manager.open_count(), 0);
        assert!(!manager.close_session(id));
    }

    #[test]
    fn session_ids_are_unique_and_increasing() {
        let mut manager = manager();
        let first = open(&mut manager, SessionMode::ReadOnly).unwrap();
        let second = open(&mut manager, SessionMode::ReadOnly).unwrap();
        assert!(second > first);
    }

    #[test]
    fn each_stale_expectation_is_distinct() {
        let mut manager = manager();

        let mut stale = identity();
        stale.boot_id = "boot-2".to_string();
        assert_eq!(
            manager.open_session(RunContext::default(), SessionMode::ReadOnly, &stale, []),
            Err(OpenError::StaleBootId)
        );

        let mut stale = identity();
        stale.proxy_generation = 8;
        assert_eq!(
            manager.open_session(RunContext::default(), SessionMode::ReadOnly, &stale, []),
            Err(OpenError::StaleProxyGeneration)
        );

        let mut stale = identity();
        stale.resource_digest = "sha256:cc".to_string();
        assert_eq!(
            manager.open_session(RunContext::default(), SessionMode::ReadOnly, &stale, []),
            Err(OpenError::StaleResourceDigest)
        );

        let mut stale = identity();
        stale.policy_digest = "sha256:dd".to_string();
        assert_eq!(
            manager.open_session(RunContext::default(), SessionMode::ReadOnly, &stale, []),
            Err(OpenError::StalePolicyDigest)
        );

        assert_eq!(manager.open_count(), 0);
    }

    #[test]
    fn rejected_allowlist_creates_no_session() {
        let mut manager = manager();
        let rule =
            AccessRule { resource: 99, offset: 0, width: WIDTH32, class: AccessClass::ReadOnce };
        let result =
            manager.open_session(RunContext::default(), SessionMode::ReadOnly, &identity(), [rule]);
        assert_eq!(
            result,
            Err(OpenError::RejectedAllowlist { rule, denial: Denial::UnknownResource })
        );
        assert_eq!(manager.open_count(), 0);
    }

    #[test]
    fn mutation_lease_is_exclusive() {
        let mut manager = manager();
        let holder = open(&mut manager, SessionMode::Mutating).unwrap();
        assert_eq!(open(&mut manager, SessionMode::Mutating), Err(OpenError::MutationLeaseHeld));
        // Read-only sessions still coexist.
        assert!(open(&mut manager, SessionMode::ReadOnly).is_ok());
        // Closing the holder releases the lease.
        assert!(manager.close_session(holder));
        assert!(open(&mut manager, SessionMode::Mutating).is_ok());
    }

    #[test]
    fn reject_new_sessions_fails_closed() {
        let mut manager = manager();
        manager.reject_new_sessions();
        assert_eq!(open(&mut manager, SessionMode::ReadOnly), Err(OpenError::NotAccepting));
    }

    #[test]
    fn mutating_session_rejected_when_not_permitted() {
        let resources = BTreeMap::from([(
            CTRL,
            MmioResource {
                name: "ctrl".to_string(),
                kind: crate::access_policy::ResourceKind::Mmio,
                logical_size: 0x100,
                mapped_size: 0x100,
            },
        )]);
        let ceiling = BTreeMap::from([(
            CTRL,
            ResourceCeiling {
                hard_denied: vec![],
                allow_unknown_reads: true,
                allow_poll: false,
                writable_registers: vec![],
                protocol: None,
            },
        )]);
        let mut manager = SessionManager::new(identity(), resources, ceiling, false);
        assert_eq!(open(&mut manager, SessionMode::Mutating), Err(OpenError::MutationNotPermitted));
        // Read-only sessions still work.
        assert!(open(&mut manager, SessionMode::ReadOnly).is_ok());
    }
}
