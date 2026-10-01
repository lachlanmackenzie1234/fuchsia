// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

//! Generic Segmentation Offload (GSO) support.

use core::fmt::Debug;
use core::marker::PhantomData;

use net_types::ip::{IpAddress, Ipv4, Ipv6};
use netstack3_base::{GsoInfo, MAX_GSO_PAYLOAD_LEN, NetworkSerializationContext, Payload};
use netstack3_filter::{DynTransportSerializer, FilterIpExt, ForwardedPacket, RawIpBody};
use packet::{
    BufferMut, EmptyBuf, InnerPacketBuilder, InnerSerializer, NestablePacketBuilder,
    NestableSerializer as _, Nested, PacketBuilder, ParseBuffer,
};
use packet_formats::icmp::{IcmpIpExt, IcmpMessage, IcmpPacketBuilder};
use packet_formats::ip::IpExt;
use packet_formats::ipv4::{Ipv4PacketBuilder, Ipv4PacketBuilderWithOptions};
use packet_formats::ipv6::{Ipv6PacketBuilder, Ipv6PacketBuilderWithHbhOptions};
use packet_formats::tcp::TcpSegmentBuilderWithOptions;
use packet_formats::udp::UdpPacketBuilder;

use crate::internal::fragmentation::{AsFragmentableIpPacketBuilder, FragmentationIpExt};

/// Reasons a packet can't be segmented in software.
#[derive(Debug, Eq, PartialEq)]
pub enum GsoError {
    /// The packet's headers couldn't be parsed.
    Parse,
    /// The packet doesn't carry a protocol that can be segmented in software.
    NotSegmentable,
    /// The packet carries headers that can't be faithfully replicated into
    /// each segment, e.g. IPv4 options or IPv6 extension headers.
    UnsupportedHeaders,
    /// The packet's payload is longer than [`MAX_GSO_PAYLOAD_LEN`].
    PayloadTooLong,
}

/// An IP version extension trait providing the version-specific parts of
/// software segmentation.
pub trait GsoIpExt:
    IpExt<PacketBuilder<NetworkSerializationContext>: AsSegmentableIpPacketBuilder<Self>>
    + FragmentationIpExt
{
    /// The IP header template for the segments of a forwarded packet.
    type ForwardedSegmentBuilder: SegmentableIpPacketBuilder<Self>;
}

impl GsoIpExt for Ipv4 {
    type ForwardedSegmentBuilder = Ipv4PacketBuilder;
}

impl GsoIpExt for Ipv6 {
    type ForwardedSegmentBuilder = Ipv6PacketBuilder;
}

/// A [`packet::Serializer`] the IP egress path may be able to split into
/// segments.
///
/// This is the transport-layer segmentation counterpart of
/// [`FragmentableIpSerializer`]: rather than serializing the packet and parsing
/// it back, implementations hand out the header builders and payload they
/// already hold, which a [`GsoSegmenter`] stamps onto each segment.
pub trait MaybeSegmentableIpSerializer<I: GsoIpExt> {
    /// The IP header template for every segment.
    type IpBuilder<'a>: SegmentableIpPacketBuilder<I>
    where
        Self: 'a;

    /// The transport header template for every segment.
    type TransportBuilder<'a>: SegmentableTransportBuilder
    where
        Self: 'a;

    /// The transport payload split across the segments.
    type Payload<'a>: Payload + InnerPacketBuilder + Copy
    where
        Self: 'a;

    /// Returns a segmenter splitting this packet into `gso_info`-sized
    /// segments.
    ///
    /// Returns an error if the packet can't be segmented in software.
    fn try_segmenter(
        &self,
        gso_info: GsoInfo,
    ) -> Result<
        GsoSegmenter<I, Self::IpBuilder<'_>, Self::TransportBuilder<'_>, Self::Payload<'_>>,
        GsoError,
    >;
}

impl<I, B> MaybeSegmentableIpSerializer<I> for ForwardedPacket<I, B>
where
    I: GsoIpExt,
    B: BufferMut,
{
    type IpBuilder<'a>
        = I::ForwardedSegmentBuilder
    where
        Self: 'a;
    // TODO(https://fxbug.dev/452980285): Recover the TCP header builder from
    // the packet's bytes.
    type TransportBuilder<'a>
        = !
    where
        Self: 'a;
    type Payload<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn try_segmenter(
        &self,
        _gso_info: GsoInfo,
    ) -> Result<GsoSegmenter<I, I::ForwardedSegmentBuilder, !, &[u8]>, GsoError> {
        // TODO(https://fxbug.dev/452980285): Parse the packet and segment it.
        Err(GsoError::NotSegmentable)
    }
}

