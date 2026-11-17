# Configuration Management Refactor Plan

## Overview

Streamline configuration management to ensure consistency between frontend DTO and backend models, consolidate validation logic, and simplify configuration flow.

## Executive Summary - Key Decisions

### ✅ Field Renaming (Breaking Change)

- Rename all sub-config fields for consistency:
  - `warehouse_map` → `map_config`
  - `robots` → `robot_config`
  - `operator` → `operator_config`
  - `algorithms` → `algorithm_config`

### ✅ Legacy Fields Removal (Breaking Change)

- **`csv_file`**: Remove - data from ScenarioManager
- **`rack_capacity`**: Remove - move to `StoragePositionManager.DEFAULT_RACK_CAPACITY = 88`
- **`target_operator_utilization`**: Remove - move to manager class constant (default: 0.85)

### ✅ TPQ Parameters (Breaking Change for Frontend)

- Remove from DTO: `tpqSafetyFactor`, `tpqTolerance`
- Add to `AlgorithmConfig` as class constants: `TPQ_SAFETY_FACTOR = 1.2`, `TPQ_TOLERANCE = 0.1`

### ✅ Algorithm Config Expansion (Non-Breaking Addition)

- Add to DTO: `jobShopAlgorithm`, `enableJobShopScheduling`, `enableTpqOptimization`
- Frontend can add UI controls for these new options

### ✅ Migration Strategy

- **Breaking changes**: Update all code at once, no gradual migration
- **Config passing**: Pass config objects directly (no dict unpacking)
- **Validation**: Keep scenario validation in ConfigManager (no dependency injection in Pydantic)
- **API-only**: No config file support needed

## Problem Statement

### Current Issues

1. **Field Mismatch**: DTO and SimulationConfig have incomplete field coverage

   - DTO missing: full algorithm config (job_shop settings), csv_file (legacy?), rack_capacity (legacy), target_operator_utilization (legacy)
   - SimulationConfig has internal fields not exposed to frontend

2. **Inconsistent Field Naming**: SimulationConfig uses mixed naming conventions

   - Sub-configs use inconsistent names: `warehouse_map`, `robots`, `operator`, `algorithms`
   - Should use consistent `*_config` suffix: `map_config`, `robot_config`, `operator_config`, `algorithm_config`

3. **Scattered Validation**:

   - Some validators in DTO (`@validator` for orderSelectionAlgorithm)
   - Some validators in SimulationConfig (Field constraints: ge, le)
   - Additional validation in ConfigManager.validate_configuration()

4. **Redundant Configuration Passing**:

   - `resolve_dependencies()` returns a dict unpacking config into separate components
   - QueueSimulatorRefactored:96-103 reconstructs SimulationConfig from this dict
   - Could just pass the config object directly

5. **Legacy Field Confusion**:

   - rack_capacity, target_operator_utilization, csv_file marked as legacy
   - Not clear if these should be removed or maintained

6. **Overly Detailed TPQ Configuration**:
   - TPQ safety factor and tolerance exposed in frontend/DTO
   - These are internal tuning parameters that should have sensible defaults
   - Clutters UI and API with advanced options most users don't need

## Business Requirements

### User Stories

- As a **frontend developer**, I want the DTO to expose all configurable simulation parameters so users can control the full simulation
- As a **backend developer**, I want all validation logic in Pydantic models so I don't duplicate validation code
- As a **system maintainer**, I want clear configuration flow so I can easily trace config from API to simulation execution
- As a **developer**, I want to know which fields are legacy vs active so I can safely refactor

### Functional Requirements

1. **Complete Field Coverage**: All SimulationConfig fields should be accessible via DTO (except truly internal fields)
2. **Centralized Validation**: All validation logic in Pydantic models using validators and Field constraints
3. **Simplified Config Flow**: Configuration passed as objects, not dicts
4. **Clear Legacy Handling**: Legacy fields either removed or clearly documented
5. **Bidirectional Conversion**: DTO ↔ SimulationConfig conversion should be lossless

### Non-Functional Requirements

- **Backward Compatibility**: Existing API contracts maintained during transition
- **Type Safety**: Full type hints with no `Any` where possible
- **Documentation**: Clear docstrings for all config fields and their purposes
- **Performance**: Validation should not introduce significant overhead

