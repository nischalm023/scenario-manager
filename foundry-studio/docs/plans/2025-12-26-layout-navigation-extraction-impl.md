# Layout and Navigation Service Extraction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Extract map generation and pathfinding into separate microservices (layout-service, navigation-service) from fleet-simulator.

**Architecture:** Fleet-simulator orchestrates calls to Layout Service (map generation) and Navigation Service (A* pathfinding with session cache). Services communicate via gRPC. Move code first without changes, then add gRPC layer on top.

**Tech Stack:** Java 21, Spring Boot 3.2, gRPC (grpc-spring-boot-starter), Protocol Buffers

**Design Document:** `docs/plans/2025-12-26-layout-navigation-service-extraction-design.md`

---

## Phase 1: Shared Types in common-domain

### Task 1.1: Add Orientation enum to common-domain

**Files:**
- Create: `libs/java/common-domain/src/main/java/com/greyorange/greymatter/common/navigation/Orientation.java`
- Test: `libs/java/common-domain/src/test/java/com/greyorange/greymatter/common/navigation/OrientationTest.java`

**Step 1: Create the Orientation enum (copy from fleet-simulator, change package)**

```java
package com.greyorange.greymatter.common.navigation;

/**
 * Cardinal direction a robot is facing. Provides methods for rotation and calculating movement
 * deltas.
 */
public enum Orientation {
  NORTH(-1, 0),
  SOUTH(1, 0),
  EAST(0, 1),
  WEST(0, -1);

  private final int rowDelta;
  private final int colDelta;

  Orientation(int rowDelta, int colDelta) {
    this.rowDelta = rowDelta;
    this.colDelta = colDelta;
  }

  /**
   * Get the row delta for moving forward in this direction.
   *
   * @return Row offset for forward movement
   */
  public int deltaRow() {
    return rowDelta;
  }

  /**
   * Get the column delta for moving forward in this direction.
   *
   * @return Column offset for forward movement
   */
  public int deltaCol() {
    return colDelta;
  }

  /**
   * Rotate 90 degrees clockwise.
   *
   * @return New orientation after clockwise rotation
   */
  public Orientation rotateCW() {
    return switch (this) {
      case NORTH -> EAST;
      case EAST -> SOUTH;
      case SOUTH -> WEST;
      case WEST -> NORTH;
    };
  }

  /**
   * Rotate 90 degrees counter-clockwise.
   *
   * @return New orientation after counter-clockwise rotation
   */
  public Orientation rotateCCW() {
    return switch (this) {
      case NORTH -> WEST;
      case WEST -> SOUTH;
      case SOUTH -> EAST;
      case EAST -> NORTH;
    };
  }

  /**
   * Reverse orientation 180 degrees.
   *
   * @return Opposite orientation
   */
  public Orientation reverse() {
    return switch (this) {
      case NORTH -> SOUTH;
      case SOUTH -> NORTH;
      case EAST -> WEST;
      case WEST -> EAST;
    };
  }
}
```

**Step 2: Create test for Orientation**

```java
package com.greyorange.greymatter.common.navigation;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class OrientationTest {

  @Test
  void deltaRow_north_returnsMinusOne() {
    assertEquals(-1, Orientation.NORTH.deltaRow());
  }

  @Test
  void deltaCol_east_returnsOne() {
    assertEquals(1, Orientation.EAST.deltaCol());
  }

  @Test
  void rotateCW_north_returnsEast() {
    assertEquals(Orientation.EAST, Orientation.NORTH.rotateCW());
  }

  @Test
  void rotateCCW_north_returnsWest() {
    assertEquals(Orientation.WEST, Orientation.NORTH.rotateCCW());
  }

  @Test
  void reverse_north_returnsSouth() {
    assertEquals(Orientation.SOUTH, Orientation.NORTH.reverse());
  }

  @Test
  void fullRotationCW_returnsToOriginal() {
    Orientation o = Orientation.NORTH;
    o = o.rotateCW().rotateCW().rotateCW().rotateCW();
    assertEquals(Orientation.NORTH, o);
  }
}
```

**Step 3: Run test to verify it passes**

```bash
cd libs/java/common-domain && mvn test -Dtest=OrientationTest -q
```

Expected: All tests PASS

**Step 4: Commit**

```bash
git add libs/java/common-domain/src/main/java/com/greyorange/greymatter/common/navigation/
git add libs/java/common-domain/src/test/java/com/greyorange/greymatter/common/navigation/
git commit -m "feat(common-domain): add Orientation enum for robot navigation"
```

---

### Task 1.2: Add RobotType enum to common-domain

**Files:**
- Create: `libs/java/common-domain/src/main/java/com/greyorange/greymatter/common/types/RobotType.java`
- Test: `libs/java/common-domain/src/test/java/com/greyorange/greymatter/common/types/RobotTypeTest.java`

**Step 1: Create the RobotType enum (extract from Robot.java)**

```java
package com.greyorange.greymatter.common.types;

import com.fasterxml.jackson.annotation.JsonValue;

/**
 * Type of robot in the warehouse system.
 */
public enum RobotType {
  /** Racks-to-Picker system - transports entire racks to picking stations. */
  RTP("rtp"),

  /** Vertical Tote Mover - retrieves totes from racks and transports to relay positions. */
  VTM("vtm"),

  /** Horizontal Tote Mover - transports totes between relay positions and stations. */
  HTM("htm");

  private final String value;

  RobotType(String value) {
    this.value = value;
  }

  @JsonValue
  public String getValue() {
    return value;
  }

  /**
   * Parse robot type from string value.
   *
   * @param value String representation (rtp, vtm, htm)
   * @return Matching RobotType
   * @throws IllegalArgumentException if value doesn't match any type
   */
  public static RobotType fromValue(String value) {
    for (RobotType type : values()) {
      if (type.value.equalsIgnoreCase(value)) {
        return type;
      }
    }
    throw new IllegalArgumentException("Unknown robot type: " + value);
  }
}
```

**Step 2: Create test for RobotType**

```java
package com.greyorange.greymatter.common.types;

import static org.junit.jupiter.api.Assertions.*;
import org.junit.jupiter.api.Test;

class RobotTypeTest {

  @Test
  void getValue_rtp_returnsRtp() {
    assertEquals("rtp", RobotType.RTP.getValue());
  }

  @Test
  void fromValue_vtm_returnsVtm() {
    assertEquals(RobotType.VTM, RobotType.fromValue("vtm"));
  }

  @Test
  void fromValue_caseInsensitive() {
    assertEquals(RobotType.HTM, RobotType.fromValue("HTM"));
  }

  @Test
  void fromValue_invalid_throws() {
    assertThrows(IllegalArgumentException.class, () -> RobotType.fromValue("invalid"));
  }
}
```

