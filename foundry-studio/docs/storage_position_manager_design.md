# Storage Position Manager Design

## Overview

This document outlines the design principles and algorithms for the Storage Position Manager, which handles intelligent placement of totes back into storage positions within warehouse aisles. The system optimizes for load balancing, accessibility, and operational efficiency.

---

## Back-to-Store Algorithm

### Hard Filtering Criteria

Aisles are eligible for back-to-store operations only if they meet both conditions:
- **Free relay positions available** for tote handoff
- **Free tote storage locations** matching the tote's category

---

## Aisle Selection Logic

### For Non-Empty Totes

The system follows a multi-criteria selection process with hierarchical sorting:

#### 1. Popularity-Based Location Preference
- Loop through available popularity categories in sequence:
  - **Category A totes**: Search priority A → B → C
  - **Category B totes**: Search priority B → A → C
  - **Category C totes**: Search priority C → B → A

#### 2. VTM Load Balancing
- Track number of all VTM tasks per aisle (both storage-to-relay and relay-to-storage movements)
- Apply bucketing factor to group aisles with similar load levels
- Prefer aisles with zero active tasks when possible when number of active aisles is less than available VTMs

#### 3. Availability Ratio
- Calculate availability ratio for tote's category within each aisle:
  ```
  Availability Ratio = Empty Locations / Occupied Locations
  ```
- Only consider unblocked locations in the calculation
- Higher ratios indicate more available capacity and are preferred
- Use discrete values to create buckets of aisles with similar availability
- Category-specific calculation ensures matching tote types with appropriate locations

#### 4. Distance from Pick PPS (Primary Sorting Criteria)
- Measure aisle distance from the relevant Pick Put Position Station (PPS)
- Apply bucketing factor to group aisles by distance bands
- Prefer closer aisles to minimize travel time

### For Empty Totes

#### 1. VTM Load Factor
- Same load balancing logic as non-empty totes
- Distribute tasks evenly across available VTMs

#### 2. Empty Location Percentage
- Calculate warehouse-wide empty location ratio:
  ```
  Empty Ratio = Total Empty Locations / Total Locations
  ```
- Consider all location categories (irrespective of individual category)
- Higher percentages indicate more storage flexibility

#### 3. Distance from Put PPS
- Measure aisle distance from Put Position Station
- Prefer aisles closer to the put operation origin

---

## Storage Location Selection Within Aisle

### For Non-Empty Totes
- **Random selection** among eligible locations within the chosen aisle

### For Empty Totes
- **Popularity-based sequence** (configurable priority):
  - Default order: A → B → C
  - Prioritize higher popularity locations from pick PPS perspective
- Ensures empty totes are staged in optimal positions for future picks

---

## Location Category (Popularity) Calculation

The popularity score for each tote storage location is calculated using weighted cost factors:

```
Total Storage Popularity =
  (Floor Cost Factor × Ground IDC) +
  (Height Cost Factor × Vertical Movement Time) +
  (Depth Cost Factor × Depth Movement Time) +
  (Telescopic Cost Factor × Telescopic Transition Time) +
  (Highway Factor for HTM accessibility)
```

### Factor Definitions

- **Floor Cost Factor**: Based on average Inverse Distance Cost (IDC) from nearest 3 Pick PPS (for empty totes, IDC from Put PPS)
- **Height Cost Factor**: Penalty for vertical travel time (High impact)
- **Depth Cost Factor**: Penalty for depth movement time (Very high impact)
- **Telescopic Cost Factor**: Transition time for telescopic VTM operations (to be confirmed)
- **Highway Factor**: HTM accessibility and traffic in the aisle

---

## Simulation Test Scenarios

To validate the Storage Position Manager, the following warehouse states should be tested:

### Warehouse State Variations
1. **Empty warehouse** - Baseline performance
2. **Completely filled warehouse** - Maximum capacity operations
3. **Post-aggressive pick/put** - Real-world operational state
4. **Chaotic warehouse** - High popularity mismatches
   - Sub-cases with varying levels of individual category mismatches
5. **Optimized warehouse** - Low popularity mismatches
6. **Storage depth variations** - 1-deep vs 0-deep configurations
7. **PPS placement scenarios**:
   - Pick and Put PPS on same side of warehouse
   - Pick and Put PPS on opposite sides of warehouse
8. **Warehouse with existing tasks** - Active pick/put operations in progress

### Back-to-Store Scenarios
- Back-to-store for empty totes
- Back-to-store for non-empty totes
- Operations from Pick PPS
- Operations from Put PPS

---

## Performance Metrics

### Primary Metrics

1. **Category Deviation Analysis**
   - Measure deviation between tote categories and assigned location categories
   - Track mismatches over time as warehouse state evolves

2. **Clustering Quality**
   - Evaluate quality of tote groupings based on updated storage positions
   - Assess whether similar totes are co-located effectively

3. **Number of TLOC deviations between expected & actual**
   - Number of TLOCs that had 10% deviation between the probably of picking from expeced / actual when number of active aisles is less than available VTMs

### Secondary Metrics

- VTM utilization balance across zones
- Average travel distance for back-to-store operations
- Relay position utilization and blocking events
- Location fill rates by category
- Task completion times for storage operations

---

## Design Principles

1. **Load Balancing**: Distribute VTM tasks evenly to prevent zone bottlenecks
2. **Accessibility Optimization**: Prioritize locations based on distance and movement costs
3. **Category Matching**: Align tote categories with location popularity scores
4. **Flexibility**: Support configurable priority sequences and cost factors
5. **Real-World Validation**: Test across diverse warehouse states and operational scenarios
