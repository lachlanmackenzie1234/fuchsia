// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Read-path executor: policy check, hardware access, and audit for scalar
//! reads and bounded snapshots.
//!
//! Every attempted operation is audited, including rejections, which never
//! touch hardware. A snapshot validates every item before the first read
//! and is an ordered series of reads, not an atomic hardware snapshot.

use crate::access_policy::{AccessClass, AccessPolicy, Denial, ResourceId};
use crate::audit_ring::{AuditRecord, AuditRing, Decision, OpStatus};
use crate::hardware_backend::{BackendError, Clock, MmioBackend};
use std::collections::BTreeMap;

/// Instance-wide execution limits, derived from the target ceiling.
#[derive(Clone, Copy, Debug)]
pub struct ExecLimits {
    /// Maximum number of items in one snapshot.
    pub max_snapshot_items: usize,
}

/// One requested snapshot read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotItem {
    /// Resource to read.
    pub resource: ResourceId,
    /// Byte offset from the start of the logical resource.
    pub offset: u64,
}

/// A completed scalar read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOutcome {
    /// The value read.
    pub value: u32,
    /// Audit sequence assigned to the read.
    pub audit_seq: u64,
    /// Timestamp of the read.
    pub timestamp_ns: i64,
}

/// Why a scalar read did not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Policy rejected the read; hardware was not touched.
    Denied {
        /// The rejecting policy layer's reason.
        denial: Denial,
        /// Audit sequence of the rejection record.
        audit_seq: u64,
    },
    /// The backend failed after policy allowed the read.
    Backend {
        /// The backend error.
        error: BackendError,
        /// Audit sequence of the failure record.
        audit_seq: u64,
    },
}

/// The per-item result of a snapshot read that began execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotItemResult {
    /// The requested item.
    pub item: SnapshotItem,
    /// The value read, or the backend error.
    pub value: Result<u32, BackendError>,
    /// Audit sequence assigned to this item.
    pub audit_seq: u64,
    /// Timestamp of this item's read.
    pub timestamp_ns: i64,
}

/// Outcome of a snapshot whose prevalidation succeeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotOutcome {
    /// One result per item that began execution, in request order.
    pub results: Vec<SnapshotItemResult>,
    /// Whether every requested item completed successfully. When false the
    /// snapshot stopped at the first backend failure and later items never
    /// began.
    pub complete: bool,
}

/// Why a snapshot was rejected before any hardware access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// More items than the execution limit permits.
    TooManyItems {
        /// The enforced limit.
        max: usize,
        /// Audit sequence of the rejection record.
        audit_seq: u64,
    },
    /// An item failed prevalidation; no item was executed.
    Rejected {
        /// Index of the first offending item.
        index: usize,
        /// The rejecting policy layer's reason.
        denial: Denial,
        /// Audit sequence of the rejection record.
        audit_seq: u64,
    },
}

/// Read-path executor shared by all sessions. Each call takes the calling
/// session's validated [`AccessPolicy`].
#[derive(Debug)]
pub struct Executor<B, C> {
    backends: BTreeMap<ResourceId, B>,
    clock: C,
    limits: ExecLimits,
}

impl<B: MmioBackend, C: Clock> Executor<B, C> {
    /// Creates an executor over the instance's backends.
    pub fn new(backends: BTreeMap<ResourceId, B>, clock: C, limits: ExecLimits) -> Self {
        Self { backends, clock, limits }
    }

    /// The execution limits, for capability reporting.
    pub fn limits(&self) -> ExecLimits {
        self.limits
    }

    /// The backend for `resource`, for diagnostics and tests.
    pub fn backend(&self, resource: ResourceId) -> Option<&B> {
        self.backends.get(&resource)
    }

