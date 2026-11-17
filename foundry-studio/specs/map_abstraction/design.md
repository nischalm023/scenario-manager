# Map Abstraction Feature Design

## Architecture Overview

The map abstraction feature introduces a layered architecture with explicit grid-based coordinate system. All domain logic operates on GridPosition coordinates, with WorldPosition coordinates only used at the rendering and distance calculation layers. This design enables flexible station placement, highway systems, and sophisticated aisle management with clear coordinate separation.

```
┌─────────────────────────────────────────────────────────────┐
│                    Frontend Layer                           │
│  ┌─────────────────┐  ┌─────────────────┐  ┌──────────────┐ │
│  │ WarehouseMap    │  │ ConfigurationUI │  │ LayoutEditor │ │
│  │ (Konva.js)      │  │                 │  │              │ │
│  └─────────────────┘  └─────────────────┘  └──────────────┘ │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      API Layer                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │           Map Configuration Schemas                     │ │
│  │  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐  │ │
│  │  │ WarehouseMap │  │ StationLayout│  │ HighwaySystem │  │ │
│  │  │ Config       │  │ Config       │  │ Config        │  │ │
│  │  └──────────────┘  └──────────────┘  └───────────────┘  │ │
│  └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   Domain Layer                              │
│  ┌──────────────────┐  ┌──────────────────┐                │
│  │ WarehouseMap     │  │ CoordinateSystem │                │
│  │ Abstract         │  │                  │                │
│  │ ┌──────────────┐ │  │ ┌──────────────┐ │                │
│  │ │ StationEdge  │ │  │ │ GridToWorld  │ │                │
│  │ │ Manager      │ │  │ │ Transform    │ │                │
│  │ └──────────────┘ │  │ └──────────────┘ │                │
│  │ ┌──────────────┐ │  │ ┌──────────────┐ │                │
│  │ │ HighwayNet   │ │  │ │ Distance     │ │                │
│  │ │ Manager      │ │  │ │ Calculator   │ │                │
│  │ └──────────────┘ │  │ └──────────────┘ │                │
│  │ ┌──────────────┐ │  └──────────────────┘                │
│  │ │ AisleSystem  │ │                                       │
│  │ │ Manager      │ │                                       │
│  │ └──────────────┘ │                                       │
│  └──────────────────┘                                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                 Integration Layer                          │
│  ┌──────────────────┐  ┌──────────────────┐                │
│  │ Domain Models    │  │ QueueSimulator   │                │
│  │ (Robot, Rack,    │  │ (Grid-based)     │                │
│  │ Station)         │  └──────────────────┘                │
│  └──────────────────┘  ┌──────────────────┐                │
│  ┌──────────────────┐  │ Pathfinding      │                │
│  │ GridToWorld      │  │ (Grid coords)    │                │
│  │ Converter        │  └──────────────────┘                │
│  └──────────────────┘                                      │
└─────────────────────────────────────────────────────────────┘
```

## Core Components Design

### 1. WarehouseMapConfig (Pydantic Model)

Central configuration model that defines the entire warehouse layout.

