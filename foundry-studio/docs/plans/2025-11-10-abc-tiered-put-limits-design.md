# ABC-Tiered Per-Rack Put Limits Design

**Date:** 2025-11-10
**Status:** Approved
**Component:** Put Assignment (Greedy Algorithm)

## Problem Statement

The current greedy put algorithm has two issues:

1. **Limited rack consideration**: Only considers top 100 closest racks per SKU, leaving 64% of warehouse (180/280 racks) completely empty
2. **SKU monopolization**: Popular SKUs can occupy entire racks (80/80 units), preventing SKU mixing and diversity

**Current Results (scenario_a with 280 racks):**
- Only 100 racks used (35.7%)
- Only 82 SKUs placed out of 500
- Most racks have 1-2 SKUs only
- Popular items dominate closest racks completely

## Design Goals

1. **Maximize warehouse utilization**: Use more of the available 280 racks
2. **Enforce SKU mixing**: Prevent single SKU from monopolizing a rack
3. **Follow warehouse ABC principles**: Popular items get higher per-rack limits (concentrated), less popular items get lower limits (spread out)
4. **Keep it simple**: No complex formulas, use existing scenario Pareto distribution

## Solution: ABC-Tiered Per-Rack Limits

### High-Level Approach

Classify SKUs into three tiers (A/B/C) based on popularity, then apply tier-specific limits on how much of a single rack each SKU can occupy:

- **Category A (Popular)**: Can occupy up to 80% of a rack
- **Category B (Medium)**: Can occupy up to 50% of a rack
- **Category C (Slow movers)**: Can occupy up to 30% of a rack

This naturally forces SKU mixing: Category C racks will have 3-4 SKUs, Category B racks will have 2-3 SKUs, Category A racks will have 1-2 SKUs.

### ABC Classification Algorithm

**Category A: Use existing `is_popular` flag**
```
Category A = all SKUs where item.isPopular() == true
Expected: ~20% of SKUs (100 out of 500 in scenario_a)
```

**Categories B & C: Split non-popular by order frequency**
```
Non-popular SKUs (is_popular == false) → sort by pastOrderedQuantity descending
├─ Category B: Top 37.5% of non-popular SKUs (30% of total)
└─ Category C: Bottom 62.5% of non-popular SKUs (50% of total)

Math: 30% total / 80% non-popular = 37.5%
      50% total / 80% non-popular = 62.5%
```

**Example (500 SKUs):**
- Category A: 100 SKUs (20%) - very popular
- Category B: 150 SKUs (30%) - medium popularity
- Category C: 250 SKUs (50%) - slow movers

### Per-Rack Quantity Limits

**Constants to add to `PutProcessor`:**
```java
private static final double CATEGORY_A_MAX_RACK_UTILIZATION = 0.8; // 80%
private static final double CATEGORY_B_MAX_RACK_UTILIZATION = 0.5; // 50%
private static final double CATEGORY_C_MAX_RACK_UTILIZATION = 0.3; // 30%
private static final double CATEGORY_B_PERCENTILE = 0.375; // Top 37.5% of non-popular
```

**Applied limits (example for 80-unit racks):**
- Category A: max 64 units per rack (80 × 0.8)
- Category B: max 40 units per rack (80 × 0.5)
- Category C: max 24 units per rack (80 × 0.3)

**Applied limits (example for 6-unit totes in relay system):**
- Category A: max 4 units per tote (6 × 0.8 = 4.8 → 4)
- Category B: max 3 units per tote (6 × 0.5)
- Category C: max 1 unit per tote (6 × 0.3 = 1.8 → 1)

### Implementation Changes

**Location:** `PutProcessor.initializeInventoryGreedy()`

**Change 1: Pre-compute ABC categories and per-rack limits**

Insert after line 103 (after `sortedSkus` is populated):