    /// Performs one policy-checked, audited 32-bit read.
    pub fn read32(
        &mut self,
        policy: &AccessPolicy,
        audit: &mut AuditRing,
        session: u64,
        resource: ResourceId,
        offset: u64,
    ) -> Result<ReadOutcome, ReadError> {
        let timestamp_ns = self.clock.now_ns();
        if let Err(denial) = policy.check_read32(resource, offset, AccessClass::ReadOnce) {
            let audit_seq = audit.append(op_record(
                session,
                resource,
                "read32",
                offset,
                Decision::Denied(denial),
                OpStatus::Rejected,
                None,
                timestamp_ns,
            ));
            return Err(ReadError::Denied { denial, audit_seq });
        }
        let Some(backend) = self.backends.get_mut(&resource) else {
            let audit_seq = audit.append(op_record(
                session,
                resource,
                "read32",
                offset,
                Decision::Allowed,
                OpStatus::BackendFault,
                None,
                timestamp_ns,
            ));
            return Err(ReadError::Backend { error: BackendError::Fault, audit_seq });
        };
        match backend.read32(offset) {
            Ok(value) => {
                let audit_seq = audit.append(op_record(
                    session,
                    resource,
                    "read32",
                    offset,
                    Decision::Allowed,
                    OpStatus::Ok,
                    Some(value),
                    timestamp_ns,
                ));
                Ok(ReadOutcome { value, audit_seq, timestamp_ns })
            }
            Err(error) => {
                let audit_seq = audit.append(op_record(
                    session,
                    resource,
                    "read32",
                    offset,
                    Decision::Allowed,
                    OpStatus::BackendFault,
                    None,
                    timestamp_ns,
                ));
                Err(ReadError::Backend { error, audit_seq })
            }
        }
    }

    /// Performs a bounded snapshot: every item is validated before the
    /// first read, then items execute in order, stopping at the first
    /// backend failure.
    pub fn snapshot(
        &mut self,
        policy: &AccessPolicy,
        audit: &mut AuditRing,
        session: u64,
        items: &[SnapshotItem],
    ) -> Result<SnapshotOutcome, SnapshotError> {
        if items.len() > self.limits.max_snapshot_items {
            let timestamp_ns = self.clock.now_ns();
            let audit_seq = audit.append(AuditRecord {
                session: Some(session),
                resource: None,
                operation: "snapshot",
                offset: None,
                decision: Decision::Denied(Denial::LimitExceeded),
                status: OpStatus::Rejected,
                value: None,
                timestamp_ns,
                run_id: None,
                item_index: None,
            });
            return Err(SnapshotError::TooManyItems {
                max: self.limits.max_snapshot_items,
                audit_seq,
            });
        }
        for (index, item) in items.iter().enumerate() {
            if let Err(denial) =
                policy.check_read32(item.resource, item.offset, AccessClass::Snapshot)
            {
                let timestamp_ns = self.clock.now_ns();
                let mut record = op_record(
                    session,
                    item.resource,
                    "snapshot",
                    item.offset,
                    Decision::Denied(denial),
                    OpStatus::Rejected,
                    None,
                    timestamp_ns,
                );
                record.item_index = Some(index as u32);
                let audit_seq = audit.append(record);
                return Err(SnapshotError::Rejected { index, denial, audit_seq });
            }
        }
        let mut results = Vec::with_capacity(items.len());
        let mut complete = true;
        for (index, item) in items.iter().enumerate() {
            let timestamp_ns = self.clock.now_ns();
            let value = match self.backends.get_mut(&item.resource) {
                Some(backend) => backend.read32(item.offset),
                None => Err(BackendError::Fault),
            };
            let (status, audited_value) = match value {
                Ok(read) => (OpStatus::Ok, Some(read)),
                Err(_) => (OpStatus::BackendFault, None),
            };
            let mut record = op_record(
                session,
                item.resource,
                "snapshot_read32",
                item.offset,
                Decision::Allowed,
                status,
                audited_value,
                timestamp_ns,
            );
            record.item_index = Some(index as u32);
            let audit_seq = audit.append(record);
            results.push(SnapshotItemResult { item: *item, value, audit_seq, timestamp_ns });
            if value.is_err() {
                complete = false;
                break;
            }
        }
        Ok(SnapshotOutcome { results, complete })
    }
}

