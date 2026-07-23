// Copyright 2026 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_INCLUDE_KERNEL_FFI_H_
#define ZIRCON_KERNEL_INCLUDE_KERNEL_FFI_H_

// This header defines helper macros for C++ to Rust FFI routines in the Zircon kernel.
//
// Short C++ FFI helper routines (such as trivial forwarding shims or simple accessors)
// should be annotated with `FFI_ALWAYS_INLINE` to ensure inlining under Clang without
// triggering `-Werror=attributes` under GCC (which requires the `inline` keyword for
// `[[gnu::always_inline]]` definitions, but adding `inline` alters non-static linkage
// semantics across translation units).
//
// TODO(https://fxbug.dev/537458631): Remove this header and annotations once cross-language
// inlining and toolchain support are resolved.
#ifdef __clang__
#define FFI_ALWAYS_INLINE [[gnu::always_inline]]
#else
#define FFI_ALWAYS_INLINE
#endif

#endif  // ZIRCON_KERNEL_INCLUDE_KERNEL_FFI_H_