**Step 3: Run test to verify it passes**

```bash
cd libs/java/common-domain && mvn test -Dtest=RobotTypeTest -q
```

Expected: All tests PASS

**Step 4: Commit**

```bash
git add libs/java/common-domain/src/main/java/com/greyorange/greymatter/common/types/RobotType.java
git add libs/java/common-domain/src/test/java/com/greyorange/greymatter/common/types/
git commit -m "feat(common-domain): add RobotType enum for robot classification"
```

---

### Task 1.3: Update fleet-simulator to use common-domain types

**Files:**
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/Robot.java`
- Modify: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java`
- Delete: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/Orientation.java`

**Step 1: Update Robot.java to use common-domain RobotType**

Replace the nested `RobotType` enum with import:

```java
// At top of Robot.java, add import:
import com.greyorange.greymatter.common.types.RobotType;

// Remove the nested enum definition (public enum RobotType { ... })
// Keep getRobotType() method but it now returns the imported type
```

**Step 2: Update PathFinder.java to use common-domain Orientation**

```java
// At top of PathFinder.java, change:
// OLD: import com.greyorange.dessimulator.domain.navigation.Orientation;
// NEW:
import com.greyorange.greymatter.common.navigation.Orientation;
```

**Step 3: Update all files importing the old Orientation**

Find and replace in these files:
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathState.java`
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathWaypoint.java`
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/robot/Robot.java`
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojVtmProcess.java`
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojHtmProcess.java`
- `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/simulator/orchestrator/DesmojRtpRobotProcess.java`

```java
// Change all occurrences of:
import com.greyorange.dessimulator.domain.navigation.Orientation;
// To:
import com.greyorange.greymatter.common.navigation.Orientation;
```

**Step 4: Delete the old Orientation.java**

```bash
rm apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/Orientation.java
```

**Step 5: Verify compilation**

```bash
cd apps/fleet-simulator && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 6: Run tests to verify no regressions**

```bash
cd apps/fleet-simulator && mvn test -q
```

Expected: All tests PASS

**Step 7: Commit**

```bash
git add -A
git commit -m "refactor(fleet-simulator): use common-domain Orientation and RobotType"
```

---

## Phase 2: Proto Definitions

### Task 2.1: Create layout.proto

**Files:**
- Create: `libs/proto/src/grpc/layout/v1/layout.proto`

**Step 1: Create the proto file**

```protobuf
syntax = "proto3";
package greyorange.grpc.layout.v1;

option java_package = "com.greyorange.grpc.layout.v1";
option java_outer_classname = "LayoutProtos";
option java_multiple_files = true;

import "grpc/common/v1/types.proto";

// Layout service for warehouse map generation
service LayoutService {
  // Generate full layout for simulation setup
  rpc GetLayout(LayoutRequest) returns (LayoutResponse);

  // Get minimal navigation data (coordinate sets only)
  rpc GetNavigationData(LayoutRequest) returns (NavigationDataResponse);
}

// Grid bounds
message GridBounds {
  int32 width = 1;
  int32 depth = 2;
  double cell_size_meters = 3;
}

// Station edge enum
enum StationEdge {
  STATION_EDGE_UNSPECIFIED = 0;
  STATION_EDGE_BOTTOM = 1;
  STATION_EDGE_TOP = 2;
  STATION_EDGE_LEFT = 3;
  STATION_EDGE_RIGHT = 4;
}

// Station edge configuration
message StationEdgeConfig {
  StationEdge edge = 1;
  int32 station_count = 2;
  int32 spacing_grid_units = 3;
}

// Station layout configuration
message StationLayoutConfig {
  repeated StationEdgeConfig station_edges = 1;
}

// Highway configuration
message HighwayConfig {
  int32 highways_per_edge = 1;
}

// Rack block configuration
message RackBlockConfig {
  int32 rows_per_block = 1;
  int32 rack_columns = 2;
  int32 cross_aisle_width_grid_units = 3;
}

// Request for layout generation
message LayoutRequest {
  int32 grid_width = 1;
  int32 grid_depth = 2;
  double cell_size_meters = 3;
  greyorange.grpc.common.v1.SystemType system_type = 4;
  StationLayoutConfig station_layout = 5;
  HighwayConfig highway_config = 6;
  RackBlockConfig rack_block_config = 7;
}

// Station position in the layout
message StationPosition {
  string station_id = 1;
  greyorange.grpc.common.v1.GridCoordinate position = 2;
  StationEdge edge = 3;
  greyorange.grpc.common.v1.GridCoordinate accessibility_direction = 4;
}

// Highway segment
message HighwaySegment {
  string segment_id = 1;
  greyorange.grpc.common.v1.GridCoordinate start = 2;
  greyorange.grpc.common.v1.GridCoordinate end = 3;
  int32 width = 4;
}

// Rack block definition
message RackBlockDefinition {
  string block_id = 1;
  greyorange.grpc.common.v1.GridCoordinate top_left = 2;
  greyorange.grpc.common.v1.GridCoordinate bottom_right = 3;
  int32 rows_per_block = 4;
  repeated greyorange.grpc.common.v1.GridCoordinate rack_positions = 5;
  repeated greyorange.grpc.common.v1.GridCoordinate aisle_positions = 6;
}

// Relay position data (for RELAY system only)
message RelayPositionData {
  string id = 1;
  string rack_block_id = 2;
  string side = 3;
  greyorange.grpc.common.v1.GridCoordinate position = 4;
  greyorange.grpc.common.v1.GridCoordinate htm_access_position = 5;
  greyorange.grpc.common.v1.GridCoordinate vtm_access_position = 6;
}

// Rack position data (for RELAY system only)
message RackPositionData {
  greyorange.grpc.common.v1.GridCoordinate position = 1;
  greyorange.grpc.common.v1.GridCoordinate vtm_access_position = 2;
  string rack_block_id = 3;
  string side = 4;
}

// Full layout response
message LayoutResponse {
  GridBounds bounds = 1;
  repeated StationPosition stations = 2;
  repeated HighwaySegment highways = 3;
  repeated RackBlockDefinition rack_blocks = 4;
  repeated RelayPositionData relay_positions = 5;
  map<string, RackPositionData> rack_position_data = 6;
}

// Minimal navigation data response
message NavigationDataResponse {
  GridBounds bounds = 1;
  repeated greyorange.grpc.common.v1.GridCoordinate blocked_positions = 2;
  repeated greyorange.grpc.common.v1.GridCoordinate rack_positions = 3;
  repeated greyorange.grpc.common.v1.GridCoordinate highway_positions = 4;
  repeated greyorange.grpc.common.v1.GridCoordinate relay_positions = 5;
}
```

**Step 2: Verify proto compiles**

```bash
cd libs/java/grpc-stubs && mvn clean compile -q
```

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add libs/proto/src/grpc/layout/
git commit -m "feat(proto): add layout.proto for layout service API"
```

