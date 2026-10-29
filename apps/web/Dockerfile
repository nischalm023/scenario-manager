# Build from the repo root:
#   docker build -f apps/web/Dockerfile -t scenes/web \
#     --build-arg NEXT_PUBLIC_API_BASE_URL=/api .
FROM node:20-alpine AS base
RUN corepack enable
WORKDIR /repo

FROM base AS build
# NEXT_PUBLIC_* values are inlined into the client bundle at build time.
ARG NEXT_PUBLIC_API_BASE_URL=/api
ENV NEXT_PUBLIC_API_BASE_URL=$NEXT_PUBLIC_API_BASE_URL
COPY . .
RUN --mount=type=cache,id=pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter web... && \
    pnpm store prune
RUN pnpm --filter web build

FROM base AS runtime
ENV NODE_ENV=production
COPY --from=build --chown=node:node /repo /repo
USER node
WORKDIR /repo/apps/web
EXPOSE 3000
CMD ["pnpm", "start"]
