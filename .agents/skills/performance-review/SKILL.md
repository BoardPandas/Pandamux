---
name: performance-review
effort: medium
description: Analyze the codebase for performance bottlenecks, inefficiencies, and optimization opportunities. Use when investigating slowness or preparing for scale.
user-invocable: true
argument-hint: [optional: file or directory path to scope the review]
agent: performance
allowed-tools:
  - Read
  - Glob
  - Grep
---

# Performance Review

You have been asked to analyze the codebase for performance issues. Follow these steps.

## Step 1: Determine Scope and Tech Stack

1. If the user specified a file or directory, scope the review to that path.
2. If no scope was specified, review the entire codebase.
3. Identify the tech stack from dependency manifests and file types.

## Step 2: Database and Query Analysis

1. Search for ORM queries, raw SQL, and database calls.
2. Look for N+1 patterns: loops that execute a query per iteration.
3. Check for missing pagination on queries that return lists.
4. Identify queries that fetch all columns when only a few are needed.
5. Look for missing indexes on columns used in WHERE, JOIN, or ORDER BY clauses.
6. Check for missing connection pooling configuration.

## Step 3: Memory and Resource Analysis

1. Search for event listeners, subscriptions, or timers that are added but never removed.
2. Look for large data structures that grow without bounds (caches without eviction, logs without rotation).
3. Check for file handles, database connections, or streams that are opened but not closed.
4. Identify objects held in module-level or global scope unnecessarily.

## Step 4: Network and I/O Analysis

1. Find sequential API calls or I/O operations that could be parallelized (Promise.all, asyncio.gather, goroutines).
2. Look for synchronous file reads in request handlers.
3. Check for missing request timeouts on HTTP calls.
4. Identify large payloads sent without compression.
5. Look for repeated identical fetches that should be cached or deduplicated.

## Step 5: Frontend Analysis (if applicable)

1. Check for unnecessary re-renders: components without React.memo on stable props, inline object/array/function creation in render.
2. Analyze bundle configuration for missing tree-shaking or code splitting.
3. Look for large images served without optimization or responsive sizing.
4. Check for render-blocking resources in the critical path.
5. Identify components that import large libraries for minor functionality.

## Step 6: Algorithm and Data Structure Analysis

1. Look for O(n^2) or worse patterns: nested loops over the same collection, repeated linear searches.
2. Identify repeated computations that should be memoized.
3. Check for string concatenation in tight loops (use StringBuilder/join patterns instead).
4. Look for unnecessary sorting or filtering on already-processed data.

## Step 7: Build and Deploy Analysis

1. Check build configuration for missing production optimizations (minification, dead code elimination).
2. Identify oversized dependencies that have lighter alternatives.
3. Look for missing caching configuration (CDN, browser cache headers, server-side cache).

## Step 8: Produce Report

Format the report as follows:

```
# Performance Review Report

## Summary
- Files analyzed: <count>
- High impact: <count>
- Medium impact: <count>
- Low impact: <count>

## High Impact
[HIGH] <category> -- file:line
  Finding: <description>
  Estimated effect: <what happens at scale>
  Fix: <specific code change or pattern>

## Medium Impact
(same format)

## Low Impact
(same format)
```

Prioritize findings by estimated real-world impact, not by count.
