// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};

use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use rapidhash::RapidBuildHasher;

use super::error::{ParseError, SerializeError, ValidateError};
use super::parser::PolicyCursor;
use super::traits::{Parse, Serialize, Validate};
use super::{AccessVector, ClassId, NewPolicy, TypeId, U24Index};

pub use selinux_policy_derive::{Parse, Serialize};

// Constants
/// Flag bit for standard `allow` rules.
pub const AV_ALLOW_RULE_FLAG: u16 = 0x1;
/// Flag bit for `auditallow` rules.
pub const AV_AUDITALLOW_RULE_FLAG: u16 = 0x2;
/// Flag bit for `dontaudit` rules.
pub const AV_DONTAUDIT_RULE_FLAG: u16 = 0x4;

/// Flag bit for `type_transition` rules.
pub const AV_TYPE_TRANSITION_RULE_FLAG: u16 = 0x10;
/// Flag bit for `type_member` rules.
pub const AV_TYPE_MEMBER_RULE_FLAG: u16 = 0x20;
/// Flag bit for `type_change` rules.
pub const AV_TYPE_CHANGE_RULE_FLAG: u16 = 0x40;

/// Flag bit for `allowxperm` extended permissions rules.
pub const AV_ALLOWXPERM_RULE_FLAG: u16 = 0x100;
/// Flag bit for `auditallowxperm` extended permissions rules.
pub const AV_AUDITALLOWXPERM_RULE_FLAG: u16 = 0x200;
/// Flag bit for `dontauditxperm` extended permissions rules.
pub const AV_DONTAUDITXPERM_RULE_FLAG: u16 = 0x400;

/// Mask for high bit in rule type flags indicating whether rule is enabled.
pub const AV_ENABLED_RULE_FLAG: u16 = 0x8000;

/// [`AccessDecision::flags`] value indicating that policy marks source domain permissive.
pub const SELINUX_AVD_FLAGS_PERMISSIVE: u32 = 1;

/// Extended permissions type for ioctl driver prefix and 8-bit postfix sets.
pub const XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES: u8 = 1;
/// Extended permissions type for ioctl 8-bit driver prefixes.
pub const XPERMS_TYPE_IOCTL_PREFIXES: u8 = 2;
/// Extended permissions type for netlink message types.
pub const XPERMS_TYPE_NLMSG: u8 = 3;

/// Number of 64-bit words in 256-bit [`XpermsBitmap`].
pub const XPERMS_BITMAP_BLOCKS: usize = 4;

/// 256-bit bitmap used for extended permissions (such as ioctls and netlink messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct XpermsBitmap([u64; XPERMS_BITMAP_BLOCKS]);

impl Parse for XpermsBitmap {
    fn parse(cursor: &mut PolicyCursor<'_>) -> Result<Self, ParseError> {
        let mut words = [0u64; XPERMS_BITMAP_BLOCKS];
        for word in words.iter_mut() {
            let low = cursor.parse::<u32>()? as u64;
            let high = cursor.parse::<u32>()? as u64;
            *word = low | (high << 32);
        }
        Ok(Self(words))
    }
}

impl Serialize for XpermsBitmap {
    fn serialize(&self, writer: &mut Vec<u8>) -> Result<(), SerializeError> {
        for &word in self.0.iter() {
            (word as u32).serialize(writer)?;
            ((word >> 32) as u32).serialize(writer)?;
        }
        Ok(())
    }
}

impl XpermsBitmap {
    pub const BITMAP_BLOCKS: usize = XPERMS_BITMAP_BLOCKS;
    /// Bitmap with all 256 bits set to 1.
    pub const ALL: Self = Self([u64::MAX; Self::BITMAP_BLOCKS]);
    /// Empty bitmap with all bits set to 0.
    pub const NONE: Self = Self([0u64; Self::BITMAP_BLOCKS]);

    /// Constructs a new [`XpermsBitmap`] from an array of four 64-bit words.
    pub fn new(elements: [u64; Self::BITMAP_BLOCKS]) -> Self {
        Self(elements)
    }

    /// Returns `true` if the bit corresponding to `value` is set in this bitmap.
    pub fn contains(&self, value: u8) -> bool {
        let block_index = (value as usize) / (u64::BITS as usize);
        let bit_index = (value as usize) % (u64::BITS as usize);
        self.0[block_index] & (1u64 << bit_index) != 0
    }