## Technical Design

### Architecture Principles

1. **Single Source of Truth**: SimulationConfig is the core model, DTO is a frontend facade
2. **Validation at Boundaries**: Validate at API layer (DTO) and before simulation execution (Config model)
3. **Composition Over Dict Unpacking**: Pass structured config objects instead of dicts
4. **Explicit Over Implicit**: Make field mappings and conversions explicit

### Component Design

#### 1. SimulationConfig (Core Model)

**Location**: `apps/backend/src/queue_simulator/config/models.py`

**Responsibilities**:

- Define all simulation configuration parameters
- Enforce validation rules using Pydantic Field constraints and validators
- Group related config into sub-models (RobotConfig, OperatorConfig, etc.)
- Provide clear documentation for each field

**Changes Needed**:

```python
class AlgorithmConfig(BaseModel):
    """Algorithm selection configuration."""

    order_selection_algorithm: str = Field(
        "picks_per_face",
        description="Order selection algorithm"
    )
    job_shop_algorithm: str = Field(
        "dispatching",
        description="Job shop scheduling algorithm"
    )
    enable_job_shop_scheduling: bool = Field(
        False,
        description="Enable job shop scheduling optimization"
    )
    enable_tpq_optimization: bool = Field(
        False,
        description="Enable TPQ optimization"
    )

    # Internal TPQ parameters (class-level constants, not configurable via API)
    TPQ_SAFETY_FACTOR: float = 1.2
    TPQ_TOLERANCE: float = 0.1

    @validator("order_selection_algorithm")
    @classmethod
    def validate_order_algorithm(cls, v):
        valid_algorithms = ["greedy", "picks_per_face", "tpq_optimized"]
        if v not in valid_algorithms:
            raise ValueError(f"Must be one of {valid_algorithms}")
        return v

    @validator("job_shop_algorithm")
    @classmethod
    def validate_job_shop_algorithm(cls, v):
        valid_algorithms = ["dispatching", "fcfs", "spt", "edd"]
        if v not in valid_algorithms:
            raise ValueError(f"Must be one of {valid_algorithms}")
        return v


class SimulationConfig(BaseModel):
    """Unified simulation configuration."""

    # Core simulation parameters
    simulation_duration: float = Field(
        14400.0,
        ge=10.0,
        le=86400.0,
        description="Simulation duration in seconds"
    )
    scenario_id: str = Field(
        "scenario_a",
        description="Scenario ID to use for simulation"
    )

    # Integrated configurations - RENAMED for consistency
    map_config: WarehouseMapConfig = Field(description="Warehouse map and layout configuration")
    robot_config: RobotConfig = Field(description="Robot configuration")
    operator_config: OperatorConfig = Field(description="Operator configuration")
    algorithm_config: AlgorithmConfig = Field(description="Algorithm configuration")

    # Remove legacy fields after investigation
    # rack_capacity, target_operator_utilization, csv_file

    @validator("scenario_id")
    @classmethod
    def validate_scenario(cls, v):
        # Could integrate scenario validation here
        # Or keep it in ConfigManager for dependency injection
        return v

    class Config:
        """Pydantic configuration."""
        validate_assignment = True
        extra = "forbid"
```

#### 2. SimulationConfigDTO (Frontend Facade)

**Location**: `apps/backend/src/queue_simulator/api/schemas/simulation.py`

**Responsibilities**:

- Provide user-friendly field names for frontend
- Convert between frontend naming (camelCase) and backend naming (snake_case)
- Expose ALL configurable parameters (not just subset)
- Handle unit conversions (hours ↔ seconds)

**Changes Needed**:

```python
class SimulationConfigDTO(BaseModel):
    """Frontend DTO for simulation configuration with user-friendly field names."""

    # Core simulation parameters
    duration: int = Field(4, ge=1, le=24, description="Simulation duration in hours")
    scenario: str = Field("scenario_a", description="Scenario ID to use for simulation")

    # Warehouse map configuration
    mapConfig: WarehouseMapConfig

    # Robot configuration
    robotCount: int = Field(4, ge=1, le=100, description="Number of robots")
    robotSpeed: float = Field(1.5, ge=0.1, le=5.0, description="Robot speed (m/s)")

    # Operator configuration
    operatorTime: float = Field(12.0, ge=5.0, le=30.0, description="Operator time per pick (seconds)")
    picksPerFace: float = Field(3.0, ge=1.0, le=10.0, description="Target picks per face")
    binsCount: int = Field(8, ge=4, le=32, description="Number of bins per station")

    # Algorithm configuration - EXPANDED (TPQ params removed - using internal defaults)
    orderSelectionAlgorithm: str = Field("picks_per_face", description="Order selection algorithm")
    jobShopAlgorithm: str = Field("dispatching", description="Job shop scheduling algorithm")
    enableJobShopScheduling: bool = Field(False, description="Enable job shop scheduling")
    enableTpqOptimization: bool = Field(False, description="Enable TPQ optimization")

    # Validators moved from core config
    @validator("orderSelectionAlgorithm")
    @classmethod
    def validate_order_algorithm(cls, v):
        valid_algorithms = ["greedy", "picks_per_face", "tpq_optimized"]
        if v not in valid_algorithms:
            raise ValueError(f"Must be one of {valid_algorithms}")
        return v

    @validator("jobShopAlgorithm")
    @classmethod
    def validate_job_shop_algorithm(cls, v):
        valid_algorithms = ["dispatching", "fcfs", "spt", "edd"]
        if v not in valid_algorithms:
            raise ValueError(f"Must be one of {valid_algorithms}")
        return v

    def to_core_config(self) -> CoreSimulationConfig:
        """Convert frontend DTO to core simulation config.

        This conversion should be lossless - all DTO fields map to config fields.
        Note: TPQ safety factor and tolerance use internal defaults.
        """
        return CoreSimulationConfig(
            simulation_duration=self.duration * 3600,  # hours → seconds
            scenario_id=self.scenario,
            map_config=self.mapConfig,
            robot_config=RobotConfig(
                num_robots=self.robotCount,
                robot_speed=self.robotSpeed,
            ),
            operator_config=OperatorConfig(
                operator_time_per_pick=self.operatorTime,
                target_picks_per_face=self.picksPerFace,
                num_bins=self.binsCount,
            ),
            algorithm_config=AlgorithmConfig(
                order_selection_algorithm=self.orderSelectionAlgorithm,
                job_shop_algorithm=self.jobShopAlgorithm,
                enable_job_shop_scheduling=self.enableJobShopScheduling,
                enable_tpq_optimization=self.enableTpqOptimization,
            ),
        )

    @classmethod
    def from_core_config(cls, config: CoreSimulationConfig) -> "SimulationConfigDTO":
        """Convert core config to DTO (for completeness).

        Useful for serializing config back to frontend.
        """
        return cls(
            duration=int(config.simulation_duration / 3600),  # seconds → hours
            scenario=config.scenario_id,
            mapConfig=config.map_config,
            robotCount=config.robot_config.num_robots,
            robotSpeed=config.robot_config.robot_speed,
            operatorTime=config.operator_config.operator_time_per_pick,
            picksPerFace=config.operator_config.target_picks_per_face,
            binsCount=config.operator_config.num_bins,
            orderSelectionAlgorithm=config.algorithm_config.order_selection_algorithm,
            jobShopAlgorithm=config.algorithm_config.job_shop_algorithm,
            enableJobShopScheduling=config.algorithm_config.enable_job_shop_scheduling,
            enableTpqOptimization=config.algorithm_config.enable_tpq_optimization,
        )
```

#### 3. SimulationConfigManager (Simplified)

**Location**: `apps/backend/src/queue_simulator/simulator/config_manager.py`

**Responsibilities**:

- Validate configuration with external dependencies (scenarios, etc.)
- Provide helper methods for accessing config data
- NO dict unpacking - work with config object directly

**Changes Needed**:

