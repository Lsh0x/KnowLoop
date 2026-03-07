#!/usr/bin/env bash
# Seed script: Create the "system-inference" protocol for self-maintenance
# Requires: PO backend running on localhost:6600
# Usage: ./scripts/seed-system-inference-protocol.sh [PROJECT_ID] [SKILL_ID]

set -euo pipefail

BASE_URL="${PO_URL:-http://localhost:6600}"
PROJECT_ID="${1:-00333b5f-2d0a-4467-9c98-155e55d2b7e5}"
SKILL_ID="${2:-e615142b-33dd-4bcd-9254-5687b91fbc19}"

echo "=== Creating system-inference protocol ==="
echo "Project: $PROJECT_ID"
echo "Skill:   $SKILL_ID"
echo "API:     $BASE_URL"
echo ""

# 1. Create the protocol
PROTOCOL=$(curl -s -X POST "$BASE_URL/api/protocols" \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"system-inference\",
    \"description\": \"Self-maintenance protocol: audits knowledge graph gaps, backfills missing relations, infers new connections, recomputes scores, and produces health reports. Applies the system's own mechanics to maintain coherence.\",
    \"project_id\": \"$PROJECT_ID\",
    \"protocol_category\": \"system\",
    \"relevance_vector\": {\"phase\": 0.0, \"structure\": 0.8, \"domain\": 0.5, \"resource\": 0.3, \"lifecycle\": 1.0}
  }")

PROTOCOL_ID=$(echo "$PROTOCOL" | jq -r '.id')
echo "✓ Protocol created: $PROTOCOL_ID"

# 2. Add states
declare -A STATES
for state_def in \
  "TRIGGER:start:Evaluate trigger condition (scheduled, manual, or threshold-based):evaluate_trigger" \
  "AUDIT_GAPS:intermediate:Scan knowledge graph for gaps — orphan notes, decisions without AFFECTS, commits without TOUCHES, skills without members:audit_gaps" \
  "BACKFILL:intermediate:Execute backfill operations — backfill_touches, backfill_synapses, backfill_discussed, backfill_decision_embeddings, reindex_decisions, delete_meilisearch_orphans, reinforce_isomorphic, auto_anchor_notes:backfill_all" \
  "INFER_RELATIONS:intermediate:Infer new relations — predict_missing_links, detect_skills, enrich_communities, maintain_skills:infer_relations" \
  "RECOMPUTE:intermediate:Recompute scores — cleanup_sync_data, cleanup_builtin_calls, cleanup_cross_project_calls, update_staleness_scores, update_energy_scores, decay_synapses, update_fabric_scores:recompute_scores" \
  "HEALTH_CHECK:intermediate:Run health diagnostics — get_health, get_knowledge_gaps, get_risk_assessment. Persist report as Note (type observation):health_check" \
  "COMPLETE:terminal:Inference cycle complete. All gaps filled, scores recomputed, health report persisted.:complete"
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

# 3. Add transitions (with conditional guards for skip path)
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
  echo "  ✓ $from → $to ($trigger${guard:+ [$guard]})"
}

echo ""
echo "=== Adding transitions ==="
# Normal path: TRIGGER → AUDIT → BACKFILL → INFER → RECOMPUTE → HEALTH → COMPLETE
add_transition "TRIGGER" "AUDIT_GAPS" "start"
add_transition "AUDIT_GAPS" "BACKFILL" "gaps_found" "gaps_count > 0"
# Skip path: no gaps → jump straight to RECOMPUTE
add_transition "AUDIT_GAPS" "RECOMPUTE" "no_gaps" "gaps_count == 0"
add_transition "BACKFILL" "INFER_RELATIONS" "backfill_complete"
add_transition "INFER_RELATIONS" "RECOMPUTE" "inference_complete"
add_transition "RECOMPUTE" "HEALTH_CHECK" "recompute_complete"
add_transition "HEALTH_CHECK" "COMPLETE" "report_persisted"

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
echo "States: TRIGGER → AUDIT_GAPS → BACKFILL → INFER_RELATIONS → RECOMPUTE → HEALTH_CHECK → COMPLETE"
echo "Skip path: AUDIT_GAPS → RECOMPUTE (when gaps_count == 0)"
echo ""
echo "Run: curl $BASE_URL/api/protocols/$PROTOCOL_ID | jq"
