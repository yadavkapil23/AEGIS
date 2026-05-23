#!/bin/bash

##############################################################################
# AEGIS Gateway - Real LLM Inference Test Script
#
# This script tests the real LLM inference with vLLM and llama.cpp
# Run this after starting the gateway and at least one LLM backend
#
# Usage: chmod +x test_real_inference.sh && ./test_real_inference.sh
##############################################################################

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

GATEWAY_URL="http://localhost:8080"
API_KEY="sk-demo123"
PASSED=0
FAILED=0

##############################################################################
# Helper functions
##############################################################################

print_header() {
    echo -e "${BLUE}=================================================================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}=================================================================================${NC}"
}

print_test() {
    echo -e "\n${YELLOW}▶ $1${NC}"
}

print_success() {
    echo -e "${GREEN}✓ $1${NC}"
    ((PASSED++))
}

print_error() {
    echo -e "${RED}✗ $1${NC}"
    ((FAILED++))
}

print_info() {
    echo -e "${BLUE}ℹ $1${NC}"
}

check_response() {
    local response=$1
    local expected_key=$2
    local test_name=$3

    if echo "$response" | grep -q "\"$expected_key\""; then
        print_success "$test_name"
        echo "$response" | jq '.' 2>/dev/null || echo "$response"
        return 0
    else
        print_error "$test_name"
        echo "Response: $response"
        return 1
    fi
}

##############################################################################
# Main test suite
##############################################################################