```python
from enum import Enum
from typing import List, Optional, Union
from pydantic import BaseModel, Field, validator

class StationEdge(str, Enum):
    """Defines which edges of the warehouse can have stations"""
    BOTTOM = "bottom"
    TOP = "top"
    LEFT = "left"
    RIGHT = "right"

class StationLayoutType(str, Enum):
    """Predefined station layout patterns"""
    SINGLE_BOTTOM = "single_bottom"        # Current default
    SINGLE_TOP = "single_top"
    SINGLE_LEFT = "single_left"
    SINGLE_RIGHT = "single_right"
    OPPOSITE_VERTICAL = "opposite_vertical"  # Top + Bottom
    OPPOSITE_HORIZONTAL = "opposite_horizontal"  # Left + Right
    THREE_SIDES = "three_sides"            # Exclude one side
    FOUR_SIDES = "four_sides"              # All sides
    CUSTOM = "custom"                      # User-defined

class GridPosition(BaseModel):
    """Grid coordinate position (row, column)"""
    row: int = Field(ge=0, description="Grid row (0-based)")
    col: int = Field(ge=0, description="Grid column (0-based)")

class WorldPosition(BaseModel):
    """World coordinate position in meters (for rendering only)"""
    x: float = Field(description="X coordinate in meters")
    y: float = Field(description="Y coordinate in meters")

class StationEdgeConfig(BaseModel):
    """Configuration for stations on a specific edge"""
    edge: StationEdge
    station_count: int = Field(ge=1, le=20, description="Number of stations on this edge")
    spacing_grid_units: int = Field(default=5, ge=1, le=20, description="Spacing between stations in grid units")

class HighwayConfig(BaseModel):
    """Highway visualization configuration (for frontend display only - robots "fly" between points)"""
    enable_station_highways: bool = Field(default=True, description="Show highways parallel to station edges for visual clarity")
    station_highway_width_grid_units: int = Field(default=2, ge=1, le=5, description="Visual width of station highways in grid units")
    station_highway_offset_grid_units: int = Field(default=3, ge=1, le=10, description="Visual distance from stations in grid units")

class RackBlockConfig(BaseModel):
    """Rack block system configuration (all measurements in grid units)"""
    rows_per_block: int = Field(default=1, ge=1, le=5, description="Number of rack rows per block")
    aisle_width_grid_units: int = Field(default=1, ge=1, le=3, description="Width of aisles (walkways between rack blocks) in grid units")
    cross_aisle_width_grid_units: int = Field(default=2, ge=1, le=3, description="Width of cross-aisles (auto-positioned between rack blocks)")

    # Cross-aisle frequency is automatically determined by rows_per_block
    # Cross-aisles are placed between each group of rows_per_block rack rows

class WarehouseMapConfig(BaseModel):
    """Complete warehouse map configuration (all in grid coordinates)"""

    # Basic grid configuration
    grid_width: int = Field(ge=1, le=200, description="Grid width in cells")
    grid_depth: int = Field(ge=1, le=200, description="Grid depth in cells")
    cell_size_meters: float = Field(default=2.0, ge=0.5, le=10.0, description="Size of each grid cell in meters (for world coordinate conversion only)")

    # Station configuration
    station_layout_type: StationLayoutType = Field(default=StationLayoutType.SINGLE_BOTTOM)
    station_edges: List[StationEdgeConfig] = Field(default_factory=list)
    total_stations: int = Field(default=1, ge=1, le=50, description="Total stations across all edges")

    # Highway system
    highway_config: HighwayConfig = Field(default_factory=HighwayConfig)

    # Rack block system
    rack_block_config: RackBlockConfig = Field(default_factory=RackBlockConfig)

    @validator('station_edges', always=True)
    def validate_station_edges(cls, v, values):
        """Ensure station edges match layout type and total stations"""
        layout_type = values.get('station_layout_type')
        total_stations = values.get('total_stations', 1)

        if layout_type != StationLayoutType.CUSTOM:
            # Auto-generate station edges for predefined layouts
            return cls._generate_station_edges_for_layout(layout_type, total_stations)

        # For custom layouts, validate provided edges
        if not v:
            raise ValueError("Custom layout requires explicit station_edges configuration")

        total_configured = sum(edge.station_count for edge in v)
        if total_configured != total_stations:
            raise ValueError(f"Station edges total ({total_configured}) must match total_stations ({total_stations})")

        return v

    @classmethod
    def _generate_station_edges_for_layout(cls, layout_type: StationLayoutType, total_stations: int) -> List[StationEdgeConfig]:
        """Auto-generate station edge configurations for predefined layouts"""
        if layout_type == StationLayoutType.SINGLE_BOTTOM:
            return [StationEdgeConfig(edge=StationEdge.BOTTOM, station_count=total_stations)]
        elif layout_type == StationLayoutType.SINGLE_TOP:
            return [StationEdgeConfig(edge=StationEdge.TOP, station_count=total_stations)]
        elif layout_type == StationLayoutType.SINGLE_LEFT:
            return [StationEdgeConfig(edge=StationEdge.LEFT, station_count=total_stations)]
        elif layout_type == StationLayoutType.SINGLE_RIGHT:
            return [StationEdgeConfig(edge=StationEdge.RIGHT, station_count=total_stations)]
        elif layout_type == StationLayoutType.OPPOSITE_VERTICAL:
            # Distribute stations between top and bottom
            bottom_count = total_stations // 2
            top_count = total_stations - bottom_count
            return [
                StationEdgeConfig(edge=StationEdge.BOTTOM, station_count=bottom_count),
                StationEdgeConfig(edge=StationEdge.TOP, station_count=top_count)
            ]
        elif layout_type == StationLayoutType.OPPOSITE_HORIZONTAL:
            # Distribute stations between left and right
            left_count = total_stations // 2
            right_count = total_stations - left_count
            return [
                StationEdgeConfig(edge=StationEdge.LEFT, station_count=left_count),
                StationEdgeConfig(edge=StationEdge.RIGHT, station_count=right_count)
            ]
        # Add more layout types as needed
        else:
            return [StationEdgeConfig(edge=StationEdge.BOTTOM, station_count=total_stations)]
```

