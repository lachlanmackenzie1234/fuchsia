// Copyright 2018 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

// WARNING: THIS FILE IS MACHINE GENERATED. DO NOT EDIT.
// Generated from the banjo.examples.tables banjo file

#pragma once


#include <zircon/compiler.h>
#include <zircon/types.h>

__BEGIN_CDECLS

// Forward declarations
typedef struct c c_t;
typedef struct b b_t;
typedef struct a a_t;
typedef struct d d_t;

// Declarations
struct c {
    zx_handle_t baz;
};

struct b {
    a_t bar;
};

struct a {
    b_t foo;
};

struct d {
    c_t qux;
};


__END_CDECLS
