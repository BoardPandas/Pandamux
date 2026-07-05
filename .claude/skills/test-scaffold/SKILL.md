---
name: test-scaffold
model: sonnet
effort: medium
description: Generate test files for untested modules. Detects the test framework and creates test stubs matching existing patterns. Use to improve test coverage.
user-invocable: true
argument-hint: [optional: file or directory to generate tests for]
allowed-tools:
  - Read
  - Write
  - Glob
  - Grep
  - Agent
---

# Test Scaffolding

You have been asked to generate test files for untested code. Follow these steps.

## Step 1: Detect Test Framework

1. Read the project's dependency manifest (package.json, pyproject.toml, Cargo.toml, etc.).
2. Identify the test framework in use:
   - **JavaScript/TypeScript:** Jest, Vitest, Mocha, Playwright, Cypress
   - **Python:** pytest, unittest
   - **Go:** built-in testing package
   - **Rust:** built-in #[test], plus any test crates
3. Find existing test files using Glob (`**/*.test.*`, `**/*.spec.*`, `**/test_*`, `**/*_test.*`, `**/tests/**`).
4. Read 2-3 existing test files to understand the project's test patterns, conventions, and imports.

## Step 2: Identify Untested Modules

1. List all source files using Glob.
2. For each source file, check if a corresponding test file exists.
3. Also check if the source file is imported in any test file (it may be tested indirectly).
4. Build a list of source files with no test coverage.

## Step 3: Determine Scope

1. If the user specified a file or directory, generate tests only for that scope.
2. If no scope was specified, present the list of untested modules and ask the user which ones to scaffold.

## Step 4: Generate Test Files

For each module to test:

1. Read the source file completely.
2. Identify all exported/public functions, classes, and methods.
3. Create a test file following the project's naming convention (e.g., `foo.test.ts` alongside `foo.ts`, or `test_foo.py` alongside `foo.py`).
4. For each exported function or method, generate:
   - A test for the happy path (valid inputs, expected output)
   - A test for edge cases (empty input, null, boundary values)
   - A test for error cases (invalid input, expected exceptions)
5. Use the same imports, assertion style, and patterns observed in existing test files.
6. Mark generated test bodies with assertions that will fail until implemented, so the user can verify which tests pass immediately and which need the implementation filled in.

## Step 5: Report

```
# Test Scaffold Report

## Files Created
- <path to test file> -- tests for <source file> (<count> test cases)

## Coverage Summary
- Modules without tests before: <count>
- Test files generated: <count>
- Test cases generated: <count>

## Next Steps
1. Run the test suite to see which generated tests pass immediately.
2. Fill in test logic for any cases marked as pending.
3. Add additional edge case tests as needed.
```
