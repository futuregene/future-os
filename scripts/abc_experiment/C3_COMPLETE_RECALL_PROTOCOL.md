# C3 complete-recall prompt iterations

User requested continued prompt optimization until complete recall. Acceptance
is recorded separately as 18/18 cases issuing queries and 178/178 positive
literal occurrences with zero false positives. Neither one alone is completion.

Keep all 18 original C3 cases, full-history questions/candidate order/projections,
future/deepseek-flash, thinking disabled, 8192 output cap, native Future history
query adapters, 64 logical-query/256KiB return-stop allowance and AUTO API choice.
Every prompt version runs the WHOLE cohort once; no low-score-only retries and
no model answer or gold label is supplied to the inference context. Keep every
failure and cost. These are tuning-set results, not independent generalization.

Planned variants, executed in order until a verified target is observed:
1. head: concise literal-occurrence/archive-audit rules added to the system
   prompt. Explicitly separate original snippets/text from query echoes and
   response metadata, count literal substrings rather than numeric equality,
   retain corrected-but-originally-seen statements, and audit the candidate
   list against the original archive.
2. tail: same rules as a final system message after the unchanged user question,
   testing instruction salience in long contexts. This changes prompt placement,
   not data/tool definitions or the question itself; not a native-runtime claim.
3. example-tail: tail plus clearly labelled synthetic schema examples showing
   an empty result versus original snippet evidence; no test answers/labels.

These prompts explicitly instruct original-archive verification of the candidate
list. That is stronger than merely offering tools. Do not describe the resulting
calls as unprompted spontaneous lookup. There is still no API required setting,
fixed minimum-call gate, generated per-case pending list, or code selecting
queries for the model. Original historical tool arguments may count as original
text; current lookup arguments/query echoes/response metadata may not.

The existing grader uses literal string occurrence, including substrings within
identifiers/numeric text. State this convention explicitly rather than making
semantic quantity claims (e.g. a substring is not numeric equality). Do not
change gold labels, supply specific missed candidates, or rewrite scores.

Each version freezes its own directory and input/source/binary hashes. A
successful observed result requires offline request/trace/database/scorer/cost
verification before completion. If no planned variant meets the target, inspect
remaining errors and decide a new whole-cohort version rather than silently
retrying the same failed samples. The durable todo remains open.

Starting cumulative spend CNY53.99044076, global ceiling CNY300 including all
history. Each phase reserves within a CNY5 incremental ceiling; subsequent
phases chain the actual verified ending balance. STOP is checked before another
paid model request. No production deployment or freezing-binary rebuild is part
of this optimization.
