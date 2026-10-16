# Policy decision briefs for the surface audit

These are recommendations for Val, not adopted policy. The implementation status is based on `origin/main` at `45a400f` on 2026-10-16. No release or policy decision follows from this document.

## Decisions for Val

### Logical 89: environment substitution cycles

**Question.** `substitute_env_vars` repeatedly replaces `{env:NAME}`. A self reference or two-variable cycle does not terminate; a growing cycle can also consume unbounded work. The public helper's existing finite nested expansion behavior should be retained where possible.

**Recommendation.** Bound replacement count and output size (proposed: 10,000 replacements and 1 MiB), stop before either limit is crossed, return the last complete result, and emit a value-free warning. Treat an oversized initial input as unchanged. This changes very large finite expansions; the limits and warning contract need Val's approval before implementation. Test self and two-variable cycles, growing cycles, finite nesting, and both boundaries.

### Logical 95: saved plugin state and project trust

**Question.** Should unreadable or malformed user settings prevent plugin activation, and can project plugins activate from repository-owned settings?

**Recommendation.** Keep the fail-closed behavior now on main: `load_plugins` applies user-owned saved state, disables every discovered plugin if that state cannot be read or parsed, and records an actionable error. Project plugins require an explicit user-owned grant tied to the canonical project and plugin folders. Keep this as the startup trust contract and retain load, reload, and failed-settings coverage. No further policy change is needed unless Val wants discovery to remain active after a settings failure.

### Logical 121: poisoned permission manager

**Question.** Should the interactive handler allow tools when its shared policy lock is poisoned?

**Recommendation.** Keep the deny-on-poison behavior now on main (#197). Both interactive entrypoints deny when policy cannot be evaluated; healthy prompt and allow behavior remains. Recovering the poisoned lock cannot prove that a mutation completed, so automatic recovery should require a separate design and explicit decision. No further policy change is needed for this gate.

### Issue #267: omitted project defaults

**Question.** A project settings file that omits `permission_mode`, `theme`, or `output_format` currently replaces global values with deserialized defaults. CLI startup separately overrides `permission_mode` from flags, but other `Settings` consumers may observe the merged value.

**Recommendation.** Track field presence so omission preserves the global value for `theme` and `output_format`. Exclude `permission_mode` from untrusted project settings, as is already done for persisted permission rules. This changes the behavior of project files that rely on omission to reset a global setting or explicitly set a permission mode. Val must choose whether that compatibility change belongs before the release. Then test omitted and explicit values through `load_hierarchical`, including non-CLI consumers.
