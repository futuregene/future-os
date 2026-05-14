#!/bin/bash
# Test gRPC streaming for xihu agent

set -e

HOST="${1:-localhost}"
GRPC_PORT="${2:-50051}"
HTTP_PORT="${3:-8080}"

echo "=== Testing xihu agent gRPC ==="
echo "Host: $HOST"
echo "gRPC port: $GRPC_PORT"
echo "HTTP port: $HTTP_PORT"
echo ""

# Check if grpcurl is available
if ! command -v grpcurl &> /dev/null; then
    echo "Installing grpcurl..."
    brew install grpcurl 2>/dev/null || go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest
fi

# Test 1: List services
echo "=== Test 1: List gRPC services ==="
grpcurl -plaintext ${HOST}:${GRPC_PORT} list 2>/dev/null || echo "grpcurl failed, trying reflection..."

# Test 2: Describe service
echo -e "\n=== Test 2: Describe XihuAgent service ==="
grpcurl -plaintext ${HOST}:${GRPC_PORT} describe proto.XihuAgent 2>/dev/null || echo "Service description failed"

# Test 3: Call get_state via gRPC
echo -e "\n=== Test 3: get_state via gRPC ==="
grpcurl -plaintext -d '{"id": "test-1", "type": "get_state"}' ${HOST}:${GRPC_PORT} proto.XihuAgent/ExecuteCommand 2>/dev/null || echo "get_state failed"

# Test 4: Call get_available_models via gRPC
echo -e "\n=== Test 4: get_available_models via gRPC ==="
grpcurl -plaintext -d '{"id": "test-2", "type": "get_available_models"}' ${HOST}:${GRPC_PORT} proto.XihuAgent/ExecuteCommand 2>/dev/null || echo "get_available_models failed"

# Test 5: Stream events via gRPC
echo -e "\n=== Test 5: StreamEvents (SSE-like) via gRPC ==="
echo "Starting stream (will receive initial ping)..."
timeout 5 grpcurl -plaintext -d '{}' ${HOST}:${GRPC_PORT} proto.XihuAgent/StreamEvents 2>/dev/null || echo "(timeout expected)"

# Test 6: HTTP endpoint still works
echo -e "\n=== Test 6: HTTP endpoint ==="
curl -s -X POST http://${HOST}:${HTTP_PORT} -H "Content-Type: application/json" -d '{"type":"get_state","id":"test-3"}' 2>/dev/null | python3 -c "import json,sys; d=json.load(sys.stdin); print('success:', d.get('success'), '| model:', d.get('data',{}).get('model'))" || echo "HTTP failed"

# Test 7: HTTP SSE endpoint
echo -e "\n=== Test 7: HTTP SSE endpoint ==="
echo "Starting HTTP SSE stream..."
timeout 3 curl -sN http://${HOST}:${HTTP_PORT}/events 2>/dev/null | head -3 || echo "(timeout expected)"

echo -e "\n=== All tests completed ==="
