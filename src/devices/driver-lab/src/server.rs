// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! FIDL server for the `fuchsia.driver.lab` wire contract, bridging the
//! host-facing protocol to the host-testable core logic.

use crate::platform_provider::MappedMmio;
use fidl_fuchsia_driver_lab as flab;
use fuchsia_async::ScopeHandle;
use futures::TryStreamExt;
use lab_proxy_core::access_policy::{AccessClass, AccessRule, Denial};
use lab_proxy_core::audit_ring::{AuditRecord, AuditRing, Decision, OpStatus};
use lab_proxy_core::executor::{Executor, ReadError, SnapshotError, SnapshotItem};
use lab_proxy_core::hardware_backend::Clock;
use lab_proxy_core::session::{OpenError, ProxyIdentity, RunContext, SessionManager, SessionMode};
use std::sync::{Arc, Mutex};

/// Wire-contract version served by this driver.
pub const PROTOCOL_MAJOR: u32 = 1;
/// Wire-contract minor version.
pub const PROTOCOL_MINOR: u32 = 0;

/// Clock backed by zx monotonic time.
#[derive(Debug, Default)]
pub struct ZxClock;

impl Clock for ZxClock {
    fn now_ns(&mut self) -> i64 {
        zx::MonotonicInstant::get().into_nanos()
    }
}

/// Shared proxy state behind one lock.
#[derive(Debug)]
pub struct ProxyState {
    /// Session lifecycle and per-session policy.
    pub sessions: SessionManager,
    /// The shared read-path executor over the acquired MMIO mappings.
    pub executor: Executor<MappedMmio, ZxClock>,
    /// The instance audit ring.
    pub audit: AuditRing,
    /// Per-resource description digests, reported by `Describe` for
    /// persistent grant matching.
    pub resource_digests: std::collections::BTreeMap<u32, String>,
}

/// Handle to the shared proxy state.
pub type SharedState = Arc<Mutex<ProxyState>>;

fn now_ns() -> i64 {
    zx::MonotonicInstant::get().into_nanos()
}

fn denial_to_fidl(denial: Denial) -> flab::OperationError {
    match denial {
        Denial::UnknownResource => flab::OperationError::UnknownResource,
        Denial::UnsupportedWidth => flab::OperationError::UnsupportedWidth,
        Denial::UnsupportedAccessClass => flab::OperationError::UnsupportedAccessClass,
        Denial::Misaligned => flab::OperationError::Misaligned,
        Denial::OffsetOverflow => flab::OperationError::OffsetOverflow,
        Denial::OutOfLogicalBounds => flab::OperationError::OutOfLogicalBounds,
        Denial::OutOfMappedBounds => flab::OperationError::OutOfMappedBounds,
        Denial::NotPermittedByCeiling => flab::OperationError::NotPermittedByCeiling,
        Denial::HardDenied => flab::OperationError::HardDenied,
        Denial::UnknownReadsNotPermitted => flab::OperationError::UnknownReadsNotPermitted,
        Denial::PollNotPermitted => flab::OperationError::NotPermittedByCeiling,
        Denial::NotInAllowlist => flab::OperationError::NotInAllowlist,
        Denial::LimitExceeded => flab::OperationError::LimitExceeded,
        Denial::StaleIdentity => flab::OperationError::StaleIdentity,
        Denial::MutationLeaseContention => flab::OperationError::MutationLeaseContention,
        Denial::NotAccepting => flab::OperationError::NotAccepting,
        Denial::UnsupportedExpectation => flab::OperationError::UnsupportedExpectation,
    }
}

