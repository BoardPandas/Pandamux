---
name: security-scan
effort: high
description: Run a security audit covering OWASP Top 10, secrets detection, dependency vulnerabilities, and input validation. Use before releases or after security-sensitive changes.
user-invocable: true
argument-hint: [optional: file or directory path to scope the scan]
agent: security
allowed-tools:
  - Read
  - Glob
  - Grep
  - Bash
---

# Security Scan

You have been asked to perform a security audit. Follow these steps.

## Step 1: Determine Scope

1. If the user specified a file or directory, scope the scan to that path.
2. If no scope was specified, scan the entire codebase.
3. Read `.gitignore` and respect its patterns.

## Step 2: Secrets Detection

Search the entire codebase for leaked secrets:

1. Use Grep to search for these patterns (case-insensitive):
   - `API_KEY`, `APIKEY`, `api_key`
   - `SECRET`, `secret_key`, `SECRET_KEY`
   - `TOKEN`, `ACCESS_TOKEN`, `REFRESH_TOKEN`
   - `PASSWORD`, `PASSWD`, `passwd`
   - `PRIVATE_KEY`, `private_key`
   - `credentials`, `CREDENTIALS`
   - `aws_access_key_id`, `aws_secret_access_key`
   - Patterns matching base64-encoded strings longer than 40 characters
   - Patterns matching `-----BEGIN .* KEY-----`
2. Check `.env` files, `.env.local`, `.env.production`, and any other dotenv files.
3. Check CI/CD config files (`.github/workflows/`, `.gitlab-ci.yml`, `Jenkinsfile`, etc.).
4. For each finding, verify whether it is a placeholder/example or an actual secret.

## Step 3: OWASP Top 10 Analysis

Scan for each OWASP category:

1. **Injection:** Search for string concatenation in SQL queries, shell commands built from user input, template injection patterns.
2. **Broken Authentication:** Check for hardcoded credentials, weak password validation, missing rate limiting on auth endpoints.
3. **Sensitive Data Exposure:** Look for PII in logs, unencrypted storage, missing HTTPS enforcement.
4. **XXE:** Check XML parsers for external entity processing enabled.
5. **Broken Access Control:** Look for endpoints missing authorization middleware, direct object references without ownership checks.
6. **Security Misconfiguration:** Check for debug mode in production configs, default credentials, overly permissive CORS.
7. **XSS:** Search for `innerHTML`, `dangerouslySetInnerHTML`, unescaped template variables, `eval()`.
8. **Insecure Deserialization:** Look for `JSON.parse` on untrusted input without validation, `pickle.loads`, `yaml.load` without SafeLoader.
9. **Vulnerable Dependencies:** Check dependency manifests. If npm is available, run `npm audit`. If pip is available, check for `pip audit`. If cargo is available, check for `cargo audit`.
10. **Insufficient Logging:** Check if auth failures, access denials, and data modifications are logged.

## Step 4: Input Validation

1. Find all entry points (API routes, form handlers, CLI argument parsers).
2. Check each for input validation and sanitization.
3. Look for unsafe regex patterns vulnerable to ReDoS.
4. Check file upload handlers for missing type/size validation.
5. Look for unvalidated redirects.

## Step 5: Produce Report

Format the report as follows:

```
# Security Scan Report

## Summary
- Files scanned: <count>
- Critical: <count>
- High: <count>
- Medium: <count>
- Low: <count>

## Critical Findings
[CRITICAL] <category> -- file:line
  Finding: <description>
  Impact: <what an attacker could do>
  Remediation: <specific fix steps>

## High
(same format)

## Medium
(same format)

## Low
(same format)
```

Never include actual secret values in the report. Redact them.
