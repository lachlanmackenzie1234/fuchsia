// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package config

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"go.fuchsia.dev/fuchsia/tools/check-licenses/v2/stages/classify"
	"go.fuchsia.dev/fuchsia/tools/check-licenses/v2/stages/discover"
	"go.fuchsia.dev/fuchsia/tools/check-licenses/v2/stages/validate"
)

// MasterConfig is the fully assembled configuration injected into the pipeline stages.
// It is constructed by the ConfigBuilder during the Assembly Phase by merging all
// scattered JSON files from the open-source and proprietary assets directories.
type MasterConfig struct {
	// FuchsiaDir is the authoritative absolute root path of the workspace.
	FuchsiaDir string

	// --- LEGACY FIELDS FOR INCREMENTAL MIGRATION ---
	SkipPaths           []string
	SkipAnywhere        []string
	TargetExtensions    map[string]bool
	CopyrightExtensions map[string]bool
	BarrierPaths        []string
	OutOfTreeReadmes    map[string]string
	PatternsDir         string
	PolicyExceptions    map[string]map[string]validate.RuleMetadata
	AllowedLicenses     map[string]map[string]validate.RuleMetadata

	// --- Injected into Discoverer (Stage 1) ---

	Discover discover.Config

	// --- Injected into Classifier (Stage 4) ---

	Classify classify.Config

	// --- Injected into Validator (Stage 5) ---

	// Validate holds the configuration for the validator stage.
	Validate validate.Config

	// ManifestProjectNames maps a project's filesystem path to its name in the manifest.
	// Key: Project path (e.g., "prebuilt/media/firmware/amlogic-decoder")
	// Value: Package name (e.g., "fuchsia_internal/firmware/amlogic-video")
	ManifestProjectNames map[string]string

	// ManifestPrivateProjects tracks if a project path was found in a private manifest.
	ManifestPrivateProjects map[string]bool

	// LicenseCategories maps a license name (e.g., "GPL-2.0") to its policy category ("Restricted").
	LicenseCategories map[string]string
}

func resolveFuchsiaDir(dir string) string {
	if dir != "" {
		return dir
	}
	if env := os.Getenv("FUCHSIA_DIR"); env != "" {
		return env
	}
	return "."
}

// IsPrivateProject returns true if the project path belongs to a proprietary/private
// repository. It prevents open-source compliance configs from being contaminated.
func (c *MasterConfig) IsPrivateProject(projectPath string) bool {
	if c == nil {
		return false
	}
	projectPath = filepath.Clean(projectPath)
	slashPath := filepath.ToSlash(projectPath)

	parts := strings.Split(strings.TrimPrefix(slashPath, "/"), "/")
	if len(parts) > 0 && parts[0] == "vendor" {
		return true
	}

	for p := projectPath; p != "." && p != "/" && p != filepath.Dir(p); p = filepath.Dir(p) {
		// 1. Check if marked private from integration folder
		if c.ManifestPrivateProjects[p] {
			return true
		}

		// 2. Check manifest name prefix
		if name, ok := c.ManifestProjectNames[p]; ok {
			if strings.HasPrefix(name, "fuchsia_internal/") || strings.HasPrefix(name, "vendor/") {
				return true
			}
		}
	}

	return false
}

// AssetRootFor returns the base asset directory path for a project based on its privacy state.
func (c *MasterConfig) AssetRootFor(projectPath string) string {
	fDir := resolveFuchsiaDir("")
	if c != nil {
		fDir = resolveFuchsiaDir(c.FuchsiaDir)
	}

	cleanPath := strings.TrimPrefix(filepath.ToSlash(filepath.Clean(projectPath)), "/")
	isPrivate := (c != nil && c.IsPrivateProject(projectPath)) || (c == nil && (strings.HasPrefix(cleanPath, "vendor/") || cleanPath == "vendor"))
	if isPrivate {
		return filepath.Join(fDir, "vendor", "google", "tools", "check-licenses", "assets")
	}
	return filepath.Join(fDir, "tools", "check-licenses", "assets")
}