impl<I, S, B> MaybeSegmentableIpSerializer<I> for Nested<S, B>
where
    I: GsoIpExt,
    S: MaybeSegmentableTransportSerializer,
    B: AsSegmentableIpPacketBuilder<I>,
{
    type IpBuilder<'a>
        = B::Builder<'a>
    where
        Self: 'a;
    type TransportBuilder<'a>
        = S::Builder<'a>
    where
        Self: 'a;
    type Payload<'a>
        = S::Payload<'a>
    where
        Self: 'a;

    fn try_segmenter(
        &self,
        gso_info: GsoInfo,
    ) -> Result<GsoSegmenter<I, B::Builder<'_>, S::Builder<'_>, S::Payload<'_>>, GsoError> {
        let ip_builder = self.outer().try_as_segmentable()?;
        let (transport_builder, payload) = self.inner().try_as_segmentable()?;
        GsoSegmenter::new(ip_builder, transport_builder, payload, gso_info)
    }
}

/// A type that may be viewed as the IP header template for a packet's
/// segments.
pub trait AsSegmentableIpPacketBuilder<I: GsoIpExt> {
    /// The segment header template borrowed from this type.
    type Builder<'a>: SegmentableIpPacketBuilder<I>
    where
        Self: 'a;

    /// Attempts to borrow a segment header template, returning an error if
    /// this builder's headers can't be replicated into every segment.
    fn try_as_segmentable(&self) -> Result<Self::Builder<'_>, GsoError>;
}

impl AsSegmentableIpPacketBuilder<Ipv4> for Ipv4PacketBuilder {
    type Builder<'a>
        = &'a Ipv4PacketBuilder
    where
        Self: 'a;

    fn try_as_segmentable(&self) -> Result<&Ipv4PacketBuilder, GsoError> {
        Ok(self)
    }
}

impl AsSegmentableIpPacketBuilder<Ipv6> for Ipv6PacketBuilder {
    type Builder<'a>
        = &'a Ipv6PacketBuilder
    where
        Self: 'a;

    fn try_as_segmentable(&self) -> Result<&Ipv6PacketBuilder, GsoError> {
        Ok(self)
    }
}

impl<O> AsSegmentableIpPacketBuilder<Ipv4> for Ipv4PacketBuilderWithOptions<'_, O> {
    type Builder<'a>
        = !
    where
        Self: 'a;

    fn try_as_segmentable(&self) -> Result<!, GsoError> {
        // GSO doesn't currently support IPv4 options.
        Err(GsoError::UnsupportedHeaders)
    }
}

impl<O> AsSegmentableIpPacketBuilder<Ipv6> for Ipv6PacketBuilderWithHbhOptions<'_, O> {
    type Builder<'a>
        = !
    where
        Self: 'a;

    fn try_as_segmentable(&self) -> Result<!, GsoError> {
        // GSO doesn't currently support IPv6 extension headers.
        Err(GsoError::UnsupportedHeaders)
    }
}

/// A template for the IP header of each of the segments a packet is split into.
///
/// This is IP-layer counterpart of [`SegmentableTransportBuilder`].
pub trait SegmentableIpPacketBuilder<I: GsoIpExt> {
    /// The builder for a single segment's IP header.
    type SegmentBuilder: PacketBuilder<NetworkSerializationContext>
        + NestablePacketBuilder
        + AsFragmentableIpPacketBuilder<I>;

    /// Returns the header builder for the Nth segment, where N is a zero-based
    /// ordinal given by `index`.
    fn segment_builder(&self, index: u16, gso_info: GsoInfo) -> Self::SegmentBuilder;
}

impl SegmentableIpPacketBuilder<Ipv4> for Ipv4PacketBuilder {
    type SegmentBuilder = Self;

    fn segment_builder(&self, _index: u16, _gso_info: GsoInfo) -> Self {
        // TODO(https://fxbug.dev/452980285): Set each packet's ID according to
        // `gso_info.ipv4_id_mode`.
        self.clone()
    }
}

