# Triaging reports

For maintainers. Dutchman is the one place users report problems, whatever code the problem
is in: they never need to know whether cadmpeg, Dutchman or SolidCraft has to change. We sort
each report and take what belongs upstream there. Reporters who want to help can choose
[optional steps from report to fix](testing-your-library.md#want-to-go-further), using only
synthetic files. Every step needs their agreement; their own designs stay local.

## Labels

| Label | Meaning |
|---|---|
| `library-report` | Came from a library test report (the issue form). |
| `bug` | Came from the **A file didn't convert** form (one file), or is a bug found otherwise. |
| `stage:input` | The file itself or how it was given: missing parts, a file too large. |
| `stage:decode` | cadmpeg could not read the file's bytes. |
| `stage:project` | cadmpeg read it but did not carry something into its neutral model. |
| `stage:map` | Dutchman does not map something onto SolidCraft. |
| `stage:write` | Writing the design: Dutchman's writer or cadmpeg's STEP restatement. |
| `stage:solidcraft` | SolidCraft refuses, or shows wrongly, a design Dutchman wrote. |
| `upstream:cadmpeg` | The fix belongs in cadmpeg. |
| `upstream:solidcraft` | The fix belongs in SolidCraft. |
| `needs-fixture` | We need a public file that shows it before it can be fixed. |

## Each new issue

1. **Versions.** Check the Dutchman version, SolidCraft version (or "none" if not installed),
   and source program/version, plus the report's `versions` block verbatim when available.
   Follow [Report what you found](testing-your-library.md#5-report-what-you-found) for
   command outputs and what to record when exact source builds are unknown.
2. **Stage.** Take it from the report's `stage` (every loss and failure has one; see
   [Stages](json.md#stages)) and add its `stage:` label. Correct it when the cause says
   otherwise: a `map` refusal whose reason is that cadmpeg left something unresolved is
   `stage:project`.
3. **Upstream.** Add `upstream:cadmpeg` or `upstream:solidcraft` from the report's `upstream`,
   or from what you find. Leave it off when the fix is Dutchman's.
4. **Duplicates.** Search open issues for the same message, code or blocked reason (numbers
   aside). Close a duplicate with a link to the first, and add its counts and ids there.
5. **A file to work from.** If no synthetic public file shows the cause, add `needs-fixture`.
   Offer the [minimum-reproduction step](testing-your-library.md#want-to-go-further).
   Never ask for their original design or library; only a synthetic file rebuilt from
   scratch is shared in this flow.

## Taking a cause upstream

For `upstream:cadmpeg`:

1. **Group by root cause.** Many issues (and many formats) can share one: one loss code, one
   record cadmpeg does not read. Read the codes and messages across issues before filing
   anything.
2. **One upstream item per root cause**, never one per user report. Best is a cadmpeg pull
   request with a synthetic test that shows the problem; next best, a recipe for a public test
   file (one the fixture booklet can add) that reproduces it.
3. **Nothing private goes upstream.** No user's file, no part of one, no name, path or
   company from a report, and no proprietary format content in public text. Describe the
   cause in cadmpeg's own terms (the loss code, the record kind) with the synthetic test.
4. **Link** the upstream item from every Dutchman issue it covers.
5. **Close** those issues when Dutchman's cadmpeg pin has moved past the fix and a Dutchman
   release carries it, saying "fixed in 0.1.x" and which release to install.

For `upstream:solidcraft`, the same: group by root cause, one upstream item per cause, linked
from each issue, closed when a Dutchman release that writes for the fixed SolidCraft ships.
While SolidCraft's kernel work is paused, record kernel problems (a shape SolidCraft cannot
build or mesh) in the Dutchman issue with the evidence, and do not file them on SolidCraft;
other SolidCraft problems are filed as usual.

## Keeping users with us

Messages Dutchman prints point to Dutchman's issues, not to cadmpeg's or SolidCraft's, so a
user never has to choose where to report. Keep it that way in new messages and docs.