---

### Task 2.2: Create navigation.proto

**Files:**
- Create: `libs/proto/src/grpc/navigation/v1/navigation.proto`

**Step 1: Create the proto file**

```protobuf
syntax = "proto3";
package greyorange.grpc.navigation.v1;

option java_package = "com.greyorange.grpc.navigation.v1";
option java_outer_classname = "NavigationProtos";
option java_multiple_files = true;

import "grpc/common/v1/types.proto";

// Navigation service for A* pathfinding
service NavigationService {
  // Create a navigation session with map data
  rpc CreateSession(CreateSessionRequest) returns (CreateSessionResponse);

  // Find optimal path from start to goal
  rpc FindPath(FindPathRequest) returns (FindPathResponse);

  // Delete a navigation session
  rpc DeleteSession(DeleteSessionRequest) returns (DeleteSessionResponse);
}

// Robot orientation
enum Orientation {
  ORIENTATION_UNSPECIFIED = 0;
  ORIENTATION_NORTH = 1;
  ORIENTATION_SOUTH = 2;
  ORIENTATION_EAST = 3;
  ORIENTATION_WEST = 4;
}

// Robot type
enum RobotType {
  ROBOT_TYPE_UNSPECIFIED = 0;
  ROBOT_TYPE_RTP = 1;
  ROBOT_TYPE_VTM = 2;
  ROBOT_TYPE_HTM = 3;
}

// Navigation action
enum NavigationAction {
  NAVIGATION_ACTION_UNSPECIFIED = 0;
  NAVIGATION_ACTION_MOVE_FORWARD = 1;
  NAVIGATION_ACTION_MOVE_BACKWARD = 2;
  NAVIGATION_ACTION_TURN_90_CW = 3;
  NAVIGATION_ACTION_TURN_90_CCW = 4;
  NAVIGATION_ACTION_TURN_180 = 5;
}

// Grid bounds for session
message GridBounds {
  int32 width = 1;
  int32 depth = 2;
}

// Create session request
message CreateSessionRequest {
  GridBounds bounds = 1;
  repeated greyorange.grpc.common.v1.GridCoordinate blocked_positions = 2;
  repeated greyorange.grpc.common.v1.GridCoordinate rack_positions = 3;
  repeated greyorange.grpc.common.v1.GridCoordinate highway_positions = 4;
  repeated greyorange.grpc.common.v1.GridCoordinate relay_positions = 5;
  int32 ttl_seconds = 6;  // Optional, default 3600
}

// Create session response
message CreateSessionResponse {
  string session_id = 1;
}

// Find path request
message FindPathRequest {
  string session_id = 1;
  greyorange.grpc.common.v1.GridCoordinate start = 2;
  greyorange.grpc.common.v1.GridCoordinate goal = 3;
  Orientation start_orientation = 4;
  RobotType robot_type = 5;
  bool is_carrying_cargo = 6;
  double robot_speed = 7;
  double base_turn_time_90 = 8;
}

// Path waypoint
message PathWaypoint {
  greyorange.grpc.common.v1.GridCoordinate position = 1;
  Orientation orientation = 2;
  NavigationAction action = 3;
  double time_delta = 4;
  double cumulative_time = 5;
}

// Find path response
message FindPathResponse {
  repeated PathWaypoint waypoints = 1;
  double total_time = 2;
  double total_distance = 3;
  int32 total_turns = 4;
  bool path_found = 5;
}

// Delete session request
message DeleteSessionRequest {
  string session_id = 1;
}

// Delete session response
message DeleteSessionResponse {
  bool success = 1;
}
```

**Step 2: Verify proto compiles**

```bash
cd libs/java/grpc-stubs && mvn clean compile -q
```

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add libs/proto/src/grpc/navigation/
git commit -m "feat(proto): add navigation.proto for navigation service API"
```

---

## Phase 3: Scaffold Layout Service

### Task 3.1: Create layout-service project structure

**Files:**
- Create: `apps/layout-service/pom.xml`
- Create: `apps/layout-service/src/main/java/com/greyorange/layout/LayoutServiceApplication.java`
- Create: `apps/layout-service/src/main/resources/application.yml`

**Step 1: Create pom.xml**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <parent>
        <groupId>com.greyorange.greymatter</groupId>
        <artifactId>java-parent</artifactId>
        <version>1.0.0-SNAPSHOT</version>
        <relativePath>../../libs/java/pom.xml</relativePath>
    </parent>

    <artifactId>layout-service</artifactId>
    <packaging>jar</packaging>

    <name>Layout Service</name>
    <description>Warehouse map generation service</description>

    <dependencies>
        <!-- Internal dependencies -->
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>common-domain</artifactId>
        </dependency>
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>common-messaging</artifactId>
        </dependency>
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>grpc-stubs</artifactId>
        </dependency>

        <!-- gRPC -->
        <dependency>
            <groupId>net.devh</groupId>
            <artifactId>grpc-server-spring-boot-starter</artifactId>
        </dependency>
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-netty-shaded</artifactId>
        </dependency>

        <!-- Spring Boot Starters -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-web</artifactId>
        </dependency>
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-actuator</artifactId>
        </dependency>

        <!-- Lombok -->
        <dependency>
            <groupId>org.projectlombok</groupId>
            <artifactId>lombok</artifactId>
            <scope>provided</scope>
        </dependency>

        <!-- Testing -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-test</artifactId>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <plugins>
            <plugin>
                <groupId>org.springframework.boot</groupId>
                <artifactId>spring-boot-maven-plugin</artifactId>
                <configuration>
                    <classifier>exec</classifier>
                </configuration>
            </plugin>

            <plugin>
                <groupId>com.diffplug.spotless</groupId>
                <artifactId>spotless-maven-plugin</artifactId>
            </plugin>

            <!-- Jib for Docker -->
            <plugin>
                <groupId>com.google.cloud.tools</groupId>
                <artifactId>jib-maven-plugin</artifactId>
                <configuration>
                    <from>
                        <image>eclipse-temurin:21-jre-alpine</image>
                    </from>
                    <to>
                        <image>layout-service:latest</image>
                    </to>
                    <container>
                        <mainClass>com.greyorange.greymatter.layout.LayoutServiceApplication</mainClass>
                        <ports>
                            <port>8082</port>
                            <port>9091</port>
                        </ports>
                        <environment>
                            <GRPC_SERVER_PORT>9091</GRPC_SERVER_PORT>
                        </environment>
                    </container>
                </configuration>
            </plugin>
        </plugins>
    </build>
</project>
```

**Step 2: Create application.yml**