### 2. WarehouseMapAbstraction (Core Engine)

The main abstraction engine that converts configuration to coordinate systems.

```python
from typing import Dict, List, Tuple, Set
from dataclasses import dataclass
import math

@dataclass
class GridCoordinate:
    """Grid coordinate (row, col) - primary coordinate system"""
    row: int
    col: int

@dataclass
class WorldCoordinate:
    """World coordinate in meters (for rendering/calculations only)"""
    x: float
    y: float

@dataclass
class StationGridPosition:
    """Station with its grid position and metadata"""
    station_id: int
    edge: StationEdge
    grid_position: GridCoordinate
    accessible_directions: List[str]  # ["north", "south", "east", "west"]

@dataclass
class HighwayGridSegment:
    """Highway segment definition in grid coordinates (for visualization only)"""
    highway_id: str
    start_grid: GridCoordinate
    end_grid: GridCoordinate
    width_grid_units: int
    highway_type: str  # "station_highway"

@dataclass
class RackBlockGridDefinition:
    """Rack block with its rack positions in grid coordinates"""
    block_id: str
    rack_grid_positions: List[GridCoordinate]
    cross_aisle_grid_positions: List[GridCoordinate]
    direction: str  # "horizontal", "vertical"

class WarehouseMapAbstraction:
    """Core abstraction engine for warehouse layout (operates in grid coordinates)"""

    def __init__(self, config: WarehouseMapConfig):
        self.config = config
        self.grid_bounds: Dict[str, int] = {}
        self.stations: List[StationGridPosition] = []
        self.highways: List[HighwayGridSegment] = []
        self.rack_blocks: List[RackBlockGridDefinition] = []
        self.rack_grid_positions: Dict[Tuple[int, int], GridCoordinate] = {}
        self.occupied_grid_cells: Set[Tuple[int, int]] = set()

        self._calculate_grid_bounds()
        self._generate_station_grid_positions()
        self._generate_highway_grid_network()
        self._generate_rack_block_grid_system()
        self._generate_rack_grid_positions()

    def _calculate_grid_bounds(self):
        """Calculate the grid coordinate bounds based on configuration"""
        # Add space for highways (stations are on the edge, no extra space needed)
        highway_space = 0
        if self.config.highway_config.enable_station_highways:
            highway_space = (
                self.config.highway_config.station_highway_offset_grid_units +
                self.config.highway_config.station_highway_width_grid_units
            )

        self.grid_bounds = {
            'min_col': -highway_space,
            'max_col': self.config.grid_width - 1 + highway_space,
            'min_row': -highway_space,
            'max_row': self.config.grid_depth - 1 + highway_space,
            'width': self.config.grid_width + 2 * highway_space,
            'height': self.config.grid_depth + 2 * highway_space
        }

    def _generate_station_grid_positions(self):
        """Generate station grid positions based on edge configurations"""
        self.stations = []
        station_id = 0

        for edge_config in self.config.station_edges:
            edge_stations = self._generate_stations_for_edge(edge_config, station_id)
            self.stations.extend(edge_stations)
            station_id += len(edge_stations)

    def _generate_stations_for_edge(self, edge_config: StationEdgeConfig, start_id: int) -> List[StationGridPosition]:
        """Generate stations for a specific edge (stations are positioned directly on the edge)"""
        stations = []
        edge = edge_config.edge
        count = edge_config.station_count
        spacing = edge_config.spacing_grid_units

        if edge == StationEdge.BOTTOM:
            # Bottom edge stations - positioned at row -1 (just outside the grid)
            start_col = (self.config.grid_width - 1 - (count - 1) * spacing) // 2
            for i in range(count):
                col = start_col + i * spacing
                row = -1  # Just outside the bottom edge
                stations.append(StationGridPosition(
                    station_id=start_id + i,
                    edge=edge,
                    grid_position=GridCoordinate(row, col),
                    accessible_directions=["north"]
                ))

        elif edge == StationEdge.TOP:
            # Top edge stations - positioned at row grid_depth (just outside the grid)
            start_col = (self.config.grid_width - 1 - (count - 1) * spacing) // 2
            for i in range(count):
                col = start_col + i * spacing
                row = self.config.grid_depth  # Just outside the top edge
                stations.append(StationGridPosition(
                    station_id=start_id + i,
                    edge=edge,
                    grid_position=GridCoordinate(row, col),
                    accessible_directions=["south"]
                ))

        elif edge == StationEdge.LEFT:
            # Left edge stations - positioned at col -1 (just outside the grid)
            start_row = (self.config.grid_depth - 1 - (count - 1) * spacing) // 2
            for i in range(count):
                col = -1  # Just outside the left edge
                row = start_row + i * spacing
                stations.append(StationGridPosition(
                    station_id=start_id + i,
                    edge=edge,
                    grid_position=GridCoordinate(row, col),
                    accessible_directions=["east"]
                ))

        elif edge == StationEdge.RIGHT:
            # Right edge stations - positioned at col grid_width (just outside the grid)
            start_row = (self.config.grid_depth - 1 - (count - 1) * spacing) // 2
            for i in range(count):
                col = self.config.grid_width  # Just outside the right edge
                row = start_row + i * spacing
                stations.append(StationGridPosition(
                    station_id=start_id + i,
                    edge=edge,
                    grid_position=GridCoordinate(row, col),
                    accessible_directions=["west"]
                ))

        return stations

    def _generate_highway_grid_network(self):
        """Generate highway network for visualization based on configuration"""
        self.highways = []
        highway_config = self.config.highway_config

        if highway_config.enable_station_highways:
            self._generate_station_highways()

    def _generate_station_highways(self):
        """Generate highways parallel to station edges for visualization"""
        highway_config = self.config.highway_config
        width_grid_units = highway_config.station_highway_width_grid_units
        offset_grid_units = highway_config.station_highway_offset_grid_units

        # Group stations by edge
        stations_by_edge = {}
        for station in self.stations:
            edge = station.edge
            if edge not in stations_by_edge:
                stations_by_edge[edge] = []
            stations_by_edge[edge].append(station)

        # Generate highways for each edge that has stations
        for edge, edge_stations in stations_by_edge.items():
            if edge == StationEdge.BOTTOM:
                # Highway parallel to bottom edge for visualization
                highway_row = -1 - offset_grid_units  # Position highway away from stations
                self.highways.append(HighwayGridSegment(
                    highway_id=f"station_highway_bottom",
                    start_grid=GridCoordinate(highway_row, self.grid_bounds['min_col']),
                    end_grid=GridCoordinate(highway_row, self.grid_bounds['max_col']),
                    width_grid_units=width_grid_units,
                    highway_type="station_highway"
                ))
            # Add similar logic for other edges...

    def _generate_rack_block_grid_system(self):
        """Generate rack block system with multi-row support and automatic cross-aisle placement"""
        self.rack_blocks = []
        rack_block_config = self.config.rack_block_config

        # Cross-aisles (zone separators) are automatically placed between groups of rows_per_block
        # Example configurations:
        #
        # rows_per_block = 1 (traditional): RB-A-RB-A-RB-A-RB (RB=Rack Block, A=Aisle/walkway)
        # rows_per_block = 2: RB-RB-CA-RB-RB-CA-RB-RB (CA=Cross-Aisle/Zone separator)
        # rows_per_block = 3: RB-RB-RB-CA-RB-RB-RB-CA-RB-RB-RB
        #
        # This creates logical rack blocks with cross-aisles as visual zone dividers

        current_row = 0
        block_id = 1

        while current_row < self.config.grid_depth:
            # Create rack block with rows_per_block rack rows
            block_rows = []
            for i in range(rack_block_config.rows_per_block):
                if current_row + i < self.config.grid_depth:
                    block_rows.append(current_row + i)

            if block_rows:
                # Create rack block definition for this group
                block_def = RackBlockGridDefinition(
                    block_id=f"rack_block_{block_id}",
                    rack_grid_positions=[
                        GridCoordinate(row, col)
                        for row in block_rows
                        for col in range(self.config.grid_width)
                        if not self._should_skip_grid_position(row, col)
                    ],
                    cross_aisle_grid_positions=[],  # Will be populated below
                    direction="horizontal"
                )
                self.rack_blocks.append(block_def)

                # Add cross-aisle after this block (except for the last block)
                cross_aisle_row = current_row + rack_block_config.rows_per_block
                if cross_aisle_row < self.config.grid_depth:
                    # Mark cross-aisle positions for visualization
                    for col in range(self.config.grid_width):
                        block_def.cross_aisle_grid_positions.append(
                            GridCoordinate(cross_aisle_row, col)
                        )
                        # Mark these positions as occupied so racks aren't placed here
                        self.occupied_grid_cells.add((cross_aisle_row, col))

            current_row += rack_block_config.rows_per_block + 1  # +1 for cross-aisle
            block_id += 1

    def _generate_coordinate_mappings(self):
        """Generate grid-to-world coordinate mappings"""
        # Skip highway and station areas when placing racks
        for row in range(self.config.grid_depth):
            for col in range(self.config.grid_width):
                grid_coord = GridCoordinate(row, col)

                # Check if this position should be skipped (highways, stations, etc.)
                if self._should_skip_grid_position(row, col):
                    continue

                # Calculate world position
                x = -(self.config.grid_width - 1) * self.config.rack_spacing / 2 + \
                    col * self.config.rack_spacing
                y = row * self.config.rack_spacing

                world_coord = WorldCoordinate(x, y)
                self.grid_to_world_map[grid_coord] = world_coord
                self.rack_positions[(row, col)] = world_coord

    def _should_skip_grid_position(self, row: int, col: int) -> bool:
        """Check if a grid position should be skipped for rack placement"""
        # Check for highway intersections
        # Check for station areas
        # Check for cross-aisles
        # Implementation based on configuration
        return False

    # Coordinate conversion utilities (grid to world only for rendering/distance calculations)
    def grid_to_world(self, grid_coord: GridCoordinate) -> WorldCoordinate:
        """Convert grid coordinate to world coordinate for rendering"""
        cell_size = self.config.cell_size_meters
        # Center the grid around origin
        center_col = (self.config.grid_width - 1) / 2
        center_row = (self.config.grid_depth - 1) / 2

        x = (grid_coord.col - center_col) * cell_size
        y = (grid_coord.row - center_row) * cell_size
        return WorldCoordinate(x, y)

    def world_to_grid(self, world_coord: WorldCoordinate) -> GridCoordinate:
        """Convert world coordinate to grid coordinate (for external integration only)"""
        cell_size = self.config.cell_size_meters
        center_col = (self.config.grid_width - 1) / 2
        center_row = (self.config.grid_depth - 1) / 2

        col = round(world_coord.x / cell_size + center_col)
        row = round(world_coord.y / cell_size + center_row)
        return GridCoordinate(row, col)

    # Public API methods (return world coordinates for external compatibility)
    def get_station_world_positions(self) -> List[Tuple[float, float]]:
        """Get all station positions in world coordinates for external integration"""
        world_positions = []
        for station in self.stations:
            world_coord = self.grid_to_world(station.grid_position)
            world_positions.append((world_coord.x, world_coord.y))
        return world_positions

    def get_station_grid_positions(self) -> List[StationGridPosition]:
        """Get all station positions in grid coordinates (preferred for internal use)"""
        return self.stations

    def get_rack_world_positions_with_ids(self) -> List[Tuple[int, Tuple[float, float]]]:
        """Get rack positions with IDs in world coordinates for external integration"""
        positions = []
        rack_id = 1
        for (row, col), grid_coord in self.rack_grid_positions.items():
            world_coord = self.grid_to_world(grid_coord)
            positions.append((rack_id, (world_coord.x, world_coord.y)))
            rack_id += 1
        return positions

    def get_rack_grid_positions_with_ids(self) -> List[Tuple[int, GridCoordinate]]:
        """Get rack positions with IDs in grid coordinates (preferred for internal use)"""
        positions = []
        rack_id = 1
        for grid_coord in self.rack_grid_positions.values():
            positions.append((rack_id, grid_coord))
            rack_id += 1
        return positions

    def get_nearest_station_grid_position(self, grid_pos: GridCoordinate) -> GridCoordinate:
        """Find nearest station for any given grid position"""
        min_distance = float('inf')
        nearest_station = self.stations[0]

        for station in self.stations:
            distance = abs(grid_pos.row - station.grid_position.row) + \
                      abs(grid_pos.col - station.grid_position.col)  # Manhattan distance
            if distance < min_distance:
                min_distance = distance
                nearest_station = station

        return nearest_station.grid_position

    def get_highway_data_for_frontend(self) -> List[Dict]:
        """Get highway data formatted for frontend visualization"""
        highway_data = []
        for highway in self.highways:
            start_world = self.grid_to_world(highway.start_grid)
            end_world = self.grid_to_world(highway.end_grid)
            width_world = highway.width_grid_units * self.config.cell_size_meters

            highway_data.append({
                "id": highway.highway_id,
                "start": {"x": start_world.x, "y": start_world.y},
                "end": {"x": end_world.x, "y": end_world.y},
                "width": width_world,
                "type": highway.highway_type
            })
        return highway_data
```