fn open_error_to_fidl(error: &OpenError) -> flab::OpenSessionError {
    match error {
        OpenError::NotAccepting => flab::OpenSessionError::NotAccepting,
        OpenError::StaleBootId => flab::OpenSessionError::StaleBootId,
        OpenError::StaleProxyGeneration => flab::OpenSessionError::StaleProxyGeneration,
        OpenError::StaleResourceDigest => flab::OpenSessionError::StaleResourceDigest,
        OpenError::StalePolicyDigest => flab::OpenSessionError::StalePolicyDigest,
        OpenError::RejectedAllowlist { .. } => flab::OpenSessionError::RejectedAllowlist,
        OpenError::MutationLeaseHeld | OpenError::MutationNotPermitted => {
            flab::OpenSessionError::MutationLeaseHeld
        }
        OpenError::UnsupportedExpectation => flab::OpenSessionError::UnsupportedExpectation,
    }
}

fn open_error_denial(error: &OpenError) -> Denial {
    match error {
        OpenError::NotAccepting => Denial::NotAccepting,
        OpenError::StaleBootId
        | OpenError::StaleProxyGeneration
        | OpenError::StaleResourceDigest
        | OpenError::StalePolicyDigest => Denial::StaleIdentity,
        OpenError::RejectedAllowlist { denial, .. } => *denial,
        OpenError::MutationLeaseHeld => Denial::MutationLeaseContention,
        OpenError::MutationNotPermitted => Denial::NotPermittedByCeiling,
        OpenError::UnsupportedExpectation => Denial::UnsupportedExpectation,
    }
}

fn describe(state: &SharedState) -> flab::ProxyDescription {
    let state = state.lock().unwrap();
    let identity = state.sessions.identity();
    let resources: Vec<flab::ResourceDescription> = state
        .sessions
        .resources()
        .iter()
        .map(|(id, resource)| flab::ResourceDescription {
            id: Some(*id),
            name: Some(resource.name.clone()),
            kind: Some(flab::ResourceKind::Mmio),
            logical_size: Some(resource.logical_size),
            digest: state.resource_digests.get(id).cloned(),
            ..Default::default()
        })
        .collect();
    flab::ProxyDescription {
        protocol_major: Some(PROTOCOL_MAJOR),
        protocol_minor: Some(PROTOCOL_MINOR),
        proxy_generation: Some(identity.proxy_generation),
        boot_id: Some(identity.boot_id.clone()),
        resource_digest: Some(identity.resource_digest.clone()),
        policy_digest: Some(identity.policy_digest.clone()),
        resources: Some(resources),
        max_snapshot_items: Some(state.executor.limits().max_snapshot_items as u32),
        audit_capacity: Some(state.audit.capacity() as u64),
        // Phase 1 never activates takeover; node_moniker and
        // topology_generation stay absent until the proxy binds to a
        // real node.
        takeover: Some(flab::TakeoverState::NotActive),
        ..Default::default()
    }
}

