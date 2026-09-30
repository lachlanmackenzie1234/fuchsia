// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Layered access validation for proxy operations.
//!
//! Every hardware access must be permitted by every layer: actual resource
//! bounds, the immutable target ceiling, the exact session allowlist, and
//! per-operation structural validation (width, alignment, and overflow).
//! Rejection at any layer denies the access without touching hardware.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Identifies a resource within one proxy instance.
pub type ResourceId = u32;

/// The only MMIO access width supported by the V1 wire contract, in bytes.
pub const WIDTH32: u32 = 4;

/// Access classes distinguished by policy. A grant for one class never
/// authorizes another: a one-shot read grant does not authorize polling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AccessClass {
    /// A single 32-bit read.
    ReadOnce,
    /// A read performed as part of a bounded snapshot.
    Snapshot,
    /// A repeated bounded read. Not supported by the V1 read-only executor.
    Poll,
    /// A masked write. Not supported by the V1 read-only executor.
    Write,
}

/// Description of an MMIO resource offered by the parent node. Descriptions
/// never contain physical addresses or raw handles.
#[derive(Clone, Debug)]
pub struct MmioResource {
    /// Stable logical name.
    pub name: String,
    /// Size of the logical resource in bytes.
    pub logical_size: u64,
    /// Size of the actual backing mapping in bytes. Accesses must fit both
    /// the logical and the mapped size.
    pub mapped_size: u64,
}

/// Immutable per-resource read ceiling. A resource with no ceiling entry
/// permits nothing.
#[derive(Clone, Debug, Default)]
pub struct ResourceCeiling {
    /// Byte ranges that may never be read, even with operator consent.
    pub hard_denied: Vec<Range<u64>>,
    /// Whether otherwise-unclassified reads may be enabled by an exact
    /// session allowlist rule.
    pub allow_unknown_reads: bool,
}

/// One exact session allowlist rule. Wildcards do not exist: a rule matches
/// exactly one resource, offset, width, and access class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccessRule {
    /// Resource the rule applies to.
    pub resource: ResourceId,
    /// Byte offset from the start of the logical resource.
    pub offset: u64,
    /// Access width in bytes.
    pub width: u32,
    /// Access class the rule authorizes.
    pub class: AccessClass,
}

/// Why an access or allowlist rule was rejected. Reasons stay
/// machine-readable; they must never exist only in a human detail string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    /// The resource ID is not offered by this proxy instance.
    UnknownResource,
    /// The requested width is not supported.
    UnsupportedWidth,
    /// The requested access class is not supported by the V1 executor.
    UnsupportedAccessClass,
    /// The offset is not aligned to the access width.
    Misaligned,
    /// `offset + width` overflows.
    OffsetOverflow,
    /// The access does not fit the logical resource size.
    OutOfLogicalBounds,
    /// The access does not fit the actual backing mapping.
    OutOfMappedBounds,
    /// The target ceiling has no entry for this resource.
    NotPermittedByCeiling,
    /// The offset lies in a hard-denied range; operator consent cannot
    /// enable it.
    HardDenied,
    /// The ceiling does not permit operator-authorized unknown reads on
    /// this resource.
    UnknownReadsNotPermitted,
    /// No exact session allowlist rule matches the access.
    NotInAllowlist,
    /// A bounded-execution limit (for example maximum snapshot items) was
    /// exceeded.
    LimitExceeded,
    /// A session-open expectation (boot, generation, or digest) was stale.
    /// Used in audit records for rejected session opens.
    StaleIdentity,
    /// The exclusive mutation lease was already held.
    MutationLeaseContention,
    /// The proxy is stopping and accepts no new sessions.
    NotAccepting,
    /// The request set a field this phase does not implement (for
    /// example a phase 2 takeover expectation). Fail closed rather than
    /// silently ignoring the field.
    UnsupportedExpectation,
}

/// The validated policy engine for one session.
///
/// Construction validates the entire allowlist against the resources and
/// ceiling; a rejected allowlist creates no engine and therefore no
/// session, and no hardware operation is performed.
#[derive(Debug)]
pub struct AccessPolicy {
    resources: BTreeMap<ResourceId, MmioResource>,
    ceiling: BTreeMap<ResourceId, ResourceCeiling>,
    allowlist: BTreeSet<AccessRule>,
}

impl AccessPolicy {
    /// Builds a policy engine, validating every allowlist rule structurally
    /// and against the ceiling. Returns the first offending rule and the
    /// reason on failure.
    pub fn new(
        resources: BTreeMap<ResourceId, MmioResource>,
        ceiling: BTreeMap<ResourceId, ResourceCeiling>,
        allowlist: impl IntoIterator<Item = AccessRule>,
    ) -> Result<Self, (AccessRule, Denial)> {
        let allowlist: BTreeSet<AccessRule> = allowlist.into_iter().collect();
        let policy = Self { resources, ceiling, allowlist };
        for rule in &policy.allowlist {
            policy.validate_rule(rule).map_err(|denial| (*rule, denial))?;
        }
        Ok(policy)
    }

