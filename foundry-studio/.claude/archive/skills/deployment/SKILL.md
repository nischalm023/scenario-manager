---
name: deployment
description: Use when deploying to Cloud Run, setting up infrastructure, running Docker Compose, or troubleshooting CI/CD pipelines.
---

# Deployment & Infrastructure

---

## Quick Reference

```bash
# Local development (primary)
make run                           # Start all services via NX
make run-dev                       # Start with debug ports
make stop                          # Stop all services

# Docker (integration testing)
make run-docker                    # Build and run containerized

# Production deployment
./deployment/deploy.sh             # Deploy to Cloud Run
./deployment/deploy.sh PROJECT us-central1  # Specific project/region
```

---

## Development Approaches

| Aspect | NX/Make (Local) | Docker Compose | Cloud Run |
|--------|-----------------|----------------|-----------|
| **Use case** | Day-to-day development | Integration testing | Production |
| **Services** | Hot-reloaded on host | Containerized locally | Containerized in GCP |
| **Command** | `make run` | `make run-docker` | `./deployment/deploy.sh` |
| **Debug** | `make run-dev` (5005-5008) | Not available | Not available |
| **Startup time** | Fast (seconds) | Medium (builds images) | N/A (pre-built) |

---

## Local Development (NX/Makefile)

**Primary approach for day-to-day development.** Services run directly on host with hot-reload.

```bash
# Install dependencies (one-time)
make deps

# Start all services
make run              # Interleaved output, all services

# Start with debug ports for IDE attachment
make run-dev
# Ports: fleet-simulator=5005, put-service=5006, layout-service=5007, navigation=5008

# Start with separate log files
make run-debug
# Logs written to /tmp/des-logs/*.log
tail -f /tmp/des-logs/*.log

# Stop all services
make stop

# Build all projects
make build

# Run all tests
make test

# Format code
make format
```

> **Tip**: Use `make run-dev` when you need to attach a debugger. Use `make run-debug` when troubleshooting startup issues.

---

## Docker Compose (Integration Testing)

Use when testing containerized behavior before deployment.

```bash
# Build images and start (auto-builds if images missing)
make run-docker

# Or manually
./docker-build.sh                          # Build all images
docker-compose -f docker-compose.dev.yml up

# Stop
docker-compose -f docker-compose.dev.yml down
```

**Configurations**:
- `docker-compose.dev.yml` - Microservices architecture (6 containers)
- `docker-compose.yml` - Monolithic container (single JAR)

**Container startup order** (handled automatically):
```
put-service (9090 gRPC)
    └─> layout-service (9091 gRPC)
        └─> navigation-service (9092 gRPC)
            └─> fleet-simulator (8085 REST)
                └─> nginx (8090 static frontend)
                    └─> envoy (8080 gateway)
```

---

## Production Deployment (Google Cloud Run)

### Prerequisites

- Google Cloud CLI installed
- GCP project with billing enabled
- Docker (for local testing)

### One-Time Setup

```bash
# Make scripts executable
chmod +x deployment/*.sh

# Run setup (enables APIs, sets project)
./deployment/setup-gcp.sh
```

**APIs enabled by setup**:
- `cloudbuild.googleapis.com`
- `run.googleapis.com`
- `containerregistry.googleapis.com`

### Deploy

```bash
# Using default gcloud project
./deployment/deploy.sh

# Specify project and region
./deployment/deploy.sh YOUR_PROJECT_ID us-central1
```

**What `deploy.sh` does**:
1. Builds frontend (`npx nx build frontend --configuration=production`)
2. Builds nginx image with frontend (`--platform linux/amd64`)
3. Builds shared libraries (common-domain, grpc-stubs, common-messaging)
4. Builds and pushes Java service images via Jib
5. Deploys to Cloud Run using `cloud-run-service.yaml`
6. Grants public access (unauthenticated)

### Multi-Container Architecture

Cloud Run uses a sidecar pattern with 6 containers in one instance:

| Container | CPU | Memory | Port | Purpose |
|-----------|-----|--------|------|---------|
| envoy | 0.5 | 512Mi | 8080 (ingress) | API Gateway (routing only) |
| nginx | 0.5 | 256Mi | 8090 | Static frontend |
| fleet-simulator | 4 | 8Gi | 8085 | Simulation engine (REST API) |
| put-service | 1 | 4Gi | 9090 gRPC | Storage algorithms |
| layout-service | 1 | 4Gi | 9091 gRPC | Map service |
| navigation-service | 1 | 4Gi | 9092 gRPC | Pathfinding |

