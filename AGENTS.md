# Repository workflow

- Finish every requested change by testing it, committing only the files that
  belong to that change, and deploying each affected service from that exact
  commit.
- After every commit that changes a deployable service, deploy it immediately
  and verify its health so the running version stays synchronized with `HEAD`.
- Do not leave completed work only in the working tree, and do not include
  unrelated working-tree changes in a commit or deployment.
