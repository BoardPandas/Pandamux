# Command Execution and User Communication Rules

## Pre-Command Chat Explanation

Before initiating any command that takes measurable time or can run asynchronously in the background (such as `cargo test`, `cargo build`, long scripts, test suites, or network operations):

1. **Explain the action**: State clearly in chat what command is being executed and why.
2. **Estimate duration**: Provide a rough expected duration (for example: ~10 to 30 seconds).
3. **Interrupt safety guidance**: Explicitly state whether the command is safe to interrupt or cancel early if it appears stuck (for example: "Safe to cancel early: read-only test suite with no workspace side effects", or "Canceling early is safe; partial build caches will resume on the next invocation").
4. **Never execute silently**: Always emit text to the chat window before starting long commands so the user is never left wondering whether the agent is active or stuck.