// ConfigRootFor returns the base config directory path for a project based on its privacy state.
func (c *MasterConfig) ConfigRootFor(projectPath string) string {
	return filepath.Join(c.AssetRootFor(projectPath), "configs")
}

// ReadmeRootFor returns the base readmes asset directory path for a project based on its privacy state.
func (c *MasterConfig) ReadmeRootFor(projectPath string) string {
	return filepath.Join(c.AssetRootFor(projectPath), "readmes")
}

// RootReadmePath returns the absolute file path to the primary first-party repository root README.fuchsia.
func (c *MasterConfig) RootReadmePath() string {
	return filepath.Join(c.ReadmeRootFor(""), "README.fuchsia")
}

// ResolveReadmeWritePath determines the appropriate filesystem target path for writing
// or updating a project's README.fuchsia file. If a project resides under prebuilt/, its README
// is redirected to the corresponding virtual assets directory.
func (c *MasterConfig) ResolveReadmeWritePath(projectRoot, currentReadmePath string) (string, error) {
	relRoot, err := filepath.Rel(c.FuchsiaDir, projectRoot)
	if err != nil {
		return currentReadmePath, err
	}
	if strings.HasPrefix(relRoot, "prebuilt/") && !strings.Contains(currentReadmePath, "assets/readmes") {
		writePath := filepath.Join(c.ReadmeRootFor(relRoot), relRoot, "README.fuchsia")
		if err := os.MkdirAll(filepath.Dir(writePath), 0755); err != nil {
			return "", fmt.Errorf("failed to create asset directory for %s: %w", projectRoot, err)
		}
		return writePath, nil
	}
	if physPath := c.Discover.OutOfTreeReadmes[relRoot]; physPath != "" {
		absPhys := physPath
		if !filepath.IsAbs(absPhys) {
			absPhys = filepath.Join(c.FuchsiaDir, absPhys)
		}
		if absPhys != filepath.Join(c.FuchsiaDir, relRoot, "README.fuchsia") {
			return absPhys, nil
		}
	}
	return currentReadmePath, nil
}

// ResolveAndValidatePath normalizes the fuchsia root and ensures the given input path
// resides safely within that root. Returns the relative target path,
// or an error if the path escapes the root workspace.
func (c *MasterConfig) ResolveAndValidatePath(inputPath string) (string, error) {
	c.FuchsiaDir = resolveFuchsiaDir(c.FuchsiaDir)
	absFuchsiaDir, err := filepath.Abs(c.FuchsiaDir)
	if err != nil {
		return "", fmt.Errorf("failed to get absolute path for fuchsia_dir %s: %w", c.FuchsiaDir, err)
	}
	c.FuchsiaDir = absFuchsiaDir

	if strings.HasPrefix(inputPath, "//") {
		inputPath = filepath.Join(absFuchsiaDir, strings.TrimPrefix(inputPath, "//"))
	}

	absInputPath, err := filepath.Abs(inputPath)
	if err != nil {
		return "", fmt.Errorf("failed to get absolute path for %s: %w", inputPath, err)
	}

	rel, err := filepath.Rel(absFuchsiaDir, absInputPath)
	if err != nil || strings.HasPrefix(rel, "..") {
		return "", fmt.Errorf("path %s must be inside fuchsia root %s", inputPath, absFuchsiaDir)
	}
	if rel == "." {
		rel = ""
	}
	return rel, nil
}

// CategoryForLicense returns the approved policy category (e.g., Restricted, Notice, Exception)
// for the given license name, or "Uncategorized" if the license is unapproved or unknown.
func (c *MasterConfig) CategoryForLicense(licenseName string) string {
	if c != nil {
		if cat := c.LicenseCategories[licenseName]; cat != "" && cat != "allowed_licenses" {
			return cat
		}
	}
	return "Uncategorized"
}

