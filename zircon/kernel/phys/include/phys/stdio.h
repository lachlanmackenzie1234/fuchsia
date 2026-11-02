// Copyright 2021 The Fuchsia Authors
//
// Use of this source code is governed by a MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT

#ifndef ZIRCON_KERNEL_PHYS_INCLUDE_PHYS_STDIO_H_
#define ZIRCON_KERNEL_PHYS_INCLUDE_PHYS_STDIO_H_

#include <stddef.h>
#include <stdio.h>
#include <zircon/compiler.h>

// Returns a FILE that discards everything written to it.  *stdout starts out
// as a copy of it.
FILE* NullFile();

struct CallableFile {
  FILE* f = nullptr;

  int operator()(std::string_view str) const { return f->Write(str); }
};

// A printf that respects the `kernel.phys.verbose` boot option: if the option
// is false, nothing will be printed.
void debugf(const char* fmt, ...) __PRINTFLIKE(1, 2);

#endif  // ZIRCON_KERNEL_PHYS_INCLUDE_PHYS_STDIO_H_