impl SegmentableIpPacketBuilder<Ipv6> for Ipv6PacketBuilder {
    type SegmentBuilder = Self;

    fn segment_builder(&self, _index: u16, _gso_info: GsoInfo) -> Self {
        // No header fields need to be updated.
        self.clone()
    }
}

impl SegmentableIpPacketBuilder<Ipv4> for ! {
    type SegmentBuilder = Ipv4PacketBuilder;

    fn segment_builder(&self, _index: u16, _gso_info: GsoInfo) -> Ipv4PacketBuilder {
        match *self {}
    }
}

impl SegmentableIpPacketBuilder<Ipv6> for ! {
    type SegmentBuilder = Ipv6PacketBuilder;

    fn segment_builder(&self, _index: u16, _gso_info: GsoInfo) -> Ipv6PacketBuilder {
        match *self {}
    }
}

// Allow both owned builders and builder references to be used as segment header
// templates.
impl<I: GsoIpExt, T: SegmentableIpPacketBuilder<I>> SegmentableIpPacketBuilder<I> for &T {
    type SegmentBuilder = T::SegmentBuilder;

    fn segment_builder(&self, index: u16, gso_info: GsoInfo) -> T::SegmentBuilder {
        T::segment_builder(*self, index, gso_info)
    }
}

/// A transport-layer [`packet::Serializer`] that may carry a payload splittable
/// in software.
pub trait MaybeSegmentableTransportSerializer {
    /// The transport header stamped onto every segment.
    type Builder<'a>: SegmentableTransportBuilder
    where
        Self: 'a;

    /// The transport payload split across the segments.
    type Payload<'a>: Payload + InnerPacketBuilder + Copy
    where
        Self: 'a;

    /// Attempts to extract the transport header template and the payload to
    /// split, returning an error if this serializer can't be segmented.
    fn try_as_segmentable(&self) -> Result<(Self::Builder<'_>, Self::Payload<'_>), GsoError>;
}

/// Implements [`MaybeSegmentableTransportSerializer`] for serializers that do
/// not support segmentation.
macro_rules! impl_not_segmentable {
    ([$($generics:tt)*] $ty:ty) => {
        impl<$($generics)*> MaybeSegmentableTransportSerializer for $ty {
            type Builder<'x> = ! where Self: 'x;
            type Payload<'x> = &'x [u8] where Self: 'x;

            fn try_as_segmentable(
                &self,
            ) -> Result<(!, &[u8]), GsoError> {
                Err(GsoError::NotSegmentable)
            }
        }
    };
}

// TODO(https://fxbug.dev/452980285): Support TCP GSO.
impl_not_segmentable!([S, A: IpAddress, O] Nested<S, TcpSegmentBuilderWithOptions<A, O>>);
// TODO(https://fxbug.dev/566314871): Support UDP GSO.
impl_not_segmentable!([S, A: IpAddress] Nested<S, UdpPacketBuilder<A>>);

