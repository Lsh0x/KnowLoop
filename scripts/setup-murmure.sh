#!/usr/bin/env bash
# Setup script for Murmure STT sidecar
# Downloads the Parakeet model and prepares resources

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
RESOURCES_DIR="${PROJECT_ROOT}/resources"
MODEL_DIR="${RESOURCES_DIR}/parakeet-tdt-0.6b-v3-int8"
CC_RULES_DIR="${RESOURCES_DIR}/cc-rules"
MODEL_URL="https://github.com/Kieirra/murmure-model/releases/download/1.0.0/parakeet-tdt-0.6b-v3-int8.zip"

echo "=== Murmure STT Setup ==="

# Create directories
mkdir -p "$RESOURCES_DIR" "$CC_RULES_DIR"

# Download model if not present
if [ -d "$MODEL_DIR" ]; then
    echo "✓ Model already downloaded at $MODEL_DIR"
else
    echo "→ Downloading Parakeet model (~600MB)..."
    TMP_ZIP="/tmp/parakeet-model.zip"

    if command -v curl &>/dev/null; then
        curl -L -o "$TMP_ZIP" "$MODEL_URL"
    elif command -v wget &>/dev/null; then
        wget -O "$TMP_ZIP" "$MODEL_URL"
    else
        echo "✗ Neither curl nor wget found. Please install one."
        exit 1
    fi

    echo "→ Extracting model..."
    unzip -q "$TMP_ZIP" -d "$RESOURCES_DIR"
    rm "$TMP_ZIP"
    echo "✓ Model extracted to $MODEL_DIR"
fi

# Verify model files
if [ -f "$MODEL_DIR/model.onnx" ] || [ -f "$MODEL_DIR/encoder.onnx" ]; then
    echo "✓ Model files verified"
else
    echo "⚠ Warning: Expected model files not found in $MODEL_DIR"
    echo "  Contents:"
    ls -la "$MODEL_DIR/" 2>/dev/null || echo "  (directory empty or missing)"
fi

# Create default cc-rules if none exist
if [ -z "$(ls -A "$CC_RULES_DIR" 2>/dev/null)" ]; then
    echo "→ Creating default cc-rules..."
    cat > "$CC_RULES_DIR/default.toml" << 'TOML'
# Murmure custom correction rules
# Format: phonetic pattern → corrected text
# These help with technical terms that the model might mishear

[corrections]
# Add project-specific corrections here
# "no loop" = "KnowLoop"
# "mur mure" = "Murmure"
TOML
    echo "✓ Default cc-rules created"
else
    echo "✓ cc-rules already present"
fi

echo ""
echo "=== Setup Complete ==="
echo "To start Murmure with docker-compose:"
echo "  docker-compose up murmure"
echo ""
echo "To test manually:"
echo "  grpcurl -plaintext localhost:50051 list"
