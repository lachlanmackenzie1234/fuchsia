// Copyright 2024 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::policy::view::Hashable;

use super::error::ValidateError;
use super::parser::{PolicyCursor, PolicyData, PolicyOffset};
use super::view::{ArrayView, Walk};
use super::{
    Array, ClassId, Counted, MlsLevel, MlsRange, Parse, PolicyValidationContext, RoleId, TypeId,
    UserId, Validate, ValidateArray, array_type, array_type_validate_deref_both,
};
use crate::new_policy::TypeSet;

use crate::new_policy::traits::PolicyId;
use anyhow::Context as _;
use std::hash::{Hash, Hasher};
use zerocopy::{FromBytes, Immutable, KnownLayout, Unaligned, little_endian as le};

pub(super) const MIN_POLICY_VERSION_FOR_INFINITIBAND_PARTITION_KEY: u32 = 31;

#[allow(type_alias_bounds)]
pub(super) type SimpleArray<T> = Array<le::U32, T>;

impl<T: Validate> Validate for SimpleArray<T> {
    type Error = <T as Validate>::Error;
    /// Default implementation of `Validate` for `SimpleArray<T>`, validating individual T
    /// objects. It assumes no internal constraints between the objects.
    /// Override this function for types with more complex validation requirements.
    fn validate(&self, context: &PolicyValidationContext) -> Result<(), Self::Error> {
        self.data.validate(context)
    }
}

pub(super) type SimpleArrayView<T> = ArrayView<le::U32, T>;

impl<T: Validate + Parse + Walk> Validate for SimpleArrayView<T> {
    type Error = anyhow::Error;

    /// Defers to `self.data` for validation. `self.data` has access to all information, including
    /// size stored in `self.metadata`.
    fn validate(&self, context: &PolicyValidationContext) -> Result<(), Self::Error> {
        for item in self.data().iter(&context.data) {
            item.validate(context)?;
        }
        Ok(())
    }
}

impl Counted for le::U32 {
    fn count(&self) -> u32 {
        self.get()
    }
}

array_type!(RoleTransitions, le::U32, RoleTransition);

array_type_validate_deref_both!(RoleTransitions);

impl ValidateArray<le::U32, RoleTransition> for RoleTransitions {
    type Error = anyhow::Error;

