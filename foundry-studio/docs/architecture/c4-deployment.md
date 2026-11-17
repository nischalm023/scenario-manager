# Deployment Diagram — Production (Cloud Run)

> Mermaid (renders on GitHub). For Structurizr, use [`workspace.dsl`](workspace.dsl).

Multi-container topology on Google Cloud Run.

```mermaid
C4Deployment
  title Deployment Diagram — Production (Cloud Run, us-central1)

  Deployment_Node(browser, "Engineer's Browser", "Chrome / Firefox") {
    Container(spa, "Frontend SPA", "React 19", "Warehouse visualization")
  }

  Deployment_Node(gcp, "Google Cloud Platform", "us-central1") {
    Deployment_Node(cloudrun, "Cloud Run Service", "greymatter-des-simulator, concurrency=1") {
      Deployment_Node(envoyNode, "Envoy Sidecar", "512Mi, 0.5 CPU") {
        Container(envoy, "Envoy Gateway", "Envoy 1.28", "Ingress on :8080")
      }
      Deployment_Node(nginxNode, "nginx Sidecar", "256Mi, 0.5 CPU") {
        Container(nginx, "nginx", "nginx:alpine", "Static files on :8090")
      }
      Deployment_Node(fleetNode, "Fleet Simulator", "8Gi, 4 CPU") {
        Container(fleetSim, "Fleet Simulator", "Spring Boot 3.2", "REST :8085")
      }
      Deployment_Node(layoutNode, "Layout Service", "4Gi, 1 CPU") {
        Container(layoutSvc, "Layout Service", "Spring Boot 3.2", "REST :8082, gRPC :9091")
      }
      Deployment_Node(navNode, "Navigation Service", "4Gi, 1 CPU") {
        Container(navSvc, "Navigation Service", "Spring Boot 3.2", "REST :8083, gRPC :9092")
      }
      Deployment_Node(putNode, "Put Service", "4Gi, 1 CPU") {
        Container(putSvc, "Put Service", "Spring Boot 3.2", "REST :8081, gRPC :9090")
      }
    }

    Deployment_Node(gcr, "Container Registry", "gcr.io") {
      Container(images, "Container Images", "Docker", "6 service images")
    }
  }

  Rel(spa, envoy, "API calls", "HTTPS")
  Rel(envoy, nginx, "Static files", "HTTP")
  Rel(envoy, fleetSim, "/api/*", "HTTP")
  Rel(envoy, layoutSvc, "/api/warehouse-map/*", "HTTP")
  Rel(fleetSim, layoutSvc, "GetLayout", "gRPC")
  Rel(fleetSim, navSvc, "FindPath", "gRPC")
  Rel(fleetSim, putSvc, "InitInventory", "gRPC")
```

**Key details:**
- All 6 containers run as sidecars in a **single Cloud Run revision** sharing localhost network
- **Concurrency = 1** — one simulation at a time per instance
- **Min/max instances = 1** — always-warm, no scale-out (simulations are CPU-bound)
- **Request timeout = 3600s** (1 hour max for long simulations)
- **Total resources: 24Gi memory, 8 CPU**
- Envoy starts last (depends on all backends being healthy)
