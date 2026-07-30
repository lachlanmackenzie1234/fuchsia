// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package classify

// Config holds the configuration for the classify stage.
type Config struct {
	// TargetExtensions is a map of file extensions (including the dot, e.g., ".cc")
	// that the classifier should attempt to read and analyze for licenses.
	TargetExtensions map[string]bool

	// PatternDirs is the list of directories containing license patterns.
	PatternDirs []string
}