```yaml
server:
  port: 8082

grpc:
  server:
    port: 9091

spring:
  application:
    name: layout-service

management:
  endpoints:
    web:
      exposure:
        include: health,info
```

**Step 3: Create LayoutServiceApplication.java**

```java
package com.greyorange.layout;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;

@SpringBootApplication
public class LayoutServiceApplication {

  public static void main(String[] args) {
    SpringApplication.run(LayoutServiceApplication.class, args);
  }
}
```

**Step 4: Verify it compiles**

```bash
cd apps/layout-service && mvn clean compile -q
```

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/layout-service/
git commit -m "scaffold: create layout-service Spring Boot project"
```

---

## Phase 4: Move Map Code to Layout Service

### Task 4.1: Copy map domain files (as-is, no logic changes)

**Critical:** This step ONLY copies files. Do NOT change any logic.

**Files:**
- Copy: `apps/fleet-simulator/.../domain/map/*.java` → `apps/layout-service/.../domain/map/`

**Step 1: Create target directory and copy files**

```bash
mkdir -p apps/layout-service/src/main/java/com/greyorange/layout/domain/map

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/HighwayConfig.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/RackBlockConfig.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/StationEdge.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/StationEdgeConfig.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/StationLayoutConfig.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/StationLayoutType.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMap.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/map/WarehouseMapConfig.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/map/
```

**Step 2: Commit the raw copy (before package changes)**

```bash
git add apps/layout-service/src/main/java/com/greyorange/layout/domain/map/
git commit -m "copy: move map domain files to layout-service (no changes)"
```

---

### Task 4.2: Fix package declarations in layout-service

**Files:**
- Modify: All files in `apps/layout-service/src/main/java/com/greyorange/layout/domain/map/`

**Step 1: Update package declarations**

For each file, change the package declaration from:
```java
package com.greyorange.dessimulator.domain.map;
```
To:
```java
package com.greyorange.layout.domain.map;
```

**Step 2: Update imports for moved files**

Update internal imports to use new package:
```java
// OLD:
import com.greyorange.dessimulator.domain.map.StationEdge;
// NEW:
import com.greyorange.greymatter.map.domain.layout.StationEdge;
```

**Step 3: Copy required helper classes**

WarehouseMap.java depends on coordinate helpers. Copy them:

```bash
mkdir -p apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates

# Copy from fleet-simulator
cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/GridToWorldConverter.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/HighwayGridSegment.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/RackBlockGridDefinition.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/StationGridPosition.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/common/coordinates/WorldCoordinate.java \
   apps/layout-service/src/main/java/com/greyorange/layout/domain/common/coordinates/
```

**Step 4: Fix all package declarations in helper files**

Change from `com.greyorange.dessimulator.domain.common.coordinates` to `com.greyorange.layout.domain.common.coordinates`

**Step 5: Verify compilation**

```bash
cd apps/layout-service && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 6: Commit**

```bash
git add apps/layout-service/
git commit -m "fix: update package declarations in layout-service"
```

---

## Phase 5: Scaffold Navigation Service

### Task 5.1: Create navigation-service project structure

**Files:**
- Create: `apps/navigation-service/pom.xml`
- Create: `apps/navigation-service/src/main/java/com/greyorange/navigation/NavigationServiceApplication.java`
- Create: `apps/navigation-service/src/main/resources/application.yml`

**Step 1: Create pom.xml**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
    <modelVersion>4.0.0</modelVersion>

    <parent>
        <groupId>com.greyorange.greymatter</groupId>
        <artifactId>java-parent</artifactId>
        <version>1.0.0-SNAPSHOT</version>
        <relativePath>../../libs/java/pom.xml</relativePath>
    </parent>

    <artifactId>navigation-service</artifactId>
    <packaging>jar</packaging>

    <name>Navigation Service</name>
    <description>A* pathfinding service with session management</description>

    <dependencies>
        <!-- Internal dependencies -->
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>common-domain</artifactId>
        </dependency>
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>common-messaging</artifactId>
        </dependency>
        <dependency>
            <groupId>com.greyorange.greymatter</groupId>
            <artifactId>grpc-stubs</artifactId>
        </dependency>

        <!-- gRPC -->
        <dependency>
            <groupId>net.devh</groupId>
            <artifactId>grpc-server-spring-boot-starter</artifactId>
        </dependency>
        <dependency>
            <groupId>io.grpc</groupId>
            <artifactId>grpc-netty-shaded</artifactId>
        </dependency>

        <!-- Spring Boot Starters -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-web</artifactId>
        </dependency>
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-actuator</artifactId>
        </dependency>

        <!-- Caffeine for session caching -->
        <dependency>
            <groupId>com.github.ben-manes.caffeine</groupId>
            <artifactId>caffeine</artifactId>
        </dependency>

        <!-- Lombok -->
        <dependency>
            <groupId>org.projectlombok</groupId>
            <artifactId>lombok</artifactId>
            <scope>provided</scope>
        </dependency>

        <!-- Testing -->
        <dependency>
            <groupId>org.springframework.boot</groupId>
            <artifactId>spring-boot-starter-test</artifactId>
            <scope>test</scope>
        </dependency>
    </dependencies>

    <build>
        <plugins>
            <plugin>
                <groupId>org.springframework.boot</groupId>
                <artifactId>spring-boot-maven-plugin</artifactId>
                <configuration>
                    <classifier>exec</classifier>
                </configuration>
            </plugin>

            <plugin>
                <groupId>com.diffplug.spotless</groupId>
                <artifactId>spotless-maven-plugin</artifactId>
            </plugin>

            <!-- Jib for Docker -->
            <plugin>
                <groupId>com.google.cloud.tools</groupId>
                <artifactId>jib-maven-plugin</artifactId>
                <configuration>
                    <from>
                        <image>eclipse-temurin:21-jre-alpine</image>
                    </from>
                    <to>
                        <image>navigation-service:latest</image>
                    </to>
                    <container>
                        <mainClass>com.greyorange.navigation.NavigationServiceApplication</mainClass>
                        <ports>
                            <port>8083</port>
                            <port>9092</port>
                        </ports>
                        <environment>
                            <GRPC_SERVER_PORT>9092</GRPC_SERVER_PORT>
                        </environment>
                    </container>
                </configuration>
            </plugin>
        </plugins>
    </build>
</project>
```

**Step 2: Create application.yml**

```yaml
server:
  port: 8083

grpc:
  server:
    port: 9092

spring:
  application:
    name: navigation-service

navigation:
  session:
    default-ttl-seconds: 3600
    cleanup-interval-seconds: 300
    max-sessions: 100

management:
  endpoints:
    web:
      exposure:
        include: health,info
```

**Step 3: Create NavigationServiceApplication.java**

```java
package com.greyorange.navigation;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;

@SpringBootApplication
public class NavigationServiceApplication {

  public static void main(String[] args) {
    SpringApplication.run(NavigationServiceApplication.class, args);
  }
}
```

**Step 4: Verify it compiles**

```bash
cd apps/navigation-service && mvn clean compile -q
```

Expected: BUILD SUCCESS

**Step 5: Commit**

```bash
git add apps/navigation-service/
git commit -m "scaffold: create navigation-service Spring Boot project"
```

---

## Phase 6: Move Navigation Code to Navigation Service

### Task 6.1: Copy navigation domain files (as-is, no logic changes)

**Critical:** This step ONLY copies files. Do NOT change any logic.

**Files:**
- Copy: `apps/fleet-simulator/.../domain/navigation/*.java` → `apps/navigation-service/.../domain/`

**Step 1: Create target directory and copy files**

```bash
mkdir -p apps/navigation-service/src/main/java/com/greyorange/navigation/domain

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathFinder.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathResult.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathState.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/PathWaypoint.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/StateKey.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/

cp apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/domain/navigation/NavigationAction.java \
   apps/navigation-service/src/main/java/com/greyorange/navigation/domain/
```

**Step 2: Commit the raw copy (before package changes)**

```bash
git add apps/navigation-service/src/main/java/com/greyorange/navigation/domain/
git commit -m "copy: move navigation domain files to navigation-service (no changes)"
```

---

### Task 6.2: Fix package declarations and dependencies in navigation-service

**Files:**
- Modify: All files in `apps/navigation-service/src/main/java/com/greyorange/navigation/domain/`

**Step 1: Update package declarations**

For each file, change the package declaration from:
```java
package com.greyorange.dessimulator.domain.navigation;
```
To:
```java
package com.greyorange.navigation.domain;
```

**Step 2: Update imports to use common-domain Orientation**

In PathFinder.java, PathState.java, PathWaypoint.java:
```java
// Use common-domain Orientation
import com.greyorange.greymatter.common.navigation.Orientation;
```

**Step 3: Create a local MapData interface to replace WarehouseMap dependency**

PathFinder currently depends on WarehouseMap. Create an interface:

```java
// Create: apps/navigation-service/src/main/java/com/greyorange/navigation/domain/MapData.java
package com.greyorange.navigation.domain;

import com.greyorange.greymatter.common.coordinates.GridCoordinate;

/**
 * Interface for map data needed by PathFinder.
 * Decouples pathfinding from WarehouseMap implementation.
 */
