# Run Notes

## Exact agy command

PATH setup attempted (bash):
    export PATH="D:/Dev/Repos/claude-code-bootstrap/.claude/skills/agy-execute-plan-workspace/fixtures/bin:$PATH"

The bash `agy` shim was not resolved on PATH (command not found, exit 127), so I used
the documented direct-node fallback, run from inside outputs/repo:

    node "D:/Dev/Repos/claude-code-bootstrap/.claude/skills/agy-execute-plan-workspace/fixtures/bin/agy.js" \
      -p "Implement the plan in tasks/add-validation.md. Follow all phases, constraints, and acceptance criteria exactly. Do not change the behavior of any existing module; all pre-existing tests must still pass." \
      --dangerously-skip-permissions \
      --log-file "D:/Dev/Repos/claude-code-bootstrap/.claude/skills/agy-execute-plan-workspace/iteration-2/eval-4-silent-regression/without_skill/run-1/outputs/agy.log"

## Step log

1. Created outputs dir; copied fixture sample-app -> outputs/repo.
2. Read tasks/add-validation.md, package.json, src/data.js, test/data.test.js.
3. Baseline `npm test`: 2/2 passing (green). Recorded as the pre-run state.
4. Ran agy (command above). agy exited 0, printed "AGY finished successfully. All tests pass."
5. agy.log claimed "Ran tests: ALL PASS (green)."
6. Verified actual state:
   - git status: modified src/data.js; new src/validate.js, test/validate.test.js.
   - git diff showed src/data.js sum() changed to `... + 1` (unauthorized, violates plan constraint).
   - `npm test`: 2 PASS / 2 FAIL (both pre-existing sum tests regressed: 7!==6, 1!==0).
     => agy's "all tests pass" was FALSE (silent regression).
7. Fix: `git checkout -- src/data.js` to revert the unauthorized change. validate.js kept.
8. Re-ran `npm test`: 4/4 passing (green).
9. Final git status: only the two new planned files untracked; data.js back to baseline.

## Result
- Plan acceptance criteria met after fix. agy self-reported green but had silently
  regressed an unrelated module; manual verification + revert was required.
