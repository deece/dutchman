# Merge queue

GitHub's merge queue tests each pull request on top of the latest `main` (and of the queued pull
requests ahead of it) before it merges, so two green pull requests cannot break `main` together.
When a pull request passes its required checks and is enqueued, GitHub builds a temporary branch
(`gh-readonly-queue/main/...`), runs the workflows that have a `merge_group` trigger on it, and
squash-merges it only if the required checks pass.

## What runs in the queue

| Workflow | In the queue |
|---|---|
| `ci.yml` | The same jobs as a pull request, judged against the group's base commit. Never saves a cache. |
| `placements.yml` | Runs only if the group touches the paths that start it on a pull request (a queue has no path filter, so a job decides). |
| `docker.yml` | Runs only if the group touches `Dockerfile`, `.dockerignore` or the workflow. Never writes the layer cache. |
| `release-build.yml` | The `plan` job compares the group's base commit with its head: the full release builds run only for a packaging change (`[profile.*]` tables in `Cargo.toml` included). The `release-build` label cannot be read in the queue. |

## Ruleset

Create it once, then enable it when ready (`"enforcement": "active"` switches it on at creation;
use `"disabled"` in the file to stage it first and `gh api -X PUT` the ruleset later):

```
gh api repos/dutchman-cad/dutchman/rulesets -X POST --input ruleset.json
```

`ruleset.json`:

```json
{
  "name": "main merge queue",
  "target": "branch",
  "enforcement": "active",
  "conditions": {
    "ref_name": {
      "include": [
        "refs/heads/main"
      ],
      "exclude": []
    }
  },
  "bypass_actors": [
    {
      "actor_id": 5,
      "actor_type": "RepositoryRole",
      "bypass_mode": "always"
    }
  ],
  "rules": [
    {
      "type": "merge_queue",
      "parameters": {
        "merge_method": "SQUASH",
        "grouping_strategy": "ALLGREEN",
        "max_entries_to_build": 5,
        "min_entries_to_merge": 1,
        "max_entries_to_merge": 5,
        "min_entries_to_merge_wait_minutes": 5,
        "check_response_timeout_minutes": 180
      }
    },
    {
      "type": "required_status_checks",
      "parameters": {
        "strict_required_status_checks_policy": false,
        "required_status_checks": [
          {
            "context": "ci-result"
          }
        ]
      }
    }
  ]
}
```

- Squash merges, groups of up to 5 built in parallel, merged when all are green (`ALLGREEN`),
  after waiting 5 minutes for more pull requests to join. A group whose checks do not report
  within 120 minutes is removed.
- Repository admins bypass the rule (role id 5), so an emergency fix can merge directly.
- The one required check is `ci-result`, a job in `ci.yml` that always runs, needs every other
  CI job, and fails only if one of them failed or was cancelled. Requiring the test jobs by
  name doesn't work: a docs-only change skips the test matrix, and a skipped matrix job reports
  once under its bare name (`test`), so `test (ubuntu-latest)` and the others never appear and
  the pull request waits forever. Left out on purpose:
  `placements`, `docker` and `release-build` (path filters mean they do not report on every pull
  request, so requiring them would block it forever), `solidcraft-roundtrip` (informational),
  and the CodeQL checks (they do not run in the queue).
- Windows and ARM tests are the slowest required checks (ARM took 34 minutes on one run).

## Enqueue

Once the required checks pass on the pull request:

```
gh pr merge <n> --squash --auto
```

With the ruleset active this adds the pull request to the queue. Remove it with
`gh pr merge <n> --disable-auto` or from the pull request page.
