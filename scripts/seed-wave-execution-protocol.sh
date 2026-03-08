#!/usr/bin/env bash
# Seed script: Create the "wave-execution" protocol as the first Pattern Federation instance
# Requires: PO backend running on localhost:6600
# Usage: ./scripts/seed-wave-execution-protocol.sh [PROJECT_ID] [SKILL_ID]

set -euo pipefail

BASE_URL="${PO_URL:-http://localhost:6600}"
PROJECT_ID="${1:-00333b5f-2d0a-4467-9c98-155e55d2b7e5}"
SKILL_ID="${2:-4ffd8f18-a121-4f17-bd98-e3c8f185e79a}"

echo "=== Creating wave-execution protocol ==="
echo "Project: $PROJECT_ID"
echo "Skill:   $SKILL_ID"
echo "API:     $BASE_URL"
echo ""

# 1. Create the protocol
PROTOCOL=$(curl -s -X POST "$BASE_URL/api/protocols" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"wave-execution\",
    \"description\": \"Decomposes a plan into parallel execution waves via topological sort of the dependency DAG, with conflict splitting by affected_files intersection. Guides the agent through wave computation and dispatch.\",
    \"project_id\": \"$PROJECT_ID\",
    \"protocol_category\": \"business\"
  }")

PROTOCOL_ID=$(echo "$PROTOCOL" | jq -r '.id')
echo "✓ Protocol created: $PROTOCOL_ID"

# 2. Add states
declare -A STATES
for state_def in \
  "DETECT:start:Evaluate if plan is eligible for wave execution (>4 tasks with dependencies):Check task count and dependency presence" \
  "READ_GRAPH:intermediate:Read the dependency graph via plan(get_dependency_graph):Load tasks and edges" \
  "COMPUTE_LEVELS:intermediate:Compute topological levels via Kahn's algorithm:Run topological sort" \
  "CHECK_CONFLICTS:intermediate:Detect affected_files intersections within each wave:Build conflict graph" \
  "SPLIT_WAVES:intermediate:Partition conflicting waves into sub-waves via graph coloring:Run greedy coloring" \
  "DISPLAY_PLAN:intermediate:Display the computed waves to the user:Show wave view" \
  "DISPATCH:terminal:Launch wave-by-wave execution for large plans:Execute waves sequentially" \
  "SEQUENTIAL:terminal:Fallback for small plans — execute tasks sequentially:Skip wave execution"
do
  IFS=':' read -r name stype desc action <<< "$state_def"
  RESULT=$(curl -s -X POST "$BASE_URL/api/protocols/$PROTOCOL_ID/states" \
    -H "Content-Type: application/json" \
    -d "{
      \"name\": \"$name\",
      \"description\": \"$desc\",
      \"state_type\": \"$stype\",
      \"action\": \"$action\"
    }")
  STATE_ID=$(echo "$RESULT" | jq -r '.id')
  STATES[$name]=$STATE_ID
  echo "  ✓ State $name ($stype): $STATE_ID"
done

# 3. Add transitions
add_transition() {
  local from=$1 to=$2 trigger=$3 guard=${4:-}
  local body="{\"from_state\": \"${STATES[$from]}\", \"to_state\": \"${STATES[$to]}\", \"trigger\": \"$trigger\""
  if [ -n "$guard" ]; then
    body="$body, \"guard\": \"$guard\""
  fi
  body="$body}"
  curl -s -X POST "$BASE_URL/api/protocols/$PROTOCOL_ID/transitions" \
    -H "Content-Type: application/json" \
    -d "$body" > /dev/null
  echo "  ✓ $from → $to ($trigger)"
}

echo ""
echo "=== Adding transitions ==="
add_transition "DETECT" "READ_GRAPH" "plan_eligible" "task_count > 4 AND has_dependencies"
add_transition "DETECT" "SEQUENTIAL" "plan_too_small" "task_count <= 4 OR no_dependencies"
add_transition "READ_GRAPH" "COMPUTE_LEVELS" "graph_loaded"
add_transition "COMPUTE_LEVELS" "CHECK_CONFLICTS" "levels_computed"
add_transition "CHECK_CONFLICTS" "SPLIT_WAVES" "conflicts_found" "conflicts_detected > 0"
add_transition "CHECK_CONFLICTS" "DISPLAY_PLAN" "no_conflicts" "conflicts_detected == 0"
add_transition "SPLIT_WAVES" "DISPLAY_PLAN" "waves_split"
add_transition "DISPLAY_PLAN" "DISPATCH" "user_approves"

# 4. Link protocol to skill
echo ""
echo "=== Linking to skill ==="
curl -s -X PUT "$BASE_URL/api/protocols/$PROTOCOL_ID/skill" \
  -H "Content-Type: application/json" \
  -d "{\"skill_id\": \"$SKILL_ID\"}" > /dev/null
echo "✓ Protocol linked to skill $SKILL_ID"

echo ""
echo "=== Done ==="
echo "Protocol ID: $PROTOCOL_ID"
echo "Run: curl $BASE_URL/api/protocols/$PROTOCOL_ID | jq"