    /// IDs of every resource this policy knows about.
    pub fn resource_ids(&self) -> impl Iterator<Item = ResourceId> {
        self.resources.keys().copied()
    }

    /// Immediately-before-access validation for a 32-bit read. This covers
    /// resource lookup, width, alignment, overflow, logical and mapped
    /// bounds, ceiling hard denies, unknown-read permission, and the exact
    /// allowlist match. Session/generation staleness is the session
    /// server's job and the volatile read itself is the executor's.
    pub fn check_read32(
        &self,
        resource: ResourceId,
        offset: u64,
        class: AccessClass,
    ) -> Result<(), Denial> {
        if !matches!(class, AccessClass::ReadOnce | AccessClass::Snapshot) {
            return Err(Denial::UnsupportedAccessClass);
        }
        self.validate_read(resource, offset, WIDTH32)?;
        let rule = AccessRule { resource, offset, width: WIDTH32, class };
        if !self.allowlist.contains(&rule) {
            return Err(Denial::NotInAllowlist);
        }
        Ok(())
    }

    /// Pre-session validation of one allowlist rule.
    fn validate_rule(&self, rule: &AccessRule) -> Result<(), Denial> {
        if !matches!(rule.class, AccessClass::ReadOnce | AccessClass::Snapshot) {
            return Err(Denial::UnsupportedAccessClass);
        }
        self.validate_read(rule.resource, rule.offset, rule.width)
    }