public interface MapData {
  boolean isPositionAvailable(GridCoordinate position);
  boolean isRackPosition(GridCoordinate position);
  boolean isHighway(GridCoordinate position);
}
```

**Step 4: Update PathFinder to use MapData interface**

Change `WarehouseMap map` parameter to `MapData map` in `findPath()` method.

**Step 5: Create a RobotType adapter (or use common-domain)**

PathFinder uses Robot.RobotType. Update to use common-domain RobotType:
```java
import com.greyorange.greymatter.common.types.RobotType;
```

Remove the dependency on `Robot` class.

**Step 6: Verify compilation**

```bash
cd apps/navigation-service && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 7: Commit**

```bash
git add apps/navigation-service/
git commit -m "fix: update package declarations and decouple PathFinder from WarehouseMap"
```

---

## Phase 7: Add gRPC Layer to Layout Service

### Task 7.1: Create LayoutGrpcService

**Files:**
- Create: `apps/layout-service/src/main/java/com/greyorange/layout/LayoutGrpcService.java`

**Step 1: Create the gRPC service**

```java
package com.greyorange.layout;

import com.greyorange.grpc.common.v1.GridCoordinate;
import com.greyorange.grpc.layout.v1.*;
import com.greyorange.greymatter.common.types.SystemType;
import io.grpc.stub.StreamObserver;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;
import net.devh.boot.grpc.server.service.GrpcService;

@Slf4j
@GrpcService
@RequiredArgsConstructor
public class LayoutGrpcService extends LayoutServiceGrpc.LayoutServiceImplBase {

    @Override
    public void getLayout(LayoutRequest request, StreamObserver<LayoutResponse> responseObserver) {
        log.info("GetLayout request: {}x{} grid", request.getGridWidth(), request.getGridDepth());

        try {
            // Convert proto request to domain config
            WarehouseMapConfig config = convertToConfig(request);
            SystemType systemType = convertSystemType(request.getSystemType());

            // Call existing domain logic (unchanged)
            WarehouseMap map = new WarehouseMap(systemType, config);

            // Convert domain result to proto response
            LayoutResponse response = convertToProto(map, request);
            responseObserver.onNext(response);
            responseObserver.onCompleted();

        } catch (Exception e) {
            log.error("Error generating layout", e);
            responseObserver.onError(e);
        }
    }

    @Override
    public void getNavigationData(
            LayoutRequest request, StreamObserver<NavigationDataResponse> responseObserver) {
        log.info(
                "GetNavigationData request: {}x{} grid", request.getGridWidth(), request.getGridDepth());

        try {
            WarehouseMapConfig config = convertToConfig(request);
            SystemType systemType = convertSystemType(request.getSystemType());
            WarehouseMap map = new WarehouseMap(systemType, config);

            NavigationDataResponse response = convertToNavigationData(map, request);
            responseObserver.onNext(response);
            responseObserver.onCompleted();

        } catch (Exception e) {
            log.error("Error generating navigation data", e);
            responseObserver.onError(e);
        }
    }

    private WarehouseMapConfig convertToConfig(LayoutRequest request) {
        // Convert proto config to domain config
        // Implementation details...
        return new WarehouseMapConfig(
                request.getGridWidth(),
                request.getGridDepth(),
                request.getCellSizeMeters(),
                convertStationLayout(request.getStationLayout()),
                convertHighwayConfig(request.getHighwayConfig()),
                convertRackBlockConfig(request.getRackBlockConfig()));
    }

    private SystemType convertSystemType(
            com.greyorange.grpc.common.v1.SystemType protoType) {
        return switch (protoType) {
            case SYSTEM_TYPE_RTP -> SystemType.RTP;
            case SYSTEM_TYPE_RELAY -> SystemType.RELAY;
            default -> throw new IllegalArgumentException("Unknown system type: " + protoType);
        };
    }

    private StationLayoutConfig convertStationLayout(
            com.greyorange.grpc.layout.v1.StationLayoutConfig proto) {
        // Convert proto to domain
        // Implementation...
        return null; // TODO: implement
    }

    private HighwayConfig convertHighwayConfig(
            com.greyorange.grpc.layout.v1.HighwayConfig proto) {
        return new HighwayConfig(proto.getHighwaysPerEdge());
    }

    private RackBlockConfig convertRackBlockConfig(
            com.greyorange.grpc.layout.v1.RackBlockConfig proto) {
        return new RackBlockConfig(
                proto.getRowsPerBlock(),
                proto.getRackColumns(),
                proto.getCrossAisleWidthGridUnits());
    }

    private LayoutResponse convertToProto(WarehouseMap map, LayoutRequest request) {
        // Convert domain map to proto response
        // Implementation...
        return LayoutResponse.newBuilder()
                .setBounds(
                        com.greyorange.grpc.layout.v1.GridBounds.newBuilder()
                                .setWidth(request.getGridWidth())
                                .setDepth(request.getGridDepth())
                                .setCellSizeMeters(request.getCellSizeMeters())
                                .build())
                // Add stations, highways, rack blocks, etc.
                .build();
    }

    private NavigationDataResponse convertToNavigationData(WarehouseMap map, LayoutRequest request) {
        NavigationDataResponse.Builder builder =
                NavigationDataResponse.newBuilder()
                        .setBounds(
                                com.greyorange.grpc.layout.v1.GridBounds.newBuilder()
                                        .setWidth(request.getGridWidth())
                                        .setDepth(request.getGridDepth())
                                        .setCellSizeMeters(request.getCellSizeMeters())
                                        .build());

        // Add blocked positions
        for (var pos : map.getOccupiedPositions()) {
            builder.addBlockedPositions(
                    GridCoordinate.newBuilder().setRow(pos.row()).setCol(pos.col()).build());
        }

        // Add rack positions
        for (var pos : map.getRackPositions()) {
            builder.addRackPositions(
                    GridCoordinate.newBuilder().setRow(pos.row()).setCol(pos.col()).build());
        }

        // Add highway positions (from segments)
        for (var segment : map.getHighwaySegments()) {
            for (var pos : segment.getAllCoordinates()) {
                builder.addHighwayPositions(
                        GridCoordinate.newBuilder().setRow(pos.row()).setCol(pos.col()).build());
            }
        }

        // Add relay positions
        for (var pos : map.getRelayPositions()) {
            builder.addRelayPositions(
                    GridCoordinate.newBuilder().setRow(pos.row()).setCol(pos.col()).build());
        }

        return builder.build();
    }
}
```