```python
class SimulationConfigManager:
    """Manages and validates simulation configuration with external dependencies."""

    def __init__(self, config: SimulationConfig):
        self.config = config
        self.scenario_manager: ScenarioManager | None = None
        self._validated = False

    def validate(self) -> bool:
        """Validate configuration including external dependencies.

        Note: Pydantic already validates field constraints.
        This method validates dependencies like scenario existence.
        """
        try:
            # Validate scenario exists
            scenario_manager = ScenarioManager()
            if not scenario_manager.validate_scenario(self.config.scenario_id):
                raise ValueError(f"Scenario '{self.config.scenario_id}' not found")

            self.scenario_manager = scenario_manager
            self._validated = True
            return True

        except Exception as e:
            print(f"❌ Configuration validation failed: {e}")
            return False

    def get_scenario_manager(self) -> ScenarioManager:
        """Get scenario manager (after validation)."""
        if not self._validated:
            raise RuntimeError("Configuration not validated")
        return self.scenario_manager

    def is_valid(self) -> bool:
        """Check if configuration is valid."""
        return self._validated
```

#### 4. QueueSimulatorRefactored Updates

**Location**: `apps/backend/src/queue_simulator/simulator/queue_simulator_refactored.py`

**Responsibilities**:

- Accept SimulationConfig directly
- Pass config to components instead of dict
- Remove redundant config reconstruction

**Changes Needed**:

```python
class QueueSimulatorRefactored:
    def __init__(self, config: SimulationConfig):
        print(f"🎯 Initializing QueueSimulator with {config.robot_config.num_robots} robots...")

        # Validate configuration
        self.config = config
        self.config_manager = SimulationConfigManager(config)
        if not self.config_manager.validate():
            raise ValueError("Invalid simulation configuration")

        # Component initialization (lazy loading)
        self.warehouse_builder = None
        self.simulation_engine = None
        self.metrics_manager = None
        self.results_processor = None
        self.warehouse_components = None

        print("✅ QueueSimulator initialized successfully")

    def _build_warehouse(self):
        """Build warehouse infrastructure using WarehouseBuilder."""
        if self.warehouse_components is not None:
            return  # Already built

        print("🏗️ Building warehouse infrastructure...")

        # Pass config directly - no reconstruction needed!
        scenario_manager = self.config_manager.get_scenario_manager()
        self.warehouse_builder = WarehouseBuilder(self.config, scenario_manager)

        # Build all components
        self.warehouse_components = self.warehouse_builder.build()
        self.warehouse_builder.optimize_rack_positions()

        print("✅ Warehouse infrastructure ready")

    def _initialize_metrics(self):
        """Initialize metrics collection system."""
        print("📊 Initializing metrics collection...")

        initial_inventory = self._get_initial_inventory_state()
        self.metrics_manager = MetricsManager(
            self.config.simulation_duration,
            initial_inventory
        )

        print("✅ Metrics collection initialized")

    def _run_simulation_engine(self):
        """Execute simulation using SimulationEngine."""
        print("⚙️ Starting simulation engine...")

        # Pass config directly
        self.simulation_engine = SimulationEngine(self.config)
        self.simulation_engine.run(self.warehouse_components, self.metrics_manager)

        print("✅ Simulation engine completed")

    def _process_results(self) -> dict[str, Any]:
        """Process simulation results using ResultsProcessor."""
        print("📊 Processing simulation results...")

        # Pass config directly
        self.results_processor = ResultsProcessor(self.config)
        results = self.results_processor.process(
            self.metrics_manager,
            self.warehouse_components
        )

        print("✅ Results processing completed")
        return results
```

### Data Flow

```
API Request (JSON)
    ↓
SimulationConfigDTO (validates, friendly names)
    ↓ to_core_config()
SimulationConfig (core model, all fields)
    ↓
SimulationConfigManager (validates external deps)
    ↓
QueueSimulatorRefactored (orchestrates)
    ├→ WarehouseBuilder(config)
    ├→ SimulationEngine(config)
    └→ ResultsProcessor(config)
        ↓
SimulationResult
    ↓
API Response (JSON)
```

### Migration Strategy for Legacy Fields

**Decision Matrix**:

| Field                         | Used in Code? | Migration Action                     |
| ----------------------------- | ------------- | ------------------------------------ |
| `rack_capacity`               | Check usage   | Remove or move to WarehouseMapConfig |
| `target_operator_utilization` | Check usage   | Remove or move to OperatorConfig     |
| `csv_file`                    | Check usage   | Remove (scenarios handle data)       |

**Investigation Needed**:

1. Search codebase for `rack_capacity` usage
2. Search codebase for `target_operator_utilization` usage
3. Search codebase for `csv_file` usage
4. Determine if these are truly legacy or still used

