# System Context — GTP Queue Simulator

> Mermaid (renders on GitHub). For Structurizr, use [`workspace.dsl`](workspace.dsl).

Who uses it, what it is, where it runs.

```mermaid
C4Context
  title System Context — GTP Queue Simulator

  Person(engineer, "Warehouse Engineer", "Configures scenarios and analyzes simulation results")

  System(simulator, "GTP Queue Simulator", "Discrete event simulator for warehouse goods-to-person operations using Desmo-J")

  System_Ext(cloudRun, "Google Cloud Run", "Hosts the simulator as a multi-container service")

  Rel(engineer, simulator, "Configures scenarios and runs simulations", "HTTPS")
  Rel(simulator, cloudRun, "Deployed to", "us-central1")
```

**Notes:**
- Single user type — warehouse engineers who design and tune GTP layouts
- No external system integrations (self-contained simulator)
- Cloud Run provides serverless hosting with concurrency=1 (one simulation at a time)
