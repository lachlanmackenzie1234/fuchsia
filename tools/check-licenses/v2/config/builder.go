// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package config

import (
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

// Builder handles the Assembly Phase, scanning directories for config files
// and aggregating them into a MasterConfig.
type Builder struct {
	FuchsiaDir string
	Config     *MasterConfig
	seen       map[string]bool
}

// NewBuilder creates a new config assembler.
func NewBuilder(fuchsiaDir string) *Builder {
	b := &Builder{
		FuchsiaDir: fuchsiaDir,
		Config:     NewMasterConfig(fuchsiaDir),
		seen:       make(map[string]bool),
	}
	b.Config.PatternsDir = filepath.Join(fuchsiaDir, "tools", "check-licenses", "assets", "patterns")
	return b
}

// Assemble starts the recursive configuration discovery from the root v2 config file,
// merging all found JSON files into the internal MasterConfig.
func (b *Builder) Assemble() error {
	if err := b.LoadManifests(); err != nil {
		fmt.Fprintf(os.Stderr, "Warning: Failed to load manifests: %v\n", err)
	}

	seedFile := filepath.Join(b.FuchsiaDir, "tools", "check-licenses", "v2", "config.json")
	if _, err := os.Stat(seedFile); os.IsNotExist(err) {
		// Fallback for tests or environments where the seed file doesn't exist
		return nil
	}
	return b.parseConfigFile(seedFile)
}

func (b *Builder) walkDir(baseDir string) error {
	return filepath.WalkDir(baseDir, func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if d.IsDir() {
			return nil
		}

		relPath, err := filepath.Rel(baseDir, path)
		if err != nil {
			return err
		}
		parts := strings.Split(relPath, string(os.PathSeparator))
		if len(parts) == 0 {
			return nil
		}

		// Backward compatibility logic for the "assets" directory structure
		category := parts[0]
		if category == "readmes" && filepath.Base(path) == "README.fuchsia" {
			if len(parts) > 1 {
				logicalParts := parts[1 : len(parts)-1]
				logicalPath := filepath.Clean(filepath.Join(logicalParts...))
				b.Config.OutOfTreeReadmes[logicalPath] = path
			}
			return nil
		}

		if filepath.Ext(path) == ".json" {
			if filepath.Base(path) == "template.json" || filepath.Base(path) == "config.json" {
				return nil
			}
			return b.parseConfigFile(path)
		}

		return nil
	})
}

func (b *Builder) parseConfigFile(path string) error {
	path, err := filepath.Abs(path)
	if err != nil {
		return err
	}
	if b.seen[path] {
		return nil
	}
	b.seen[path] = true

	bytes, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var f ConfigFile
	if err := json.Unmarshal(bytes, &f); err != nil {
		return fmt.Errorf("failed to parse config file %q: %w", path, err)
	}

	// 0. Process Includes
	for _, include := range f.Includes {
		absInclude := include
		if !filepath.IsAbs(include) {
			absInclude = filepath.Join(b.FuchsiaDir, include)
		}

		info, err := os.Stat(absInclude)
		if err != nil {
			continue
		}

		if info.IsDir() {
			if err := b.walkDir(absInclude); err != nil {
				return err
			}
		} else {
			if err := b.parseConfigFile(absInclude); err != nil {
				return err
			}
		}
	}

	// 1. Process Skips
	for _, skip := range f.Skips {
		if skip.Bug == "" && filepath.Base(path) != "default.json" && filepath.Base(path) != "hidden_dirs.json" && filepath.Base(path) != "test_dirs.json" && filepath.Base(path) != "bazel_vendor.json" {
			return fmt.Errorf("validation error in %s: a 'bug' field is required to track this exception", path)
		}
		if skip.SkipAnywhere {
			b.Config.SkipAnywhere = append(b.Config.SkipAnywhere, skip.Paths...)
			b.Config.Discover.SkipAnywhere = append(b.Config.Discover.SkipAnywhere, skip.Paths...)
		} else {
			b.Config.SkipPaths = append(b.Config.SkipPaths, skip.Paths...)
			b.Config.Discover.SkipPaths = append(b.Config.Discover.SkipPaths, skip.Paths...)
		}
	}

	// 2. Process TargetExtensions
	if f.TargetExtensions != nil {
		for _, ext := range f.TargetExtensions.Extensions {
			// Ensure consistent dot formatting (e.g., "cc" -> ".cc")
			if !strings.HasPrefix(ext, ".") {
				ext = "." + ext
			}
			b.Config.TargetExtensions[ext] = true
			b.Config.Classify.TargetExtensions[ext] = true
		}
	}

	// 2.5 Process CopyrightExtensions
	if f.CopyrightExtensions != nil {
		for _, ext := range f.CopyrightExtensions.Extensions {
			if !strings.HasPrefix(ext, ".") {
				ext = "." + ext
			}
			b.Config.CopyrightExtensions[ext] = true
			b.Config.Validate.CopyrightExtensions[ext] = true
		}
	}

	// 3. Process Barriers
	for _, barrier := range f.Barriers {
		if barrier.Bug == "" && isBugRequired(path) {
			return fmt.Errorf("validation error in %s: a 'bug' field is required to track this exception", path)
		}
		b.Config.BarrierPaths = append(b.Config.BarrierPaths, barrier.Paths...)
		b.Config.Discover.BarrierPaths = append(b.Config.Discover.BarrierPaths, barrier.Paths...)
	}

	// 4. Process PolicyExceptions
	for checkName, entries := range f.PolicyExceptions {
		if err := b.addRuleException(b.Config.PolicyExceptions, b.Config.Validate.PolicyExceptions, checkName, entries, path); err != nil {
			return err
		}
	}

	// 5. Process AllowedLicenses
	for licenseName, entries := range f.AllowedLicenses {
		if err := b.addRuleException(b.Config.AllowedLicenses, b.Config.Validate.AllowedLicenses, licenseName, entries, path); err != nil {
			return err
		}
	}

	return nil
}