// TODO(https://fxbug.dev/452980285): Require serializers behind
// `DynTransportSerializer` to be GSO-aware.
impl_not_segmentable!(['a, I: FilterIpExt] DynTransportSerializer<'a, I>);
impl_not_segmentable!([I: IpExt, B: ParseBuffer] RawIpBody<I, B>);
impl_not_segmentable!([S, I: IcmpIpExt, M: IcmpMessage<I>] Nested<S, IcmpPacketBuilder<I, M>>);
impl_not_segmentable!([S] Nested<S, ()>);
impl_not_segmentable!([P, B] InnerSerializer<P, B>);

/// A template for the transport header of each of the segments a packet is
/// split into.
///
/// This is the transport-layer counterpart of [`SegmentableIpPacketBuilder`].
/// Implemented both for owned builders and for references to them, so a
/// template the stack already holds can be used without copying it up front.
pub trait SegmentableTransportBuilder {
    /// The builder for a single segment's transport header.
    type SegmentBuilder: PacketBuilder<NetworkSerializationContext> + NestablePacketBuilder;

    /// Returns the header builder for the segment carrying the payload bytes
    /// starting at `offset`.
    ///
    /// `has_more` indicates that further segments follow this one.
    fn segment_builder(&self, offset: u16, has_more: bool) -> Self::SegmentBuilder;
}

impl<A: IpAddress, O: InnerPacketBuilder + Clone> SegmentableTransportBuilder
    for TcpSegmentBuilderWithOptions<A, O>
{
    type SegmentBuilder = Self;

    fn segment_builder(&self, _offset: u16, _has_more: bool) -> Self {
        // TODO(https://fxbug.dev/452980285): Update each segment's flags and
        // sequence number if necessary.
        self.clone()
    }
}

impl<T: SegmentableTransportBuilder> SegmentableTransportBuilder for &T {
    type SegmentBuilder = T::SegmentBuilder;

    fn segment_builder(&self, offset: u16, has_more: bool) -> T::SegmentBuilder {
        T::segment_builder(*self, offset, has_more)
    }
}

impl SegmentableTransportBuilder for ! {
    type SegmentBuilder = !;

    fn segment_builder(&self, _offset: u16, _has_more: bool) -> ! {
        match *self {}
    }
}

/// Splits a single oversized packet into discrete segments.
///
/// Created via [`MaybeSegmentableIpSerializer::try_segmenter`], which fails if
/// the packet doesn't require (or doesn't support) software segmentation.
///
/// The header templates may be owned or borrowed (see
/// [`SegmentableIpPacketBuilder`] and [`SegmentableTransportBuilder`]); either
/// way, they are only copied once per emitted segment.
pub struct GsoSegmenter<I: GsoIpExt, IB, B, P> {
    /// The template for each segment's IP header.
    ip_builder: IB,
    /// The template for each segment's transport header.
    transport_builder: B,
    /// The transport payload being split.
    payload: P,
    /// The length of `payload`.
    payload_len: u16,
    /// The number of payload bytes already emitted.
    consumed: u16,
    /// The index of the next segment to be emitted.
    index: u16,
    gso_info: GsoInfo,
    _marker: PhantomData<I>,
}

impl<I, IB, B, P> GsoSegmenter<I, IB, B, P>
where
    I: GsoIpExt,
    IB: SegmentableIpPacketBuilder<I>,
    B: SegmentableTransportBuilder,
    P: Payload + InnerPacketBuilder + Copy,
{
    fn new(
        ip_builder: IB,
        transport_builder: B,
        payload: P,
        gso_info: GsoInfo,
    ) -> Result<Self, GsoError> {
        if payload.len() > usize::from(MAX_GSO_PAYLOAD_LEN) {
            return Err(GsoError::PayloadTooLong);
        }
        let payload_len =
            u16::try_from(payload.len()).expect("payload length is at most MAX_GSO_PAYLOAD_LEN");
        Ok(Self {
            ip_builder,
            transport_builder,
            payload,
            payload_len,
            consumed: 0,
            index: 0,
            gso_info,
            _marker: PhantomData,
        })
    }
}

/// The serializer for a single segment produced by a [`GsoSegmenter`].
type SegmentSerializer<P, TB, IB> = Nested<Nested<InnerSerializer<P, EmptyBuf>, TB>, IB>;

impl<I, IB, B, P> Iterator for GsoSegmenter<I, IB, B, P>
where
    I: GsoIpExt,
    IB: SegmentableIpPacketBuilder<I>,
    B: SegmentableTransportBuilder,
    P: Payload + InnerPacketBuilder + Copy,
{
    /// The serializer for the next segment and whether more segments are
    /// pending.
    type Item = (SegmentSerializer<P, B::SegmentBuilder, IB::SegmentBuilder>, bool);

    fn next(&mut self) -> Option<Self::Item> {
        let Self {
            ip_builder,
            transport_builder,
            payload,
            payload_len,
            consumed,
            index,
            gso_info,
            _marker: _,
        } = self;
        let GsoInfo { gso_size: max_segment_body, ipv4_id_mode: _ } = gso_info;

        let remaining = *payload_len - *consumed;
        if remaining == 0 {
            return None;
        }
        let take = remaining.min(max_segment_body.get());
        let has_more = take < remaining;
        let segment_body = payload.slice(u32::from(*consumed)..u32::from(*consumed + take));

        let transport_builder = transport_builder.segment_builder(*consumed, has_more);
        let ip_builder = ip_builder.segment_builder(*index, *gso_info);

        *consumed += take;
        *index = index.checked_add(1).expect("number of segments cannot exceed u16::MAX");

        Some((
            segment_body.into_serializer().wrap_in(transport_builder).wrap_in(ip_builder),
            has_more,
        ))
    }
}
