// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package classify

// Config holds the configuration for the classify stage.
type Config struct {
	// Threshold is the confidence threshold for license classification (default 0.8).
	Threshold float64

	// TargetExtensions is a map of file extensions (including the dot, e.g., ".cc")
	// that the classifier should attempt to read and analyze for licenses.
	TargetExtensions map[string]bool

	// PatternDirs is the list of directories containing license patterns.
	PatternDirs []string

	// LicenseCategories maps a license name (e.g., "GPL-2.0") to its policy category ("Restricted").
	LicenseCategories map[string]string
}

// NewConfig initializes an empty classify configuration with default threshold.
func NewConfig() Config {
	return Config{
		Threshold:         0.8,
		TargetExtensions:  make(map[string]bool),
		LicenseCategories: make(map[string]string),
	}
}

// CategoryForLicense returns the approved policy category (e.g., Restricted, Notice, Exception)
// for the given license pattern name. Defaults to "Uncategorized" if unknown.
func (c *Config) CategoryForLicense(licenseName string) string {
	if c == nil || c.LicenseCategories == nil {
		return "Uncategorized"
	}
	if cat, ok := c.LicenseCategories[licenseName]; ok && cat != "" && cat != "allowed_licenses" {
		return cat
	}
	return "Uncategorized"
}
