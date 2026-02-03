#!/bin/bash
#
# Vaanyze Pilot Runner
#
# Main entrypoint for running Vaanyze pilots.
#
# Usage:
#   ./run_pilot.sh observe [OPTIONS]        # Run observation (default)
#   ./run_pilot.sh apply-one [OPTIONS]      # Apply single recommendation
#   ./run_pilot.sh case-study [OPTIONS]     # Generate case study
#   ./run_pilot.sh doctor [OPTIONS]         # Run doctor checks only
#
# Modes:
#   observe     Run doctor, then observe for N hours (default: 48)
#   apply-one   Run doctor, then apply a single recommendation
#   case-study  Generate case study from collected artifacts
#   doctor      Run readiness checks only
#
# Examples:
#   ./run_pilot.sh observe --hours 48
#   ./run_pilot.sh apply-one --pick-top-validated --verification-minutes 60
#   ./run_pilot.sh case-study --input-dir ./pilot-artifacts/20240115_120000
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Colors
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    BOLD='\033[1m'
    NC='\033[0m'
else
    RED='' GREEN='' YELLOW='' BLUE='' BOLD='' NC=''
fi

# Default mode
MODE="observe"

# Parse mode (first argument if not a flag)
if [[ $# -gt 0 ]] && [[ ! "$1" =~ ^-- ]]; then
    MODE="$1"
    shift
fi

# Environment defaults
export VAANYZE_BASE_URL="${VAANYZE_BASE_URL:-http://localhost:9090}"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
OUTPUT_DIR="${SCRIPT_DIR}/../../pilot-artifacts/${TIMESTAMP}"

# Parse common options
HOURS=48
INTERVAL_MINUTES=15
VERIFICATION_MINUTES=30
RECOMMENDATION_ID=""
PICK_TOP=false
FORCE=false
INPUT_DIR=""
REDACT_LEVEL="normal"

while [[ $# -gt 0 ]]; do
    case $1 in
        --hours) HOURS="$2"; shift 2 ;;
        --interval) INTERVAL_MINUTES="$2"; shift 2 ;;
        --verification-minutes) VERIFICATION_MINUTES="$2"; shift 2 ;;
        --recommendation-id) RECOMMENDATION_ID="$2"; shift 2 ;;
        --pick-top-validated) PICK_TOP=true; shift ;;
        --force) FORCE=true; shift ;;
        --input-dir) INPUT_DIR="$2"; shift 2 ;;
        --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
        --redact-level) REDACT_LEVEL="$2"; shift 2 ;;
        -h|--help)
            cat << EOF
Vaanyze Pilot Runner

Usage:
  $0 [MODE] [OPTIONS]

Modes:
  observe     Run observation session (default)
  apply-one   Apply single validated recommendation
  case-study  Generate case study from artifacts
  doctor      Run readiness checks only

Options for 'observe':
  --hours N             Observation duration (default: 48)
  --interval N          Snapshot interval in minutes (default: 15)
  --output-dir DIR      Output directory

Options for 'apply-one':
  --recommendation-id ID   Specific recommendation to apply
  --pick-top-validated     Auto-pick top validated recommendation
  --verification-minutes N Verification window (default: 30)
  --force                  Skip confirmations

Options for 'case-study':
  --input-dir DIR       Pilot artifacts directory (required)
  --redact-level LEVEL  none, normal, strict (default: normal)

