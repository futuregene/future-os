#!/usr/bin/env python3
"""Test gRPC streaming for xihu agent"""

import subprocess
import time
import sys
import json

def run(cmd):
    """Run shell command and return output"""
    result = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    return result.stdout + result.stderr

def main():
    print("=== Testing xihu agent gRPC ===\n")
    
    # Start server
    print("Starting server...")
    subprocess.Popen(
        ["../target/release/xihu", "--server", "--grpc-port", "50051", "--port", "8080"],
        cwd="/Users/geilige/xihu/agent",
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL
    )
    time.sleep(3)
    
    # Test 1: HTTP endpoint
    print("=== Test 1: HTTP get_state ===")
    result = run('curl -s -X POST http://localhost:8080 -H "Content-Type: application/json" -d \'{"type":"get_state","id":"1"}\'')
    try:
        d = json.loads(result)
        print(f"  success: {d.get('success')}")
        print(f"  model: {d.get('data', {}).get('model')}")
    except:
        print(f"  Failed: {result[:200]}")
    
    # Test 2: HTTP SSE
    print("\n=== Test 2: HTTP SSE (2 seconds) ===")
    result = run('curl -sN --max-time 2 http://localhost:8080/events 2>/dev/null | head -3')
    print(f"  Received: {result[:200] if result else 'nothing'}")
    
    # Test 3: Check ports
    print("\n=== Test 3: Port check ===")
    http_ok = run("nc -z localhost 8080 && echo OK || echo FAIL").strip()
    grpc_ok = run("nc -z localhost 50051 && echo OK || echo FAIL").strip()
    print(f"  HTTP (8080): {http_ok}")
    print(f"  gRPC (50051): {grpc_ok}")
    
    # Cleanup
    run("pkill -f 'xihu.*50051' 2>/dev/null || true")
    
    print("\n=== Tests completed ===")

if __name__ == "__main__":
    main()
