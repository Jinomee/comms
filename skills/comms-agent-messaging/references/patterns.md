# Tested Multi-Agent Patterns

Every pattern below has been tested with real agents. Event outputs are from real runs. Full working scripts are in `references/scripts/`.

---

## Pattern 1: Basic Two-Agent Messaging

Worker sends result, reviewer acknowledges, DONE signal to orchestrator.

**Script:** `scripts/basic-messaging.sh`

Key logic:

```bash
# Worker does task, sends result to reviewer
comms 1 claude --tag worker --go --headless \
  --comms-prompt "Do: ${task}. Send result: comms send \"@reviewer-\" --thread ${thread} --intent inform -- \"RESULT: <answer>\". Then: comms stop"

# Reviewer acks worker, sends DONE to orchestrator
comms 1 claude --tag reviewer --go --headless \
  --comms-prompt "Wait for @worker-. Reply ACK. Send DONE to @bigboss. Then: comms stop"

# Orchestrator waits for DONE
comms events --wait 120 --sql "type='message' AND msg_thread='${thread}' AND msg_text LIKE '%DONE%'"
```

**Real event JSON from test run:**
```json
{"id":42,"type":"message","instance":"mila","data":{"from":"mila","text":"RESULT: 1, 2, 3","scope":"mentions","mentions":["niro"],"intent":"inform","thread":"basic-1774354927","sender_kind":"instance","delivered_to":["niro"]}}
{"id":45,"type":"message","instance":"niro","data":{"from":"niro","text":"DONE","scope":"broadcast","intent":"inform","thread":"basic-1774354927","sender_kind":"instance","delivered_to":["mila"]}}
```

---

## Pattern 2: Worker-Reviewer Feedback Loop

Worker does task, reviewer evaluates, sends APPROVED or FIX feedback, worker corrects if needed.

**Script:** `scripts/review-loop.sh`

Key logic:

```bash
# Worker: does task, sends ROUND N DONE, listens for FIX/APPROVED
--comms-prompt "Task: ${task}. Send ROUND 1 DONE to @reviewer-. If FIX feedback, fix and resend as ROUND 2 DONE. After APPROVED, send FINAL to @bigboss."

# Reviewer: checks each round, sends APPROVED or FIX
--comms-prompt "On ROUND N DONE: if correct send APPROVED, if wrong send FIX: <issue>."

# Orchestrator waits for FINAL (after APPROVED)
comms events --wait 120 --sql "type='message' AND msg_thread='${thread}' AND msg_text LIKE '%FINAL%'"
```

**Key insight:** The FIX/APPROVED protocol creates a natural feedback loop. Workers self-correct based on reviewer feedback. Multiple rounds happen automatically.

---

## Pattern 3: Ensemble Consensus (N Agents + Judge)

N agents independently answer the same question, judge reads all answers and aggregates.

**Script:** `scripts/ensemble-consensus.sh`

Key logic:

```bash
# Launch N contestants in a loop
for i in 1 2 3; do
  comms 1 claude --tag "c${i}" --go --headless \
    --comms-prompt "Answer independently: ${task}. Send ONLY your answer: comms send \"@judge-\" --thread ${thread} --intent inform -- \"C${i}: <answer>\". Then: comms stop."
done

# Judge reads all answers via event query
--comms-prompt "Wait for 3 answers. Check: comms events --sql \"msg_thread='${thread}' AND msg_text LIKE 'C%'\" --last 10. Synthesize. Send VERDICT."

# Orchestrator waits for VERDICT
comms events --wait 120 --sql "type='message' AND msg_thread='${thread}' AND msg_text LIKE '%VERDICT%'"
```

**Key insight:** The judge uses `comms events --sql` to query thread messages, reading all answers in one call. Agents run in parallel so N agents cost same wall-clock as 1.

---

## Pattern 4: Sequential Cascade Pipeline

Each stage reads previous stage's transcript for full context handoff.

**Script:** `scripts/cascade-pipeline.sh`

Key logic:

```bash
# Stage 1: Planner
comms 1 claude --tag plan --go --headless \
  --comms-prompt "Plan: ${task}. Send PLAN DONE."

# Wait for plan, then launch stage 2 with transcript reference
comms events --wait 60 --sql "msg_thread='${thread}' AND msg_text LIKE '%PLAN DONE%'"

# Stage 2: Executor reads planner's transcript
comms 1 claude --tag exec --go --headless \
  --comms-prompt "Read planner transcript: comms transcript @${planner} --last 3. Execute the plan. Send EXEC DONE."
```

**Key insight:** `comms transcript @name --full` is the context handoff mechanism. Each pipeline stage gets the complete work product of the previous stage. Use `--detailed` to include tool I/O (Bash output, file edits).

---

## Pattern 5: Cross-Tool (Claude + Codex)

Claude designs the spec, Codex implements in sandbox.

**Script:** `scripts/cross-tool-duo.sh`

Key logic:

```bash
# Codex waits for spec, implements
comms 1 codex --tag eng --go --headless \
  --comms-prompt "Wait for spec from @arch-. Implement it. Send IMPLEMENTED."

# Claude designs spec, sends to Codex, waits for confirmation
comms 1 claude --tag arch --go --headless \
  --comms-prompt "Design spec: ${task}. Send SPEC to @eng-. Wait for IMPLEMENTED. Send APPROVED."

# Orchestrator waits for APPROVED
comms events --wait 180 --sql "msg_thread='${thread}' AND msg_text LIKE '%APPROVED%'"
```

---

## Pattern 6: Codex Codes, Claude Reviews Transcript

Codex writes and runs code, Claude reads Codex's full transcript to review.

**Script:** `scripts/codex-worker.sh`

Key logic:

```bash
# Codex does the work
comms 1 codex --tag coder --go --headless \
  --comms-prompt "Do: ${task}. Send CODE DONE to @reviewer-."

# Claude reviews by reading Codex's transcript
comms 1 claude --tag reviewer --go --headless \
  --comms-prompt "Wait for CODE DONE. Read transcript: comms transcript @${coder} --last 5 --full. Send REVIEWED: pass/fail."
```

**Key insight:** Claude reads Codex's complete transcript (including Bash output, file writes, command results) via `comms transcript @name --full --detailed`. This enables deep code review without sharing files.

---

## Summary Table

| # | Pattern | Agents | Tools | Use case |
|---|---------|--------|-------|----------|
| 1 | Basic messaging | 2 | Claude x2 | Simple task delegation |
| 2 | Review loop | 2 | Claude x2 | Self-correcting feedback |
| 3 | Ensemble consensus | 4 | Claude x4 | Diverse perspectives, best answer |
| 4 | Cascade pipeline | 2 | Claude x2 | Sequential plan-then-execute |
| 5 | Cross-tool duo | 2 | Claude+Codex | Design + sandbox implementation |
| 6 | Codex->Claude review | 2 | Codex+Claude | Code execution + transcript review |
