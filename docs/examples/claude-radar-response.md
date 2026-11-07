## MR.DE risk radar: **Deep** verdict

**Input:** committed changes, `HEAD~1` (`519b7b0`) → `HEAD` (`80b6189`), using `--config-dir /tmp/mrde-radar-documentation-config`. External checks were off.

**Mean:** 2.65/10 across **6 of 9 axes measured**. This is a partial profile, and the three missing axes are not zero or an all-clear. The mean is below the 6.0 Deep threshold, so the verdict appears to come from `sensitive_paths` exceeding the 8.0 single-axis threshold.

| Axis | Raw measurement | Score /10 | Limitation |
|---|---|---:|---|
| Diff size | 11 lines | 0.44 | Excludes binary lines |
| Files | 2 files | 1.33 | Renames count once |
| Complexity | n/a | n/a | External checks disabled |
| Test gap | 8 lines (estimate) | 0.64 | Heuristic based on related test changes, not coverage |
| Sensitive paths | 1.09 weighted fraction | **10.00** | Based on configured globs and weights |
| Coupling | 1 edge (estimate) | 1.00 | Textual heuristic |
| Lint | n/a | n/a | No supported linter detected |
| Debt | 2.5 weighted matches (estimate) | 2.50 | Regex matches; syntax and comments aren't interpreted |
| Ticket age | n/a | n/a | No creation date for a primary linked ticket |

No team median is available because this is a standalone scan.

### Highest-risk evidence
- **Sensitive path:** `src/auth/session.ts` matches `src/auth/**` at weight 2, with 6 changed lines out of 11 in the diff.
- **Loose typing in an access function:** `src/auth/session.ts:1` adds `export function canRead(role: any, override: boolean): boolean {`.
- **Unfinished override:** `src/auth/session.ts:2` adds `// TODO: replace the temporary override with an audited policy.`
- **Unresolved rule:** `src/cart.ts:3` adds `// FIXME: define the rounding rule before production use.`
- **New dependency:** `src/cart.ts:1` imports `./auth/session` for the first time, so cart code now depends on the auth module.
- **Estimated test gap:** both files have 4 added lines each with no related test change.

### Interpretation and follow-up
The change is small, but it touches access control, so the Deep verdict looks reasonable. These suggestions are based on the scan evidence; I didn't read the full source files.

1. **Review `canRead` closely:** check who can set `override`, and whether the temporary override can reach production without an audit trail.
2. **Tighten the type:** replace `role: any` with a specific role type so invalid roles are caught at compile time.
3. **Add tests:** cover `canRead` allow and deny cases, with and without the override.
4. **Settle the rounding rule:** resolve the `FIXME` before production use, and check whether cart code should depend on the auth module directly.
5. **Fill the unmeasured axes:** use `--run-checks` for complexity, and add a `[[lint]]` command in `risk.toml` for lint.

The scan didn't run tests or linters, and I made no changes to the repository. Plan mode is active, but there's no implementation to plan, so I didn't write a plan file.