fn op_record(
    session: u64,
    resource: ResourceId,
    operation: &'static str,
    offset: u64,
    decision: Decision,
    status: OpStatus,
    value: Option<u32>,
    timestamp_ns: i64,
) -> AuditRecord {
    AuditRecord {
        session: Some(session),
        resource: Some(resource),
        operation,
        offset: Some(offset),
        decision,
        status,
        value,
        timestamp_ns,
        run_id: None,
        item_index: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access_policy::{AccessRule, MmioResource, ResourceCeiling, WIDTH32};
    use crate::hardware_backend::{FakeClock, FakeMmio};

    const CTRL: ResourceId = 1;
    const SESSION: u64 = 7;

    fn rule(offset: u64, class: AccessClass) -> AccessRule {
        AccessRule { resource: CTRL, offset, width: WIDTH32, class }
    }

    fn make_executor(
        rules: &[AccessRule],
    ) -> (Executor<FakeMmio, FakeClock>, AccessPolicy, AuditRing) {
        let resources = BTreeMap::from([(
            CTRL,
            MmioResource { name: "ctrl".to_string(), logical_size: 0x100, mapped_size: 0x100 },
        )]);
        let ceiling = BTreeMap::from([(
            CTRL,
            ResourceCeiling { hard_denied: vec![], allow_unknown_reads: true, allow_poll: false },
        )]);
        let policy = AccessPolicy::new(resources, ceiling, rules.iter().copied()).unwrap();
        let mut mmio = FakeMmio::new();
        mmio.set(0x3c, 0xdead_beef);
        mmio.set(0x40, 0x1234_5678);
        mmio.fail_at(0x44);
        let backends = BTreeMap::from([(CTRL, mmio)]);
        let clock = FakeClock { now: 1000, step: 10 };
        let executor = Executor::new(backends, clock, ExecLimits { max_snapshot_items: 4 });
        (executor, policy, AuditRing::new(16))
    }

    fn accesses(executor: &Executor<FakeMmio, FakeClock>) -> &[u64] {
        &executor.backend(CTRL).unwrap().accesses
    }

    #[test]
    fn read_allowed_returns_value_and_audits() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x3c, AccessClass::ReadOnce)]);
        let outcome = executor.read32(&policy, &mut audit, SESSION, CTRL, 0x3c).unwrap();
        assert_eq!(outcome.value, 0xdead_beef);
        assert_eq!(outcome.timestamp_ns, 1000);
        assert_eq!(accesses(&executor), &[0x3c]);

        let entry = &audit.read(outcome.audit_seq, 1).entries[0];
        assert_eq!(entry.record.decision, Decision::Allowed);
        assert_eq!(entry.record.status, OpStatus::Ok);
        assert_eq!(entry.record.value, Some(0xdead_beef));
        assert_eq!(entry.record.session, Some(SESSION));
    }

    #[test]
    fn denied_read_touches_no_hardware_but_is_audited() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x3c, AccessClass::ReadOnce)]);
        let error = executor.read32(&policy, &mut audit, SESSION, CTRL, 0x40).unwrap_err();
        let ReadError::Denied { denial, audit_seq } = error else {
            panic!("expected denial, got {error:?}");
        };
        assert_eq!(denial, Denial::NotInAllowlist);
        assert!(accesses(&executor).is_empty());

        let entry = &audit.read(audit_seq, 1).entries[0];
        assert_eq!(entry.record.decision, Decision::Denied(Denial::NotInAllowlist));
        assert_eq!(entry.record.status, OpStatus::Rejected);
        assert_eq!(entry.record.value, None);
    }

    #[test]
    fn read_backend_fault_is_audited() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x44, AccessClass::ReadOnce)]);
        let error = executor.read32(&policy, &mut audit, SESSION, CTRL, 0x44).unwrap_err();
        let ReadError::Backend { error, audit_seq } = error else {
            panic!("expected backend error, got {error:?}");
        };
        assert_eq!(error, BackendError::Fault);

        let entry = &audit.read(audit_seq, 1).entries[0];
        assert_eq!(entry.record.decision, Decision::Allowed);
        assert_eq!(entry.record.status, OpStatus::BackendFault);
    }

    #[test]
    fn snapshot_reads_in_order_with_distinct_timestamps() {
        let (mut executor, policy, mut audit) =
            make_executor(&[rule(0x3c, AccessClass::Snapshot), rule(0x40, AccessClass::Snapshot)]);
        let items = [
            SnapshotItem { resource: CTRL, offset: 0x3c },
            SnapshotItem { resource: CTRL, offset: 0x40 },
        ];
        let outcome = executor.snapshot(&policy, &mut audit, SESSION, &items).unwrap();
        assert!(outcome.complete);
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(outcome.results[0].value, Ok(0xdead_beef));
        assert_eq!(outcome.results[1].value, Ok(0x1234_5678));
        assert!(outcome.results[0].timestamp_ns < outcome.results[1].timestamp_ns);
        assert!(outcome.results[0].audit_seq < outcome.results[1].audit_seq);
        assert_eq!(accesses(&executor), &[0x3c, 0x40]);
    }

    #[test]
    fn snapshot_prevalidation_blocks_every_read() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x3c, AccessClass::Snapshot)]);
        let items = [
            SnapshotItem { resource: CTRL, offset: 0x3c },
            SnapshotItem { resource: CTRL, offset: 0x48 },
        ];
        let error = executor.snapshot(&policy, &mut audit, SESSION, &items).unwrap_err();
        let SnapshotError::Rejected { index, denial, .. } = error else {
            panic!("expected rejection, got {error:?}");
        };
        assert_eq!(index, 1);
        assert_eq!(denial, Denial::NotInAllowlist);
        assert!(accesses(&executor).is_empty());
    }

    #[test]
    fn read_once_grant_does_not_authorize_snapshot() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x3c, AccessClass::ReadOnce)]);
        let items = [SnapshotItem { resource: CTRL, offset: 0x3c }];
        let error = executor.snapshot(&policy, &mut audit, SESSION, &items).unwrap_err();
        let SnapshotError::Rejected { index, denial, .. } = error else {
            panic!("expected rejection, got {error:?}");
        };
        assert_eq!(index, 0);
        assert_eq!(denial, Denial::NotInAllowlist);
        assert!(accesses(&executor).is_empty());
    }

    #[test]
    fn snapshot_stops_at_first_backend_fault() {
        let (mut executor, policy, mut audit) = make_executor(&[
            rule(0x3c, AccessClass::Snapshot),
            rule(0x44, AccessClass::Snapshot),
            rule(0x40, AccessClass::Snapshot),
        ]);
        let items = [
            SnapshotItem { resource: CTRL, offset: 0x3c },
            SnapshotItem { resource: CTRL, offset: 0x44 },
            SnapshotItem { resource: CTRL, offset: 0x40 },
        ];
        let outcome = executor.snapshot(&policy, &mut audit, SESSION, &items).unwrap();
        assert!(!outcome.complete);
        assert_eq!(outcome.results.len(), 2);
        assert_eq!(outcome.results[0].value, Ok(0xdead_beef));
        assert_eq!(outcome.results[1].value, Err(BackendError::Fault));
        // The third item never began.
        assert_eq!(accesses(&executor), &[0x3c, 0x44]);

        let entry = &audit.read(outcome.results[1].audit_seq, 1).entries[0];
        assert_eq!(entry.record.status, OpStatus::BackendFault);
    }

    #[test]
    fn snapshot_item_limit_enforced() {
        let (mut executor, policy, mut audit) = make_executor(&[rule(0x3c, AccessClass::Snapshot)]);
        let items = [SnapshotItem { resource: CTRL, offset: 0x3c }; 5];
        let error = executor.snapshot(&policy, &mut audit, SESSION, &items).unwrap_err();
        let SnapshotError::TooManyItems { max, audit_seq } = error else {
            panic!("expected limit rejection, got {error:?}");
        };
        assert_eq!(max, 4);
        assert!(accesses(&executor).is_empty());

        let entry = &audit.read(audit_seq, 1).entries[0];
        assert_eq!(entry.record.decision, Decision::Denied(Denial::LimitExceeded));
    }

    #[test]
    fn missing_backend_is_an_audited_fault() {
        let resources = BTreeMap::from([(
            CTRL,
            MmioResource { name: "ctrl".to_string(), logical_size: 0x100, mapped_size: 0x100 },
        )]);
        let ceiling = BTreeMap::from([(
            CTRL,
            ResourceCeiling { hard_denied: vec![], allow_unknown_reads: true, allow_poll: false },
        )]);
        let policy =
            AccessPolicy::new(resources, ceiling, [rule(0x3c, AccessClass::ReadOnce)]).unwrap();
        let clock = FakeClock { now: 0, step: 1 };
        let mut executor = Executor::<FakeMmio, _>::new(
            BTreeMap::new(),
            clock,
            ExecLimits { max_snapshot_items: 4 },
        );
        let mut audit = AuditRing::new(4);
        let error = executor.read32(&policy, &mut audit, SESSION, CTRL, 0x3c).unwrap_err();
        let ReadError::Backend { error, audit_seq } = error else {
            panic!("expected backend error, got {error:?}");
        };
        assert_eq!(error, BackendError::Fault);
        let entry = &audit.read(audit_seq, 1).entries[0];
        assert_eq!(entry.record.status, OpStatus::BackendFault);
    }
}
