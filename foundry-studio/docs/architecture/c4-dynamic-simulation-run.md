# Dynamic Diagram — Run Simulation

> Mermaid (renders on GitHub). For Structurizr, use [`workspace.dsl`](workspace.dsl).

The core workflow: user submits a config, system runs a discrete event simulation, returns results.

```mermaid
C4Dynamic
  title Run Simulation Flow

  Person(engineer, "Warehouse Engineer", "Triggers simulation")
  Container(envoy, "Envoy Gateway", "Envoy 1.28", "API ingress")
  Container(fleetSim, "Fleet Simulator", "Spring Boot 3.2", "Simulation engine")
  Container(layoutSvc, "Layout Service", "Spring Boot 3.2", "Layout generation")
  Container(putSvc, "Put Service", "Spring Boot 3.2", "Storage algorithms")
  Container(navSvc, "Navigation Service", "Spring Boot 3.2", "A* pathfinding")

  Rel(engineer, envoy, "1. POST /api/simulation/run", "JSON config")
  Rel(envoy, fleetSim, "2. Forward request", "HTTP :8085")
  Rel(fleetSim, layoutSvc, "3. GetLayout", "gRPC :9091")
  Rel(fleetSim, putSvc, "4. InitializeInventory", "gRPC :9090")
  Rel(fleetSim, navSvc, "5. CreateSession (sends map)", "gRPC :9092")
  Rel(fleetSim, navSvc, "6. FindPath (repeated per robot move)", "gRPC :9092")
  Rel(fleetSim, navSvc, "7. DeleteSession", "gRPC :9092")
  Rel(fleetSim, envoy, "8. Return SimulationResult", "JSON")
  Rel(envoy, engineer, "9. Results + metrics", "HTTPS")
```

**Step details:**

| Step | What happens |
|------|-------------|
| 1-2 | JSON config includes warehouse dimensions, robot count, order profiles, algorithm selection |
| 3 | Fleet Simulator gets the physical warehouse layout (rack positions, aisles, operator stations) |
| 4 | Put Service initializes bin inventory using the selected put algorithm (greedy, ABC classification) |
| 5 | Navigation Service creates a pathfinding session with the warehouse grid for collision-aware routing |
| 5→6 | *Internally*, Desmo-J runs the discrete event loop — robots pick orders, travel paths, queue at operators |
| 6 | Each robot movement triggers an A* pathfinding call (most frequent gRPC call during simulation) |
| 7 | Session cleanup frees navigation service memory |
| 8-9 | Results include throughput metrics, queue depths, robot utilization, operator idle time |