```java
// ABC Tier Classification
Map<String, Integer> skuMaxPerRack = new HashMap<>();

// Get capacity from first storage space (all have same capacity)
int storageCapacity = storageSpaces.get(0) instanceof Rack
    ? ((Rack) storageSpaces.get(0)).getMaxCapacity()
    : ((Tote) storageSpaces.get(0)).getMaxCapacity();

// Category A: is_popular = true
// Categories B & C: split non-popular by pastOrderedQuantity
List<String> nonPopular = new ArrayList<>();

for (String sku : sortedSkus) {
  Item item = itemCatalog.getItems().get(sku);
  if (item.isPopular()) {
    int maxPerRack = (int)(storageCapacity * CATEGORY_A_MAX_RACK_UTILIZATION);
    skuMaxPerRack.put(sku, maxPerRack);
    log.debug("SKU {} → Category A (maxPerRack: {})", sku, maxPerRack);
  } else {
    nonPopular.add(sku);
  }
}

// Split non-popular into B and C
int categoryBCount = (int)(nonPopular.size() * CATEGORY_B_PERCENTILE);
for (int i = 0; i < nonPopular.size(); i++) {
  String sku = nonPopular.get(i);
  boolean isCategoryB = i < categoryBCount;
  double utilization = isCategoryB
      ? CATEGORY_B_MAX_RACK_UTILIZATION
      : CATEGORY_C_MAX_RACK_UTILIZATION;
  int maxPerRack = (int)(storageCapacity * utilization);
  skuMaxPerRack.put(sku, maxPerRack);
  log.debug("SKU {} → Category {} (maxPerRack: {})",
      sku, isCategoryB ? "B" : "C", maxPerRack);
}

log.info("ABC Classification: A={} SKUs, B={} SKUs, C={} SKUs",
    sortedSkus.size() - nonPopular.size(),
    categoryBCount,
    nonPopular.size() - categoryBCount);
```

**Change 2: Remove MAX_STORAGE_SPACES_PER_SKU constraint**

Remove lines 129-132:
```java
// DELETE THESE LINES:
// int maxStorageSpacesPerSku = (systemType == SystemType.RELAY) ? MAX_STORAGE_SPACES_PER_SKU_RELAY : MAX_STORAGE_SPACES_PER_SKU_RTP;
// int storagesToConsiderPerSku = Math.min(maxStorageSpacesPerSku, sortedStorageSpaces.size());
```

Update line 152 (loop over ALL storage spaces):
```java
// BEFORE:
for (int i = 0; i < storagesToConsiderPerSku && remaining > 0; i++)

// AFTER:
for (int i = 0; i < sortedStorageSpaces.size() && remaining > 0; i++)
```

Remove log statement at line 206-208 (references removed variable).

**Change 3: Apply per-SKU limit during placement**

Replace lines 165-167:
```java
// BEFORE:
int spaceAvailable = maxCapacity - currentCapacity;
int qtyToPlace = Math.min(remaining, spaceAvailable);

// AFTER:
int spaceAvailable = maxCapacity - currentCapacity;
int maxAllowedForThisSku = skuMaxPerRack.get(sku);
Map<String, Integer> rackInventory =
    inventory.getStorageSpaceInventory().get(storageSpaceId);
int currentSkuQtyInRack = rackInventory.getOrDefault(sku, 0);
int qtyToPlace = Math.min(remaining,
    Math.min(spaceAvailable, maxAllowedForThisSku - currentSkuQtyInRack));
```

**Change 4: Update logging**

Add after line 204 (before final log.info):
```java
log.info("Per-rack limits: Category A ≤ {} units, Category B ≤ {} units, Category C ≤ {} units",
    (int)(storageCapacity * CATEGORY_A_MAX_RACK_UTILIZATION),
    (int)(storageCapacity * CATEGORY_B_MAX_RACK_UTILIZATION),
    (int)(storageCapacity * CATEGORY_C_MAX_RACK_UTILIZATION));
```

## Expected Outcomes

**Warehouse Utilization:**
- Before: 100/280 racks (35.7%)
- After: ~200-250/280 racks (71-89%)