    /// Constructs an [`XpermsBitmap`] by loading words from an array of atomic 64-bit integers using relaxed ordering.
    pub fn from_atomics(atomics: &[AtomicU64; Self::BITMAP_BLOCKS]) -> Self {
        let mut words = [0u64; Self::BITMAP_BLOCKS];
        for (i, word) in words.iter_mut().enumerate() {
            *word = atomics[i].load(Ordering::Relaxed);
        }
        Self(words)
    }

    /// Stores this bitmap into an array of atomic 64-bit integers using relaxed ordering.
    pub fn to_atomics(&self, atomics: &[AtomicU64; Self::BITMAP_BLOCKS]) {
        for (i, word) in self.0.iter().enumerate() {
            atomics[i].store(*word, Ordering::Relaxed);
        }
    }
}

impl std::ops::BitAnd for XpermsBitmap {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(std::array::from_fn(|i| self.0[i] & rhs.0[i]))
    }
}

impl std::ops::BitAndAssign for XpermsBitmap {
    fn bitand_assign(&mut self, rhs: Self) {
        for i in 0..4 {
            self.0[i] &= rhs.0[i];
        }
    }
}

impl std::ops::BitOr for XpermsBitmap {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(std::array::from_fn(|i| self.0[i] | rhs.0[i]))
    }
}

impl std::ops::BitOrAssign for XpermsBitmap {
    fn bitor_assign(&mut self, rhs: Self) {
        for i in 0..4 {
            self.0[i] |= rhs.0[i];
        }
    }
}

impl std::ops::Sub for XpermsBitmap {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(std::array::from_fn(|i| self.0[i] & !rhs.0[i]))
    }
}

impl std::ops::SubAssign for XpermsBitmap {
    fn sub_assign(&mut self, rhs: Self) {
        for i in 0..4 {
            self.0[i] &= !rhs.0[i];
        }
    }
}

impl std::ops::Not for XpermsBitmap {
    type Output = Self;
    fn not(self) -> Self::Output {
        Self(self.0.map(|word| !word))
    }
}

impl Validate for XpermsBitmap {
    fn validate(&self, _policy: &NewPolicy) -> Result<(), ValidateError> {
        Ok(())
    }
}

/// Extended permissions specification (e.g., ioctl commands or netlink message types) associated with an access vector rule.
#[derive(Clone, Debug, PartialEq, Eq, Parse, Serialize)]
pub struct ExtendedPermissions {
    xperms_type: u8,
    xperms_optional_prefix: u8,
    xperms_bitmap: XpermsBitmap,
}

impl Validate for ExtendedPermissions {
    fn validate(&self, _policy: &NewPolicy) -> Result<(), ValidateError> {
        match self.xperms_type {
            XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES
            | XPERMS_TYPE_IOCTL_PREFIXES
            | XPERMS_TYPE_NLMSG => Ok(()),
            v => Err(ValidateError::InvalidExtendedPermissionsType { value: v }),
        }
    }
}

impl ExtendedPermissions {
    /// Returns the raw extended permissions type identifier (e.g. ioctl or netlink message format).
    pub fn xperms_type(&self) -> u8 {
        self.xperms_type
    }

    /// Returns the optional 8-bit prefix specified by this extended permissions block, if any.
    pub fn xperms_optional_prefix(&self) -> u8 {
        self.xperms_optional_prefix
    }

    /// Returns a reference to the underlying [`XpermsBitmap`].
    pub fn xperms_bitmap(&self) -> &XpermsBitmap {
        &self.xperms_bitmap
    }

    /// Returns the total number of individual permissions specified by this bitmap.
    #[cfg(test)]
    pub fn count(&self) -> u64 {
        let count = self
            .xperms_bitmap
            .0
            .iter()
            .fold(0, |count, block| count as u64 + block.count_ones() as u64);
        match self.xperms_type {
            XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES | XPERMS_TYPE_NLMSG => count,
            XPERMS_TYPE_IOCTL_PREFIXES => count * 0x100,
            _ => unreachable!("invalid xperms_type in validated ExtendedPermissions"),
        }
    }

    /// Returns `true` if the specified extended permission `xperm` is included in this rule.
    #[cfg(test)]
    pub fn contains(&self, xperm: u16) -> bool {
        let [postfix, prefix] = xperm.to_le_bytes();
        if (self.xperms_type == XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES
            || self.xperms_type == XPERMS_TYPE_NLMSG)
            && self.xperms_optional_prefix != prefix
        {
            return false;
        }
        let value = match self.xperms_type {
            XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES | XPERMS_TYPE_NLMSG => postfix,
            XPERMS_TYPE_IOCTL_PREFIXES => prefix,
            _ => unreachable!("invalid xperms_type in validated ExtendedPermissions"),
        };
        self.xperms_bitmap.contains(value)
    }
}