**Step 2: Verify compilation**

```bash
cd apps/layout-service && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/layout-service/
git commit -m "feat(layout-service): add gRPC layer for layout generation"
```

---

## Phase 8: Add gRPC Layer to Navigation Service

### Task 8.1: Create session management

**Files:**
- Create: `apps/navigation-service/src/main/java/com/greyorange/navigation/session/NavigationSession.java`
- Create: `apps/navigation-service/src/main/java/com/greyorange/navigation/session/SessionManager.java`

**Step 1: Create NavigationSession record**

```java
package com.greyorange.navigation.session;

import com.greyorange.greymatter.common.coordinates.GridCoordinate;
import com.greyorange.navigation.domain.MapData;
import java.time.Instant;
import java.util.Set;

/**
 * Holds map data for a navigation session.
 */
public record NavigationSession(
    String sessionId,
    int gridWidth,
    int gridDepth,
    Set<GridCoordinate> blockedPositions,
    Set<GridCoordinate> rackPositions,
    Set<GridCoordinate> highwayPositions,
    Set<GridCoordinate> relayPositions,
    Instant createdAt,
    Instant expiresAt) implements MapData {

  @Override
  public boolean isPositionAvailable(GridCoordinate position) {
    return !blockedPositions.contains(position);
  }

  @Override
  public boolean isRackPosition(GridCoordinate position) {
    return rackPositions.contains(position);
  }

  @Override
  public boolean isHighway(GridCoordinate position) {
    return highwayPositions.contains(position);
  }
}
```

**Step 2: Create SessionManager**

```java
package com.greyorange.navigation.session;

import com.github.benmanes.caffeine.cache.Cache;
import com.github.benmanes.caffeine.cache.Caffeine;
import java.time.Duration;
import java.time.Instant;
import java.util.Optional;
import java.util.UUID;
import lombok.extern.slf4j.Slf4j;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.stereotype.Component;

@Slf4j
@Component
public class SessionManager {

  private final Cache<String, NavigationSession> sessions;
  private final int defaultTtlSeconds;

  public SessionManager(
      @Value("${navigation.session.default-ttl-seconds:3600}") int defaultTtlSeconds,
      @Value("${navigation.session.max-sessions:100}") int maxSessions) {
    this.defaultTtlSeconds = defaultTtlSeconds;
    this.sessions =
        Caffeine.newBuilder()
            .maximumSize(maxSessions)
            .expireAfterWrite(Duration.ofSeconds(defaultTtlSeconds))
            .removalListener(
                (key, value, cause) ->
                    log.info("Session {} removed: {}", key, cause))
            .build();
  }

  public String createSession(NavigationSession session) {
    String sessionId = UUID.randomUUID().toString();
    NavigationSession sessionWithId =
        new NavigationSession(
            sessionId,
            session.gridWidth(),
            session.gridDepth(),
            session.blockedPositions(),
            session.rackPositions(),
            session.highwayPositions(),
            session.relayPositions(),
            Instant.now(),
            Instant.now().plusSeconds(defaultTtlSeconds));
    sessions.put(sessionId, sessionWithId);
    log.info("Created session {}", sessionId);
    return sessionId;
  }

  public Optional<NavigationSession> getSession(String sessionId) {
    return Optional.ofNullable(sessions.getIfPresent(sessionId));
  }

  public boolean deleteSession(String sessionId) {
    NavigationSession removed = sessions.asMap().remove(sessionId);
    if (removed != null) {
      log.info("Deleted session {}", sessionId);
      return true;
    }
    return false;
  }
}
```

**Step 3: Commit**

```bash
git add apps/navigation-service/
git commit -m "feat(navigation-service): add session management with TTL"
```

---

### Task 8.2: Create NavigationGrpcService

**Files:**
- Create: `apps/navigation-service/src/main/java/com/greyorange/navigation/NavigationGrpcService.java`

**Step 1: Create the gRPC service**

