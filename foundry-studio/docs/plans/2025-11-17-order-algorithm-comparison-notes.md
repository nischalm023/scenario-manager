# Order Algorithm Comparison - Implementation Notes

## Overview

Extended the comparison runner to support order selection algorithm comparisons alongside existing put algorithm comparisons. Users can now select any 2 algorithms to compare side-by-side using a modal-based selection interface.

## Key Design Decisions

### Modal-Based Selection Pattern

**Decision:** Use modal for algorithm selection for both put and order comparisons

**Rationale:**
- Consistent UX pattern for both comparison types
- Allows future extension (compare robot speeds, scenarios, etc.)
- More flexible than hardcoded algorithm pairs
- Reusable modal component reduces code duplication

### Backend Algorithm Parameters

**Decision:** Add `algorithm1` and `algorithm2` to `ComparisonConfigDTO`

**Rationale:**
- Type-safe contract between frontend and backend
- Backend validates algorithm values exist
- Easy to extend to 3+ algorithm comparisons in future
- OpenAPI generates TypeScript types automatically

### Frontend Display Metadata

**Decision:** Maintain algorithm display metadata (labels, emojis) in frontend constants

**Rationale:**
- Display concerns belong in presentation layer
- Backend controls valid algorithm values via enum
- No runtime API calls for static metadata
- Type safety ensures coverage of all algorithms

### Reuse Existing Comparison Infrastructure

**Decision:** Reuse comparison state, loading modal, and results modal

**Rationale:**
- `AlgorithmComparison` component is generic - displays any `ComparisonResponse`
- Users won't run both comparisons simultaneously
- Reduces code duplication
- Easier to maintain single code path

## Architecture

```
User clicks button -> Modal opens -> Select 2 algorithms -> Run Comparison
  |
  v
Frontend sends ComparisonConfigDTO { comparisonType, algorithm1, algorithm2 }
  |
  v
ComparisonService.getOperationModes(type, alg1, alg2) -> builds mode configs
  |
  v
ComparisonService.applyModeConfig() -> overrides algorithmConfig via toBuilder()
  |
  v
Run 2 simulations with different algorithms -> calculate metrics
  |
  v
Return ComparisonResponse -> Frontend displays results
```

## Files Modified

### Backend

- **`ComparisonConfigDTO.java`** (in SimulationsController) - Added algorithm1, algorithm2 fields with validation
- **`ComparisonService.java`** - Refactored getOperationModes to accept algorithm parameters, added buildPutModeInfo and buildOrderModeInfo helpers, implemented applyModeConfig to override algorithm settings

### Frontend

- **`constants/algorithms.ts`** - NEW: Algorithm display metadata (PUT_ALGORITHMS, ORDER_ALGORITHMS, defaults)
- **`AlgorithmSelectionModal.tsx`** - NEW: Reusable modal component for algorithm selection
- **`ConfigurationPanel.tsx`** - Added "Compare Order Algorithms" button and onCompareOrderAlgorithms prop
- **`SimulatorPage.tsx`** - Added modal state, updated handleRunComparison to accept algorithm parameters, added click handlers
- **`generated-api.ts`** - Regenerated with new DTO fields (algorithm1, algorithm2)

## Testing

### Backend Unit Tests

- **`ComparisonServiceTest.java`** - NEW
  - `testGetOperationModes_Put_ReturnsTwoModes()` - Validates put algorithm mode generation
  - `testGetOperationModes_Order_ReturnsTwoModes()` - Validates order algorithm mode generation
  - `testGetOperationModes_InvalidComparisonType_ThrowsException()` - Error handling
  - `testGetOperationModes_InvalidPutAlgorithm_ThrowsException()` - Error handling
  - `testGetOperationModes_InvalidOrderAlgorithm_ThrowsException()` - Error handling
  - `testApplyModeConfig_OrderAlgorithm_OverridesCorrectly()` - Order algorithm override
  - `testApplyModeConfig_PutAlgorithm_OverridesCorrectly()` - Put algorithm override
  - `testApplyModeConfig_EmptyConfig_ReturnsOriginal()` - No-op case

### Frontend Component Tests

- **`AlgorithmSelectionModal.test.tsx`** - NEW
  - Modal rendering tests for put and order comparison types
  - Default algorithm initialization
  - Validation (same algorithm selection disabled)
  - Button click handlers (confirm and cancel)

### Manual Testing Checklist

**Put Algorithm Comparison:**
- [x] Modal opens correctly
- [x] Dropdowns show proper labels with emojis
- [x] Validation works (same algorithm disabled)
- [x] API call succeeds
- [x] Results display correctly

**Order Algorithm Comparison:**
- [x] Modal opens correctly
- [x] All 3 order algorithms selectable
- [x] API call with order comparison succeeds
- [x] Results display correctly
- [x] Can run multiple comparisons
- [x] Timeline data shows correctly

## Algorithm Support

### Put Algorithms
- `greedy` - Greedy Put (fast greedy placement)
- `min_weighted_cycle_distance` - Min Weighted Cycle Distance (optimizes robot travel)

### Order Selection Algorithms
- `maximize_ppf_directly` - Maximize PPF Directly (greedy algorithm maximizing picks per face)
- `maximize_ppf_with_tiering` - PPF with Tiering (PPF optimization with rack reuse preference)
- `maximize_tpq_with_vut` - Maximize TPQ with VUT (tasks per tote with variety, urgency, and tiering)

## Future Extensions

### Easy to Add

1. **Compare 3+ algorithms** - Change validation, add third dropdown, update backend to handle List
2. **Compare robot speeds** - New comparison type with numeric parameters
3. **Compare scenarios** - New comparison type with scenario selection
4. **Export comparison results** - Add export button to AlgorithmComparison component
5. **Save comparison history** - Store results in localStorage or backend

### Pattern to Follow

1. Add new `comparisonType` value to backend enum/validation
2. Add metadata helper method (like `buildOrderModeInfo`)
3. Update `applyModeConfig` to handle new config field
4. Add algorithm/parameter metadata to frontend constants
5. Update AlgorithmSelectionModal if needed (or create new modal for different parameter types)
6. Reuse existing comparison state, loading, and results components

## Performance Considerations

- Each comparison runs 2 full simulations sequentially
- No caching of simulation results (each run is independent)
- Frontend uses React state for comparison results (not persisted)
- Consider adding comparison result caching for repeated comparisons with same config

## Related Documentation

- `CLAUDE.md` - Algorithm Comparison section describes the overall comparison feature
- `specs/relay_system/design.md` - Relay system architecture
- `docs/JAVA_STYLE_GUIDE.md` - Java coding standards (required for backend changes)
