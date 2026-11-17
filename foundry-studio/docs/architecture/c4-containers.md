# Container Diagram — GTP Queue Simulator

> Mermaid (renders on GitHub). For Structurizr, use [`workspace.dsl`](workspace.dsl).

All services, protocols, and ports.

```mermaid
C4Container
  title Container Diagram — GTP Queue Simulator

  Person(engineer, "Warehouse Engineer", "Configures and runs simulations")

  System_Boundary(sim, "GTP Queue Simulator") {
    Container(envoy, "Envoy Gateway", "Envoy 1.28", "Routes API and static requests")
    Container(nginx, "nginx", "nginx:alpine", "Serves SPA static files")
    Container(frontend, "Frontend SPA", "React 19, TypeScript, Vite", "Warehouse visualization and simulation config")
    Container(fleetSim, "Fleet Simulator", "Spring Boot 3.2, Java 21", "Simulation engine and REST API")
    Container(layoutSvc, "Layout Service", "Spring Boot 3.2, Java 21", "Warehouse layout generation")
    Container(navSvc, "Navigation Service", "Spring Boot 3.2, Java 21", "A* robot pathfinding")
    Container(putSvc, "Put Service", "Spring Boot 3.2, Java 21", "Storage algorithms and inventory init")
  }

  Rel(engineer, envoy, "Uses", "HTTPS :8080")
  Rel(envoy, nginx, "Serves frontend", "HTTP :8090")
  Rel(envoy, fleetSim, "Routes /api/*", "HTTP :8085")
  Rel(envoy, layoutSvc, "Routes /api/warehouse-map/*", "HTTP :8082")
  Rel(nginx, frontend, "Serves", "Static files")
  Rel(fleetSim, layoutSvc, "Gets warehouse layout", "gRPC :9091")
  Rel(fleetSim, navSvc, "Finds robot paths", "gRPC :9092")
  Rel(fleetSim, putSvc, "Initializes inventory", "gRPC :9090")
  Rel(layoutSvc, putSvc, "Calculates bin popularity", "gRPC :9090")
```

**Protocol boundaries:**
- **External → Envoy**: HTTPS (Cloud Run terminates TLS)
- **Envoy → backends**: HTTP (localhost network in Cloud Run)
- **Service-to-service**: gRPC (localhost network)

**Routing rules** (Envoy):
| Pattern | Target | Timeout |
|---------|--------|---------|
| `/api/warehouse-map/*` | layout-service :8082 | 30s |
| `/api/*` | fleet-simulator :8085 | 300s |
| `/`, `/assets/*` | nginx :8090 | 30s |