/// Compact enum identifying the type and target array for a rule in sequential policy order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuleKind {
    Allow,
    AuditAllow,
    DontAudit,
    TypeTransition,
    TypeMember,
    TypeChange,
    AllowXperm,
    AuditAllowXperm,
    DontAuditXperm,
}

impl TryFrom<u16> for RuleKind {
    type Error = ParseError;

    fn try_from(rule_type: u16) -> Result<Self, Self::Error> {
        let base_kind = rule_type & !AV_ENABLED_RULE_FLAG;
        match base_kind {
            AV_ALLOW_RULE_FLAG => Ok(Self::Allow),
            AV_AUDITALLOW_RULE_FLAG => Ok(Self::AuditAllow),
            AV_DONTAUDIT_RULE_FLAG => Ok(Self::DontAudit),
            AV_TYPE_TRANSITION_RULE_FLAG => Ok(Self::TypeTransition),
            AV_TYPE_MEMBER_RULE_FLAG => Ok(Self::TypeMember),
            AV_TYPE_CHANGE_RULE_FLAG => Ok(Self::TypeChange),
            AV_ALLOWXPERM_RULE_FLAG => Ok(Self::AllowXperm),
            AV_AUDITALLOWXPERM_RULE_FLAG => Ok(Self::AuditAllowXperm),
            AV_DONTAUDITXPERM_RULE_FLAG => Ok(Self::DontAuditXperm),
            _ => {
                Err(ParseError::InvalidEnumValue { enum_name: "RuleKind", value: rule_type as u64 })
            }
        }
    }
}

impl From<RuleKind> for u16 {
    fn from(kind: RuleKind) -> Self {
        match kind {
            RuleKind::Allow => AV_ALLOW_RULE_FLAG,
            RuleKind::AuditAllow => AV_AUDITALLOW_RULE_FLAG,
            RuleKind::DontAudit => AV_DONTAUDIT_RULE_FLAG,
            RuleKind::TypeTransition => AV_TYPE_TRANSITION_RULE_FLAG,
            RuleKind::TypeMember => AV_TYPE_MEMBER_RULE_FLAG,
            RuleKind::TypeChange => AV_TYPE_CHANGE_RULE_FLAG,
            RuleKind::AllowXperm => AV_ALLOWXPERM_RULE_FLAG,
            RuleKind::AuditAllowXperm => AV_AUDITALLOWXPERM_RULE_FLAG,
            RuleKind::DontAuditXperm => AV_DONTAUDITXPERM_RULE_FLAG,
        }
    }
}

/// Standard access vector rule (allow, auditallow, dontaudit).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRule {
    key: RuleKey,
    access_vector: AccessVector,
}

/// Type transition, change, or member rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeTransitionRule {
    key: RuleKey,
    new_type: TypeId,
}

/// Extended permissions rule (allowxperm, auditallowxperm, dontauditxperm).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XpermRule {
    key: RuleKey,
    kind: RuleKind,
    extended_permissions: ExtendedPermissions,
}

/// Lookup key for indexing and matching access vector rules by source domain, target domain, and class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuleKey {
    source_type: TypeId,
    target_type: TypeId,
    class: ClassId,
}

impl RuleKey {
    /// Constructs a [`RuleKey`] for the specified source domain, target domain, and security class.
    pub fn new(source_type: TypeId, target_type: TypeId, class: ClassId) -> Self {
        Self { source_type, target_type, class }
    }

    /// Hashes this [`RuleKey`] using `hasher`.
    pub fn hash(&self, hasher: &RapidBuildHasher) -> u64 {
        use std::hash::{BuildHasher, Hash, Hasher};
        let mut state = hasher.build_hasher();
        Hash::hash(self, &mut state);
        state.finish()
    }

    /// Constructs a [`BinaryAccessVectorRuleHeader`] for this [`RuleKey`] and specified [`RuleKind`].
    fn to_header(&self, kind: RuleKind) -> BinaryAccessVectorRuleHeader {
        BinaryAccessVectorRuleHeader {
            source_type: self.source_type.as_u16(),
            target_type: self.target_type.as_u16(),
            class: self.class.as_u16(),
            rule_flags: kind.into(),
        }
    }
}

