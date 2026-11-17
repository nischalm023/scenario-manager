# AI Analysis Prompt Context Documentation

## Overview

The AI analysis feature uses a comprehensive, context-rich prompt that provides Claude Sonnet 4.5 with deep domain knowledge about GreyOrange warehouse automation systems, specifically the RTP (Racks-to-Picker) architecture.

## Prompt Structure

### 1. System Context (RTP Architecture)

**Components Explained:**
- **Robots**: General-purpose mobile robots that transport full racks
- **Racks**: Storage units containing multiple SKUs in grid-based warehouse
- **Stations**: Operator workstations where picks are performed
- **Bins**: Buffer capacity limiting parallel order processing

**Purpose**: Establishes foundational understanding of physical system architecture

### 2. Key Performance Metrics

**Picks Per Face (PPF)**
- Formula: `totalPicks / totalFacePresentations`
- Target: >2.0 (good batching)
- Low PPF (<1.5) = poor order batching or inventory distribution

**Picks Per Hour Per Station**
- System throughput per operator
- Typical range: 200-400 picks/hour/station
- Influenced by: operator time, PPF, robot utilization, bin management

**Operator Utilization**
- High (>80%) = operators rarely wait for racks (good robot performance)
- Low (<60%) = operators starved for work (robot/bin bottleneck)

**Robot Utilization**
- High (>80%) with low throughput = robots working inefficiently
- Low (<60%) = not enough work (bin management or order issue)

**Purpose**: Provides interpretation framework for metric values

### 3. Order Selection Algorithms

#### MAXIMIZE_PPF_DIRECTLY
- **Strategy**: Greedy iterative selection maximizing marginal PPF
- **Set Cover**: Finds minimum racks needed per order
- **Active Space Bonus**: +1000 for racks already in transit/at station
- **Benefits**: High PPF, efficient rack sharing
- **Trade-off**: May delay orders for better batching

#### MAXIMIZE_PPF_WITH_TIERING
- **Strategy**: Aggressive penalty for new racks (-1000 per new rack)
- **Reuse Bonus**: +100 per active rack, +100 per opened-this-round rack
- **Goal**: Extreme rack consolidation
- **Benefits**: Reduces robot trips
- **Trade-off**: Can create starvation waiting for perfect batches

**Purpose**: Enables algorithm-specific performance assessment and recommendations

### 4. Set Cover Order Fulfillment

**Mechanism:**
1. Find minimum set of racks covering all required SKUs
2. Prefer racks already in transit (+1000 bonus)
3. Support split quantities across multiple racks
4. Atomic reservation validation (all-or-nothing)

**Impact**: 20-40% PPF improvement vs. random rack selection

**Configuration**: `maxTotesPerOrder` (default 10 racks per order)

**Purpose**: Explains core optimization technique affecting PPF

### 5. Bin Management System

**Lifecycle:**
1. Order selected and "binned" (reserves bin slot)
2. Robots fetch required racks, deliver to station
3. Operator picks all tasks for order
4. Bin freed when order completes

**Critical Point**: Bin starvation blocks new orders even if robots are idle

**Tuning**: More bins = more parallel orders but higher memory/complexity

**Purpose**: Clarifies often-misunderstood bottleneck source

### 6. Common Bottlenecks & Diagnostics

#### Low PPF (<1.5)
**Root Causes**: Poor batching, bad inventory distribution, algorithm misconfiguration, insufficient bins

**Diagnostics**: Check `totalFacePresentations` vs `totalPicks`, examine `rackUtilizations`

#### Low Operator Utilization (<60%)
**Root Causes**: Robot bottleneck, bin starvation, poor order selection, layout issues

**Diagnostics**: Check `robotMetrics.overallRobotUtilization`, `operatorIdleTime`

#### High Robot Utilization (>80%) + Low Throughput
**Root Causes**: Inefficient routing, poor PPF, station congestion, suboptimal algorithm

**Diagnostics**: Calculate picks per robot-hour, check cycle times, station queues

#### Order Completion Bottleneck
**Root Causes**: Set cover failures, inventory exhaustion, bin capacity, algorithm starvation

**Diagnostics**: Check `totalOrdersCompleted`, `averageOrderCompletionTime`, logs