// NewMasterConfig initializes an empty configuration ready to be populated by the builder.
func NewMasterConfig(fuchsiaDir string) *MasterConfig {
	fuchsiaDir = resolveFuchsiaDir(fuchsiaDir)
	if abs, err := filepath.Abs(fuchsiaDir); err == nil {
		fuchsiaDir = abs
	}
	c := &MasterConfig{
		FuchsiaDir: fuchsiaDir,
		Discover: discover.Config{
			SkipPaths:    make([]string, 0),
			SkipAnywhere: make([]string, 0),
		},

		Classify: classify.Config{
			TargetExtensions: make(map[string]bool),
		},
		Validate: validate.Config{
			PolicyExceptions:    make(map[string]map[string]validate.RuleMetadata),
			AllowedLicenses:     make(map[string]map[string]validate.RuleMetadata),
			CopyrightExtensions: make(map[string]bool),
			OutOfTreeReadmes:    make(map[string]string),
		},
		PolicyExceptions:        make(map[string]map[string]validate.RuleMetadata),
		AllowedLicenses:         make(map[string]map[string]validate.RuleMetadata),
		ManifestProjectNames:    make(map[string]string),
		ManifestPrivateProjects: make(map[string]bool),
		LicenseCategories:       make(map[string]string),
		TargetExtensions:        make(map[string]bool),
		CopyrightExtensions:     make(map[string]bool),
		OutOfTreeReadmes:        make(map[string]string),
	}
	c.Classify.PatternDirs = []string{
		filepath.Join(c.AssetRootFor(""), "patterns"),
		filepath.Join(c.AssetRootFor("vendor/google"), "patterns"),
	}
	return c
}

// --- JSON File Schemas ---
// These structs define the expected shape of the individual JSON files scattered
// throughout the `assets/configs/` directory. Any JSON file can contain any combination
// of these fields, allowing configuration to be organized by project or by theme.

type ConfigFile struct {
	Includes            []string                    `json:"includes,omitempty"`
	Skips               []SkipEntry                 `json:"skips,omitempty"`
	TargetExtensions    *ExtensionEntry             `json:"target_extensions,omitempty"`
	CopyrightExtensions *ExtensionEntry             `json:"copyright_extensions,omitempty"`
	Barriers            []BarrierEntry              `json:"barriers,omitempty"`
	PolicyExceptions    map[string][]AllowlistEntry `json:"policy_exceptions,omitempty"`
	AllowedLicenses     map[string][]AllowlistEntry `json:"allowed_licenses,omitempty"`
}

type SkipEntry struct {
	Bug          string   `json:"bug,omitempty"`
	Description  string   `json:"description,omitempty"`
	Paths        []string `json:"paths"`
	SkipAnywhere bool     `json:"skipAnywhere,omitempty"`
}

type ExtensionEntry struct {
	Description string   `json:"description,omitempty"`
	Extensions  []string `json:"extensions"` // E.g., [".cc", ".cpp", ".h"]
}

type BarrierEntry struct {
	Bug         string   `json:"bug,omitempty"`
	Description string   `json:"description,omitempty"`
	Paths       []string `json:"paths"`
}

type AllowlistEntry struct {
	Bug         string   `json:"bug,omitempty"`
	Description string   `json:"description,omitempty"`
	Paths       []string `json:"paths"` // Paths to allowed projects/files
}

// LEGACY CONSTANTS
const (
	PolicyCheckAllLicenseTextsMustBeRecognized                     = validate.PolicyUnrecognizedLicense
	PolicyCheckAllFuchsiaAuthorSourceFilesMustHaveCopyrightHeaders = validate.PolicyFuchsiaCopyright
	PolicyCheckAllProjectsMustHaveALicense                         = validate.PolicyNoLicense
	CheckNameReadmeFuchsiaNeedsUpdate                              = validate.CheckReadmeNeedsUpdate
	CheckNameAllLicensePatternUsagesMustBeApproved                 = validate.CheckPatternApproval
)

type RuleMetadata = validate.RuleMetadata

var ValidPolicyChecks = map[string]bool{
	PolicyCheckAllLicenseTextsMustBeRecognized:                     true,
	PolicyCheckAllFuchsiaAuthorSourceFilesMustHaveCopyrightHeaders: true,
	PolicyCheckAllProjectsMustHaveALicense:                         true,
}
