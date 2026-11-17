---
description: Review the WebSocket layer for live run updates against the SM WebSocket Topology proposal
argument-hint: "[optional file, diff, or question]"
---

Review the WebSocket layer that lets Scenario Manager watch a simulation run live. The reference is the "SM WebSocket Topology" proposal, option A.

Launch the `ws-layer-reviewer` agent. It is read-only. Pass this scope:

- the socket consumer and its routing
- the ticket endpoint (`POST /sm/ws-ticket`) and ticket storage
- the push from stored events to the run group, including the Redis failure path
- the status and activity REST endpoints
- the connection order: subscribe, read history, send `replay_done`, then live frames with duplicates dropped
- the user's extra focus, if any: $ARGUMENTS

Ask the agent for a verdict of pass, pass with notes, or block, with file and line for each finding. Do not edit code. Report the agent's verdict as-is. If it blocks, quote the finding and stop.