impl Validate for RuleKey {
    fn validate(&self, policy: &NewPolicy) -> Result<(), ValidateError> {
        self.source_type.validate(policy)?;
        self.target_type.validate(policy)?;
        self.class.validate(policy)?;
        Ok(())
    }
}

/// Trait implemented by rule types that provide a [`RuleKey`].
pub trait HasRuleKey {
    /// Returns the [`RuleKey`] for this rule.
    fn key(&self) -> RuleKey;
}

impl HasRuleKey for AccessRule {
    fn key(&self) -> RuleKey {
        self.key
    }
}

impl HasRuleKey for TypeTransitionRule {
    fn key(&self) -> RuleKey {
        self.key
    }
}

impl HasRuleKey for XpermRule {
    fn key(&self) -> RuleKey {
        self.key
    }
}

/// Encapsulates the result of a permissions calculation, between
/// source & target domains, for a specific class. Decisions describe
/// which permissions are allowed, and whether permissions should be
/// audit-logged when allowed, and when denied.
#[derive(Debug, Clone, PartialEq)]
pub struct AccessDecision {
    pub allow: AccessVector,
    pub auditallow: AccessVector,
    pub auditdeny: AccessVector,
    pub flags: u32,

    /// If this field is set then denials should be audit-logged with "todo_deny" as the reason, with
    /// the `bug` number included in the audit message.
    pub todo_bug: Option<NonZeroU32>,
}

impl Default for AccessDecision {
    fn default() -> Self {
        Self::allow(AccessVector::NONE)
    }
}

impl AccessDecision {
    /// Returns an [`AccessDecision`] with the specified permissions to `allow`, and default audit
    /// behaviour.
    pub const fn allow(allow: AccessVector) -> Self {
        Self {
            allow,
            auditallow: AccessVector::NONE,
            auditdeny: AccessVector::ALL,
            flags: 0,
            todo_bug: None,
        }
    }
}

/// Standard access vector decisions (allow, auditallow, dontaudit) for a source, target, and class tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessVectorDecision {
    /// Permissions explicitly granted by matching allow rules.
    pub allow: Option<AccessVector>,
    /// Permissions explicitly marked for audit logging on grant.
    pub auditallow: Option<AccessVector>,
    /// Permissions explicitly suppressed from audit logging on denial.
    pub dontaudit: Option<AccessVector>,
}

/// Container for access vector rules optimizing rule lookups and memory layout while preserving
/// byte-for-byte serialization.
///
/// This structure balances three primary goals:
/// 1. **Separate Rule-Type Lookup Tables**: Maintains independent lookup tables for each
///    [`RuleKind`] (such as `allow`, `dontaudit`, and `type_transition`) to avoid the overhead of
///    hashing rule-type values and eliminate query-side type filtering.
/// 2. **Homogeneous Payload Arrays**: Stores distinct rule payload types ([`AccessRule`],
///    [`TypeTransitionRule`], and [`XpermRule`]) in separate contiguous arrays to eliminate memory
///    and performance penalties from padding when mixing differently sized and aligned payload
///    types in the same table.
/// 3. **Byte-for-Byte Serialization**: Retains the original binary policy order via `rule_order` to
///    re-serialize policy data losslessly without storing ordering metadata inside individual rules.
///
/// Lookups map [`RuleKey`] hashes to array positions via compact [`U24Index`] values.
#[derive(Debug, Clone)]
pub struct AccessVectorRules {
    av_rules: Box<[AccessRule]>,
    type_transitions: Box<[TypeTransitionRule]>,
    xperm_rules: Box<[XpermRule]>,
    rule_order: Box<[RuleKind]>,

    allow: HashTable<U24Index>,
    auditallow: HashTable<U24Index>,
    dontaudit: HashTable<U24Index>,
    type_transition: HashTable<U24Index>,
    allowxperm: HashTable<U24Index>,
    auditallowxperm: HashTable<U24Index>,
    dontauditxperm: HashTable<U24Index>,
    hasher: RapidBuildHasher,
}

impl PartialEq for AccessVectorRules {
    fn eq(&self, other: &Self) -> bool {
        self.av_rules == other.av_rules
            && self.type_transitions == other.type_transitions
            && self.xperm_rules == other.xperm_rules
            && self.rule_order == other.rule_order
    }
}

impl Eq for AccessVectorRules {}

/// Extended permission decisions (allowxperm, auditallowxperm, dontauditxperm) for a source, target, and class tuple.
pub struct XpermsDecisions<'a> {
    /// Iterator yielding extended permissions granted by allowxperm rules.
    pub allow: XpermsIter<'a>,
    /// Iterator yielding extended permissions marked for audit logging by auditallowxperm rules.
    pub auditallow: XpermsIter<'a>,
    /// Iterator yielding extended permissions suppressed from audit logging by dontauditxperm rules.
    pub dontaudit: XpermsIter<'a>,
}