## Implementation Tasks

### Phase 1: Investigation & Planning ✅ DECISIONS MADE

- [ ] **Search codebase for `rack_capacity` usage** and update all references
  - Find all usages: `grep -r "rack_capacity" apps/backend/`
  - Decision: Move to `StoragePositionManager.DEFAULT_RACK_CAPACITY = 88`
  - Update all code to use the constant instead of config
- [ ] **Search codebase for `target_operator_utilization` usage** and update all references
  - Find all usages: `grep -r "target_operator_utilization" apps/backend/`
  - Decision: Move to appropriate manager class (StationManager or OrderProcessor)
  - Update all code to use the constant instead of config
- [ ] **Search codebase for `csv_file` usage** and update all references
  - Find all usages: `grep -r "csv_file" apps/backend/`
  - Decision: Remove from config - data comes from ScenarioManager
  - Update all code to get data from ScenarioManager instead
- [ ] **Review current validator coverage** across all models
  - Document which validators exist in DTO vs. core config
  - Plan migration of validators to appropriate models

### Phase 2: Core Model Updates

- [ ] **Rename fields in `SimulationConfig`** for consistency:
  - `warehouse_map` → `map_config`
  - `robots` → `robot_config`
  - `operator` → `operator_config`
  - `algorithms` → `algorithm_config`
- [ ] **Remove TPQ parameters from `AlgorithmConfig`**:
  - Remove `tpq_safety_factor` and `tpq_tolerance` as public fields
  - Add as private/internal constants or class-level defaults
- [ ] Move all algorithm validators to `AlgorithmConfig` in `models.py`
- [ ] Add comprehensive field documentation to all config models
- [ ] Remove or migrate legacy fields based on Phase 1 findings
- [ ] Ensure all Field constraints are properly set (ge, le, etc.)
- [ ] Update all code references to use new field names

### Phase 3: DTO Updates

- [ ] Add missing algorithm fields to `SimulationConfigDTO`:
  - `jobShopAlgorithm`
  - `enableJobShopScheduling`
  - `enableTpqOptimization`
- [ ] **Remove TPQ parameter fields** (using internal defaults):
  - Remove `tpqSafetyFactor`
  - Remove `tpqTolerance`
- [ ] Implement `from_core_config()` method for bidirectional conversion
- [ ] Update `to_core_config()` to map to renamed config fields
- [ ] Ensure DTO validators match core model validators
- [ ] Add comprehensive docstrings

### Phase 4: ConfigManager Simplification

- [ ] Remove `resolve_dependencies()` method (return dict → use config object)
- [ ] Simplify `validate()` to only check external dependencies
- [ ] Remove helper methods like `get_rack_capacity()` (use config.field directly)
- [ ] Add `get_scenario_manager()` method for safe access
- [ ] Update all docstrings

### Phase 5: Simulator Updates

- [ ] Update `QueueSimulatorRefactored.__init__()` to store config directly
- [ ] Remove config dict reconstruction in `_build_warehouse()`
- [ ] Update `WarehouseBuilder` to accept config object
- [ ] Update `SimulationEngine` to accept config object
- [ ] Update `ResultsProcessor` to accept config object
- [ ] Remove any dict unpacking code

### Phase 6: Testing & Validation

- [ ] Write unit tests for DTO ↔ Config conversion (bidirectional)
- [ ] Write unit tests for all validators
- [ ] Test with existing scenarios to ensure no regressions
- [ ] Update integration tests
- [ ] Test API endpoints with expanded config fields
- [ ] Performance testing (ensure validation overhead is minimal)

### Phase 7: Documentation & Cleanup

- [ ] Update API documentation (OpenAPI/Swagger)
- [ ] Update CLAUDE.md with new config structure
- [ ] Add migration guide for frontend to use new fields
- [ ] Remove deprecated code and TODOs
- [ ] Code review and final cleanup

## Testing Strategy

### Unit Tests

1. **Config Model Tests** (`test_config_models.py`):

   - Test all Field validators (min/max bounds)
   - Test enum validators (valid/invalid algorithm names)
   - Test cross-field validators
   - Test default values