main() {
    print_header "AEGIS Gateway - Real LLM Inference Test Suite"

    echo -e "\n${BLUE}Configuration:${NC}"
    echo "Gateway URL: $GATEWAY_URL"
    echo "API Key: $API_KEY"
    echo ""

    # Check if gateway is running
    print_test "Checking if gateway is running"
    if ! curl -s "$GATEWAY_URL/health/live" > /dev/null 2>&1; then
        print_error "Gateway is not responding at $GATEWAY_URL"
        exit 1
    fi
    print_success "Gateway is running"

    ##########################################################################
    # 1. Health Checks
    ##########################################################################

    print_header "Test 1: Health Checks"

    print_test "GET /health/live - Liveness probe"
    RESPONSE=$(curl -s "$GATEWAY_URL/health/live")
    check_response "$RESPONSE" "status" "Liveness probe" || true

    print_test "GET /health/ready - Readiness probe"
    RESPONSE=$(curl -s "$GATEWAY_URL/health/ready")
    check_response "$RESPONSE" "status" "Readiness probe" || true

    print_test "GET /health/startup - Startup probe"
    RESPONSE=$(curl -s "$GATEWAY_URL/health/startup")
    check_response "$RESPONSE" "status" "Startup probe" || true

    ##########################################################################
    # 2. Backend Status
    ##########################################################################

    print_header "Test 2: Backend Status"

    print_test "GET /backends/status - Check backend health and circuit breakers"
    RESPONSE=$(curl -s "$GATEWAY_URL/backends/status")
    check_response "$RESPONSE" "vllm" "Backend status endpoint" || true
    echo "$RESPONSE" | jq '.' 2>/dev/null || echo "$RESPONSE"

    ##########################################################################
    # 3. Authentication
    ##########################################################################

    print_header "Test 3: Authentication"

    print_test "POST /infer without credentials (should fail)"
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -d '{
            "model": "llama-7b",
            "prompt": "test",
            "max_tokens": 10
        }')

    if [ "$HTTP_CODE" == "401" ]; then
        print_success "Request rejected without auth (HTTP $HTTP_CODE)"
    else
        print_error "Expected 401, got $HTTP_CODE"
    fi

    print_test "POST /infer with invalid API key (should fail)"
    HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
        -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: invalid-key" \
        -d '{
            "model": "llama-7b",
            "prompt": "test",
            "max_tokens": 10
        }')

    if [ "$HTTP_CODE" == "401" ]; then
        print_success "Request rejected with invalid key (HTTP $HTTP_CODE)"
    else
        print_error "Expected 401, got $HTTP_CODE"
    fi

    ##########################################################################
    # 4. Request Validation
    ##########################################################################

    print_header "Test 4: Request Validation"

    print_test "POST /infer with empty prompt (should fail)"
    RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $API_KEY" \
        -d '{
            "model": "llama-7b",
            "prompt": "",
            "max_tokens": 10
        }')

    check_response "$RESPONSE" "error" "Empty prompt validation" || true

    print_test "POST /infer with invalid max_tokens (should fail)"
    RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $API_KEY" \
        -d '{
            "model": "llama-7b",
            "prompt": "test",
            "max_tokens": 50000
        }')

    check_response "$RESPONSE" "error" "Invalid max_tokens validation" || true

    print_test "POST /infer with invalid temperature (should fail)"
    RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $API_KEY" \
        -d '{
            "model": "llama-7b",
            "prompt": "test",
            "max_tokens": 10,
            "temperature": 3.5
        }')

    check_response "$RESPONSE" "error" "Invalid temperature validation" || true

    ##########################################################################
    # 5. Real Inference (This is the key test!)
    ##########################################################################

    print_header "Test 5: Real LLM Inference (CRITICAL)"

    print_test "POST /infer - Simple completion with vLLM/llama.cpp"
    echo "Sending inference request..."
    RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $API_KEY" \
        -d '{
            "model": "llama-7b",
            "prompt": "What is artificial intelligence? Answer in one sentence.",
            "max_tokens": 50,
            "temperature": 0.7
        }')

    echo "Response received:"
    echo "$RESPONSE" | jq '.' 2>/dev/null || echo "$RESPONSE"

    # Check for success
    if echo "$RESPONSE" | grep -q '"success":true'; then
        print_success "Real inference request succeeded!"

        # Extract output if available
        OUTPUT=$(echo "$RESPONSE" | jq -r '.output' 2>/dev/null || echo "N/A")
        TOKENS=$(echo "$RESPONSE" | jq '.tokens_generated' 2>/dev/null || echo "N/A")
        LATENCY=$(echo "$RESPONSE" | jq '.latency_ms' 2>/dev/null || echo "N/A")

        print_info "Output: $OUTPUT"
        print_info "Tokens generated: $TOKENS"
        print_info "Latency: ${LATENCY}ms"
    else
        print_error "Real inference request failed"
        if echo "$RESPONSE" | grep -q '"error"'; then
            ERROR=$(echo "$RESPONSE" | jq -r '.error' 2>/dev/null)
            print_info "Error: $ERROR"
        fi
    fi

    print_test "POST /infer - Complex request with all parameters"
    RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
        -H "Content-Type: application/json" \
        -H "X-API-Key: $API_KEY" \
        -d '{
            "model": "llama-7b",
            "prompt": "Explain machine learning in simple terms.",
            "max_tokens": 100,
            "temperature": 0.5,
            "top_p": 0.9
        }')

    if echo "$RESPONSE" | grep -q '"success":true'; then
        print_success "Complex inference request succeeded"
        echo "$RESPONSE" | jq '.output' 2>/dev/null | head -c 100
        echo "..."
    else
        print_error "Complex inference request failed"
    fi

    ##########################################################################
    # 6. Performance
    ##########################################################################

    print_header "Test 6: Performance & Latency"

    print_test "Measuring inference latency (3 requests)"
    TOTAL_LATENCY=0
    for i in {1..3}; do
        RESPONSE=$(curl -s -X POST "$GATEWAY_URL/infer" \
            -H "Content-Type: application/json" \
            -H "X-API-Key: $API_KEY" \
            -d '{
                "model": "llama-7b",
                "prompt": "What is the capital of France?",
                "max_tokens": 10
            }')

        LATENCY=$(echo "$RESPONSE" | jq '.latency_ms' 2>/dev/null || echo 0)
        TOTAL_LATENCY=$((TOTAL_LATENCY + LATENCY))
        print_info "Request $i: ${LATENCY}ms"
    done

    AVG_LATENCY=$((TOTAL_LATENCY / 3))
    print_info "Average latency: ${AVG_LATENCY}ms"

    ##########################################################################
    # 7. Metrics
    ##########################################################################

    print_header "Test 7: Metrics Collection"

    print_test "GET /metrics - Check Prometheus metrics"
    RESPONSE=$(curl -s "$GATEWAY_URL/metrics")

    if echo "$RESPONSE" | grep -q "AEGIS Gateway Metrics"; then
        print_success "Metrics endpoint is working"
        # Count lines of metrics
        METRIC_COUNT=$(echo "$RESPONSE" | grep "^inference_" | wc -l)
        print_info "Found $METRIC_COUNT inference metrics"
    else
        print_error "Metrics endpoint returned unexpected format"
    fi

    ##########################################################################
    # Summary
    ##########################################################################

    print_header "Test Summary"

    TOTAL=$((PASSED + FAILED))
    PERCENTAGE=$((PASSED * 100 / TOTAL))

    echo -e "\n${GREEN}Passed: $PASSED${NC}"
    echo -e "${RED}Failed: $FAILED${NC}"
    echo -e "${BLUE}Total:  $TOTAL${NC}"
    echo -e "\n${BLUE}Success Rate: ${PERCENTAGE}%${NC}\n"

    if [ $FAILED -eq 0 ]; then
        echo -e "${GREEN}=================================================================================${NC}"
        echo -e "${GREEN}✓ ALL TESTS PASSED - Real LLM Inference is Working!${NC}"
        echo -e "${GREEN}=================================================================================${NC}"
        exit 0
    else
        echo -e "${RED}=================================================================================${NC}"
        echo -e "${RED}✗ SOME TESTS FAILED - Check the errors above${NC}"
        echo -e "${RED}=================================================================================${NC}"
        exit 1
    fi
}

# Run main function
main "$@"
