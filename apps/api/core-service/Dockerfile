# Build from the repo root:
#   docker build -f apps/api/core-service/Dockerfile -t scenes/core-service .
FROM node:20-alpine AS base
RUN corepack enable
WORKDIR /repo

FROM base AS build
COPY . .
RUN --mount=type=cache,id=pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter core-service... && \
    pnpm store prune
RUN pnpm --filter core-service... build

FROM base AS prod-deps
COPY pnpm-lock.yaml pnpm-workspace.yaml package.json ./
COPY apps/api/core-service/package.json apps/api/core-service/
COPY packages/shared/package.json packages/shared/
# `prepare` in shared runs a build; deps are installed without scripts here.
RUN --mount=type=cache,id=pnpm,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --prod --ignore-scripts --filter core-service... && \
    pnpm store prune

FROM node:20-alpine AS runtime
ENV NODE_ENV=production
WORKDIR /repo
COPY --from=prod-deps --chown=node:node /repo/node_modules ./node_modules
COPY --from=prod-deps --chown=node:node /repo/packages/shared/node_modules ./packages/shared/node_modules
COPY --from=prod-deps --chown=node:node /repo/apps/api/core-service/node_modules ./apps/api/core-service/node_modules
COPY --from=build --chown=node:node /repo/packages/shared/package.json ./packages/shared/package.json
COPY --from=build --chown=node:node /repo/packages/shared/dist ./packages/shared/dist
COPY --from=build --chown=node:node /repo/apps/api/core-service/package.json ./apps/api/core-service/package.json
COPY --from=build --chown=node:node /repo/apps/api/core-service/dist ./apps/api/core-service/dist
# shared declares @nestjs/common as a peer; pnpm doesn't link it in a prod-only install.
RUN ln -s /repo/apps/api/core-service/node_modules/@nestjs /repo/packages/shared/node_modules/@nestjs
USER node
WORKDIR /repo/apps/api/core-service
EXPOSE 3002
CMD ["node", "dist/main.js"]