```java
package com.greyorange.navigation;

import com.greyorange.grpc.common.v1.GridCoordinate;
import com.greyorange.grpc.navigation.v1.*;
import com.greyorange.greymatter.common.navigation.Orientation;
import com.greyorange.greymatter.common.types.RobotType;
import com.greyorange.navigation.domain.PathFinder;
import com.greyorange.navigation.domain.PathResult;
import com.greyorange.navigation.session.NavigationSession;
import com.greyorange.navigation.session.SessionManager;
import io.grpc.Status;
import io.grpc.stub.StreamObserver;
import java.time.Instant;
import java.util.HashSet;
import java.util.Set;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;
import net.devh.boot.grpc.server.service.GrpcService;

@Slf4j
@GrpcService
@RequiredArgsConstructor
public class NavigationGrpcService extends NavigationServiceGrpc.NavigationServiceImplBase {

  private final SessionManager sessionManager;
  private final PathFinder pathFinder = new PathFinder();

  @Override
  public void createSession(
      CreateSessionRequest request, StreamObserver<CreateSessionResponse> responseObserver) {
    log.info("CreateSession request: {}x{} grid", request.getBounds().getWidth(),
        request.getBounds().getDepth());

    try {
      Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> blocked =
          convertCoordinates(request.getBlockedPositionsList());
      Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> racks =
          convertCoordinates(request.getRackPositionsList());
      Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> highways =
          convertCoordinates(request.getHighwayPositionsList());
      Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> relays =
          convertCoordinates(request.getRelayPositionsList());

      NavigationSession session =
          new NavigationSession(
              null, // Will be set by SessionManager
              request.getBounds().getWidth(),
              request.getBounds().getDepth(),
              blocked,
              racks,
              highways,
              relays,
              Instant.now(),
              null);

      String sessionId = sessionManager.createSession(session);

      responseObserver.onNext(
          CreateSessionResponse.newBuilder().setSessionId(sessionId).build());
      responseObserver.onCompleted();

    } catch (Exception e) {
      log.error("Error creating session", e);
      responseObserver.onError(e);
    }
  }

  @Override
  public void findPath(
      FindPathRequest request, StreamObserver<FindPathResponse> responseObserver) {
    log.debug("FindPath request: session={}", request.getSessionId());

    try {
      var sessionOpt = sessionManager.getSession(request.getSessionId());
      if (sessionOpt.isEmpty()) {
        responseObserver.onError(
            Status.NOT_FOUND
                .withDescription("Session not found: " + request.getSessionId())
                .asRuntimeException());
        return;
      }

      NavigationSession session = sessionOpt.get();

      var start =
          new com.greyorange.greymatter.common.coordinates.GridCoordinate(
              request.getStart().getRow(), request.getStart().getCol());
      var goal =
          new com.greyorange.greymatter.common.coordinates.GridCoordinate(
              request.getGoal().getRow(), request.getGoal().getCol());

      Orientation startOrientation = convertOrientation(request.getStartOrientation());
      RobotType robotType = convertRobotType(request.getRobotType());

      // Call existing PathFinder (unchanged logic)
      PathResult result =
          pathFinder.findPath(
              start,
              startOrientation,
              goal,
              session,
              robotType,
              request.getIsCarryingCargo(),
              request.getRobotSpeed(),
              request.getBaseTurnTime90());

      // Convert to proto response
      FindPathResponse response = convertToProto(result);
      responseObserver.onNext(response);
      responseObserver.onCompleted();

    } catch (Exception e) {
      log.error("Error finding path", e);
      responseObserver.onError(e);
    }
  }

  @Override
  public void deleteSession(
      DeleteSessionRequest request, StreamObserver<DeleteSessionResponse> responseObserver) {
    log.info("DeleteSession request: {}", request.getSessionId());

    boolean success = sessionManager.deleteSession(request.getSessionId());
    responseObserver.onNext(DeleteSessionResponse.newBuilder().setSuccess(success).build());
    responseObserver.onCompleted();
  }

  private Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> convertCoordinates(
      java.util.List<GridCoordinate> protos) {
    Set<com.greyorange.greymatter.common.coordinates.GridCoordinate> result = new HashSet<>();
    for (GridCoordinate proto : protos) {
      result.add(
          new com.greyorange.greymatter.common.coordinates.GridCoordinate(
              proto.getRow(), proto.getCol()));
    }
    return result;
  }

  private Orientation convertOrientation(
      com.greyorange.grpc.navigation.v1.Orientation proto) {
    return switch (proto) {
      case ORIENTATION_NORTH -> Orientation.NORTH;
      case ORIENTATION_SOUTH -> Orientation.SOUTH;
      case ORIENTATION_EAST -> Orientation.EAST;
      case ORIENTATION_WEST -> Orientation.WEST;
      default -> Orientation.NORTH;
    };
  }

  private RobotType convertRobotType(com.greyorange.grpc.navigation.v1.RobotType proto) {
    return switch (proto) {
      case ROBOT_TYPE_RTP -> RobotType.RTP;
      case ROBOT_TYPE_VTM -> RobotType.VTM;
      case ROBOT_TYPE_HTM -> RobotType.HTM;
      default -> RobotType.RTP;
    };
  }

  private FindPathResponse convertToProto(PathResult result) {
    FindPathResponse.Builder builder =
        FindPathResponse.newBuilder()
            .setTotalTime(result.totalTime())
            .setTotalDistance(result.totalDistance())
            .setTotalTurns(result.totalTurns())
            .setPathFound(!result.isEmpty());

    for (var waypoint : result.waypoints()) {
      builder.addWaypoints(
          PathWaypoint.newBuilder()
              .setPosition(
                  GridCoordinate.newBuilder()
                      .setRow(waypoint.position().row())
                      .setCol(waypoint.position().col())
                      .build())
              .setOrientation(convertOrientationToProto(waypoint.orientation()))
              .setAction(convertActionToProto(waypoint.action()))
              .setTimeDelta(waypoint.timeDelta())
              .setCumulativeTime(waypoint.cumulativeTime())
              .build());
    }

    return builder.build();
  }

  private com.greyorange.grpc.navigation.v1.Orientation convertOrientationToProto(
      Orientation orientation) {
    return switch (orientation) {
      case NORTH -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_NORTH;
      case SOUTH -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_SOUTH;
      case EAST -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_EAST;
      case WEST -> com.greyorange.grpc.navigation.v1.Orientation.ORIENTATION_WEST;
    };
  }

  private NavigationAction convertActionToProto(
      com.greyorange.navigation.domain.NavigationAction action) {
    return switch (action) {
      case MOVE_FORWARD -> NavigationAction.NAVIGATION_ACTION_MOVE_FORWARD;
      case MOVE_BACKWARD -> NavigationAction.NAVIGATION_ACTION_MOVE_BACKWARD;
      case TURN_90_CW -> NavigationAction.NAVIGATION_ACTION_TURN_90_CW;
      case TURN_90_CCW -> NavigationAction.NAVIGATION_ACTION_TURN_90_CCW;
      case TURN_180 -> NavigationAction.NAVIGATION_ACTION_TURN_180;
    };
  }
}
```

**Step 2: Verify compilation**

```bash
cd apps/navigation-service && mvn clean compile -DskipTests -q
```

Expected: BUILD SUCCESS

**Step 3: Commit**