### 3. Grid-to-World Conversion Layer

Clean separation between grid coordinates (domain logic) and world coordinates (rendering/calculations).

```python
class GridToWorldConverter:
    """Handles conversion between grid and world coordinates"""

    def __init__(self, cell_size_meters: float, grid_width: int, grid_depth: int):
        self.cell_size = cell_size_meters
        self.grid_width = grid_width
        self.grid_depth = grid_depth
        self.center_col = (grid_width - 1) / 2
        self.center_row = (grid_depth - 1) / 2

    def grid_to_world(self, grid_coord: GridCoordinate) -> WorldCoordinate:
        """Convert grid coordinate to world coordinate"""
        x = (grid_coord.col - self.center_col) * self.cell_size
        y = (grid_coord.row - self.center_row) * self.cell_size
        return WorldCoordinate(x, y)

    def world_to_grid(self, world_coord: WorldCoordinate) -> GridCoordinate:
        """Convert world coordinate to grid coordinate"""
        col = round(world_coord.x / self.cell_size + self.center_col)
        row = round(world_coord.y / self.cell_size + self.center_row)
        return GridCoordinate(row, col)

    def grid_distance_to_world_distance(self, grid_distance: int) -> float:
        """Convert grid distance to world distance"""
        return grid_distance * self.cell_size

    def world_distance_to_grid_distance(self, world_distance: float) -> int:
        """Convert world distance to grid distance"""
        return round(world_distance / self.cell_size)
```