    /// [`RoleTransitions`] have no additional metadata (beyond length encoding).
    fn validate_array(
        _context: &PolicyValidationContext,
        _metadata: &le::U32,
        _items: &[RoleTransition],
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct RoleTransition {
    role: le::U32,
    role_type: le::U32,
    new_role: le::U32,
    tclass: le::U32,
}

impl RoleTransition {
    pub(super) fn current_role(&self) -> RoleId {
        RoleId::from_u32(self.role.get()).unwrap()
    }

    pub(super) fn type_(&self) -> TypeId {
        TypeId::from_u32(self.role_type.get()).unwrap()
    }

    pub(super) fn class(&self) -> ClassId {
        ClassId::try_from(self.tclass.get()).unwrap()
    }

    pub(super) fn new_role(&self) -> RoleId {
        RoleId::from_u32(self.new_role.get()).unwrap()
    }
}

impl Validate for RoleTransition {
    type Error = anyhow::Error;

    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        RoleId::from_u32(self.role.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        TypeId::from_u32(self.role_type.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        ClassId::from_u32(self.tclass.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        RoleId::from_u32(self.new_role.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        Ok(())
    }
}

array_type!(RoleAllows, le::U32, RoleAllow);

array_type_validate_deref_both!(RoleAllows);

impl ValidateArray<le::U32, RoleAllow> for RoleAllows {
    type Error = anyhow::Error;

    /// [`RoleAllows`] have no additional metadata (beyond length encoding).
    fn validate_array(
        _context: &PolicyValidationContext,
        _metadata: &le::U32,
        _items: &[RoleAllow],
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct RoleAllow {
    role: le::U32,
    new_role: le::U32,
}

impl RoleAllow {
    pub(super) fn source_role(&self) -> RoleId {
        RoleId::from_u32(self.role.get()).unwrap()
    }

    pub(super) fn new_role(&self) -> RoleId {
        RoleId::from_u32(self.new_role.get()).unwrap()
    }
}

impl Validate for RoleAllow {
    type Error = anyhow::Error;

    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        RoleId::from_u32(self.role.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        RoleId::from_u32(self.new_role.get()).ok_or(ValidateError::NonOptionalIdIsZero)?;
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum FilenameTransitionList {
    PolicyVersionGeq33(SimpleArray<FilenameTransition>),
    PolicyVersionLeq32(SimpleArray<DeprecatedFilenameTransition>),
}

impl Validate for FilenameTransitionList {
    type Error = anyhow::Error;

    fn validate(&self, context: &PolicyValidationContext) -> Result<(), Self::Error> {
        match self {
            Self::PolicyVersionLeq32(list) => {
                list.validate(context).map_err(Into::<anyhow::Error>::into)
            }
            Self::PolicyVersionGeq33(list) => {
                list.validate(context).map_err(Into::<anyhow::Error>::into)
            }
        }
    }
}

impl Validate for FilenameTransition {
    type Error = anyhow::Error;
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct FilenameTransition {
    filename: SimpleArray<u8>,
    transition_type: le::U32,
    transition_class: le::U32,
    items: SimpleArray<FilenameTransitionItem>,
}

impl FilenameTransition {
    pub(super) fn name_bytes(&self) -> &[u8] {
        &self.filename.data
    }

    pub(super) fn target_type(&self) -> TypeId {
        TypeId::from_u32(self.transition_type.get()).unwrap()
    }

    pub(super) fn target_class(&self) -> ClassId {
        ClassId::try_from(self.transition_class.get()).unwrap()
    }

    pub(super) fn outputs(&self) -> &[FilenameTransitionItem] {
        &self.items.data
    }
}

impl Parse for FilenameTransition
where
    SimpleArray<u8>: Parse,
    SimpleArray<FilenameTransitionItem>: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (filename, tail) = SimpleArray::<u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing filename for filename transition")?;

        let (transition_type, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (transition_class, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (items, tail) = SimpleArray::<FilenameTransitionItem>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing items for filename transition")?;

        Ok((Self { filename, transition_type, transition_class, items }, tail))
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct FilenameTransitionItem {
    stypes: TypeSet,
    out_type: le::U32,
}

impl FilenameTransitionItem {
    pub(super) fn has_source_type(&self, source_type: TypeId) -> bool {
        self.stypes.contains(source_type)
    }

    pub(super) fn out_type(&self) -> TypeId {
        TypeId::from_u32(self.out_type.get()).unwrap()
    }
}

impl Parse for FilenameTransitionItem
where
    TypeSet: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (stypes, tail) = TypeSet::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing stypes extensible bitmap for file transition")?;

        let (out_type, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        Ok((Self { stypes, out_type }, tail))
    }
}

impl Validate for DeprecatedFilenameTransition {
    type Error = anyhow::Error;
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct DeprecatedFilenameTransition {
    filename: SimpleArray<u8>,
    metadata: DeprecatedFilenameTransitionMetadata,
}

impl DeprecatedFilenameTransition {
    pub(super) fn name_bytes(&self) -> &[u8] {
        &self.filename.data
    }

    pub(super) fn source_type(&self) -> TypeId {
        TypeId::from_u32(self.metadata.source_type.get()).unwrap()
    }

    pub(super) fn target_type(&self) -> TypeId {
        TypeId::from_u32(self.metadata.transition_type.get()).unwrap()
    }

    pub(super) fn target_class(&self) -> ClassId {
        ClassId::try_from(self.metadata.transition_class.get()).unwrap()
    }

    pub(super) fn out_type(&self) -> TypeId {
        TypeId::from_u32(self.metadata.out_type.get()).unwrap()
    }
}

impl Parse for DeprecatedFilenameTransition
where
    SimpleArray<u8>: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (filename, tail) = SimpleArray::<u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing filename for deprecated filename transition")?;

        let (metadata, tail) = PolicyCursor::parse::<DeprecatedFilenameTransitionMetadata>(tail)?;

        Ok((Self { filename, metadata }, tail))
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct DeprecatedFilenameTransitionMetadata {
    source_type: le::U32,
    transition_type: le::U32,
    transition_class: le::U32,
    out_type: le::U32,
}

impl Validate for SimpleArray<InitialSid> {
    type Error = anyhow::Error;

    fn validate(&self, context: &PolicyValidationContext) -> Result<(), Self::Error> {
        for initial_sid in crate::InitialSid::all_variants() {
            if *initial_sid == crate::InitialSid::Init && !context.need_init_sid {
                continue;
            }
            self.data
                .iter()
                .find(|initial| initial.id().get() == *initial_sid as u32)
                .ok_or(ValidateError::MissingInitialSid { initial_sid: *initial_sid })?;
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct InitialSid {
    id: le::U32,
    context: Context,
}

impl InitialSid {
    pub(super) fn id(&self) -> le::U32 {
        self.id
    }

    pub(super) fn context(&self) -> &Context {
        &self.context
    }
}

impl Parse for InitialSid
where
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (id, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for initial sid")?;

        Ok((Self { id, context }, tail))
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Context {
    metadata: ContextMetadata,
    mls_range: MlsRange,
}

impl Context {
    pub(super) fn user_id(&self) -> UserId {
        UserId::from_u32(self.metadata.user.get()).unwrap()
    }
    pub(super) fn role_id(&self) -> RoleId {
        RoleId::from_u32(self.metadata.role.get()).unwrap()
    }
    pub(super) fn type_id(&self) -> TypeId {
        TypeId::from_u32(self.metadata.context_type.get()).unwrap()
    }
    pub(super) fn low_level(&self) -> &MlsLevel {
        self.mls_range.low()
    }
    pub(super) fn high_level(&self) -> &Option<MlsLevel> {
        self.mls_range.high()
    }
}

impl Parse for Context
where
    MlsRange: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (metadata, tail) =
            PolicyCursor::parse::<ContextMetadata>(tail).context("parsing metadata for context")?;

        let (mls_range, tail) = MlsRange::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing mls range for context")?;

        Ok((Self { metadata, mls_range }, tail))
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct ContextMetadata {
    user: le::U32,
    role: le::U32,
    context_type: le::U32,
}

impl Validate for NamedContextPair {
    type Error = anyhow::Error;

    /// TODO: Validate consistency of sequence of [`NamedContextPairs`] objects.
    ///
    /// TODO: Is different validation required for `filesystems` and `network_interfaces`? If so,
    /// create wrapper types with different [`Validate`] implementations.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct NamedContextPair {
    name: SimpleArray<u8>,
    context1: Context,
    context2: Context,
}

impl Parse for NamedContextPair
where
    SimpleArray<u8>: Parse,
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (name, tail) = SimpleArray::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing filesystem context name")?;

        let (context1, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing first context for filesystem context")?;

        let (context2, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing second context for filesystem context")?;

        Ok((Self { name, context1, context2 }, tail))
    }
}

impl Validate for Port {
    type Error = anyhow::Error;

    /// TODO: Validate consistency of sequence of [`Ports`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Port {
    metadata: PortMetadata,
    context: Context,
}

impl Parse for Port
where
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (metadata, tail) =
            PolicyCursor::parse::<PortMetadata>(tail).context("parsing metadata for context")?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for port")?;

        Ok((Self { metadata, context }, tail))
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct PortMetadata {
    protocol: le::U32,
    low_port: le::U32,
    high_port: le::U32,
}

impl Validate for Node {
    type Error = anyhow::Error;

    /// TODO: Validate consistency of sequence of [`Node`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct Node {
    address: le::U32,
    mask: le::U32,
    context: Context,
}

impl Parse for Node
where
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (address, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (mask, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for node")?;

        Ok((Self { address, mask, context }, tail))
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct FsUse {
    behavior_and_name: Array<FsUseMetadata, u8>,
    context: Context,
}

impl FsUse {
    pub fn fs_type(&self) -> &[u8] {
        &self.behavior_and_name.data
    }

    pub(super) fn behavior(&self) -> FsUseType {
        FsUseType::try_from(self.behavior_and_name.metadata.behavior).unwrap()
    }

    pub(super) fn context(&self) -> &Context {
        &self.context
    }
}

impl Parse for FsUse
where
    Array<FsUseMetadata, u8>: Parse,
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (behavior_and_name, tail) = Array::<FsUseMetadata, u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing fs use metadata")?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for fs use")?;

        Ok((Self { behavior_and_name, context }, tail))
    }
}

impl Validate for FsUse {
    type Error = anyhow::Error;

    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        FsUseType::try_from(self.behavior_and_name.metadata.behavior)?;

        Ok(())
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct FsUseMetadata {
    /// The type of `fs_use` statement.
    behavior: le::U32,
    /// The length of the name in the name_and_behavior field of FsUse.
    name_length: le::U32,
}

impl Counted for FsUseMetadata {
    fn count(&self) -> u32 {
        self.name_length.get()
    }
}

/// Discriminates among the different kinds of "fs_use_*" labeling statements in the policy; see
/// https://selinuxproject.org/page/FileStatements.
#[derive(Copy, Clone, Debug, Eq, Hash, PartialEq)]
pub enum FsUseType {
    Xattr = 1,
    Trans = 2,
    Task = 3,
}

impl TryFrom<le::U32> for FsUseType {
    type Error = anyhow::Error;

    fn try_from(value: le::U32) -> Result<Self, Self::Error> {
        match value.get() {
            1 => Ok(FsUseType::Xattr),
            2 => Ok(FsUseType::Trans),
            3 => Ok(FsUseType::Task),
            _ => Err(ValidateError::InvalidFsUseType { value: value.get() }.into()),
        }
    }
}

impl Validate for IPv6Node {
    type Error = anyhow::Error;

    /// TODO: Validate consistency of sequence of [`IPv6Node`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct IPv6Node {
    address: [le::U32; 4],
    mask: [le::U32; 4],
    context: Context,
}

impl Parse for IPv6Node
where
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (address, tail) = PolicyCursor::parse::<[le::U32; 4]>(tail)?;

        let (mask, tail) = PolicyCursor::parse::<[le::U32; 4]>(tail)?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for ipv6 node")?;

        Ok((Self { address, mask, context }, tail))
    }
}

impl Validate for InfinitiBandPartitionKey {
    type Error = anyhow::Error;

    /// TODO: Validate consistency of sequence of [`InfinitiBandPartitionKey`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct InfinitiBandPartitionKey {
    low: le::U32,
    high: le::U32,
    context: Context,
}

impl Parse for InfinitiBandPartitionKey
where
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (low, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (high, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for infiniti band partition key")?;

        Ok((Self { low, high, context }, tail))
    }
}

impl Validate for InfinitiBandEndPort {
    type Error = anyhow::Error;

    /// TODO: Validate sequence of [`InfinitiBandEndPort`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct InfinitiBandEndPort {
    port_and_name: Array<InfinitiBandEndPortMetadata, u8>,
    context: Context,
}

impl Parse for InfinitiBandEndPort
where
    Array<InfinitiBandEndPortMetadata, u8>: Parse,
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (port_and_name, tail) = Array::<InfinitiBandEndPortMetadata, u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing infiniti band end port metadata")?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for infiniti band end port")?;

        Ok((Self { port_and_name, context }, tail))
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct InfinitiBandEndPortMetadata {
    length: le::U32,
    port: le::U32,
}

impl Counted for InfinitiBandEndPortMetadata {
    fn count(&self) -> u32 {
        self.length.get()
    }
}

impl Validate for GenericFsContext {
    type Error = anyhow::Error;

    /// TODO: Validate sequence of  [`GenericFsContext`] objects.
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Information parsed parsed from `genfscon [fs_type] [partial_path] [fs_context]` statements
/// about a specific filesystem type.
#[derive(Debug)]
pub(super) struct GenericFsContext {
    fs_type: SimpleArray<u8>,
    fs_context: SimpleArrayView<FsContext>,
}

impl GenericFsContext {
    /// Returns the `fs_type` representation to be used when looking up in a CustomKeyHashedView.
    pub(super) fn for_query(fs_type: &str) -> SimpleArray<u8> {
        Array { data: fs_type.as_bytes().to_vec(), metadata: le::U32::new(fs_type.len() as u32) }
    }
}

impl Parse for GenericFsContext {
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (fs_type, tail) = SimpleArray::<u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing fs_type for generic fs context")?;

        let (fs_context, tail) = SimpleArrayView::<FsContext>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing fs_context for generic fs context")?;

        Ok((Self { fs_type, fs_context }, tail))
    }
}

impl Hashable for GenericFsContext {
    type Key = SimpleArray<u8>;
    type Value = FsContext;

    fn key(&self) -> &Self::Key {
        &self.fs_type
    }

    fn values(&self) -> &SimpleArrayView<Self::Value> {
        &self.fs_context
    }
}

impl Eq for SimpleArray<u8> {}

impl Hash for SimpleArray<u8> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data.hash(state);
    }
}

impl SimpleArrayView<FsContext> {
    fn try_validate_alphabetic_order(&self, context: &PolicyValidationContext) -> bool {
        self.data()
            .iter(&context.data)
            .map(|view| view.parse(&context.data).partial_path().to_vec())
            .is_sorted_by(|a, b| a <= b)
    }

    fn try_validate_length_descending_order(&self, context: &PolicyValidationContext) -> bool {
        self.data()
            .iter(&context.data)
            .map(|view| view.parse(&context.data).partial_path().len())
            .is_sorted_by(|a, b| a >= b)
    }
}

impl Validate for SimpleArrayView<FsContext> {
    type Error = anyhow::Error;

    /// Checks that the sequence of [`FsContext`] objects is valid.
    /// To be valid, FsContexts must be sorted by either:
    /// - the length of sub-paths (descending order).
    /// - alphabetically by sub-paths (ascending order).
    fn validate(&self, context: &PolicyValidationContext) -> Result<(), Self::Error> {
        if !self.try_validate_alphabetic_order(context)
            && !self.try_validate_length_descending_order(context)
        {
            return Err(anyhow::anyhow!(
                "FsContexts must be sorted by partial path length (descending) or alphabetically.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct FsContext {
    /// The partial path, relative to the root of the filesystem. The partial path can only be set for
    /// virtual filesystems, like `proc/`. Otherwise, this must be `/`
    partial_path: SimpleArray<u8>,
    /// Optional. When provided, the context will only be applied to files of this type. Allowed files
    /// types are: blk_file, chr_file, dir, fifo_file, lnk_file, sock_file, file. When set to 0, the
    /// context applies to all file types.
    class: le::U32,
    /// The security context allocated to the filesystem.
    context: Context,
}

impl FsContext {
    pub(super) fn partial_path(&self) -> &[u8] {
        &self.partial_path.data
    }

    pub(super) fn context(&self) -> &Context {
        &self.context
    }

    pub(super) fn class(&self) -> Option<ClassId> {
        ClassId::try_from(self.class.get()).ok()
    }
}

impl Parse for FsContext
where
    SimpleArray<u8>: Parse,
    Context: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (partial_path, tail) = SimpleArray::<u8>::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing filesystem context partial path")?;

        let (class, tail) = PolicyCursor::parse::<le::U32>(tail)?;

        let (context, tail) = Context::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing context for filesystem context")?;

        Ok((Self { partial_path, class, context }, tail))
    }
}

impl Walk for FsContext {
    fn walk(policy_data: &PolicyData, offset: PolicyOffset) -> PolicyOffset {
        let cursor = PolicyCursor::new_at(policy_data, offset);
        let (_, tail) = FsContext::parse(cursor)
            .map_err(Into::<anyhow::Error>::into)
            .expect("policy should be valid");
        tail.offset()
    }
}

impl Validate for RangeTransition {
    type Error = anyhow::Error;
    fn validate(&self, _context: &PolicyValidationContext) -> Result<(), Self::Error> {
        if self.metadata.target_class.get() == 0 {
            return Err(ValidateError::NonOptionalIdIsZero.into());
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
pub(super) struct RangeTransition {
    metadata: RangeTransitionMetadata,
    mls_range: MlsRange,
}

impl RangeTransition {
    pub fn source_type(&self) -> TypeId {
        TypeId::from_u32(self.metadata.source_type.get()).unwrap()
    }

    pub fn target_type(&self) -> TypeId {
        TypeId::from_u32(self.metadata.target_type.get()).unwrap()
    }

    pub fn target_class(&self) -> ClassId {
        ClassId::try_from(self.metadata.target_class.get()).unwrap()
    }

    pub fn mls_range(&self) -> &MlsRange {
        &self.mls_range
    }
}

impl Parse for RangeTransition
where
    MlsRange: Parse,
{
    type Error = anyhow::Error;

    fn parse<'a>(bytes: PolicyCursor<'a>) -> Result<(Self, PolicyCursor<'a>), Self::Error> {
        let tail = bytes;

        let (metadata, tail) = PolicyCursor::parse::<RangeTransitionMetadata>(tail)
            .context("parsing range transition metadata")?;

        let (mls_range, tail) = MlsRange::parse(tail)
            .map_err(Into::<anyhow::Error>::into)
            .context("parsing mls range for range transition")?;

        Ok((Self { metadata, mls_range }, tail))
    }
}

#[derive(Clone, Debug, KnownLayout, FromBytes, Immutable, PartialEq, Unaligned)]
#[repr(C, packed)]
pub(super) struct RangeTransitionMetadata {
    source_type: le::U32,
    target_type: le::U32,
    target_class: le::U32,
}

#[cfg(test)]
mod tests {
    use super::super::parse_policy_by_value;
    use crate::new_policy::rules::{
        HasRuleKey, RuleKind, XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES, XPERMS_TYPE_IOCTL_PREFIXES,
        XPERMS_TYPE_NLMSG,
    };
    use crate::new_policy::traits::HasPolicyId;

    #[test]
    fn parse_allowxperm_one_ioctl() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id =
            policy.classes().get_by_name(b"class_one_ioctl").expect("look up class_one_ioctl").id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0xabcd));
    }

    // `ioctl` extended permissions that are declared in the same rule, and have the same
    // high byte, are stored in the same `AccessVectorRule` in the compiled policy.
    #[test]
    fn parse_allowxperm_two_ioctls_same_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_two_ioctls_same_range")
            .expect("look up class_two_ioctls_same_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x12);
        assert_eq!(rules[0].count(), 2);
        assert!(rules[0].contains(0x1234));
        assert!(rules[0].contains(0x1256));
    }

    // `ioctl` extended permissions that are declared in different rules, but that have the same
    // high byte, are stored in the same `AccessVectorRule` in the compiled policy.
    #[test]
    fn parse_allowxperm_two_ioctls_same_range_diff_rules() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_four_ioctls_same_range_diff_rules")
            .expect("look up class_four_ioctls_same_range_diff_rules")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x30);
        assert_eq!(rules[0].count(), 4);
        assert!(rules[0].contains(0x3008));
        assert!(rules[0].contains(0x3009));
        assert!(rules[0].contains(0x3011));
        assert!(rules[0].contains(0x3013));
    }

    // `ioctl` extended permissions that are declared in the same rule, and have different
    // high bytes, are stored in different `AccessVectorRule`s in the compiled policy.
    #[test]
    fn parse_allowxperm_two_ioctls_different_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_two_ioctls_diff_range")
            .expect("look up class_two_ioctls_diff_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x56);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0x5678));
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x12);
        assert_eq!(rules[1].count(), 1);
        assert!(rules[1].contains(0x1234));
    }

    // If a set of `ioctl` extended permissions consists of all xperms with a given high byte,
    // then it is represented by one `AccessVectorRule`.
    #[test]
    fn parse_allowxperm_one_driver_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_one_driver_range")
            .expect("look up class_one_driver_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIXES);
        assert_eq!(rules[0].count(), 0x100);
        assert!(rules[0].contains(0x1000));
        assert!(rules[0].contains(0x10ab));
    }

    // If a rule grants `ioctl` extended permissions to a wide range that does not fall cleanly on
    // divisible-by-256 boundaries, it gets represented in the policy as three `AccessVectorRule`s:
    // two for the smaller subranges at the ends and one for the large subrange in the middle.
    #[test]
    fn parse_allowxperm_most_ioctls() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_most_ioctls")
            .expect("look up class_most_ioctls")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[0].xperms_optional_prefix(), 0xff);
        assert_eq!(rules[0].count(), 0xfe);
        for xperm in 0xff00..0xfffd {
            assert!(rules[0].contains(xperm));
        }
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[1].count(), 0xfe);
        for xperm in 0x0002..0x0100 {
            assert!(rules[1].contains(xperm));
        }
        assert_eq!(rules[2].xperms_type(), XPERMS_TYPE_IOCTL_PREFIXES);
        assert_eq!(rules[2].count(), 0xfe00);
        for xperm in 0x0100..0xff00 {
            assert!(rules[2].contains(xperm));
        }
    }

    // If a rule grants `ioctl` extended permissions to two wide ranges that do not fall cleanly on
    // divisible-by-256 boundaries, they get represented in the policy as five `AccessVectorRule`s:
    // four for the smaller subranges at the ends and one for the two large subranges.
    #[test]
    fn parse_allowxperm_most_ioctls_with_hole() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_most_ioctls_with_hole")
            .expect("look up class_most_ioctls_with_hole")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 5);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[0].xperms_optional_prefix(), 0xff);
        assert_eq!(rules[0].count(), 0xfe);
        for xperm in 0xff00..0xfffd {
            assert!(rules[0].contains(xperm));
        }
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x40);
        assert_eq!(rules[1].count(), 0xfe);
        for xperm in 0x4002..0x4100 {
            assert!(rules[1].contains(xperm));
        }
        assert_eq!(rules[2].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[2].xperms_optional_prefix(), 0x2f);
        assert_eq!(rules[2].count(), 0xfe);
        for xperm in 0x2f00..0x2ffd {
            assert!(rules[2].contains(xperm));
        }
        assert_eq!(rules[3].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[3].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[3].count(), 0xfe);
        for xperm in 0x0002..0x0100 {
            assert!(rules[3].contains(xperm));
        }
        assert_eq!(rules[4].xperms_type(), XPERMS_TYPE_IOCTL_PREFIXES);
        assert_eq!(rules[4].count(), 0xec00);
        for xperm in 0x0100..0x2f00 {
            assert!(rules[4].contains(xperm));
        }
        for xperm in 0x4100..0xff00 {
            assert!(rules[4].contains(xperm));
        }
    }

    // If a set of `ioctl` extended permissions contains all 16-bit xperms, then it is
    // then it is represented by one `AccessVectorRule`. (More generally, the representation
    // is a single `AccessVectorRule` as long as the set either fully includes or fully
    // excludes each 8-bit prefix range.)
    #[test]
    fn parse_allowxperm_all_ioctls() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_all_ioctls")
            .expect("look up class_all_ioctls")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_IOCTL_PREFIXES);
        assert_eq!(rules[0].count(), 0x10000);
    }

