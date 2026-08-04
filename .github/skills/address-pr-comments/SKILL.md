---
description: 'Review, fix, and resolve valid GitHub pull request comments using the GitHub CLI.'
---
# Address PR Comments

Use this skill when the user asks to address review comments on pull request
`#X`, where `X` is supplied by the user.

## Goal

Carefully review every pull-request comment and decide whether it identifies a
real issue or is bikeshedding. Fix only valid issues, validate the result,
commit, push, and resolve the addressed review threads through the GitHub API.

Do not resolve comments that are invalid, unclear, out of scope, or still need
user input.

## 1. Establish scope

1. Confirm the repository, current branch, worktree state, and PR metadata:

   ```powershell
   gh pr view X --json number,title,state,url,headRefName,baseRefName,body
   git status --short
   git branch --show-current
   ```

2. The PR must be open and the current branch must match its head branch.
   If unrelated worktree changes could be affected, leave them untouched.

## 2. Fetch all comments

GitHub paginates review comments. Never treat the first page as exhaustive.
Collect all three surfaces:

```powershell
gh api --paginate "repos/{owner}/{repo}/issues/X/comments?per_page=100" `
  --jq '.[] | {id, user: .user.login, created_at, body, html_url}'

gh api --paginate "repos/{owner}/{repo}/pulls/X/comments?per_page=100" `
  --jq '.[] | {id, user: .user.login, path, line, original_line, body, html_url}'

gh api --paginate "repos/{owner}/{repo}/pulls/X/reviews?per_page=100" `
  --jq '.[] | {id, user: .user.login, state, body, submitted_at, html_url}'
```

Use GraphQL review threads to obtain thread IDs and resolution state. Paginate
until `hasNextPage` is false:

```graphql
query($owner: String!, $repo: String!, $number: Int!, $cursor: String) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      reviewThreads(first: 100, after: $cursor) {
        nodes {
          id
          isResolved
          isOutdated
          path
          line
          comments(first: 100) {
            nodes {
              id
              body
              author { login }
              createdAt
              url
            }
          }
        }
        pageInfo {
          hasNextPage
          endCursor
        }
      }
    }
  }
}
```

Use `gh api graphql` with the repository owner/name and PR number. Retain a
mapping from each actionable comment to its review-thread ID. Do not resolve an
inline review comment solely by REST comment ID; resolving requires the
GraphQL review-thread ID.

## 3. Triage each comment

For every unresolved comment or thread:

1. Read the cited code and enough surrounding context to understand it.
2. Try to disprove the concern with code, tests, protocol constraints, or
   documented requirements.
3. Classify it:
   - **Valid**: demonstrable bug, security issue, correctness issue,
     regression risk, unmet requirement, or missing necessary test.
   - **Invalid**: contradicted by the implementation or repository evidence.
   - **Bikeshedding**: subjective naming/style/preference with no project
     convention or behavior impact.
   - **Needs decision**: a valid tradeoff or ambiguous product requirement
     that requires the user’s input.
4. Record the classification and evidence before editing.

Do not make changes merely to silence a reviewer. Do not fix unrelated
problems discovered while reviewing a comment.

## 4. Implement valid findings

1. Group related valid comments into the smallest coherent patch.
2. Preserve established project conventions and avoid modifying unrelated
   user changes.
3. Add or update the smallest existing validation that proves the changed
   behavior.
4. Run the narrowest relevant build, test, lint, or hardware validation.
5. Re-read the changed code and the original comment to verify the patch
   addresses the actual concern.

If no comments are valid, do not create an empty commit.

## 5. Commit and push

After all valid findings are fixed and validated:

```powershell
git status --short
git add <only files required by the valid fixes>
git commit -m "<concise PR review fix>" `
  -m "Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
git push origin <pr-head-branch>
```

Never include unrelated local changes. Do not amend existing commits unless the
user explicitly requests it.

## 6. Resolve addressed review threads

Resolve only review threads whose valid findings are fixed and pushed:

```powershell
gh api graphql `
  -f query='mutation($threadId: ID!) {
    resolveReviewThread(input: {threadId: $threadId}) {
      thread { id isResolved }
    }
  }' `
  -F threadId='<review-thread-node-id>'
```

For issue-level or general PR comments that have no review thread, reply using
the issue-comment API with the classification and, for valid findings, the
fix commit SHA. Do not claim an invalid or bikeshedding comment was fixed.

## 7. Report

Report:

- the PR URL;
- every comment classification, with concise evidence;
- fixes made and their commit SHA;
- validation performed;
- review threads resolved;
- comments left unresolved and why.