## API Integration Design

### Enhanced Configuration Schemas

```python
# In api/schemas/simulation.py
class WarehouseLayoutConfig(BaseModel):
    """Enhanced warehouse layout configuration"""

    # Backward compatible fields
    map_width: int = Field(default=6, ge=1, le=200)
    map_length: int = Field(default=10, ge=1, le=200)
    aisle_spacing: float = Field(default=20.0, ge=5.0, le=50.0)

    # New map abstraction fields (optional for backward compatibility)
    use_advanced_layout: bool = Field(default=False, description="Enable advanced map features")
    warehouse_map_config: Optional[WarehouseMapConfig] = None

    def to_warehouse_map_config(self) -> WarehouseMapConfig:
        """Convert to WarehouseMapConfig"""
        if self.use_advanced_layout and self.warehouse_map_config:
            return self.warehouse_map_config

        # Convert legacy format to new format
        return WarehouseMapConfig(
            grid_width=self.map_width,
            grid_depth=self.map_length,
            rack_spacing=2.0,  # Default
            station_layout_type=StationLayoutType.SINGLE_BOTTOM,
            total_stations=1,  # Will be overridden by num_stations
            start_y_offset=self.aisle_spacing,
            x_pathway_frequency=2,  # Legacy default
            y_pathway_frequency=10  # Legacy default
        )
```