**Purpose**: Provides structured troubleshooting framework for common issues

### 7. Analysis Instructions

**Required Structure:**
1. **Executive Summary** - Overall assessment, key bottleneck, primary recommendation
2. **Performance Metrics Breakdown** - Interpret all key metrics with actual values
3. **Root Cause Bottleneck Analysis** - Identify primary limiting factor, explain causality
4. **Algorithm Performance Assessment** - Evaluate current algorithm, suggest alternatives
5. **Configuration Optimization** - Quick wins + long-term optimizations with specific values
6. **Comparative Insights** - RTP vs Relay system trade-offs
7. **Experiment Plan** - 3-5 specific simulation configurations to test

**Purpose**: Ensures consistent, actionable output format

## Why This Context Matters

### Without Context
- Generic advice: "increase robots" or "improve efficiency"
- No understanding of PPF importance
- Can't diagnose bin starvation vs robot bottleneck
- No algorithm-specific recommendations
- Shallow pattern matching on metrics

### With Context
- Specific: "PPF of 1.2 indicates poor batching; try MAXIMIZE_PPF_WITH_TIERING"
- Root cause analysis: "65% operator utilization + 45% robot utilization → bin starvation"
- Algorithm trade-offs: "MAXIMIZE_PPF_DIRECTLY causing starvation; switch to DIRECTLY for faster flow"
- Quantified recommendations: "Increasing bins from 8 to 16 could improve throughput by 30%"
- Domain-aware experiments: Test specific parameter combinations

## Key Insights Embedded

1. **PPF is the North Star metric** for RTP systems - more important than raw throughput
2. **Bin starvation is a hidden killer** - robots can be idle while operators wait
3. **Set cover algorithm is active** - explains 20-40% PPF improvements
4. **Algorithm choice matters** - TIERING vs DIRECTLY have opposite trade-offs
5. **Active space bonus** - explains why racks in transit get +1000 score
6. **Operator utilization semantics** - high = good (not overworked, just busy)
7. **Robot utilization paradox** - high utilization with low throughput = inefficiency

## Metric Interpretation Guide (Embedded)

| Metric | Good | Bad | What It Means |
|--------|------|-----|---------------|
| PPF | >2.0 | <1.5 | Orders sharing racks well vs poorly |
| Operator Util | >80% | <60% | Operators busy vs starved for work |
| Robot Util | 60-80% | >90% or <40% | Balanced vs overworked/underutilized |
| Picks/Hour/Station | >300 | <200 | High throughput vs bottlenecked |

## Future Enhancements

**For Relay System:**
- Add VTM/HTM zone optimization context
- Explain relay position congestion (Desmo-J Res blocking)
- Zone balancing trade-offs
- Tote capacity vs rack capacity

**For Advanced Algorithms:**
- Job shop scheduling context
- TPQ (Task Pool Quality) optimization
- Dynamic slotting strategies

**For Inventory:**
- ABC analysis and slotting strategies
- Co-location optimization for PPF
- Reservation failure patterns

## Validation

The enhanced prompt has been tested to ensure:
- ✅ Correct interpretation of PPF metrics
- ✅ Identification of bin starvation vs robot bottlenecks
- ✅ Algorithm-specific recommendations
- ✅ Quantified impact estimates
- ✅ Copy-paste ready experiment configurations
- ✅ Root cause explanations using causal relationships

## Usage Example

**Input Metrics:**
- PPF: 1.3
- Operator Utilization: 58%
- Robot Utilization: 42%
- Algorithm: MAXIMIZE_PPF_WITH_TIERING

**Expected Analysis Output:**
1. **Diagnosis**: Low PPF (1.3 < target 2.0) indicates poor order batching
2. **Root Cause**: MAXIMIZE_PPF_WITH_TIERING is too conservative, causing order starvation
3. **Evidence**: Low robot utilization (42%) shows robots idle, not a capacity issue
4. **Recommendation**: Switch to MAXIMIZE_PPF_DIRECTLY for better flow
5. **Expected Impact**: PPF should increase to 1.8-2.2, throughput +25%
6. **Experiment**: Run with MAXIMIZE_PPF_DIRECTLY, 12 bins, same scenario

This level of depth is only possible with comprehensive domain context in the prompt.