    /// Structural and ceiling checks shared by rule prevalidation and
    /// per-access validation.
    fn validate_read(&self, resource: ResourceId, offset: u64, width: u32) -> Result<(), Denial> {
        let desc = self.resources.get(&resource).ok_or(Denial::UnknownResource)?;
        if width != WIDTH32 {
            return Err(Denial::UnsupportedWidth);
        }
        if offset % u64::from(width) != 0 {
            return Err(Denial::Misaligned);
        }
        let end = offset.checked_add(u64::from(width)).ok_or(Denial::OffsetOverflow)?;
        if end > desc.logical_size {
            return Err(Denial::OutOfLogicalBounds);
        }
        if end > desc.mapped_size {
            return Err(Denial::OutOfMappedBounds);
        }
        let ceiling = self.ceiling.get(&resource).ok_or(Denial::NotPermittedByCeiling)?;
        if ceiling.hard_denied.iter().any(|range| range.start < end && offset < range.end) {
            return Err(Denial::HardDenied);
        }
        if !ceiling.allow_unknown_reads {
            return Err(Denial::UnknownReadsNotPermitted);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL: ResourceId = 1;
    const LOCKED: ResourceId = 2;
    const NO_CEILING: ResourceId = 3;
    const SHORT_MAP: ResourceId = 4;

    fn resources() -> BTreeMap<ResourceId, MmioResource> {
        BTreeMap::from([
            (
                CTRL,
                MmioResource { name: "ctrl".to_string(), logical_size: 0x100, mapped_size: 0x1000 },
            ),
            (
                LOCKED,
                MmioResource {
                    name: "locked".to_string(),
                    logical_size: 0x100,
                    mapped_size: 0x100,
                },
            ),
            (
                NO_CEILING,
                MmioResource {
                    name: "no-ceiling".to_string(),
                    logical_size: 0x100,
                    mapped_size: 0x100,
                },
            ),
            (
                SHORT_MAP,
                MmioResource {
                    name: "short-map".to_string(),
                    logical_size: 0x2000,
                    mapped_size: 0x10,
                },
            ),
        ])
    }

    fn ceiling() -> BTreeMap<ResourceId, ResourceCeiling> {
        BTreeMap::from([
            (CTRL, ResourceCeiling { hard_denied: vec![0x40..0x44], allow_unknown_reads: true }),
            (LOCKED, ResourceCeiling { hard_denied: vec![], allow_unknown_reads: false }),
            (SHORT_MAP, ResourceCeiling { hard_denied: vec![], allow_unknown_reads: true }),
        ])
    }

    fn read_rule(resource: ResourceId, offset: u64) -> AccessRule {
        AccessRule { resource, offset, width: WIDTH32, class: AccessClass::ReadOnce }
    }

    fn policy(rules: &[AccessRule]) -> AccessPolicy {
        AccessPolicy::new(resources(), ceiling(), rules.iter().copied()).unwrap()
    }

    #[test]
    fn exact_read_allowed() {
        let policy = policy(&[read_rule(CTRL, 0x3c)]);
        assert_eq!(policy.check_read32(CTRL, 0x3c, AccessClass::ReadOnce), Ok(()));
    }

    #[test]
    fn nearby_offset_not_authorized() {
        let policy = policy(&[read_rule(CTRL, 0x3c)]);
        assert_eq!(
            policy.check_read32(CTRL, 0x38, AccessClass::ReadOnce),
            Err(Denial::NotInAllowlist)
        );
    }

    #[test]
    fn access_classes_are_distinct() {
        let policy = policy(&[read_rule(CTRL, 0x3c)]);
        assert_eq!(
            policy.check_read32(CTRL, 0x3c, AccessClass::Snapshot),
            Err(Denial::NotInAllowlist)
        );
        assert_eq!(
            policy.check_read32(CTRL, 0x3c, AccessClass::Poll),
            Err(Denial::UnsupportedAccessClass)
        );
        assert_eq!(
            policy.check_read32(CTRL, 0x3c, AccessClass::Write),
            Err(Denial::UnsupportedAccessClass)
        );
    }

    #[test]
    fn unknown_resource_denied() {
        let policy = policy(&[]);
        assert_eq!(
            policy.check_read32(99, 0x0, AccessClass::ReadOnce),
            Err(Denial::UnknownResource)
        );
    }

    #[test]
    fn misaligned_denied() {
        let policy = policy(&[read_rule(CTRL, 0x3c)]);
        assert_eq!(policy.check_read32(CTRL, 0x3e, AccessClass::ReadOnce), Err(Denial::Misaligned));
    }

    #[test]
    fn offset_overflow_denied() {
        let policy = policy(&[]);
        // u64::MAX - 3 is 4-byte aligned, so this reaches the overflow
        // check rather than the alignment check.
        assert_eq!(
            policy.check_read32(CTRL, u64::MAX - 3, AccessClass::ReadOnce),
            Err(Denial::OffsetOverflow)
        );
    }

    #[test]
    fn logical_bounds_enforced() {
        let policy = policy(&[read_rule(CTRL, 0xfc)]);
        assert_eq!(policy.check_read32(CTRL, 0xfc, AccessClass::ReadOnce), Ok(()));
        assert_eq!(
            policy.check_read32(CTRL, 0x100, AccessClass::ReadOnce),
            Err(Denial::OutOfLogicalBounds)
        );
    }

    #[test]
    fn mapped_bounds_enforced() {
        let policy = policy(&[read_rule(SHORT_MAP, 0xc)]);
        assert_eq!(policy.check_read32(SHORT_MAP, 0xc, AccessClass::ReadOnce), Ok(()));
        assert_eq!(
            policy.check_read32(SHORT_MAP, 0x10, AccessClass::ReadOnce),
            Err(Denial::OutOfMappedBounds)
        );
    }

    #[test]
    fn hard_denied_wins_over_allowlist() {
        // The rule cannot even enter the allowlist.
        let denied = AccessPolicy::new(resources(), ceiling(), [read_rule(CTRL, 0x40)]);
        assert_eq!(denied.unwrap_err(), (read_rule(CTRL, 0x40), Denial::HardDenied));
        // And a direct check is denied before the allowlist is consulted.
        let policy = policy(&[]);
        assert_eq!(policy.check_read32(CTRL, 0x40, AccessClass::ReadOnce), Err(Denial::HardDenied));
    }

    #[test]
    fn read_adjacent_to_hard_denied_range_allowed() {
        let policy = policy(&[read_rule(CTRL, 0x44)]);
        assert_eq!(policy.check_read32(CTRL, 0x44, AccessClass::ReadOnce), Ok(()));
    }

    #[test]
    fn unknown_reads_disabled_by_ceiling() {
        let denied = AccessPolicy::new(resources(), ceiling(), [read_rule(LOCKED, 0x0)]);
        assert_eq!(denied.unwrap_err(), (read_rule(LOCKED, 0x0), Denial::UnknownReadsNotPermitted));
    }

    #[test]
    fn missing_ceiling_entry_permits_nothing() {
        let policy = policy(&[]);
        assert_eq!(
            policy.check_read32(NO_CEILING, 0x0, AccessClass::ReadOnce),
            Err(Denial::NotPermittedByCeiling)
        );
    }

    #[test]
    fn allowlist_prevalidation_rejects_unsupported_class() {
        let rule =
            AccessRule { resource: CTRL, offset: 0x0, width: WIDTH32, class: AccessClass::Poll };
        let denied = AccessPolicy::new(resources(), ceiling(), [rule]);
        assert_eq!(denied.unwrap_err(), (rule, Denial::UnsupportedAccessClass));
    }

    #[test]
    fn allowlist_prevalidation_rejects_bad_width() {
        let rule =
            AccessRule { resource: CTRL, offset: 0x0, width: 8, class: AccessClass::ReadOnce };
        let denied = AccessPolicy::new(resources(), ceiling(), [rule]);
        assert_eq!(denied.unwrap_err(), (rule, Denial::UnsupportedWidth));
    }

    #[test]
    fn allowlist_prevalidation_rejects_out_of_bounds() {
        let rule = read_rule(CTRL, 0x200);
        let denied = AccessPolicy::new(resources(), ceiling(), [rule]);
        assert_eq!(denied.unwrap_err(), (rule, Denial::OutOfLogicalBounds));
    }
}