### New API Endpoints

```python
# In api/routes/warehouse.py
@router.post("/warehouse/layouts", response_model=WarehouseLayoutResponse)
async def create_warehouse_layout(layout_config: WarehouseMapConfig):
    """Create and validate a warehouse layout configuration"""
    try:
        map_abstraction = WarehouseMapAbstraction(layout_config)

        return WarehouseLayoutResponse(
            success=True,
            layout_id=str(uuid.uuid4()),
            stations=map_abstraction.get_station_positions(),
            highways=map_abstraction.get_highway_data_for_frontend(),
            rack_positions=map_abstraction.get_rack_positions_with_ids(),
            world_bounds=map_abstraction.world_bounds
        )
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e))

@router.get("/warehouse/layouts/presets")
async def get_layout_presets():
    """Get predefined warehouse layout templates"""
    presets = [
        {
            "id": "single_bottom_small",
            "name": "Single Bottom Station (Small)",
            "description": "Traditional single station at bottom edge",
            "config": WarehouseMapConfig(
                grid_width=6,
                grid_depth=10,
                station_layout_type=StationLayoutType.SINGLE_BOTTOM,
                total_stations=1
            ).dict()
        },
        {
            "id": "opposite_vertical_medium",
            "name": "Top-Bottom Stations (Medium)",
            "description": "Stations on both top and bottom edges",
            "config": WarehouseMapConfig(
                grid_width=12,
                grid_depth=20,
                station_layout_type=StationLayoutType.OPPOSITE_VERTICAL,
                total_stations=4,
                highway_config=HighwayConfig(enable_station_highways=True)
            ).dict()
        }
        # Add more presets...
    ]

    return {"presets": presets}
```

