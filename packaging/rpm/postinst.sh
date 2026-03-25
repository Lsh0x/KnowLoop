#!/bin/sh
# RPM post-install script for KnowLoop
set -e

# 1. Create system user 'orchestrator' if not exists
if ! getent passwd orchestrator >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin \
        --home-dir /var/lib/knowloop orchestrator
    echo "Created system user 'orchestrator'"
fi

# 2. Create working directory
mkdir -p /var/lib/knowloop
chown orchestrator:orchestrator /var/lib/knowloop
chmod 750 /var/lib/knowloop

# 3. Create config directory
mkdir -p /etc/knowloop

# 4. Generate initial config if absent
if [ ! -f /etc/knowloop/config.yaml ]; then
    cp /etc/knowloop/config.yaml.example \
       /etc/knowloop/config.yaml
    echo "Created /etc/knowloop/config.yaml from example"
fi

# 5. Generate env file with random secrets if absent
if [ ! -f /etc/knowloop/env ]; then
    JWT_SECRET=$(head -c 32 /dev/urandom | base64 | tr -d '\n')
    MEILI_KEY=$(head -c 32 /dev/urandom | base64 | tr -d '\n')
    cat > /etc/knowloop/env <<EOF
# Auto-generated secrets — change as needed
NEO4J_URI=bolt://localhost:7687
NEO4J_USER=neo4j
NEO4J_PASSWORD=change-me
MEILISEARCH_URL=http://localhost:7700
MEILISEARCH_KEY=${MEILI_KEY}
JWT_SECRET=${JWT_SECRET}
RUST_LOG=info,knowloop=debug
EOF
    chmod 600 /etc/knowloop/env
    chown orchestrator:orchestrator /etc/knowloop/env
    echo "Generated /etc/knowloop/env with random secrets"
fi

# 6. Set ownership on config files
chown -R orchestrator:orchestrator /etc/knowloop

# 7. Reload systemd
systemctl daemon-reload || true

echo ""
echo "================================================================"
echo "  KnowLoop installed successfully!"
echo ""
echo "  Before starting, ensure Neo4j and MeiliSearch are running."
echo "  A Docker Compose file is provided at:"
echo "    /etc/knowloop/docker-compose.services.yml"
echo ""
echo "  Quick start:"
echo "    cd /etc/knowloop"
echo "    docker compose -f docker-compose.services.yml up -d"
echo "    sudo systemctl enable --now knowloop"
echo ""
echo "  Edit config: /etc/knowloop/config.yaml"
echo "  Edit secrets: /etc/knowloop/env"
echo ""
echo "  Claude Code integration:"
echo "    knowloop setup-claude"
echo "    (auto-configures the MCP server in Claude Code)"
echo "================================================================"
echo ""
