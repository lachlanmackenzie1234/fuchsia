// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::error::PrettyPrintRef;
use cm_rust::{
    DebugRegistration, ExposeDecl, ExposeDeclCommon, ExposeSource, OfferDecl, OfferDeclCommon,
    OfferSource, RegistrationSource, ResolverRegistration, RunnerRegistration, UseDecl,
    UseDeclCommon, UseSource,
};
use cm_types::Name;

pub trait ToSource {
    fn to_source(&self) -> PrettyPrintRef;
}

impl ToSource for UseDecl {
    fn to_source(&self) -> PrettyPrintRef {
        match self.source() {
            UseSource::Parent => PrettyPrintRef::Parent,
            UseSource::Framework => PrettyPrintRef::Framework,
            UseSource::Debug => PrettyPrintRef::Debug,
            UseSource::Self_ => PrettyPrintRef::Self_,
            UseSource::Capability(name) => PrettyPrintRef::Capability(name.clone()),
            UseSource::Child(name) => PrettyPrintRef::Child(name.clone()),
            UseSource::Collection(name) => PrettyPrintRef::Collection(name.clone()),
            #[cfg(fuchsia_api_level_at_least = "HEAD")]
            UseSource::Environment => PrettyPrintRef::Environment,
        }
    }
}

impl ToSource for OfferDecl {
    fn to_source(&self) -> PrettyPrintRef {
        match self.source() {
            OfferSource::Framework => PrettyPrintRef::Framework,
            OfferSource::Parent => PrettyPrintRef::Parent,
            OfferSource::Child(child_ref) if child_ref.collection.is_none() => {
                PrettyPrintRef::Child(Name::new(child_ref.name.as_str()).unwrap())
            }
            OfferSource::Child(child_ref) => PrettyPrintRef::ChildInCollection(
                child_ref.name.clone(),
                child_ref.collection.clone().unwrap(),
            ),
            OfferSource::Collection(name) => PrettyPrintRef::Collection(name.clone()),
            OfferSource::Self_ => PrettyPrintRef::Self_,
            OfferSource::Capability(name) => PrettyPrintRef::Capability(name.clone()),
            OfferSource::Void => PrettyPrintRef::Void,
        }
    }
}

impl ToSource for ExposeDecl {
    fn to_source(&self) -> PrettyPrintRef {
        match self.source() {
            ExposeSource::Self_ => PrettyPrintRef::Self_,
            ExposeSource::Child(name) => PrettyPrintRef::Child(name.clone()),
            ExposeSource::Collection(name) => PrettyPrintRef::Collection(name.clone()),
            ExposeSource::Framework => PrettyPrintRef::Framework,
            ExposeSource::Capability(name) => PrettyPrintRef::Capability(name.clone()),
            ExposeSource::Void => PrettyPrintRef::Void,
        }
    }
}

fn registration_source_to_pretty_print_ref(source: &RegistrationSource) -> PrettyPrintRef {
    match source {
        RegistrationSource::Parent => PrettyPrintRef::Parent,
        RegistrationSource::Self_ => PrettyPrintRef::Self_,
        RegistrationSource::Child(name) => PrettyPrintRef::Child(Name::new(name).unwrap()),
    }
}

impl ToSource for DebugRegistration {
    fn to_source(&self) -> PrettyPrintRef {
        let DebugRegistration::Protocol(debug) = self;
        registration_source_to_pretty_print_ref(&debug.source)
    }
}

impl ToSource for RunnerRegistration {
    fn to_source(&self) -> PrettyPrintRef {
        registration_source_to_pretty_print_ref(&self.source)
    }
}

impl ToSource for ResolverRegistration {
    fn to_source(&self) -> PrettyPrintRef {
        registration_source_to_pretty_print_ref(&self.source)
    }
}
