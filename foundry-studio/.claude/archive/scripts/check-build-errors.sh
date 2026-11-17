#!/bin/bash
# Check for build errors after Claude finishes
# This hook runs when Claude stops responding

cd "$(dirname "$0")/../.." || exit 0

ERRORS_FOUND=0

# Check Java compilation in each service
for service in fleet-simulator put-service layout-service; do
  if [ -d "apps/$service" ]; then
    # Only check if there are .java files modified recently (last 5 minutes)
    RECENT_JAVA=$(find "apps/$service/src" -name "*.java" -mmin -5 2>/dev/null | head -1)
    if [ -n "$RECENT_JAVA" ]; then
      echo "Checking $service for compilation errors..."
      cd "apps/$service"
      OUTPUT=$(mvn compile -q 2>&1)
      EXIT_CODE=$?
      cd ../..

      if [ $EXIT_CODE -ne 0 ]; then
        echo ""
        echo "BUILD_ERROR: Compilation failed in $service"
        echo "$OUTPUT" | head -20
        echo ""
        ERRORS_FOUND=1
      fi
    fi
  fi
done

# Check TypeScript in frontend
if [ -d "apps/frontend" ]; then
  # Only check if there are .ts/.tsx files modified recently
  RECENT_TS=$(find "apps/frontend/src" \( -name "*.ts" -o -name "*.tsx" \) -mmin -5 2>/dev/null | head -1)
  if [ -n "$RECENT_TS" ]; then
    echo "Checking frontend for TypeScript errors..."
    cd apps/frontend
    OUTPUT=$(npm run check 2>&1)
    EXIT_CODE=$?
    cd ../..

    if [ $EXIT_CODE -ne 0 ]; then
      echo ""
      echo "BUILD_ERROR: TypeScript errors in frontend"
      echo "$OUTPUT" | head -20
      echo ""
      ERRORS_FOUND=1
    fi
  fi
fi

if [ $ERRORS_FOUND -eq 1 ]; then
  echo ""
  echo "Please fix the above errors before marking the task complete."
fi