fn open_session(
    state: &SharedState,
    context: flab::RunContext,
    mode: flab::SessionMode,
    expectations: flab::Expectations,
    allowlist: Vec<flab::AccessRule>,
) -> Result<u64, OpenError> {
    let context = RunContext {
        run_id: context.run_id.unwrap_or_default(),
        case_id: context.case_id.unwrap_or_default(),
        plan_digest: context.plan_digest.unwrap_or_default(),
        host_tool_version: context.host_tool_version.unwrap_or_default(),
    };
    let mode = match mode {
        flab::SessionMode::ReadOnly => SessionMode::ReadOnly,
        flab::SessionMode::Mutating => SessionMode::Mutating,
    };
    // Reserved phase 2 expectations fail closed rather than being
    // silently ignored.
    if expectations.topology_generation.is_some() || expectations.bound_driver_url.is_some() {
        let mut state = state.lock().unwrap();
        state.audit.append(AuditRecord {
            session: None,
            resource: None,
            operation: "open_session",
            offset: None,
            decision: Decision::Denied(Denial::UnsupportedExpectation),
            status: OpStatus::Rejected,
            value: None,
            timestamp_ns: now_ns(),
            run_id: Some(context.run_id.clone()),
            item_index: None,
        });
        return Err(OpenError::UnsupportedExpectation);
    }
    // A required expectation the host omitted can never match the real
    // identity, so absence falls through to the per-field stale check.
    let expectations = ProxyIdentity {
        boot_id: expectations.boot_id.unwrap_or_default(),
        proxy_generation: expectations.proxy_generation.unwrap_or_default(),
        resource_digest: expectations.resource_digest.unwrap_or_default(),
        policy_digest: expectations.policy_digest.unwrap_or_default(),
    };
    let rules: Vec<AccessRule> = allowlist
        .iter()
        .map(|rule| AccessRule {
            resource: rule.resource,
            offset: rule.offset,
            width: rule.width,
            class: match rule.class {
                flab::AccessClass::ReadOnce => AccessClass::ReadOnce,
                flab::AccessClass::Snapshot => AccessClass::Snapshot,
            },
        })
        .collect();

    let mut state = state.lock().unwrap();
    let run_id = context.run_id.clone();
    let result = state.sessions.open_session(context, mode, &expectations, rules);
    let record = match &result {
        Ok(id) => AuditRecord {
            session: Some(*id),
            resource: None,
            operation: "open_session",
            offset: None,
            decision: Decision::Allowed,
            status: OpStatus::Ok,
            value: None,
            timestamp_ns: now_ns(),
            run_id: Some(run_id),
            item_index: None,
        },
        Err(error) => {
            // A rejected allowlist records the first offending rule, as
            // the wire contract documents.
            let (resource, offset) = match error {
                OpenError::RejectedAllowlist { rule, .. } => {
                    (Some(rule.resource), Some(rule.offset))
                }
                _ => (None, None),
            };
            AuditRecord {
                session: None,
                resource,
                operation: "open_session",
                offset,
                decision: Decision::Denied(open_error_denial(error)),
                status: OpStatus::Rejected,
                value: None,
                timestamp_ns: now_ns(),
                run_id: Some(run_id),
                item_index: None,
            }
        }
    };
    state.audit.append(record);
    result
}

/// Serves one `Proxy` connection, spawning a task per opened
/// session on `scope`.
pub async fn serve_proxy(
    state: SharedState,
    scope: ScopeHandle,
    mut stream: flab::Proxy_RequestStream,
) {
    while let Ok(Some(request)) = stream.try_next().await {
        match request {
            flab::Proxy_Request::Describe { responder } => {
                let description = describe(&state);
                let _ = responder.send(&description);
            }
            flab::Proxy_Request::OpenSession {
                context,
                mode,
                expectations,
                allowlist,
                session,
                responder,
            } => match open_session(&state, context, mode, expectations, allowlist) {
                Ok(id) => {
                    scope.spawn(serve_session(state.clone(), id, session.into_stream()));
                    let _ = responder.send(Ok(id));
                }
                Err(error) => {
                    let _ = responder.send(Err(open_error_to_fidl(&error)));
                }
            },
            flab::Proxy_Request::_UnknownMethod { .. } => {}
        }
    }
}