impl AccessVectorRules {
    /// Constructs a new [`AccessVectorRules`] table and builds dedicated lookup indexes for each rule type.
    pub fn new(
        av_rules: Box<[AccessRule]>,
        type_transitions: Box<[TypeTransitionRule]>,
        xperm_rules: Box<[XpermRule]>,
        rule_order: Box<[RuleKind]>,
    ) -> Result<Self, ParseError> {
        let hasher = RapidBuildHasher::default();

        let mut allow: HashTable<U24Index> = HashTable::new();
        let mut auditallow: HashTable<U24Index> = HashTable::new();
        let mut dontaudit: HashTable<U24Index> = HashTable::new();
        let mut type_transition: HashTable<U24Index> = HashTable::new();

        let mut allowxperm: HashTable<U24Index> = HashTable::new();
        let mut auditallowxperm: HashTable<U24Index> = HashTable::new();
        let mut dontauditxperm: HashTable<U24Index> = HashTable::new();

        let mut av_idx = 0;
        let mut tr_idx = 0;
        let mut xp_idx = 0;

        let mut rule_order_iter = rule_order.iter().peekable();
        while let Some(&kind) = rule_order_iter.next() {
            match kind {
                RuleKind::Allow | RuleKind::AuditAllow | RuleKind::DontAudit => {
                    let table = match kind {
                        RuleKind::Allow => &mut allow,
                        RuleKind::AuditAllow => &mut auditallow,
                        RuleKind::DontAudit => &mut dontaudit,
                        _ => unreachable!(),
                    };
                    insert_rule(table, &hasher, &av_rules, av_idx.try_into()?, kind)?;
                    av_idx += 1;
                }
                RuleKind::TypeTransition | RuleKind::TypeChange | RuleKind::TypeMember => {
                    if kind == RuleKind::TypeTransition {
                        insert_rule(
                            &mut type_transition,
                            &hasher,
                            &type_transitions,
                            tr_idx.try_into()?,
                            kind,
                        )?;
                    }
                    tr_idx += 1;
                }
                RuleKind::AllowXperm | RuleKind::AuditAllowXperm | RuleKind::DontAuditXperm => {
                    let table = match kind {
                        RuleKind::AllowXperm => &mut allowxperm,
                        RuleKind::AuditAllowXperm => &mut auditallowxperm,
                        RuleKind::DontAuditXperm => &mut dontauditxperm,
                        _ => unreachable!(),
                    };
                    insert_rule(table, &hasher, &xperm_rules, xp_idx.try_into()?, kind)?;

                    while xp_idx + 1 < xperm_rules.len()
                        && xperm_rules[xp_idx + 1].kind == kind
                        && xperm_rules[xp_idx + 1].key() == xperm_rules[xp_idx].key()
                    {
                        xp_idx += 1;
                        rule_order_iter.next();
                    }
                    xp_idx += 1;
                }
            }
        }

        Ok(Self {
            av_rules,
            type_transitions,
            xperm_rules,
            rule_order,
            allow,
            auditallow,
            dontaudit,
            type_transition,
            allowxperm,
            auditallowxperm,
            dontauditxperm,
            hasher,
        })
    }

    /// Finds standard allow, auditallow, and dontaudit access vector decisions for the specified tuple.
    pub fn find_av_decisions(
        &self,
        source: TypeId,
        target: TypeId,
        class: ClassId,
    ) -> AccessVectorDecision {
        let key = RuleKey::new(source, target, class);
        let hash = key.hash(&self.hasher);

        let lookup = |table: &HashTable<U24Index>| {
            let idx = table.find(hash, |&i| self.av_rules[i].key() == key)?;
            Some(self.av_rules[*idx].access_vector)
        };

        AccessVectorDecision {
            allow: lookup(&self.allow),
            auditallow: lookup(&self.auditallow),
            dontaudit: lookup(&self.dontaudit),
        }
    }

    /// Finds the target domain type transition for the specified source, target, and class tuple.
    pub fn find_type_transition(
        &self,
        source: TypeId,
        target: TypeId,
        class: ClassId,
    ) -> Option<TypeId> {
        let key = RuleKey::new(source, target, class);
        let hash = key.hash(&self.hasher);
        let idx = self.type_transition.find(hash, |&i| self.type_transitions[i].key() == key)?;
        Some(self.type_transitions[*idx].new_type)
    }

