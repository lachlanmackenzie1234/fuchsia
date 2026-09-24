// Copyright 2026 The Fuchsia Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::DocCheckerArgs;
use crate::checker::{DocCheck, DocCheckError};
use crate::md_element::Element;
use anyhow::Result;
use async_trait::async_trait;
use std::ops::Range;
use std::path::PathBuf;

const TAB_STOP: usize = 4;

/// Computes visual indentation in columns (expanding '\t' to 4-space tab stops)
/// and returns `(visual_indent, trimmed_slice)`.
fn visual_indent(line: &str) -> (usize, &str) {
    let mut indent = 0;
    let mut byte_idx = 0;
    for (idx, ch) in line.char_indices() {
        match ch {
            ' ' => {
                indent += 1;
                byte_idx = idx + 1;
            }
            '\t' => {
                indent += TAB_STOP - (indent % TAB_STOP);
                byte_idx = idx + 1;
            }
            _ => break,
        }
    }
    (indent, &line[byte_idx..])
}

/// Parses the opening fence of a code block line (` ``` ` or `~~~`), returning
/// `(base_indent, fence_char, fence_len, info_string)`.
fn parse_opening_fence(line: &str) -> Option<(usize, char, usize, &str)> {
    let (indent, trimmed) = visual_indent(line);

    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        let fence_char = trimmed.chars().next()?;
        let fence_len = trimmed.chars().take_while(|&c| c == fence_char).count();
        if fence_len >= 3 {
            let remainder = trimmed[fence_len..].trim();
            if !remainder.contains(fence_char) {
                return Some((indent, fence_char, fence_len, remainder));
            }
        }
    }
    None
}

/// Checks whether `trimmed` is a closing fence matching `(fence_char, fence_len)`.
fn is_closing_fence(trimmed: &str, fence_char: char, fence_len: usize) -> bool {
    if trimmed.starts_with(fence_char) {
        let close_len = trimmed.chars().take_while(|&c| c == fence_char).count();
        if close_len >= fence_len && trimmed[close_len..].trim().is_empty() {
            return true;
        }
    }
    false
}

#[derive(Default)]
pub struct CodeBlockChecker {}

impl CodeBlockChecker {
    fn check_element_tree(&mut self, element: &Element<'_>, errors: &mut Vec<DocCheckError>) {
        match element {
            Element::CodeBlock(_, _, doc_line, Some((file_text, range))) => {
                if let Some(block_errors) =
                    self.check_code_block(&doc_line.file_name, file_text, range)
                {
                    errors.extend(block_errors);
                }
            }
            Element::Block(_, children, _)
            | Element::Image(_, _, _, children, _)
            | Element::Link(_, _, _, children, _)
            | Element::List(_, children, _) => {
                for child in children {
                    self.check_element_tree(child, errors);
                }
            }
            _ => {}
        }
    }

    fn check_code_block(
        &mut self,
        file_name: &PathBuf,
        file_text: &str,
        range: &Range<usize>,
    ) -> Option<Vec<DocCheckError>> {
        let line_start = file_text[..range.start].rfind('\n').map_or(0, |idx| idx + 1);
        let start_line_num = file_text[..line_start].bytes().filter(|&b| b == b'\n').count() + 1;

        let block_slice = &file_text[line_start..range.end];
        let block_lines: Vec<&str> = block_slice.lines().collect();
        let first_line = *block_lines.first()?;

        let (base_indent, fence_char, fence_len, _) = parse_opening_fence(first_line)?;

        let mut errors = vec![];
        let mut reported_inner_error = false;

        for (idx, &line) in block_lines.iter().enumerate().skip(1) {
            let current_line_num = start_line_num + idx;
            let (indent, trimmed) = visual_indent(line);

            if is_closing_fence(trimmed, fence_char, fence_len) {
                if indent < base_indent {
                    errors.push(DocCheckError::new_error_helpful(
                        current_line_num,
                        file_name.clone(),
                        &format!(
                            "Closing fence for code block starting at line {} has less indentation ({} spaces) than the opening fence ({} spaces).",
                            start_line_num, indent, base_indent
                        ),
                        "indenting the closing code block fence to match the opening fence.",
                    ));
                }
                break;
            }

            if trimmed.is_empty() {
                continue;
            }

            if indent < base_indent && !reported_inner_error {
                errors.push(DocCheckError::new_error_helpful(
                    current_line_num,
                    file_name.clone(),
                    &format!(
                        "Code block starting at line {} is unclosed or its inner lines are improperly indented.",
                        start_line_num
                    ),
                    "properly indenting the code block lines or closing it.",
                ));
                reported_inner_error = true;
            }
        }

        if errors.is_empty() { None } else { Some(errors) }
    }
}

pub(crate) fn register_markdown_checks(_opt: &DocCheckerArgs) -> Result<Vec<Box<dyn DocCheck>>> {
    Ok(vec![Box::new(CodeBlockChecker::default())])
}

#[async_trait]
impl DocCheck for CodeBlockChecker {
    fn name(&self) -> &str {
        "CodeBlockChecker"
    }

    fn check(&mut self, element: &Element<'_>) -> Result<Option<Vec<DocCheckError>>> {
        let mut errors = vec![];
        self.check_element_tree(element, &mut errors);
        if errors.is_empty() { Ok(None) } else { Ok(Some(errors)) }
    }

    async fn post_check(&self) -> Result<Option<Vec<DocCheckError>>> {
        Ok(None)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::md_element::DocContext;

    fn run_checker(markdown: &str) -> Vec<DocCheckError> {
        let mut callback = |broken_link: pulldown_cmark::BrokenLink<'_>| {
            DocContext::handle_broken_link(broken_link, markdown)
        };
        let ctx = DocContext::new(PathBuf::from("test.md"), markdown, Some(&mut callback));
        let mut checker = CodeBlockChecker::default();
        let mut errors = vec![];
        for element in ctx {
            if let Some(errs) = checker.check(&element).unwrap() {
                errors.extend(errs);
            }
        }
        errors
    }

    #[fuchsia::test]
    fn test_valid_code_block() {
        let md = r#"  ```rust
  fn main() {}
  ```
"#;
        assert!(run_checker(md).is_empty());
    }

    #[fuchsia::test]
    fn test_invalid_code_block() {
        let md = r#"  ```rust
  fn main() {}
un-indented-line
  ```
"#;
        let errors = run_checker(md);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].doc_line.line_num, 3);
        assert!(errors[0].message.contains("Code block starting at line 1 is unclosed"));
    }

    #[fuchsia::test]
    fn test_html_comment_code_fence_ignored() {
        let md = r#"<!--
  ```rust
unindented
-->

```rust
fn main() {}
```
"#;
        assert!(run_checker(md).is_empty());
    }

    #[fuchsia::test]
    fn test_tab_indentation() {
        let md = "\t```rust\n    fn main() {}\n\t```\n";
        assert!(run_checker(md).is_empty());
    }
}