## Frontend Integration Design

### Enhanced WarehouseMap Component

```typescript
// Enhanced types for new map features
interface HighwayData {
  id: string;
  start: Position;
  end: Position;
  width: number;
  type: "station_highway";
}

interface EnhancedWarehouseMapProps extends WarehouseMapProps {
  // New props for advanced features
  highways?: HighwayData[];
  showHighways?: boolean;
  stationEdges?: Array<{ edge: string; stations: Station[] }>;
  aisleSystem?: {
    aisles: Array<{ id: string; racks: Rack[] }>;
    crossAisles: Position[];
  };
}

// Updated WarehouseMap component
export const WarehouseMap: React.FC<EnhancedWarehouseMapProps> = ({
  // ... existing props
  highways = [],
  showHighways = true,
  stationEdges = [],
  aisleSystem,
  ...props
}) => {
  // ... existing logic

  // Highway rendering
  const highwayElements = useMemo(() => {
    if (!showHighways) return [];

    return highways.map((highway) => (
      <Line
        key={highway.id}
        points={[
          transformX(highway.start.x),
          transformY(highway.start.y),
          transformX(highway.end.x),
          transformY(highway.end.y),
        ]}
        stroke={highway.type === "station_highway" ? "#3b82f6" : "#6b7280"}
        strokeWidth={highway.width * effectiveGridSize}
        opacity={0.6}
      />
    ));
  }, [highways, showHighways, transformX, transformY, effectiveGridSize]);

  // Enhanced station rendering with edge information
  const stationElements = useMemo(() => {
    return stations.map((station) => {
      // Find which edge this station belongs to
      const stationEdge = stationEdges.find((edge) =>
        edge.stations.some((s) => s.id === station.id)
      );

      return (
        <Group key={station.id}>
          <Rect
            x={transformX(station.position.x) - effectiveGridSize}
            y={transformY(station.position.y) - effectiveGridSize}
            width={effectiveGridSize * 2}
            height={effectiveGridSize * 2}
            fill={COLORS.station}
            stroke={COLORS.border}
            strokeWidth={2}
            cornerRadius={6}
            onClick={() => onStationClick?.(station)}
          />
          {/* Edge indicator */}
          {stationEdge && (
            <Text
              x={transformX(station.position.x)}
              y={transformY(station.position.y) + effectiveGridSize + 5}
              text={stationEdge.edge}
              fontSize={8}
              fill={COLORS.text}
              align="center"
            />
          )}
        </Group>
      );
    });
  }, [stations, stationEdges, transformX, transformY, effectiveGridSize]);

  return (
    <div className="w-full overflow-auto border rounded-lg shadow-sm bg-white">
      <Stage width={canvasWidth} height={canvasHeight}>
        <Layer>
          {/* ... existing elements */}

          {/* Highways */}
          {highwayElements}

          {/* Enhanced stations */}
          {stationElements}

          {/* ... rest of elements */}
        </Layer>
      </Stage>
    </div>
  );
};
```

