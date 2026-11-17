# Deployment Guide

This guide covers deploying the Queue Simulator microservices to Google Cloud Run.

## Architecture

The simulator runs as a **multi-container Cloud Run service** with two containers sharing localhost:

```
┌─────────────────────────────────────────────────────────┐
│  Cloud Run Service: greymatter-des-simulator            │
│                                                         │
│  ┌─────────────────┐      ┌─────────────────┐          │
│  │ fleet-simulator │─────▶│ put-service     │          │
│  │ (ingress)       │ gRPC │ (sidecar)       │          │
│  │ :8080           │localhost:9090          │          │
│  │                 │      └─────────────────┘          │
│  │ + React UI      │                                   │
│  │   (embedded)    │                                   │
│  └─────────────────┘                                   │
│                                                         │
│  Shared localhost network - ~0.1ms latency              │
└─────────────────────────────────────────────────────────┘
```

**Frontend**: The React UI is embedded in fleet-simulator as static resources, served by Spring Boot's SPA routing.

**Why multi-container?**
- Services need sub-millisecond latency for high-frequency gRPC calls during simulation
- Cloud Run sidecars share localhost network (no network hop)
- Simpler than Kubernetes while achieving same latency characteristics
- Uses existing Jib-built images without modification

## Prerequisites

- Google Cloud SDK (`gcloud`) installed and authenticated
- Maven 3.8+
- Java 21
- GCP project with Cloud Run API enabled

## Quick Deploy

### Manual Deployment

```bash
# From project root
cd deployment
./deploy.sh YOUR_PROJECT_ID us-central1
```

The script will:
1. Build frontend (React → `dist/apps/frontend`)
2. Build shared libraries (common-domain, common-messaging, grpc-stubs)
3. Build and push Docker images using Jib (fleet-simulator includes frontend)
4. Deploy multi-container service to Cloud Run
5. Configure public access

### GitHub Actions (Automatic)

Pushes to `master` automatically trigger deployment via `.github/workflows/deploy.yml`.

**Required secrets:**
- `GCP_PROJECT_ID` - Your GCP project ID
- `GCP_SA_KEY` - Service account JSON key with Cloud Run Admin role

## Configuration

### Cloud Run Service (`deployment/cloud-run-service.yaml`)

Key settings:

| Setting | Value | Reason |
|---------|-------|--------|
| `containerConcurrency` | 1 | One simulation per instance |
| `timeoutSeconds` | 3600 | Long-running simulations |
| `minScale` | 1 | Avoid cold starts |
| `maxScale` | 1 | Single instance (no horizontal scaling) |

### Container Resources

| Container | Memory | CPU | Purpose |
|-----------|--------|-----|---------|
| fleet-simulator | 4Gi | 2 | Simulation engine, Desmo-J |
| put-service | 2Gi | 1 | Put algorithms, OR-Tools |

### Environment Variables

**fleet-simulator:**
- `PUT_SERVICE_HOST=localhost` - Sidecar communication
- `PUT_SERVICE_GRPC_PORT=9090` - gRPC port
- `SCENARIOS_PATH=/app/resources/scenarios`
- `INPUT_PATH=/app/resources/input`

**put-service:**
- `GRPC_SERVER_PORT=9090`

## Container Dependencies

The service.yaml configures startup order:

```yaml
run.googleapis.com/container-dependencies: '{"fleet-simulator": ["put-service"]}'
```

This ensures put-service is healthy before fleet-simulator starts.

## Monitoring

### View Logs

```bash
# All containers
gcloud run services logs read greymatter-des-simulator --region=us-central1

# Specific container
gcloud run services logs read greymatter-des-simulator \
  --region=us-central1 \
  --container=fleet-simulator

gcloud run services logs read greymatter-des-simulator \
  --region=us-central1 \
  --container=put-service
```

### Health Checks

- **fleet-simulator**: `GET /api/health` on port 8080
- **put-service**: `GET /actuator/health` on port 8081

### Console

View metrics and logs in the Cloud Console:
https://console.cloud.google.com/run/detail/us-central1/greymatter-des-simulator

## Local Development

For local development, use Docker Compose:

```bash
# Build images
./docker-build.sh

# Start services
docker-compose -f docker-compose.dev.yml up
```

This mirrors the Cloud Run architecture with containers communicating via Docker network.

## Troubleshooting

### Container startup failures

Check container logs for specific errors:

```bash
gcloud run services logs read greymatter-des-simulator \
  --region=us-central1 \
  --limit=100
```

### gRPC connection refused

Ensure put-service is healthy before fleet-simulator tries to connect. The container dependency annotation handles this, but verify:

```bash
# Check fleet-simulator health
curl -s https://YOUR_SERVICE_URL/api/health
```

### Memory issues

If simulations fail with OOM, increase memory limits in `cloud-run-service.yaml`:

```yaml
resources:
  limits:
    memory: 8Gi  # Increase from 4Gi
```

## Future: Migration to Kubernetes

When you need:
- Multiple concurrent simulations (horizontal scaling)
- Independent service scaling
- Advanced networking (service mesh, traffic splitting)

Migrate to GKE with:
1. Use existing Jib images unchanged
2. Convert `cloud-run-service.yaml` to Kubernetes Deployment + Service
3. Use Pod affinity or sidecar containers for low latency

The current architecture is designed for easy migration - services are already containerized with gRPC interfaces.