func isBugRequired(configPath string) bool {
	switch filepath.Base(configPath) {
	case "default.json", "hidden_dirs.json", "test_dirs.json", "bazel_vendor.json":
		return false
	default:
		return true
	}
}

func (b *Builder) addRuleException(targetMap map[string]map[string]RuleMetadata, validateMap map[string]map[string]RuleMetadata, key string, entries []AllowlistEntry, path string) error {
	for _, m := range []map[string]map[string]RuleMetadata{targetMap, validateMap} {
		if _, exists := m[key]; !exists {
			m[key] = make(map[string]RuleMetadata)
		}
	}
	for _, entry := range entries {
		if entry.Bug == "" && isBugRequired(path) {
			return fmt.Errorf("validation error in %s: a 'bug' field is required to track this exception", path)
		}
		for _, allowedPath := range entry.Paths {
			meta := RuleMetadata{
				Bug:         entry.Bug,
				Description: entry.Description,
				ConfigPath:  path,
			}
			targetMap[key][allowedPath] = meta
			validateMap[key][allowedPath] = meta
		}
	}
	return nil
}

// XML structures for Jiri manifests
type Manifest struct {
	XMLName         xml.Name  `xml:"manifest"`
	Projects        []Project `xml:"project"`
	ProjectsGrouped []Project `xml:"projects>project"`
	Packages        []Package `xml:"packages>package"`
}

type Project struct {
	Name string `xml:"name,attr"`
	Path string `xml:"path,attr"`
}

type Package struct {
	Name string `xml:"name,attr"`
	Path string `xml:"path,attr"`
}

// LoadManifests scans the manifests and integration directories and populates the mapping.
func (b *Builder) LoadManifests() error {
	dirsToScan := []string{
		filepath.Join(b.FuchsiaDir, "manifests"),
		filepath.Join(b.FuchsiaDir, "integration"),
	}

	for _, dir := range dirsToScan {
		if _, err := os.Stat(dir); os.IsNotExist(err) {
			continue
		}

		err := filepath.WalkDir(dir, func(path string, d fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if d.IsDir() {
				return nil
			}

			// Manifest files usually have no extension or .xml
			ext := filepath.Ext(path)
			if ext != "" && ext != ".xml" {
				return nil
			}

			data, err := os.ReadFile(path)
			if err != nil {
				return nil // Skip files we can't read
			}

			var m Manifest
			if err := xml.Unmarshal(data, &m); err != nil {
				return nil // Skip files that aren't valid XML manifests
			}

			isPrivate := strings.Contains(filepath.ToSlash(path), "/internal")

			// Map projects
			for _, p := range m.Projects {
				if p.Path != "" && p.Name != "" {
					cleanPath := filepath.Clean(p.Path)
					b.Config.ManifestProjectNames[cleanPath] = p.Name
					if isPrivate {
						b.Config.ManifestPrivateProjects[cleanPath] = true
					}
				}
			}

			// Map projects (grouped)
			for _, p := range m.ProjectsGrouped {
				if p.Path != "" && p.Name != "" {
					cleanPath := filepath.Clean(p.Path)
					b.Config.ManifestProjectNames[cleanPath] = p.Name
					if isPrivate {
						b.Config.ManifestPrivateProjects[cleanPath] = true
					}
				}
			}

			// Map packages (prebuilts)
			for _, p := range m.Packages {
				if p.Path != "" && p.Name != "" {
					cleanPath := filepath.Clean(p.Path)
					b.Config.ManifestProjectNames[cleanPath] = p.Name
					if isPrivate {
						b.Config.ManifestPrivateProjects[cleanPath] = true
					}
				}
			}

			return nil
		})
		if err != nil {
			return err
		}
	}

	return nil
}