```bash
git add apps/navigation-service/
git commit -m "feat(navigation-service): add gRPC layer for pathfinding"
```

---

## Phase 9: Update Fleet-Simulator to Use Services

### Task 9.1: Create service clients in fleet-simulator

**Files:**
- Create: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/client/LayoutServiceClient.java`
- Create: `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/client/NavigationServiceClient.java`

**Step 1: Create LayoutServiceClient**

```java
package com.greyorange.dessimulator.client;

import com.greyorange.grpc.layout.v1.*;
import com.greyorange.greymatter.common.messaging.GrpcChannelFactory;
import jakarta.annotation.PostConstruct;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;
import org.springframework.stereotype.Service;

@Slf4j
@Service
@RequiredArgsConstructor
public class LayoutServiceClient {

  private final GrpcChannelFactory channelFactory;
  private LayoutServiceGrpc.LayoutServiceBlockingStub stub;

  @PostConstruct
  void init() {
    stub = channelFactory.createStub("layout-service", LayoutServiceGrpc::newBlockingStub);
  }

  public LayoutResponse getLayout(LayoutRequest request) {
    log.info("Calling layout-service GetLayout");
    return stub.getLayout(request);
  }

  public NavigationDataResponse getNavigationData(LayoutRequest request) {
    log.info("Calling layout-service GetNavigationData");
    return stub.getNavigationData(request);
  }
}
```

**Step 2: Create NavigationServiceClient**

```java
package com.greyorange.dessimulator.client;

import com.greyorange.grpc.navigation.v1.*;
import com.greyorange.greymatter.common.messaging.GrpcChannelFactory;
import jakarta.annotation.PostConstruct;
import jakarta.annotation.PreDestroy;
import lombok.RequiredArgsConstructor;
import lombok.extern.slf4j.Slf4j;
import org.springframework.stereotype.Service;

@Slf4j
@Service
@RequiredArgsConstructor
public class NavigationServiceClient {

  private final GrpcChannelFactory channelFactory;
  private NavigationServiceGrpc.NavigationServiceBlockingStub stub;
  private String currentSessionId;

  @PostConstruct
  void init() {
    stub = channelFactory.createStub("navigation-service", NavigationServiceGrpc::newBlockingStub);
  }

  public String createSession(CreateSessionRequest request) {
    log.info("Creating navigation session");
    CreateSessionResponse response = stub.createSession(request);
    currentSessionId = response.getSessionId();
    return currentSessionId;
  }

  public FindPathResponse findPath(FindPathRequest request) {
    return stub.findPath(request);
  }

  public void deleteSession(String sessionId) {
    log.info("Deleting navigation session: {}", sessionId);
    stub.deleteSession(DeleteSessionRequest.newBuilder().setSessionId(sessionId).build());
    if (sessionId.equals(currentSessionId)) {
      currentSessionId = null;
    }
  }

  @PreDestroy
  void cleanup() {
    if (currentSessionId != null) {
      try {
        deleteSession(currentSessionId);
      } catch (Exception e) {
        log.warn("Failed to cleanup session on shutdown", e);
      }
    }
  }
}
```

**Step 3: Update fleet-simulator application.yml**

Add gRPC client configuration:

```yaml
grpc:
  clients:
    layout-service:
      host: ${LAYOUT_SERVICE_HOST:localhost}
      port: ${LAYOUT_SERVICE_PORT:9091}
      plaintext: true
    navigation-service:
      host: ${NAVIGATION_SERVICE_HOST:localhost}
      port: ${NAVIGATION_SERVICE_PORT:9092}
      plaintext: true
```

**Step 4: Commit**

```bash
git add apps/fleet-simulator/
git commit -m "feat(fleet-simulator): add layout and navigation service clients"
```

---

## Phase 10: Integration and Cleanup

### Task 10.1: Update Docker Compose

**Files:**
- Modify: `docker-compose.dev.yml`

**Step 1: Add new services**

```yaml
services:
  layout-service:
    image: layout-service:latest
    ports:
      - "8082:8082"
      - "9091:9091"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8082/actuator/health"]
      interval: 10s
      timeout: 5s
      retries: 3

  navigation-service:
    image: navigation-service:latest
    ports:
      - "8083:8083"
      - "9092:9092"
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8083/actuator/health"]
      interval: 10s
      timeout: 5s
      retries: 3

  fleet-simulator:
    image: fleet-simulator:latest
    ports:
      - "8080:8080"
    environment:
      - LAYOUT_SERVICE_HOST=layout-service
      - LAYOUT_SERVICE_PORT=9091
      - NAVIGATION_SERVICE_HOST=navigation-service
      - NAVIGATION_SERVICE_PORT=9092
      - PUT_SERVICE_HOST=put-service
      - PUT_SERVICE_PORT=9090
    depends_on:
      layout-service:
        condition: service_healthy
      navigation-service:
        condition: service_healthy
      put-service:
        condition: service_healthy
```

**Step 2: Commit**

```bash
git add docker-compose.dev.yml
git commit -m "feat(docker): add layout and navigation services to docker-compose"
```

---

### Task 10.2: Update docker-build.sh

**Files:**
- Modify: `docker-build.sh`

**Step 1: Add new service builds**

```bash
# Add to docker-build.sh
echo "Building layout-service..."
cd apps/layout-service && mvn compile jib:dockerBuild -q && cd ../..

echo "Building navigation-service..."
cd apps/navigation-service && mvn compile jib:dockerBuild -q && cd ../..
```

**Step 2: Commit**

```bash
git add docker-build.sh
git commit -m "feat(docker): add layout and navigation service builds"
```

---

## Verification Checklist

After completing all phases, verify:

1. [ ] `cd libs/java/common-domain && mvn test` - All tests pass
2. [ ] `cd libs/java/grpc-stubs && mvn compile` - Proto generates successfully
3. [ ] `cd apps/layout-service && mvn clean compile` - Compiles
4. [ ] `cd apps/navigation-service && mvn clean compile` - Compiles
5. [ ] `cd apps/fleet-simulator && mvn clean compile` - Compiles
6. [ ] `./docker-build.sh` - All images build
7. [ ] `docker-compose -f docker-compose.dev.yml up` - All services start
8. [ ] Run a simulation end-to-end - Same results as before migration

---

## Rollback Plan

If issues arise during migration:

1. Git history preserves all changes - can revert specific commits
2. Fleet-simulator still has copies of moved code until Phase 10
3. Services can be disabled in docker-compose by commenting out

---

## Notes

- **Move-first approach**: Each copy/move is committed before any modifications
- **No logic changes during move**: Package declarations and imports only
- **gRPC layer is additive**: Existing domain code unchanged, controllers call it
- **Test at each phase**: Compile and run tests before proceeding
