// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

package validate

import (
	"io"
	"os"
	"regexp"
	"strings"
)

var commentCleaner = strings.NewReplacer(
	"//", " ",
	"#", " ",
	"/*", " ",
	"*/", " ",
	";", " ",
	"*", " ",
	"\r", " ",
	"\n", " ",
	"\t", " ",
)

// Standard Fuchsia/Chromium/Android copyright regex (strict).
// It matches the exact core license text to enforce the correct standard.
// It ignores comment prefixes and other whitespace text via commentCleaner.
var copyrightRegex = regexp.MustCompile(
	`(?i)Copyright\s+[0-9,\-\s]+The\s+Fuchsia\s+Authors\.?\s*All\s+rights\s+reserved\.?\s+` +
		`Use\s+of\s+this\s+source\s+code\s+is\s+governed\s+by\s+a\s+BSD-style\s+license\s+` +
		`that\s+can\s+be\s+found\s+in\s+the\s+LICENSE\s+file`,
)

// CheckCopyright verifies if an absolute file path has a Fuchsia copyright header.
// It opens the file from disk (useful for standalone command-line checking).
func CheckCopyright(absPath string) (bool, error) {
	// Skip empty files (size 0). They are not required to have copyright headers.
	stat, err := os.Stat(absPath)
	if err == nil && stat.Size() == 0 {
		return true, nil
	}

	f, err := os.Open(absPath)
	if err != nil {
		return false, err
	}
	defer f.Close()

	buf := make([]byte, 8192)
	n, err := f.Read(buf)
	if err != nil && err != io.EOF {
		return false, err
	}
	return CheckCopyrightText(buf[:n]), nil
}

// CheckCopyrightText verifies if a byte slice contains a Fuchsia copyright header.
func CheckCopyrightText(text []byte) bool {
	if len(text) == 0 {
		return true
	}
	cleaned := commentCleaner.Replace(string(text))
	return copyrightRegex.MatchString(cleaned)
}
