// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package discover

// Config holds the configuration for the discover stage.
type Config struct {
	// SkipPaths are exact repository paths (relative to fuchsia dir) to ignore.
	SkipPaths []string

	// SkipAnywhere are basename patterns to ignore anywhere (e.g., ".git").
	SkipAnywhere     []string
	BarrierPaths     []string
	OutOfTreeReadmes map[string]string
}