**SKU Placement:**
- Before: 82 SKUs placed
- After: All 500 SKUs with inventory placed (goal)

**SKU Diversity per Rack:**
- Category A racks: 1-2 SKUs (e.g., 64 + 16 = 80 full)
- Category B racks: 2-3 SKUs (e.g., 40 + 40 = 80 full)
- Category C racks: 3-4 SKUs (e.g., 24 + 24 + 24 + 8 = 80 full)

**Spatial Distribution:**
- Before: Racks 1-131 only
- After: Racks distributed across 1-280

## Testing & Validation

**Test with scenario_a:**
1. Run simulation with `putAlgorithm: "greedy"`
2. Check `/tmp/put_assignment_debug1.csv` output

**Validation Criteria:**
- [ ] More than 150 racks have inventory (vs 100 before)
- [ ] No single SKU exceeds its category limit in any rack
- [ ] Category A SKUs have higher per-rack quantities than Category C
- [ ] CSV shows rack IDs beyond 131 (not clustered in first 100)
- [ ] More unique SKUs placed (goal: close to 500)

**CSV Analysis Commands:**
```bash
# Count racks with inventory
wc -l /tmp/put_assignment_debug1.csv

# Check SKU distribution (should show more 2-4 SKU racks)
awk 'BEGIN{counts[0]=0} NR>1 {
  n=gsub(/SKU/,"&"); counts[n]++
} END {
  for(i in counts) print i " SKUs:", counts[i]
}' /tmp/put_assignment_debug1.csv | sort -n

# Verify rack ID spread
awk -F',' 'NR>1 {print $1}' /tmp/put_assignment_debug1.csv | sort -n | tail -1

# Count unique SKUs placed
grep -o "SKU[0-9]*" /tmp/put_assignment_debug1.csv | sort -u | wc -l
```

## Configuration Constants

New constants in `PutProcessor`:
```java
// ABC tier utilization limits (fraction of storage space capacity)
private static final double CATEGORY_A_MAX_RACK_UTILIZATION = 0.8; // Popular items
private static final double CATEGORY_B_MAX_RACK_UTILIZATION = 0.5; // Medium items
private static final double CATEGORY_C_MAX_RACK_UTILIZATION = 0.3; // Slow movers

// Category B threshold (top X% of non-popular SKUs)
private static final double CATEGORY_B_PERCENTILE = 0.375; // 30% total / 80% non-popular
```

Removed constants (no longer needed):
```java
// DELETE:
// private static final int MAX_STORAGE_SPACES_PER_SKU_RTP = 100;
// private static final int MAX_STORAGE_SPACES_PER_SKU_RELAY = 600;
```

Note: `MAX_SKUS_PER_RACK` and `MAX_SKUS_PER_TOTE` constants remain (used for limiting distinct SKU count per storage space, not quantities).

## Rationale

**Why ABC tiers align with warehouse operations:**
- Matches industry-standard warehouse ABC classification
- Popular items (A) concentrated near stations for fast access
- Less popular items (C) spread across more racks for diversity
- Natural emergence of SKU mixing without complex rules

**Why use `is_popular` flag:**
- Already defined in scenario (20% popular SKUs)
- Represents intended Pareto distribution
- Consistent with scenario design (80% picks from popular items)

**Why remove MAX_STORAGE_SPACES_PER_SKU:**
- Originally from integer program (LP optimization constraint)
- Not needed in greedy: causes 64% of racks to sit empty
- Greedy naturally prefers closer racks (sorted by distance)
- Per-rack limits provide sufficient spread for less popular items

## Implementation Plan

1. Add new constants to `PutProcessor` class
2. Implement ABC classification logic after SKU sorting
3. Remove MAX_STORAGE_SPACES_PER_SKU constraint
4. Apply per-rack limits during greedy placement
5. Update logging to show ABC distribution
6. Test with scenario_a and validate CSV output
7. Compare before/after metrics