    /// Finds extended permission decisions (allowxperm, auditallowxperm, dontauditxperm) for the specified tuple.
    pub fn find_xperms_decisions(
        &self,
        source: TypeId,
        target: TypeId,
        class: ClassId,
    ) -> XpermsDecisions<'_> {
        let key = RuleKey::new(source, target, class);
        let hash = key.hash(&self.hasher);

        let lookup = |table: &HashTable<U24Index>, kind: RuleKind| {
            let r = table.find(hash, |&r| self.xperm_rules[r].key() == key).copied();
            match r {
                Some(r) => {
                    let slice = &self.xperm_rules[usize::from(r)..];
                    XpermsIter { iter: slice.iter(), key, kind }
                }
                None => XpermsIter { iter: [].iter(), key, kind },
            }
        };

        XpermsDecisions {
            allow: lookup(&self.allowxperm, RuleKind::AllowXperm),
            auditallow: lookup(&self.auditallowxperm, RuleKind::AuditAllowXperm),
            dontaudit: lookup(&self.dontauditxperm, RuleKind::DontAuditXperm),
        }
    }
}

/// Iterator over extended permissions rules matching a specific [`RuleKey`].
pub struct XpermsIter<'a> {
    iter: std::slice::Iter<'a, XpermRule>,
    key: RuleKey,
    kind: RuleKind,
}

impl<'a> Iterator for XpermsIter<'a> {
    type Item = &'a ExtendedPermissions;
    fn next(&mut self) -> Option<Self::Item> {
        let rule = self.iter.as_slice().first()?;
        if rule.key() != self.key || rule.kind != self.kind {
            self.iter = [].iter();
            return None;
        }
        let rule = self.iter.next()?;
        Some(&rule.extended_permissions)
    }
}

impl Parse for AccessVectorRules {
    fn parse(cursor: &mut PolicyCursor<'_>) -> Result<Self, ParseError> {
        let count = u32::parse(cursor)? as usize;

        let mut av_rules = Vec::new();
        let mut type_transitions = Vec::new();
        let mut xperm_rules = Vec::new();
        let mut rule_order = Vec::with_capacity(count);

        for _ in 0..count {
            let header = BinaryAccessVectorRuleHeader::parse(cursor)?;
            let kind = RuleKind::try_from(header.rule_flags)?;

            let source_val = header.source_type;
            let source_type = TypeId::from_u16(source_val)
                .ok_or(ParseError::InvalidId { value: source_val as u32 })?;

            let target_val = header.target_type;
            let target_type = TypeId::from_u16(target_val)
                .ok_or(ParseError::InvalidId { value: target_val as u32 })?;

            let class_val = header.class;
            let class = ClassId::from_u16(class_val)
                .ok_or(ParseError::InvalidId { value: class_val as u32 })?;

            let key = RuleKey::new(source_type, target_type, class);

            match kind {
                RuleKind::AllowXperm | RuleKind::AuditAllowXperm | RuleKind::DontAuditXperm => {
                    let extended_permissions = ExtendedPermissions::parse(cursor)?;
                    xperm_rules.push(XpermRule { key, kind, extended_permissions });
                    rule_order.push(kind);
                }
                RuleKind::TypeTransition | RuleKind::TypeChange | RuleKind::TypeMember => {
                    let new_type = TypeId::parse(cursor)?;
                    type_transitions.push(TypeTransitionRule { key, new_type });
                    rule_order.push(kind);
                }
                RuleKind::Allow | RuleKind::AuditAllow | RuleKind::DontAudit => {
                    let access_vector = AccessVector::parse(cursor)?;
                    av_rules.push(AccessRule { key, access_vector });
                    rule_order.push(kind);
                }
            }
        }

        Self::new(
            av_rules.into_boxed_slice(),
            type_transitions.into_boxed_slice(),
            xperm_rules.into_boxed_slice(),
            rule_order.into_boxed_slice(),
        )
    }
}