Environment:
  VAANYZE_BASE_URL      Vaanyze endpoint (default: http://localhost:9090)
  DATABASE_URL          PostgreSQL connection (for direct queries)

Examples:
  $0 observe --hours 24
  $0 apply-one --pick-top-validated
  $0 case-study --input-dir ./pilot-artifacts/20240115_120000

EOF
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Banner
echo ""
echo -e "${BOLD}╔═══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║                    Vaanyze Pilot Runner                       ║${NC}"
echo -e "${BOLD}╚═══════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "Mode:        ${GREEN}${MODE}${NC}"
echo -e "Vaanyze URL: ${VAANYZE_BASE_URL}"
echo -e "Output:      ${OUTPUT_DIR}"
echo ""

mkdir -p "$OUTPUT_DIR"

# ============================================================================
# Mode: doctor
# ============================================================================
run_doctor() {
    local force_flag=""
    [[ "$FORCE" == "true" ]] && force_flag="--force"
    
    echo -e "${BOLD}Running readiness checks...${NC}"
    echo ""
    
    "${SCRIPT_DIR}/pilot_doctor.sh" --output-dir "$OUTPUT_DIR" $force_flag
    return $?
}

# ============================================================================
# Mode: observe
# ============================================================================
run_observe() {
    # First run doctor
    if ! run_doctor; then
        echo ""
        echo -e "${RED}Doctor checks failed. Use --force to continue anyway.${NC}"
        exit 1
    fi
    
    echo ""
    echo -e "${BOLD}Starting observation session...${NC}"
    echo ""
    
    "${SCRIPT_DIR}/pilot_observe.sh" \
        --hours "$HOURS" \
        --interval "$INTERVAL_MINUTES" \
        --output-dir "$OUTPUT_DIR"
    
    # After observation, optionally generate initial case study
    echo ""
    echo -e "${BOLD}Generating preliminary case study...${NC}"
    "${SCRIPT_DIR}/pilot_case_study.sh" \
        --input-dir "$OUTPUT_DIR" \
        --redact-level "$REDACT_LEVEL"
    
    echo ""
    echo -e "${GREEN}Observation complete!${NC}"
    echo ""
    echo "Artifacts saved to: ${OUTPUT_DIR}"
    echo ""
    echo "Next steps:"
    echo "  1. Review case study: ${OUTPUT_DIR}/case-study.md"
    echo "  2. Apply a recommendation: $0 apply-one --input-dir ${OUTPUT_DIR} --pick-top-validated"
}

# ============================================================================
# Mode: apply-one
# ============================================================================
run_apply_one() {
    # First run doctor
    if ! run_doctor; then
        echo ""
        echo -e "${YELLOW}Doctor checks failed. Checking if we can proceed...${NC}"
        
        if [[ "$FORCE" != "true" ]]; then
            echo -e "${RED}Use --force to continue despite warnings.${NC}"
            exit 1
        fi
    fi
    
    echo ""
    echo -e "${BOLD}Running guided apply workflow...${NC}"
    echo ""
    
    # Build apply arguments
    APPLY_ARGS=("--output-dir" "$OUTPUT_DIR" "--verification-minutes" "$VERIFICATION_MINUTES")
    
    if [[ -n "$RECOMMENDATION_ID" ]]; then
        APPLY_ARGS+=("--recommendation-id" "$RECOMMENDATION_ID")
    elif [[ "$PICK_TOP" == "true" ]]; then
        APPLY_ARGS+=("--pick-top-validated")
    else
        echo -e "${RED}Error: Must specify --recommendation-id or --pick-top-validated${NC}"
        exit 1
    fi
    
    [[ "$FORCE" == "true" ]] && APPLY_ARGS+=("--force")
    
    "${SCRIPT_DIR}/pilot_apply_one.sh" "${APPLY_ARGS[@]}"
    
    # Generate updated case study
    echo ""
    echo -e "${BOLD}Updating case study...${NC}"
    "${SCRIPT_DIR}/pilot_case_study.sh" \
        --input-dir "$OUTPUT_DIR" \
        --redact-level "$REDACT_LEVEL"
    
    echo ""
    echo -e "${GREEN}Apply workflow complete!${NC}"
    echo "Artifacts saved to: ${OUTPUT_DIR}"
}

# ============================================================================
# Mode: case-study
# ============================================================================
run_case_study() {
    if [[ -z "$INPUT_DIR" ]]; then
        # Try to find most recent pilot artifacts
        ARTIFACTS_ROOT="${SCRIPT_DIR}/../../pilot-artifacts"
        if [[ -d "$ARTIFACTS_ROOT" ]]; then
            INPUT_DIR=$(ls -1td "${ARTIFACTS_ROOT}"/*/ 2>/dev/null | head -1)
        fi
        
        if [[ -z "$INPUT_DIR" ]]; then
            echo -e "${RED}Error: --input-dir is required (no recent artifacts found)${NC}"
            exit 1
        fi
        
        echo "Using most recent artifacts: ${INPUT_DIR}"
    fi
    
    "${SCRIPT_DIR}/pilot_case_study.sh" \
        --input-dir "$INPUT_DIR" \
        --output-dir "${OUTPUT_DIR}" \
        --redact-level "$REDACT_LEVEL"
}

# ============================================================================
# Main
# ============================================================================

case "$MODE" in
    doctor)
        run_doctor
        ;;
    observe)
        run_observe
        ;;
    apply-one|apply)
        run_apply_one
        ;;
    case-study|casestudy)
        run_case_study
        ;;
    *)
        echo -e "${RED}Unknown mode: ${MODE}${NC}"
        echo "Valid modes: observe, apply-one, case-study, doctor"
        exit 1
        ;;
esac