## Implementation Strategy

### Phase 1: Core Grid System (Week 1)

1. Implement GridPosition and WorldPosition types
2. Create WarehouseMapConfig with grid-based measurements
3. Implement GridToWorldConverter utility
4. Create core WarehouseMapAbstraction engine

### Phase 2: Station and Highway System (Week 2)

1. Implement multi-edge station positioning in grid coordinates
2. Add highway system with grid-based measurements
3. Create station and highway generation algorithms
4. Add comprehensive validation and error handling

### Phase 3: Integration and API (Week 3)

1. Update domain models to use grid coordinates
2. Integrate with QueueSimulator using grid coordinates
3. Create new API endpoints with grid/world coordinate support
4. Update existing APIs to use new coordinate system

### Phase 4: Frontend and Advanced Features (Week 4)

1. Enhance WarehouseMap component with coordinate conversion
2. Add configuration UI for advanced layouts
3. Implement sophisticated aisle system
4. Performance optimization and documentation

## Testing Strategy

### Unit Tests

- WarehouseMapConfig validation
- Coordinate transformation accuracy
- Station positioning algorithms
- Highway generation logic
- Backward compatibility adapters

### Integration Tests

- Full simulation runs with new layouts
- API endpoint functionality
- Frontend rendering accuracy
- Performance benchmarks

### Regression Tests

- All existing simulation configurations
- API contract compliance
- Frontend compatibility
- Performance baselines

## Performance Considerations

### Optimization Strategies

1. **Lazy Loading**: Generate coordinate mappings on-demand
2. **Caching**: Cache coordinate transformations
3. **Spatial Indexing**: Use spatial data structures for fast lookups
4. **Memory Management**: Efficient data structures for large warehouses

### Performance Targets

- Configuration parsing: < 10ms
- Coordinate generation: < 100ms for 10,000 racks
- Memory usage: < 50MB additional for largest warehouses
- Frontend rendering: < 500ms for complex layouts

This design provides a comprehensive foundation for implementing the map abstraction feature while maintaining full backward compatibility and enabling future extensibility.