    #[test]
    fn parse_allowxperm_one_nlmsg() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id =
            policy.classes().get_by_name(b"class_one_nlmsg").expect("look up class_one_nlmsg").id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0x12));
    }

    // `nlmsg` extended permissions that are declared in the same rule, and have the same
    // high byte, are stored in the same `AccessVectorRule` in the compiled policy.
    #[test]
    fn parse_allowxperm_two_nlmsg_same_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_two_nlmsg_same_range")
            .expect("look up class_two_nlmsg_same_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[0].count(), 2);
        assert!(rules[0].contains(0x12));
        assert!(rules[0].contains(0x24));
    }

    // `nlmsg` extended permissions that are declared in the same rule, and have different
    // high bytes, are stored in different `AccessVectorRule`s in the compiled policy.
    #[test]
    fn parse_allowxperm_two_nlmsg_different_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_two_nlmsg_diff_range")
            .expect("look up class_two_nlmsg_diff_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x10);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0x1024));
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[1].count(), 1);
        assert!(rules[1].contains(0x12));
    }

    // The set of `nlmsg` extended permissions with a given high byte is represented by
    // a single `AccessVectorRule` in the compiled policy.
    #[test]
    fn parse_allowxperm_one_nlmsg_range() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_one_nlmsg_range")
            .expect("look up class_one_nlmsg_range")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[0].count(), 0x100);
        for i in 0x0..0xff {
            assert!(rules[0].contains(i), "{i}");
        }
    }

    // A set of `nlmsg` extended permissions consisting of all 16-bit integers with one
    // of 2 given prefix bytes is represented by 2 `AccessVectorRule`s in the compiled policy.
    //
    // The policy compiler allows `nlmsg` extended permission sets of this form, but they
    // are not expected to appear in policies.
    #[test]
    fn parse_allowxperm_two_nlmsg_ranges() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_two_nlmsg_ranges")
            .expect("look up class_two_nlmsg_ranges")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x01);
        assert_eq!(rules[0].count(), 0x100);
        for i in 0x0100..0x01ff {
            assert!(rules[0].contains(i), "{i}");
        }
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[1].count(), 0x100);
        for i in 0x0..0xff {
            assert!(rules[1].contains(i), "{i}");
        }
    }

    // A set of `nlmsg` extended permissions consisting of all 16-bit integers with one
    // of 3 non-consecutive prefix bytes is represented by 3 `AccessVectorRule`s in the
    // compiled policy.
    //
    // The policy compiler allows `nlmsg` extended permission sets of this form, but they
    // are not expected to appear in policies.
    #[test]
    fn parse_allowxperm_three_separate_nlmsg_ranges() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_three_separate_nlmsg_ranges")
            .expect("look up class_three_separate_nlmsg_ranges")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x20);
        assert_eq!(rules[0].count(), 0x100);
        for i in 0x2000..0x20ff {
            assert!(rules[0].contains(i), "{i}");
        }
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x10);
        assert_eq!(rules[1].count(), 0x100);
        for i in 0x1000..0x10ff {
            assert!(rules[1].contains(i), "{i}");
        }
        assert_eq!(rules[2].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[2].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[2].count(), 0x100);
        for i in 0x0..0xff {
            assert!(rules[2].contains(i), "{i}");
        }
    }

    // A set of `nlmsg` extended permissions consisting of all 16-bit integers with one
    // of 3 (or more) consecutive prefix bytes is represented by 2 `AccessVectorRule`s in the
    // compiled policy, one for the smallest prefix byte and one for the largest.
    //
    // The policy compiler allows `nlmsg` extended permission sets of this form, but they
    // are not expected to appear in policies.
    #[test]
    fn parse_allowxperm_three_contiguous_nlmsg_ranges() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_three_contiguous_nlmsg_ranges")
            .expect("look up class_three_contiguous_nlmsg_ranges")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x02);
        assert_eq!(rules[0].count(), 0x100);
        for i in 0x0200..0x02ff {
            assert!(rules[0].contains(i), "{i}");
        }
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[1].count(), 0x100);
        for i in 0x0..0xff {
            assert!(rules[1].contains(i), "{i}");
        }
    }

    // The representation of extended permissions for `auditallowxperm` rules is
    // the same as for `allowxperm` rules.
    #[test]
    fn parse_auditallowxperm() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_auditallowxperm")
            .expect("look up class_auditallowxperm")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AuditAllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0x10));
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x10);
        assert_eq!(rules[1].count(), 1);
        assert!(rules[1].contains(0x1000));
    }

    // The representation of extended permissions for `dontauditxperm` rules is
    // the same as for `allowxperm` rules. In particular, the `AccessVectorRule`
    // contains the same set of extended permissions that appears in the text
    // policy. (This differs from the representation of the access vector in
    // `AccessVectorRule`s for `dontaudit` rules, where the `AccessVectorRule`
    // contains the complement of the access vector that appears in the text
    // policy.)
    #[test]
    fn parse_dontauditxperm() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_dontauditxperm")
            .expect("look up class_dontauditxperm")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::DontAuditXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].xperms_type(), XPERMS_TYPE_NLMSG);
        assert_eq!(rules[0].xperms_optional_prefix(), 0x00);
        assert_eq!(rules[0].count(), 1);
        assert!(rules[0].contains(0x11));
        assert_eq!(rules[1].xperms_type(), XPERMS_TYPE_IOCTL_PREFIX_AND_POSTFIXES);
        assert_eq!(rules[1].xperms_optional_prefix(), 0x10);
        assert_eq!(rules[1].count(), 1);
        assert!(rules[1].contains(0x1000));
    }

    // If an allowxperm rule and an auditallowxperm rule specify exactly the same permissions, they
    // are not coalesced into a single `AccessVectorRule` in the policy; two rules appear in the
    // policy.
    #[test]
    fn parse_auditallowxperm_not_coalesced() {
        let policy_bytes = include_bytes!("../../testdata/micro_policies/allowxperm_policy");
        let policy = parse_policy_by_value(policy_bytes.to_vec()).expect("parse policy");
        let policy = policy.validate().expect("validate policy");

        let class_id = policy
            .classes()
            .get_by_name(b"class_auditallowxperm_not_coalesced")
            .expect("class_auditallowxperm_not_coalesced")
            .id();

        let type0 = policy.types().get_by_name(b"type0").expect("look up type0").id();
        let allow_rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AllowXperm)
            .map(|r| r.extended_permissions())
            .collect();
        let auditallow_rules: Vec<_> = policy
            .access_vector_rules()
            .find_xperm_rules(type0, type0, class_id)
            .filter(|r| r.kind() == RuleKind::AuditAllowXperm)
            .map(|r| r.extended_permissions())
            .collect();

        assert_eq!(allow_rules.len(), 1);
        assert_eq!(allow_rules[0].count(), 1);
        assert!(allow_rules[0].contains(0xabcd));
        assert_eq!(auditallow_rules.len(), 1);
        assert_eq!(auditallow_rules[0].count(), 1);
        assert!(auditallow_rules[0].contains(0xabcd));
    }
}