2. **DTO Conversion Tests** (`test_dto_conversion.py`):

   - Test `to_core_config()` with all fields
   - Test `from_core_config()` with all fields
   - Test bidirectional conversion is lossless
   - Test unit conversions (hours ↔ seconds)
   - Test edge cases and boundary values

3. **ConfigManager Tests** (`test_config_manager.py`):
   - Test scenario validation (valid/invalid scenarios)
   - Test validation failure handling
   - Test scenario_manager access before/after validation

### Integration Tests

1. **End-to-End Config Flow** (`test_config_e2e.py`):

   - Test API request → DTO → Config → Simulation → Response
   - Test with various config combinations
   - Test error handling at each layer

2. **Backward Compatibility** (`test_backward_compat.py`):
   - Test existing API requests still work
   - Test migration from old to new config structure
   - Test legacy field handling

### Acceptance Criteria

- [ ] All DTO fields map 1:1 to SimulationConfig fields
- [ ] All validators are in Pydantic models (no scattered validation)
- [ ] No dict unpacking - config passed as objects
- [ ] All tests pass with >80% coverage
- [ ] No breaking changes to API contracts
- [ ] Documentation updated and accurate

## Success Metrics

- **Code Clarity**: Reduced config-related code complexity by 30%
- **Maintainability**: All validation logic in one place (Pydantic models)
- **Type Safety**: Full type coverage with no `Any` types in config flow
- **Test Coverage**: >90% coverage for config-related code
- **API Completeness**: Frontend can control 100% of configurable parameters

## Risks & Mitigations

| Risk                                      | Impact | Mitigation                                             |
| ----------------------------------------- | ------ | ------------------------------------------------------ |
| Breaking API changes                      | High   | Maintain backward compatibility during transition      |
| Performance degradation from validation   | Medium | Benchmark validation overhead, optimize if needed      |
| Missing field mappings                    | High   | Comprehensive tests for DTO ↔ Config conversion        |
| Legacy field removal breaks existing code | High   | Thorough codebase search before removal                |
| Frontend not updated to use new fields    | Medium | Coordinate with frontend team, provide migration guide |

## Follow-up Work

- Consider config versioning for future API evolution
- Explore config templates/presets for common scenarios
- Add config validation endpoint for frontend to pre-validate before submission
- Consider config diffing/comparison for debugging

---

## Pending Clarifications

### 1. Field Renaming for Top-Level Parameters

**Question**: Should we also rename the top-level simulation parameters for consistency?

Current naming:

- `simulation_duration` (snake_case)
- `scenario_id` (snake_case)

Options:

- **Option A**: Keep as-is (they're scalar values, not config objects)
- **Option B**: Rename for consistency with sub-configs
  - `simulation_duration` → `duration` (already mapped in DTO)
  - `scenario_id` → `scenario` (already mapped in DTO)

**Recommendation**: Keep as-is. The `*_config` suffix is for nested config objects, not scalar parameters.

### 2. TPQ Parameter Handling ✅ DECIDED

**Decision**: Use class-level constants for simplicity.

Implementation:

```python
class AlgorithmConfig(BaseModel):
    # TPQ internal defaults (not configurable via API)
    TPQ_SAFETY_FACTOR: float = 1.2
    TPQ_TOLERANCE: float = 0.1
```

Code that needs these values will access via `config.algorithm_config.TPQ_SAFETY_FACTOR`.

### 3. Legacy Field Migration Strategy ✅ DECIDED

**Decision**: Remove all legacy fields from SimulationConfig. Move to appropriate components as constants/properties.

Migration plan:

1. **`csv_file`**: Remove from config

   - Data should come from ScenarioManager/Scenario
   - No config parameter needed

2. **`rack_capacity`**: Remove from config

   - Add as constant or property in `StoragePositionManager`
   - Default value: 88 (from current config)
   - Example: `StoragePositionManager.DEFAULT_RACK_CAPACITY = 88`

3. **`target_operator_utilization`**: Remove from config
   - Add as constant or property in relevant manager (StationManager or OrderProcessor)
   - Default value: 0.85 (from current config)
   - Example: `StationManager.TARGET_UTILIZATION = 0.85`

**Action**: Phase 1 investigation still needed to find all usages and update them.

### 4. Backward Compatibility for Field Renaming ✅ DECIDED

**Decision**: Breaking change - update all code at once (Option A).

Strategy:

1. Update `SimulationConfig` field names in one commit
2. Use IDE find/replace to update all references:
   - `config.warehouse_map` → `config.map_config`
   - `config.robots` → `config.robot_config`
   - `config.operator` → `config.operator_config`
   - `config.algorithms` → `config.algorithm_config`
3. Run tests to catch any missed references
4. No deprecation period - clean break

**Rationale**: Since this is an internal refactor with no external API users yet, clean migration is simpler than maintaining compatibility layer.

### 5. Frontend Impact ✅ DECIDED

**Decision**: Remove TPQ fields, add algorithm fields. Frontend can handle the changes.

Frontend DTO changes:

- **Breaking changes** (acceptable): Removing `tpqSafetyFactor`, `tpqTolerance`
- **Non-breaking additions**: Adding `jobShopAlgorithm`, `enableJobShopScheduling`, `enableTpqOptimization`

Frontend update requirements:

1. Remove TPQ safety factor and tolerance UI controls (if they exist)
2. Add UI controls for:
   - Job shop algorithm selection dropdown
   - Job shop scheduling enable/disable toggle
   - TPQ optimization enable/disable toggle
3. Update TypeScript interfaces to match new DTO structure
4. Update default values in frontend forms

**Note**: Since these are new optional fields with defaults, frontend can be updated independently without breaking existing functionality.

### 6. Validation Strategy ✅ DECIDED

**Decision**: Keep scenario validation in ConfigManager - no dependency injection in Pydantic models.

Rationale:

- Pydantic models should remain dependency-free and portable
- ConfigManager handles external dependencies (ScenarioManager, file system, etc.)
- Clear separation of concerns:
  - Pydantic validators: Field-level validation (types, ranges, enums)
  - ConfigManager: Cross-component validation (scenario existence, file paths, etc.)

Implementation:

```python
class SimulationConfigManager:
    def validate(self) -> bool:
        """Validate configuration including external dependencies."""
        # Check scenario exists
        # Check warehouse map validity
        # Any other external dependency checks
```

### 7. Configuration File vs. API ✅ DECIDED

**Decision**: API-only approach - no config file support needed.

Rationale:

- Simpler implementation
- Frontend-driven workflow via API
- If batch processing needed later, can add config file support as enhancement

**Future enhancement**: If needed, could add YAML/JSON config file support later without breaking existing API.

### 8. Unit Conversion Strategy ✅ DECIDED

**Decision**: Keep unit conversions in DTO methods - it's a presentation layer concern.

Implementation:

- `to_core_config()`: Convert hours → seconds (`duration * 3600`)
- `from_core_config()`: Convert seconds → hours (`duration / 3600`)

Rationale:

- Simple conversions don't warrant separate utility class
- DTO is the boundary between user-facing (hours) and internal (seconds) representations
- Keeps conversion logic close to where it's used

**Future enhancement**: If more complex conversions are needed (e.g., units for distance, weight), refactor to a `UnitConverter` utility.

---

## Questions for User - ANSWERED

**Decisions confirmed:**

1. ✅ **Field renaming confirmed**: Use `*_config` suffix for all sub-configs
2. ✅ **TPQ parameters confirmed**: Remove from frontend/DTO, use internal defaults as class-level constants
3. ✅ **TPQ implementation**: Use class-level constants in `AlgorithmConfig`
4. ✅ **Legacy fields - DECIDED**:
   - `csv_file`: Remove from config - should be obtained from ScenarioManager/Scenario
   - `rack_capacity`: Remove from config - add as constant/property in StoragePositionManager
   - `target_operator_utilization`: Remove from config - add as constant/property in relevant manager
5. ✅ **Backward compatibility**: **Breaking change** - update all code at once, no gradual migration
6. ✅ **Frontend coordination**:
   - Remove `tpqSafetyFactor` and `tpqTolerance` (breaking change acceptable)
   - Add non-breaking additions: `jobShopAlgorithm`, `enableJobShopScheduling`, `enableTpqOptimization`
7. ✅ **Config file support**: Not needed - API-only approach
8. ✅ **Validation strategy**: Keep scenario validation in ConfigManager (no dependency injection in Pydantic models)

**Implementation can now proceed with these decisions!**