async fn serve_session(state: SharedState, id: u64, mut stream: flab::SessionRequestStream) {
    while let Ok(Some(request)) = stream.try_next().await {
        match request {
            flab::SessionRequest::Read32 { resource, offset, responder } => {
                let result = {
                    let guard = &mut *state.lock().unwrap();
                    let ProxyState { sessions, executor, audit, .. } = guard;
                    match sessions.session(id) {
                        None => Err(flab::OperationError::StaleIdentity),
                        Some(session) => executor
                            .read32(&session.policy, audit, id, resource, offset)
                            .map(|outcome| (outcome.value, outcome.audit_seq, outcome.timestamp_ns))
                            .map_err(|error| match error {
                                ReadError::Denied { denial, .. } => denial_to_fidl(denial),
                                ReadError::Backend { .. } => flab::OperationError::BackendFault,
                            }),
                    }
                };
                let _ = responder.send(result);
            }
            flab::SessionRequest::Snapshot { items, responder } => {
                let items: Vec<SnapshotItem> = items
                    .iter()
                    .map(|item| SnapshotItem { resource: item.resource, offset: item.offset })
                    .collect();
                let result = {
                    let guard = &mut *state.lock().unwrap();
                    let ProxyState { sessions, executor, audit, .. } = guard;
                    match sessions.session(id) {
                        None => Err(flab::OperationError::StaleIdentity),
                        Some(session) => executor
                            .snapshot(&session.policy, audit, id, &items)
                            .map(|outcome| {
                                let results: Vec<flab::SnapshotItemResult> = outcome
                                    .results
                                    .iter()
                                    .map(|result| flab::SnapshotItemResult {
                                        item: flab::SnapshotItem {
                                            resource: result.item.resource,
                                            offset: result.item.offset,
                                        },
                                        ok: result.value.is_ok(),
                                        value: result.value.unwrap_or(0),
                                        audit_seq: result.audit_seq,
                                        timestamp_ns: result.timestamp_ns,
                                    })
                                    .collect();
                                (results, outcome.complete)
                            })
                            .map_err(|error| match error {
                                SnapshotError::TooManyItems { .. } => {
                                    flab::OperationError::LimitExceeded
                                }
                                SnapshotError::Rejected { denial, .. } => denial_to_fidl(denial),
                            }),
                    }
                };
                let _ = responder.send(
                    result
                        .as_ref()
                        .map(|(results, complete)| (results.as_slice(), *complete))
                        .map_err(|error| *error),
                );
            }
            flab::SessionRequest::ReadAudit { cursor, limit, responder } => {
                let (page, boot_id, proxy_generation) = {
                    let state = state.lock().unwrap();
                    let limit = (limit.min(flab::MAX_AUDIT_PAGE_ENTRIES)) as usize;
                    let identity = state.sessions.identity();
                    (
                        state.audit.read(cursor, limit),
                        identity.boot_id.clone(),
                        identity.proxy_generation,
                    )
                };
                let entries: Vec<flab::AuditEntry> = page
                    .entries
                    .iter()
                    .map(|entry| flab::AuditEntry {
                        seq: Some(entry.seq),
                        session: entry.record.session,
                        resource: entry.record.resource,
                        operation: Some(entry.record.operation.to_string()),
                        offset: entry.record.offset,
                        decision: Some(match entry.record.decision {
                            Decision::Allowed => flab::AuditDecision::Allowed,
                            Decision::Denied(_) => flab::AuditDecision::Denied,
                        }),
                        denial: match entry.record.decision {
                            Decision::Allowed => None,
                            Decision::Denied(denial) => Some(denial_to_fidl(denial)),
                        },
                        status: Some(match entry.record.status {
                            OpStatus::Ok => flab::AuditStatus::Ok,
                            OpStatus::Rejected => flab::AuditStatus::Rejected,
                            OpStatus::BackendFault => flab::AuditStatus::BackendFault,
                        }),
                        value: entry.record.value,
                        timestamp_ns: Some(entry.record.timestamp_ns),
                        run_id: entry.record.run_id.clone(),
                        item_index: entry.record.item_index,
                        proxy_generation: Some(proxy_generation),
                        boot_id: Some(boot_id.clone()),
                        ..Default::default()
                    })
                    .collect();
                let _ = responder.send(
                    &entries,
                    page.oldest_retained.is_some(),
                    page.oldest_retained.unwrap_or(0),
                    page.next_cursor,
                );
            }
            flab::SessionRequest::_UnknownMethod { .. } => {}
        }
    }

    // Channel closed: close the session, release the mutation lease, and
    // audit the closure.
    let mut state = state.lock().unwrap();
    if state.sessions.close_session(id) {
        let mut record = AuditRecord::lifecycle("session_closed", now_ns());
        record.session = Some(id);
        state.audit.append(record);
    }
}
