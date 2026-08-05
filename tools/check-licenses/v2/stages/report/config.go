// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package report

import (
	"go.fuchsia.dev/fuchsia/tools/check-licenses/v2/stages/validate"
)

// Config holds the configuration for the report stage.
type Config struct {
	VerifyReadmes            bool
	WriteReadmes             bool
	GenerateArtifacts        bool
	OutOfTreeReadmes         map[string]string
	MissingLicenseExceptions map[string]validate.RuleMetadata
}

// NewConfig initializes an empty report configuration.
func NewConfig() Config {
	return Config{
		OutOfTreeReadmes:         make(map[string]string),
		MissingLicenseExceptions: make(map[string]validate.RuleMetadata),
	}
}
