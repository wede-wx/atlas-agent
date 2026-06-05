# Atlas Code Review Command

This file defines the rules for Atlas's code review command. Atlas must use this file when handling:

- `/代码审查`
- `/code-review`
- `/review`

The command reviews code changes. It does not fix code by default.

## Purpose

The code review command must find real risks in the current repository, selected files, directories, or a pull request.

It must not only summarize what changed. It must judge whether the changes are safe to merge or continue.

## Default Mode

Default behavior is read-only.

Atlas must not automatically:

- edit files
- run repository commands or scripts
- create commits
- push branches
- merge pull requests
- publish GitHub comments
- run untrusted scripts
- claim unverified behavior is verified

When checks are needed, the command should list the exact checks to run and mark the finding or risk as unverified until a separate verification flow runs them.

If the user asks Atlas to fix findings, Atlas must first finish the review report, then enter a separate implementation flow.

## Required Inputs

Atlas should collect these inputs before reviewing when available:

- the current user request
- repository rules such as `AGENTS.md`
- project handoff files such as `shixiang.md`
- README and docs relevant to the changed area
- current `git diff`
- changed file list
- related tests
- recent build, test, lint, smoke, or CI results
- pull request title, description, diff, review comments, and CI status when a PR is supplied

If required information is missing, Atlas must report `[信息缺失]` instead of pretending the review is complete.

## PR Description Requirements

For pull request review, the PR description should include:

- background
- change purpose
- change scope
- risk and impact
- test evidence
- docs and migration notes
- rollback plan
- requested reviewer focus

Missing sections should be called out as `[信息缺失]`.

## Review Flow

Atlas must follow this order:

1. Identify the review target: current diff, selected files, selected directory, or PR.
2. Read the repository rules and relevant documentation.
3. Read the changed files and diff.
4. Identify affected user paths, data paths, API boundaries, storage paths, UI paths, and security boundaries.
5. Check whether tests and verification evidence match the change.
6. Report findings first, ordered by severity.
7. Separate verified facts from unverified risks.
8. If no issues are found, still report residual risk and test gaps.

## Severity Labels

All findings must use one of these labels.

### `[阻断]`

Must be fixed before merge or delivery.

Use for:

- functional regressions
- data loss
- permission bypass
- security vulnerabilities
- broken primary user paths
- unsafe deletion or migration
- failed build, typecheck, or critical tests
- direct violation of repository rules

### `[强烈建议]`

High risk or high maintenance cost. Should be fixed in this round unless there is a clear reason not to.

Use for:

- missing boundary handling
- incomplete error handling
- fragile state synchronization
- UI controls that appear real but are not connected
- missing key regression tests
- unclear ownership of state, API, storage, or permissions

### `[可选]`

Quality improvement that should not block merge by itself.

Use for:

- naming improvements
- minor structure cleanup
- low-risk UI polish
- non-critical documentation additions

### `[提问]`

Use when author intent or business rules are unclear.

### `[信息缺失]`

Use when required review context is missing, such as test evidence, risk notes, rollback plan, or PR description details.

### `[表扬]`

Use sparingly for clearly good changes. Praise never replaces risk review.

## Output Format

The report must be findings first.

Use this structure:

```md
## 代码审查结果

### 发现的问题

1. [阻断] Short title
   - 文件：`path/to/file.ts:123`
   - 问题：
   - 风险：
   - 建议：
   - 需要验证：

2. [强烈建议] Short title
   - 文件：`path/to/file.rs:45`
   - 问题：
   - 风险：
   - 建议：
   - 需要验证：

### 信息缺失

- 缺少：

### 已验证内容

- 已读取：
- 已运行：
- 已确认：

### 未验证风险

- 未运行：
- 未覆盖：
- 无法确认：

### 总结

当前状态：阻断 / 需要修改 / 可合并但有风险 / 可合并 / 信息不足，无法完整审查。
```

Each actionable finding must include:

- file path
- line number when possible
- concrete problem
- why it matters
- suggested fix
- required test or verification

## Required Review Areas

Atlas must check:

- functional correctness
- edge cases
- error handling
- state ownership
- real data paths
- API and command contracts
- storage and migration behavior
- deletion, archive, restore, and rollback safety
- security and privacy
- permissions and secret handling
- performance and resource use
- maintainability
- test coverage
- docs, config, dependencies, and build impact
- frontend real interaction, loading, empty, error, disabled, permission, responsive, overflow, and text clipping states

## Frontend Review Rules

For frontend changes, Atlas must not treat visual changes as completion.

Atlas must check whether:

- buttons and menus call real logic
- dialogs, popovers, tooltips, and tabs have real states
- data is not mock data presented as real
- loading, empty, success, error, disabled, and permission states exist where relevant
- text does not overflow or overlap
- responsive layouts remain usable
- colors and selected states follow the app theme

## Automation Boundary

Atlas may use or request:

- lint
- typecheck
- unit tests
- integration tests
- smoke tests
- build
- CodeQL
- Dependency Review
- reviewdog
- Danger
- GitHub Actions summary

Automation evidence does not replace review judgment.

Build passed, tests passed, or a screenshot exists does not automatically mean the review passes.

## Agent Rules

Atlas as reviewer must:

- put findings before summary
- avoid repeating the diff as the review
- avoid vague approvals such as "looks good" when verification is missing
- disclose skipped checks
- disclose unverified paths
- disclose mock, placeholder, or fake controls
- avoid automatic fixes unless the user explicitly asks
- avoid publishing comments unless the user explicitly asks

## Final Status

The final review status must be one of:

- `阻断`
- `需要修改`
- `可合并但有风险`
- `可合并`
- `信息不足，无法完整审查`

Do not use vague statuses such as:

- looks okay
- should be fine
- probably safe
- basically done