**Key settings**:
- `minScale: 1` - Always warm (no cold starts)
- `maxScale: 1` - Single instance
- `containerConcurrency: 1` - One simulation at a time
- `timeoutSeconds: 3600` - 1 hour max for long simulations

---

## CI/CD Pipeline (GitHub Actions)

**File**: `.github/workflows/deploy.yml`

**Trigger**: Push to `master` branch

### Build Stages

1. **Setup**: Node.js 24, JDK 21, GCP auth
2. **Frontend**: `npx nx build frontend --configuration=production`
3. **Nginx**: Build nginx image with frontend (`--platform linux/amd64`)
4. **Libraries**: Parent POM, common-domain, grpc-stubs, common-messaging
5. **Services**: Build Java services via Jib, push to GCR with git SHA tag
6. **Deploy**: `gcloud run services replace cloud-run-service.yaml`
7. **Access**: Apply IAM policy for public access

### Required Secrets

| Secret | Description |
|--------|-------------|
| `GCP_SA_KEY` | Service account JSON key |
| `GCP_PROJECT_ID` | GCP project ID |

### Image Tagging

Images tagged with git commit SHA for traceability:
```
gcr.io/PROJECT/fleet-simulator:abc1234
gcr.io/PROJECT/put-service:abc1234
...
```

---

## Infrastructure (Envoy + Nginx)

### Routing Rules

| Route | Target | Timeout | Cache |
|-------|--------|---------|-------|
| `/api/warehouse-map/*` | layout-service:8082 | 30s | None |
| `/api/*` | fleet-simulator:8085 | 300s | None |
| `/assets/*` | nginx:8090 | 30s | 1 year (immutable) |
| `/*` | nginx:8090 | 30s | no-store |

Nginx handles cache headers directly (not Envoy).

### Configuration Files

| File | Usage |
|------|-------|
| `infrastructure/envoy/envoy.yaml` | Docker Compose (services on network) |
| `infrastructure/nginx/nginx.conf` | Nginx static file serving + SPA fallback |
| `infrastructure/nginx/Dockerfile` | Nginx image with frontend assets |
| Embedded in `cloud-run-service.yaml` | Cloud Run (localhost sidecars) |

### Envoy Commands

```bash
npx nx serve envoy    # Start for local dev
npx nx stop envoy     # Stop container
npx nx logs envoy     # View logs
npx nx admin envoy    # Open admin UI (localhost:9901)
```

---

## Monitoring & Logs

```bash
# View Cloud Run logs
gcloud run services logs read greymatter-des-simulator --region=us-central1

# Follow logs in real-time
gcloud run services logs tail greymatter-des-simulator --region=us-central1

# Logs by container
gcloud run services logs read greymatter-des-simulator --container=envoy
gcloud run services logs read greymatter-des-simulator --container=nginx
gcloud run services logs read greymatter-des-simulator --container=fleet-simulator
gcloud run services logs read greymatter-des-simulator --container=put-service
gcloud run services logs read greymatter-des-simulator --container=layout-service
gcloud run services logs read greymatter-des-simulator --container=navigation-service

# Service status
gcloud run services describe greymatter-des-simulator --region=us-central1

# Get service URL
gcloud run services describe greymatter-des-simulator --region=us-central1 --format="value(status.url)"
```

---

## Troubleshooting

### Deployment Fails

```bash
# Enable required APIs
gcloud services enable cloudbuild.googleapis.com run.googleapis.com containerregistry.googleapis.com

# Verify authentication
gcloud auth list
gcloud auth login  # If needed

# Check which APIs are enabled
gcloud services list --enabled
```

### Permission Denied

```bash
chmod +x deployment/*.sh
```

### Build Errors

- Frontend TypeScript errors will fail the build
- Test locally first: `npx nx build frontend --configuration=production`

### Container Won't Start

Check health endpoint locally:
```bash
curl http://localhost:8081/actuator/health  # put-service
curl http://localhost:8082/actuator/health  # layout-service
curl http://localhost:8085/actuator/health  # fleet-simulator
```

### Cost Estimate

Cloud Run (8GB RAM, 4 CPUs, min 1 instance):
- Light usage: $10-20/month
- Heavy usage: $30-50/month
- No traffic: ~$5-10/month (min instance always running)

---

## Related Skills

- `.claude/skills/commands/` - All development commands including local service start
- `.claude/skills/debugging/` - Service issues, common errors, health checks
- `.claude/skills/architecture/` - Service boundaries, gRPC communication patterns