impl Serialize for AccessVectorRules {
    fn serialize(&self, writer: &mut Vec<u8>) -> Result<(), SerializeError> {
        let count = self.rule_order.len() as u32;
        count.serialize(writer)?;

        let mut av_idx = 0;
        let mut tr_idx = 0;
        let mut xp_idx = 0;

        for &kind in self.rule_order.iter() {
            match kind {
                RuleKind::Allow | RuleKind::AuditAllow | RuleKind::DontAudit => {
                    let rule = &self.av_rules[av_idx];
                    av_idx += 1;
                    rule.key.to_header(kind).serialize(writer)?;
                    let val: u32 = rule.access_vector.into();
                    val.serialize(writer)?;
                }
                RuleKind::TypeTransition | RuleKind::TypeChange | RuleKind::TypeMember => {
                    let rule = &self.type_transitions[tr_idx];
                    tr_idx += 1;
                    rule.key.to_header(kind).serialize(writer)?;
                    rule.new_type.serialize(writer)?;
                }
                RuleKind::AllowXperm | RuleKind::AuditAllowXperm | RuleKind::DontAuditXperm => {
                    let rule = &self.xperm_rules[xp_idx];
                    xp_idx += 1;
                    rule.key.to_header(kind).serialize(writer)?;
                    rule.extended_permissions.serialize(writer)?;
                }
            }
        }
        Ok(())
    }
}

impl Validate for AccessVectorRules {
    fn validate(&self, policy: &NewPolicy) -> Result<(), ValidateError> {
        for rule in self.av_rules.iter() {
            rule.key.validate(policy)?;
        }
        for rule in self.type_transitions.iter() {
            rule.key.validate(policy)?;
            rule.new_type.validate(policy)?;
        }
        for rule in self.xperm_rules.iter() {
            rule.key.validate(policy)?;
            rule.extended_permissions.validate(policy)?;
        }
        Ok(())
    }
}

/// Inserts an entry into `table` for the given rule's [`RuleKey`].
///
/// Returns [`ParseError::DuplicateAccessVectorRule`] if an entry for the same key is already occupied.
fn insert_rule<R: HasRuleKey>(
    table: &mut HashTable<U24Index>,
    hasher: &RapidBuildHasher,
    rules: &[R],
    arr_idx: U24Index,
    kind: RuleKind,
) -> Result<(), ParseError> {
    let key = rules[arr_idx].key();
    let hash = key.hash(hasher);

    match table.entry(hash, |&i| rules[i].key() == key, |&i| rules[i].key().hash(hasher)) {
        Entry::Occupied(_) => Err(ParseError::DuplicateAccessVectorRule { key, kind }),
        Entry::Vacant(entry) => {
            entry.insert(arr_idx);
            Ok(())
        }
    }
}

/// On-wire header identifying the source, target, class, and rule flags of an access vector rule.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Parse, Serialize)]
struct BinaryAccessVectorRuleHeader {
    source_type: u16,
    target_type: u16,
    class: u16,
    rule_flags: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_policy::traits::PolicyId;

    #[test]
    fn test_xperms_bitmap_ops_and_contains() {
        let mut bitmap1 = XpermsBitmap::NONE;
        let mut bitmap2 = XpermsBitmap::NONE;
        assert!(!bitmap1.contains(10));
        assert!(!bitmap2.contains(10));

        bitmap1.0[0] = 1 << 10;
        bitmap2.0[0] = 1 << 20;
        assert!(bitmap1.contains(10));
        assert!(!bitmap1.contains(20));
        assert!(bitmap2.contains(20));

        let or_bitmap = bitmap1 | bitmap2;
        assert!(or_bitmap.contains(10));
        assert!(or_bitmap.contains(20));

        let and_bitmap = or_bitmap & bitmap1;
        assert!(and_bitmap.contains(10));
        assert!(!and_bitmap.contains(20));

        let sub_bitmap = or_bitmap - bitmap1;
        assert!(!sub_bitmap.contains(10));
        assert!(sub_bitmap.contains(20));

        let not_bitmap = !XpermsBitmap::NONE;
        assert_eq!(not_bitmap, XpermsBitmap::ALL);
        assert!(not_bitmap.contains(0));
        assert!(not_bitmap.contains(255));
    }

    #[test]
    fn test_av_rule_parse_and_serialize() {
        let data = [
            1, 0, 0, 0, // count = 1
            1, 0, // source_type = 1
            2, 0, // target_type = 2
            3, 0, // class = 3
            1, 0, // rule_type = 1 (ALLOW)
            5, 0, 0, 0, // access_vector = 5
        ];
        let mut cursor = PolicyCursor::new(&data);
        let av_rules = AccessVectorRules::parse(&mut cursor).expect("parse rules table");
        assert_eq!(av_rules.av_rules.len(), 1);
        assert_eq!(av_rules.av_rules[0].key.source_type, TypeId::from_u32(1).unwrap());
        assert_eq!(av_rules.av_rules[0].key.target_type, TypeId::from_u32(2).unwrap());
        assert_eq!(av_rules.av_rules[0].key.class, ClassId::from_u32(3).unwrap());
        assert_eq!(av_rules.av_rules[0].access_vector, AccessVector::from(5));

        let mut writer = Vec::new();
        av_rules.serialize(&mut writer).expect("serialize rules table");
        assert_eq!(writer.as_slice(), &data);
    }

