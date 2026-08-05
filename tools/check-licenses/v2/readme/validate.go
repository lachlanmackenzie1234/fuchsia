// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package readme

import (
	"path/filepath"
	"strings"

	"go.fuchsia.dev/fuchsia/tools/readme_fuchsia"
)

// Validate checks if the README.fuchsia file structures contain all required fields
// and no unknown fields. It also verifies that referenced paths exist on disk.
// Returns a slice of all encountered errors.
func Validate(fuchsiaDir, readmeFilePath string, readmes []*Readme, config Config) []error {
	readmeDir := filepath.Dir(readmeFilePath)
	if config != nil && config.OutOfTreeReadmes() != nil {
		outOfTree := config.OutOfTreeReadmes()
		for logicalPath, physicalPath := range outOfTree {
			if filepath.Clean(physicalPath) == filepath.Clean(readmeFilePath) {
				readmeDir = filepath.Join(fuchsiaDir, logicalPath)
				break
			}
		}
		IsProjectBoundary(readmeDir, fuchsiaDir, outOfTree)
	}

	relBaseDir, err := filepath.Rel(fuchsiaDir, readmeDir)
	if err != nil {
		relBaseDir = readmeDir
	}
	if relBaseDir == "." {
		relBaseDir = ""
	}

	allowMissingLicense := false
	if config != nil {
		allowMissingLicense = config.HasPolicyException("AllProjectsMustHaveALicense", relBaseDir)
	}

	allowReadmeNeedsUpdate := false
	if config != nil {
		allowReadmeNeedsUpdate = config.HasPolicyException("ReadmeFuchsiaNeedsUpdate", relBaseDir)
	}

	if allowReadmeNeedsUpdate {
		return nil
	}

	errs := readme_fuchsia.Validate(readmeDir, readmes)

	if allowMissingLicense && len(errs) > 0 {
		var filteredErrs []error
		for _, err := range errs {
			msg := err.Error()
			isMissingLicenseErr := strings.Contains(msg, "Missing required field 'License'") || strings.Contains(msg, "Missing required field 'License File'")
			if !isMissingLicenseErr {
				filteredErrs = append(filteredErrs, err)
			}
		}
		errs = filteredErrs
	}

	return errs
}
