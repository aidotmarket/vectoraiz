# Bug Fix Batch

Read the bug spec file provided as argument and fix all bugs listed.

## Workflow
1. Read the spec file to understand all bugs
2. Prioritize: P0 first, then P1, P2, P3
3. For each bug:
   - Identify affected files (backend in `app/`, frontend in `frontend/src/`)
   - Make the fix
   - Write progress to `/tmp/bugfix_state.md`
4. Run tests: `python -m pytest tests/ -x -q`
5. Commit all fixes in a single commit to main
6. Report: files changed, what was fixed, any bugs that couldn't be fixed

## Rules
- Fix in priority order
- Single commit with descriptive message
- Always run tests before committing
- Use `ultrathink` for P0 bugs — they need deep investigation
