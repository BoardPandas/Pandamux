---
name: ux-review
effort: medium
description: Review UI/UX code against Laws of UX and Gestalt principles. Produces severity-ranked findings with specific improvement recommendations. Use on frontend repos or targeted components.
user-invocable: true
argument-hint: [optional: file, directory, or component name to scope the review]
agent: ux-reviewer
allowed-tools:
  - Read
  - Glob
  - Grep
---

# UX Review

You have been asked to perform a UX review of the codebase. Follow these steps.

## Step 1: Load Knowledge Base

Read `.Codex/references/ux-laws.md` to load the full set of 30 UX laws and their code-level indicators. You will reference these throughout the review.

## Step 2: Determine Scope

1. If the user specified a file, directory, or component name, scope the review to that target.
2. If no scope was specified, identify all UI-related files in the codebase.
3. Detect the frontend framework (React, Vue, Svelte, Angular, plain HTML, etc.) and note conventions.
4. Read `.gitignore` and skip ignored paths.

Use these patterns to find UI files:

```
**/*.tsx, **/*.jsx        (React components)
**/*.vue                  (Vue components)
**/*.svelte               (Svelte components)
**/*.html                 (HTML templates)
**/*.css, **/*.scss       (Stylesheets)
**/pages/**, **/views/**  (Page-level components)
**/components/**          (Shared components)
**/layouts/**             (Layout components)
```

## Step 3: Analyze Navigation and Information Architecture

Review the top-level navigation and page structure:

1. **Hick's Law:** Count top-level navigation items. Flag if more than 7 primary options exist without grouping.
2. **Serial Position Effect:** Check whether the most important items are at the beginning and end of navigation lists.
3. **Miller's Law / Chunking:** Look for long flat lists without grouping, sectioning, or hierarchy.
4. **Jakob's Law:** Compare navigation patterns to industry conventions. Flag non-standard patterns (e.g., hamburger menus on desktop, search bars in footers, unconventional icon usage).

## Step 4: Analyze Forms and User Input

Find all forms, inputs, and data entry patterns:

1. **Cognitive Load:** Count fields per form. Flag forms with 7+ fields on a single screen without sections.
2. **Chunking:** Check whether long forms are broken into logical groups or steps.
3. **Goal-Gradient Effect:** For multi-step forms, check for progress indicators.
4. **Postel's Law:** Review input validation. Flag overly strict validation that rejects reasonable input formats. Check whether error messages explain expected format.
5. **Parkinson's Law:** Identify unnecessary fields, missing autofill attributes, or missing pre-populated defaults.
6. **Flow:** Check for interruptions mid-form (modals, redirects, forced decisions unrelated to the task).

## Step 5: Analyze Interactive Elements

Review buttons, links, and interactive controls:

1. **Fitts's Law:** Check touch/click target sizes. Flag targets under 44px on mobile or 24px on desktop. Check spacing between adjacent interactive elements.
2. **Von Restorff Effect:** Verify that primary CTAs are visually distinct from secondary actions. Flag pages where multiple elements compete for attention equally.
3. **Law of Similarity:** Ensure clickable and non-clickable elements are visually distinct. Links should look like links. Buttons should look like buttons.
4. **Doherty Threshold:** Check for loading states on interactive elements (buttons, form submits). Flag actions that fire without immediate visual feedback.

## Step 6: Analyze Layout and Visual Grouping

Review the spatial organization of UI elements:

1. **Law of Proximity:** Check spacing between related elements. Flag labels far from their inputs, related actions spread apart, or unrelated items packed closely.
2. **Law of Common Region:** Check for visual containers (cards, sections, borders) grouping related content. Flag related content without boundary separation.
3. **Law of Uniform Connectedness:** In stepper/wizard/timeline components, check for connecting visual elements.
4. **Law of Pragnanz:** Flag overly complex visual patterns, charts with too many data series, or layouts with competing visual rhythms.
5. **Cognitive Load:** Evaluate overall density. Flag screens with no visual breathing room or too many competing calls-to-action.

## Step 7: Analyze Feedback and State Communication

Review how the system communicates state to users:

1. **Doherty Threshold:** Check that all async operations (API calls, form submits, file uploads) show loading/progress states.
2. **Peak-End Rule:** Review success states, completion screens, and error pages. Flag bland or missing success feedback. Flag error pages without recovery options.
3. **Zeigarnik Effect:** For multi-step processes, check for progress tracking. Flag flows where users cannot see how far they are or how much remains.
4. **Tesler's Law:** Check whether system complexity is exposed to users. Flag raw error codes, technical jargon, or configuration screens without sensible defaults.

## Step 8: Analyze Accessibility and Inclusivity

Review for accessibility concerns that intersect with UX laws:

1. **Von Restorff Effect:** Flag emphasis that relies solely on color (color-blind users cannot perceive it).
2. **Fitts's Law:** Check that all interactive elements have adequate hit areas for users with motor impairments.
3. **Law of Similarity:** Verify interactive elements have non-color indicators (underlines for links, borders for buttons).
4. **Selective Attention:** Check that important state changes use multiple channels (visual + text, not just color shifts).

## Step 9: Analyze Mobile and Responsive Design (if applicable)

If the project has responsive styles or mobile views:

1. **Fitts's Law:** Verify touch targets are at least 44x44px with adequate spacing.
2. **Hick's Law:** Check that mobile navigation reduces visible options appropriately.
3. **Cognitive Load:** Flag desktop-density layouts served on mobile without adaptation.
4. **Serial Position Effect:** Check mobile tab bars for optimal ordering of key items.

## Step 10: Produce Report

Format the report as follows:

```
# UX Review Report

## Summary
- Components/files reviewed: <count>
- Critical: <count>
- Warning: <count>
- Suggestion: <count>
- Primary UX laws violated: <list top 3-5 most common violations>

## Scope
<description of what was reviewed and the frontend stack detected>

## Critical Findings

[CRITICAL] file:line -- Description
  UX Law: <law name>
  Impact: <what users experience>
  Recommendation: <specific code-level fix>

## Warnings

[WARNING] file:line -- Description
  UX Law: <law name>
  Impact: <what users experience>
  Recommendation: <specific code-level fix>

## Suggestions

[SUGGESTION] file:line -- Description
  UX Law: <law name>
  Impact: <what users experience>
  Recommendation: <specific code-level fix>

## Top Recommendations

Summarize the 3-5 highest-impact changes that would most improve the overall UX,
referencing findings above. Prioritize by user impact, not by count.
```

Prioritize findings by real user impact. A single critical flow broken by poor cognitive load management matters more than ten minor spacing inconsistencies.