    #[test]
    fn test_av_rule_type_transition_parse_and_serialize() {
        let data = [
            1, 0, 0, 0, // count = 1
            1, 0, // source_type = 1
            2, 0, // target_type = 2
            3, 0, // class = 3
            16, 0, // rule_type = 16 (TYPE_TRANSITION)
            10, 0, 0, 0, // new_type = 10
        ];
        let mut cursor = PolicyCursor::new(&data);
        let av_rules = AccessVectorRules::parse(&mut cursor).expect("parse rules table");
        assert_eq!(av_rules.type_transitions.len(), 1);
        assert_eq!(av_rules.type_transitions[0].key.source_type, TypeId::from_u32(1).unwrap());
        assert_eq!(av_rules.type_transitions[0].key.target_type, TypeId::from_u32(2).unwrap());
        assert_eq!(av_rules.type_transitions[0].key.class, ClassId::from_u32(3).unwrap());
        assert_eq!(av_rules.type_transitions[0].new_type, TypeId::from_u32(10).unwrap());

        let mut writer = Vec::new();
        av_rules.serialize(&mut writer).expect("serialize rules table");
        assert_eq!(writer.as_slice(), &data);
    }

    #[test]
    fn test_av_rule_xperm_parse_and_serialize() {
        let mut data = vec![
            1, 0, 0, 0, // count = 1
            1, 0, // source_type = 1
            2, 0, // target_type = 2
            3, 0, // class = 3
            0, 1, // rule_type = 0x0100 (ALLOWXPERM)
            1, // xperms_type = 1 (XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES)
            0, // xperms_optional_prefix = 0
        ];
        data.extend_from_slice(&[
            1, 0, 0, 0, 0, 0, 0, 0, // word 0 = 1
            0, 0, 0, 0, 0, 0, 0, 0, // word 1 = 0
            0, 0, 0, 0, 0, 0, 0, 0, // word 2 = 0
            0, 0, 0, 0, 0, 0, 0, 0, // word 3 = 0
        ]);
        let mut cursor = PolicyCursor::new(&data);
        let av_rules = AccessVectorRules::parse(&mut cursor).expect("parse rules table");
        assert_eq!(av_rules.xperm_rules.len(), 1);
        let xp = &av_rules.xperm_rules[0].extended_permissions;
        assert_eq!(xp.xperms_type, XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert!(xp.xperms_bitmap.contains(0));
        assert!(!xp.xperms_bitmap.contains(1));

        let mut writer = Vec::new();
        av_rules.serialize(&mut writer).expect("serialize rules table");
        assert_eq!(writer.as_slice(), &data);
    }

    #[test]
    fn test_access_vector_rules_indexing_and_decisions() {
        let data = [
            2, 0, 0, 0, // count = 2 rules
            // Rule 1: ALLOW (source 1, target 2, class 3)
            1, 0, // source = 1
            2, 0, // target = 2
            3, 0, // class = 3
            1, 0, // rule_type = 1 (ALLOW)
            7, 0, 0, 0, // access_vector = 7
            // Rule 2: TYPE_TRANSITION (source 1, target 2, class 3 -> new_type 9)
            1, 0, // source = 1
            2, 0, // target = 2
            3, 0, // class = 3
            16, 0, // rule_type = 16 (TYPE_TRANSITION)
            9, 0, 0, 0, // new_type = 9
        ];

        let mut cursor = PolicyCursor::new(&data);
        let av_rules = AccessVectorRules::parse(&mut cursor).expect("parse rules table");

        let s1 = TypeId::from_u32(1).unwrap();
        let t2 = TypeId::from_u32(2).unwrap();
        let c3 = ClassId::from_u32(3).unwrap();

        let decision = av_rules.find_av_decisions(s1, t2, c3);
        assert_eq!(decision.allow, Some(AccessVector::from(7)));
        assert_eq!(decision.auditallow, None);
        assert_eq!(decision.dontaudit, None);

        let transition = av_rules.find_type_transition(s1, t2, c3);
        assert_eq!(transition, Some(TypeId::from_u32(9).unwrap()));
    }
}
