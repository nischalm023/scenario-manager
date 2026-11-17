# AI-Powered Simulation Analysis - Implementation Plan

## Overview

Add "Analyze with AI" functionality to simulation results that provides comprehensive Socratic analysis of simulation performance using Claude Sonnet 4.5.

## User Experience Flow

1. User runs simulation → Results page with tabs: "Overview", "Put", "Pick"
2. User clicks "Analyze with AI" button on Overview tab
3. Button disables, new "AI Analysis" tab appears (grayed out, shows "Incoming analysis...")
4. Backend calls Claude API with simulation metrics
5. Analysis completes → "AI Analysis" tab enables, displays markdown-formatted insights
6. If error → tab shows error message in red

## Architecture

```
Frontend (React)
  ↓ POST /simulation/analyze { metrics: SimulationMetric }
Backend (Java Spring Boot)
  ↓ HTTP POST to Anthropic API
Claude Sonnet 4.5
  ↓ Markdown analysis response
Backend
  ↓ { analysis: string }
Frontend (renders markdown)
```

---

## Task 1: Backend - Add Anthropic SDK Dependency

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/backend-java/pom.xml`

**Action:** Add Anthropic Java SDK dependency after existing dependencies (around line 50)

**Code:**
```xml
<!-- Anthropic SDK for AI Analysis -->
<dependency>
    <groupId>com.anthropic</groupId>
    <artifactId>anthropic-java</artifactId>
    <version>0.1.0</version>
</dependency>
```

**Note:** If the official Java SDK doesn't exist, we'll use HTTP client directly (OkHttp or Spring WebClient). Check Maven Central for `com.anthropic:anthropic-java` availability.

**Verification:**
```bash
cd apps/backend-java
mvn dependency:tree | grep anthropic
```

---

## Task 2: Backend - Add API Key Configuration

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/backend-java/src/main/resources/application.yml`

**Action:** Add Anthropic configuration section after logging configuration (after line 33)

**Code:**
```yaml
anthropic:
  api-key: ${ANTHROPIC_API_KEY:}
  model: claude-sonnet-4-5-20250929
  max-tokens: 4096
  temperature: 0.7
```

**Verification:**
```bash
# Set environment variable
export ANTHROPIC_API_KEY="sk-ant-..."

# Verify config loads
nx serve backend-java
# Check logs for "Anthropic API key configured: true"
```

---

## Task 3: Backend - Create Analysis Service

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/backend-java/src/main/java/com/greyorange/dessimulator/api/simulations/AnalysisService.java`

**Action:** Create new service class for AI analysis

**Code:**
```java
package com.greyorange.dessimulator.api.simulations;

import com.fasterxml.jackson.core.JsonProcessingException;
import com.fasterxml.jackson.databind.ObjectMapper;
import com.greyorange.dessimulator.simulator.metrics.SimulationMetrics.SimulationMetric;
import lombok.extern.slf4j.Slf4j;
import org.springframework.beans.factory.annotation.Value;
import org.springframework.http.HttpEntity;
import org.springframework.http.HttpHeaders;
import org.springframework.http.HttpMethod;
import org.springframework.http.MediaType;
import org.springframework.http.ResponseEntity;
import org.springframework.stereotype.Service;
import org.springframework.web.client.RestTemplate;

import java.util.List;
import java.util.Map;

@Slf4j
@Service
class AnalysisService {

  private static final String ANTHROPIC_API_URL = "https://api.anthropic.com/v1/messages";

  @Value("${anthropic.api-key}")
  private String apiKey;

  @Value("${anthropic.model}")
  private String model;

  @Value("${anthropic.max-tokens}")
  private int maxTokens;

  @Value("${anthropic.temperature}")
  private double temperature;

  private final ObjectMapper objectMapper;
  private final RestTemplate restTemplate;

  AnalysisService(ObjectMapper objectMapper) {
    this.objectMapper = objectMapper;
    this.restTemplate = new RestTemplate();
  }

  /**
   * Analyzes simulation metrics using Claude AI.
   *
   * @param metrics Complete simulation metrics including config
   * @return AI-generated analysis in markdown format
   * @throws RuntimeException if API call fails or API key not configured
   */
  public String analyzeSimulation(SimulationMetric metrics) {
    if (apiKey == null || apiKey.isBlank()) {
      throw new RuntimeException("Anthropic API key not configured. Set ANTHROPIC_API_KEY environment variable.");
    }

    log.info("Starting AI analysis for simulation with system type: {}",
             metrics.config().systemType());

    String prompt = buildAnalysisPrompt(metrics);
    String analysis = callClaudeAPI(prompt);

    log.info("AI analysis completed. Response length: {} characters", analysis.length());
    return analysis;
  }

  private String buildAnalysisPrompt(SimulationMetric metrics) {
    try {
      String metricsJson = objectMapper.writerWithDefaultPrettyPrinter()
          .writeValueAsString(metrics);

      return """
          You are an expert warehouse automation analyst. Analyze the following simulation results
          and provide comprehensive insights in markdown format.

          # Simulation Data

          ```json
          %s
          ```

          # Analysis Requirements

          Provide a thorough Socratic analysis covering:

          ## 1. Performance Summary
          - Overall system performance (picks/hour, operator utilization, picks per face)
          - Key strengths and achievements
          - Critical bottlenecks identified

          ## 2. Bottleneck Analysis
          - Identify primary bottlenecks (robot utilization, operator idle time, storage inefficiency)
          - Explain WHY each bottleneck exists based on the metrics
          - Quantify the impact of each bottleneck on throughput

          ## 3. Configuration Optimization
          - Specific recommendations to improve performance
          - Suggested parameter changes (robot count, speed, operator time, bins)
          - Expected impact of each recommendation

          ## 4. Root Cause Analysis
          - Why are certain metrics low or high?
          - What are the causal relationships between metrics?
          - Are there any surprising or unexpected patterns?

          ## 5. System Comparison Insights
          - Is this RTP or Relay system optimal for this workload?
          - Would the alternative system type perform better? Why or why not?
          - What are the trade-offs specific to this configuration?

          ## 6. Actionable Next Steps
          - Prioritized list of improvements to test
          - Quick wins vs. long-term optimizations
          - Specific simulation runs to try next

          Use data from the metrics to support every claim. Be specific with numbers.
          Format output in clean markdown with headers, bullet points, and bold emphasis for key findings.
          """.formatted(metricsJson);

    } catch (JsonProcessingException e) {
      throw new RuntimeException("Failed to serialize metrics for AI analysis", e);
    }
  }

  private String callClaudeAPI(String prompt) {
    try {
      HttpHeaders headers = new HttpHeaders();
      headers.setContentType(MediaType.APPLICATION_JSON);
      headers.set("x-api-key", apiKey);
      headers.set("anthropic-version", "2023-06-01");

      Map<String, Object> requestBody = Map.of(
          "model", model,
          "max_tokens", maxTokens,
          "temperature", temperature,
          "messages", List.of(
              Map.of(
                  "role", "user",
                  "content", prompt
              )
          )
      );

      HttpEntity<Map<String, Object>> entity = new HttpEntity<>(requestBody, headers);

      ResponseEntity<Map> response = restTemplate.exchange(
          ANTHROPIC_API_URL,
          HttpMethod.POST,
          entity,
          Map.class
      );

      if (!response.getStatusCode().is2xxSuccessful() || response.getBody() == null) {
        throw new RuntimeException("Claude API returned non-success status: " + response.getStatusCode());
      }

      // Extract text from response: { content: [{ type: "text", text: "..." }] }
      @SuppressWarnings("unchecked")
      List<Map<String, Object>> content = (List<Map<String, Object>>) response.getBody().get("content");
      if (content == null || content.isEmpty()) {
        throw new RuntimeException("Claude API returned empty content");
      }

      return (String) content.get(0).get("text");

    } catch (Exception e) {
      log.error("Failed to call Claude API", e);
      throw new RuntimeException("AI analysis failed: " + e.getMessage(), e);
    }
  }
}
```

**Verification:**
```bash
# Create a test metrics JSON file
cat > /tmp/test-metrics.json << 'EOF'
{
  "config": {
    "systemType": "RTP",
    "simulationDuration": 14400,
    "scenarioId": "test"
  },
  "actualSimulationDuration": 14350.5,
  "taskMetrics": {
    "totalTasksCompleted": 5000,
    "taskCompletionRate": 0.35
  }
}
EOF

# Test manually via curl after server starts
curl -X POST http://localhost:8080/simulation/analyze \
  -H "Content-Type: application/json" \
  -d @/tmp/test-metrics.json
```

---

## Task 4: Backend - Add Analysis Controller Endpoint

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/backend-java/src/main/java/com/greyorange/dessimulator/api/simulations/SimulationsController.java`

**Action:** Add new endpoint after `getStatus()` method (after line 60)

**Code:**
```java
  private final AnalysisService analysisService;

  SimulationsController(
      SimulationService simulationService,
      ComparisonService comparisonService,
      AnalysisService analysisService) {
    this.simulationService = simulationService;
    this.comparisonService = comparisonService;
    this.analysisService = analysisService;
  }

  @PostMapping("/simulation/analyze")
  @Tag(name = "Simulation")
  @Operation(summary = "Analyze Simulation Results with AI")
  ResponseEntity<ApiResponse<AnalysisResponse>> analyzeSimulation(
      @Valid @RequestBody AnalysisRequest request) {
    try {
      String analysis = analysisService.analyzeSimulation(request.metrics());
      return ResponseEntity.ok(ApiResponse.success(new AnalysisResponse(analysis)));
    } catch (Exception e) {
      return ResponseEntity.ok(
          ApiResponse.error("AI analysis failed: " + e.getMessage())
      );
    }
  }

  record AnalysisRequest(@NotNull SimulationMetric metrics) {}

  record AnalysisResponse(String analysis) {}
```

**Note:** Also update the constructor at line 39 to accept `AnalysisService`.

**Verification:**
```bash
# Restart server
nx serve backend-java

# Check OpenAPI spec includes new endpoint
curl http://localhost:8080/openapi.json | grep "/simulation/analyze"
```

---

## Task 5: Backend - Compile and Test

**Action:** Rebuild and verify endpoint is accessible

**Commands:**
```bash
cd apps/backend-java
mvn clean compile -DskipTests
nx serve backend-java
```

**Verification:**
```bash
# Check server starts without errors
curl http://localhost:8080/health

# Verify OpenAPI docs include new endpoint
open http://localhost:8080/docs
# Look for POST /simulation/analyze endpoint
```

---

## Task 6: Frontend - Regenerate TypeScript Types

**Action:** Generate TypeScript types from updated OpenAPI spec

**Commands:**
```bash
# Ensure server is running on port 8080
npx openapi-typescript http://localhost:8080/openapi.json \
  -o apps/frontend/src/types/generated-api.ts
```

**Verification:**
```bash
# Check generated types include AnalysisRequest and AnalysisResponse
grep -A 5 "AnalysisRequest\|AnalysisResponse" apps/frontend/src/types/generated-api.ts
```

---

## Task 7: Frontend - Create API Hook for Analysis

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/frontend/src/api/simulationApi.ts` (or create if doesn't exist)

**Action:** Create API hook for calling analysis endpoint

**Code:**
```typescript
import type { SimulationMetric } from '../types/generated'

export interface AnalysisRequest {
  metrics: SimulationMetric
}

export interface AnalysisResponse {
  analysis: string
}

export interface ApiResponse<T> {
  success: boolean
  data?: T
  error?: string
}

export async function analyzeSimulation(
  metrics: SimulationMetric
): Promise<AnalysisResponse> {
  const response = await fetch('http://localhost:8080/simulation/analyze', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
    },
    body: JSON.stringify({ metrics }),
  })

  if (!response.ok) {
    throw new Error(`Analysis failed: ${response.statusText}`)
  }

  const result: ApiResponse<AnalysisResponse> = await response.json()

  if (!result.success || !result.data) {
    throw new Error(result.error || 'Analysis failed')
  }

  return result.data
}
```

**Verification:**
```bash
# Check TypeScript compiles without errors
npm run typecheck
```

---

## Task 8: Frontend - Create AI Analysis Tab Component

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/frontend/src/pages/simulator/components/AIAnalysis.tsx`

**Action:** Create new component for displaying AI analysis

**Code:**
```typescript
import { AlertCircle } from 'lucide-react'
import type React from 'react'
import ReactMarkdown from 'react-markdown'

interface AIAnalysisProps {
  analysis: string | null
  error: string | null
  isLoading: boolean
}

const AIAnalysis: React.FC<AIAnalysisProps> = ({ analysis, error, isLoading }) => {
  if (isLoading) {
    return (
      <div className="p-6 flex items-center justify-center min-h-[400px]">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-orange-500 mx-auto mb-4"></div>
          <p className="text-gray-600">Analyzing simulation results with AI...</p>
          <p className="text-sm text-gray-500 mt-2">This may take 10-30 seconds</p>
        </div>
      </div>
    )
  }

  if (error) {
    return (
      <div className="p-6">
        <div className="bg-red-50 border border-red-200 rounded-lg p-6 flex items-start">
          <AlertCircle className="w-5 h-5 text-red-500 mr-3 flex-shrink-0 mt-0.5" />
          <div>
            <h3 className="text-red-900 font-medium mb-1">Analysis Failed</h3>
            <p className="text-red-700 text-sm">{error}</p>
            <p className="text-red-600 text-xs mt-2">
              Please check that ANTHROPIC_API_KEY is configured in the backend.
            </p>
          </div>
        </div>
      </div>
    )
  }

  if (!analysis) {
    return (
      <div className="p-6 flex items-center justify-center min-h-[400px]">
        <div className="text-center text-gray-500">
          <p>Click "Analyze with AI" to get comprehensive insights</p>
        </div>
      </div>
    )
  }

  return (
    <div className="p-6">
      <div className="prose prose-sm max-w-none">
        <ReactMarkdown
          components={{
            h1: ({ children }) => (
              <h1 className="text-2xl font-bold text-gray-900 mb-4 mt-6">
                {children}
              </h1>
            ),
            h2: ({ children }) => (
              <h2 className="text-xl font-semibold text-gray-900 mb-3 mt-5">
                {children}
              </h2>
            ),
            h3: ({ children }) => (
              <h3 className="text-lg font-medium text-gray-800 mb-2 mt-4">
                {children}
              </h3>
            ),
            p: ({ children }) => (
              <p className="text-gray-700 mb-3 leading-relaxed">{children}</p>
            ),
            ul: ({ children }) => (
              <ul className="list-disc list-inside mb-3 space-y-1">{children}</ul>
            ),
            ol: ({ children }) => (
              <ol className="list-decimal list-inside mb-3 space-y-1">{children}</ol>
            ),
            li: ({ children }) => (
              <li className="text-gray-700 ml-4">{children}</li>
            ),
            strong: ({ children }) => (
              <strong className="font-semibold text-gray-900">{children}</strong>
            ),
            code: ({ children }) => (
              <code className="bg-gray-100 px-1.5 py-0.5 rounded text-sm font-mono text-gray-800">
                {children}
              </code>
            ),
            pre: ({ children }) => (
              <pre className="bg-gray-100 p-4 rounded-lg overflow-x-auto mb-4">
                {children}
              </pre>
            ),
          }}
        >
          {analysis}
        </ReactMarkdown>
      </div>
    </div>
  )
}

export default AIAnalysis
```

**Verification:**
```bash
# Check component compiles
npm run check
```

---

## Task 9: Frontend - Install react-markdown Dependency

**Action:** Add react-markdown for rendering markdown analysis

**Commands:**
```bash
npm install react-markdown
npm install -D @types/react-markdown
```

**Verification:**
```bash
# Check dependency installed
npm list react-markdown
```

---

## Task 10: Frontend - Update SimulationResults Component

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/apps/frontend/src/pages/simulator/components/SimulationResults.tsx`

**Action:** Add AI Analysis tab and "Analyze with AI" button

**Changes:**

1. Import new components and API (after line 10):
```typescript
import { Sparkles } from 'lucide-react'
import { analyzeSimulation } from '../../../api/simulationApi'
import AIAnalysis from './AIAnalysis'
```

2. Add state for analysis (after line 23):
```typescript
  const [analysisState, setAnalysisState] = useState<{
    analysis: string | null
    error: string | null
    isLoading: boolean
    hasRequested: boolean
  }>({
    analysis: null,
    error: null,
    isLoading: false,
    hasRequested: false,
  })
```

3. Add analysis handler function (after state declarations):
```typescript
  const handleAnalyzeClick = async () => {
    if (!metrics) return

    setAnalysisState({
      analysis: null,
      error: null,
      isLoading: true,
      hasRequested: true,
    })

    // Switch to AI Analysis tab
    setActiveTab('ai-analysis')

    try {
      const response = await analyzeSimulation(metrics)
      setAnalysisState({
        analysis: response.analysis,
        error: null,
        isLoading: false,
        hasRequested: true,
      })
    } catch (error) {
      setAnalysisState({
        analysis: null,
        error: error instanceof Error ? error.message : 'Analysis failed',
        isLoading: false,
        hasRequested: true,
      })
    }
  }
```

4. Update tabs array (replace line 39-43):
```typescript
  const tabs = [
    { id: 'overview', label: 'Overview' },
    { id: 'put', label: 'Put' },
    { id: 'picks', label: 'Pick' },
    ...(analysisState.hasRequested
      ? [{ id: 'ai-analysis', label: 'AI Analysis' }]
      : []),
  ]
```

5. Add "Analyze with AI" button in Overview tab content (after line 80, before closing tag):
```typescript
            <TabsContent value="overview" className="flex-1 bg-white overflow-y-auto m-0 min-h-0">
              <div className="p-6 border-b border-gray-200 bg-white">
                <button
                  onClick={handleAnalyzeClick}
                  disabled={analysisState.isLoading}
                  className="inline-flex items-center px-4 py-2 bg-purple-600 hover:bg-purple-700 disabled:bg-gray-400 disabled:cursor-not-allowed text-white rounded-lg font-medium transition-colors"
                >
                  <Sparkles className="w-4 h-4 mr-2" />
                  {analysisState.isLoading ? 'Analyzing...' : 'Analyze with AI'}
                </button>
              </div>
              <ResultsOverview
                results={results}
                formatNumber={formatNumber}
                formatDuration={formatDuration}
              />
            </TabsContent>
```

6. Add AI Analysis tab content (after PickResults TabsContent, before closing Tabs tag):
```typescript
            <TabsContent value="ai-analysis" className="flex-1 bg-white overflow-y-auto m-0 min-h-0">
              <AIAnalysis
                analysis={analysisState.analysis}
                error={analysisState.error}
                isLoading={analysisState.isLoading}
              />
            </TabsContent>
```

**Verification:**
```bash
npm run check
npm run dev
# Navigate to simulator, run simulation, check "Analyze with AI" button appears
```

---

## Task 11: End-to-End Integration Test

**Actions:**
1. Start backend with API key configured
2. Start frontend
3. Run simulation
4. Click "Analyze with AI" button
5. Verify tab appears and shows loading state
6. Wait for analysis to complete
7. Verify markdown renders correctly

**Commands:**
```bash
# Terminal 1: Backend
export ANTHROPIC_API_KEY="sk-ant-..."
nx serve backend-java

# Terminal 2: Frontend
npm run dev

# Browser: http://localhost:4200
# 1. Navigate to Simulator
# 2. Configure and run simulation
# 3. When results appear, click "Analyze with AI"
# 4. Verify:
#    - Button disables
#    - "AI Analysis" tab appears (grayed out)
#    - Tab shows loading spinner
#    - After 10-30s, analysis appears in markdown
#    - Markdown is properly formatted with headers, bullets, bold text
```

**Expected Result:**
- Analysis tab shows comprehensive report covering:
  - Performance summary
  - Bottleneck analysis
  - Configuration optimization suggestions
  - Root cause analysis
  - System comparison insights
  - Actionable next steps

**Error Case Testing:**
```bash
# Test without API key
unset ANTHROPIC_API_KEY
nx serve backend-java

# Click "Analyze with AI" → should show error message in tab
```

---

## Task 12: Documentation Update

**File:** `/Users/kumar.d/Work/grey_orange/greymatter/code/queue_simulator/CLAUDE.md`

**Action:** Document new AI analysis feature

**Code:** Add to "API Layer" section (after line 50):
```markdown
### AI Analysis Endpoint

**POST /simulation/analyze**: AI-powered simulation analysis using Claude Sonnet 4.5

Request body:
```json
{
  "metrics": SimulationMetric
}
```

Response:
```json
{
  "success": true,
  "data": {
    "analysis": "# Performance Summary\n\n..."
  }
}
```

**Configuration**: Set `ANTHROPIC_API_KEY` environment variable before starting backend.

**Frontend Integration**: "Analyze with AI" button on Results Overview tab triggers analysis and displays markdown-formatted insights in dedicated "AI Analysis" tab.
```

**Verification:**
```bash
cat CLAUDE.md | grep -A 10 "AI Analysis"
```

---

## Summary of Files Changed

### Backend (Java)
1. `pom.xml` - Add Anthropic SDK dependency (or HTTP client)
2. `application.yml` - Add API key configuration
3. `AnalysisService.java` (NEW) - AI analysis service
4. `SimulationsController.java` - Add `/simulation/analyze` endpoint

### Frontend (React/TypeScript)
5. `package.json` - Add react-markdown dependency
6. `generated-api.ts` - Regenerate from OpenAPI spec
7. `simulationApi.ts` (NEW) - API client for analysis endpoint
8. `AIAnalysis.tsx` (NEW) - Analysis display component
9. `SimulationResults.tsx` - Add tab and button integration

### Documentation
10. `CLAUDE.md` - Document AI analysis feature

---

## Rollback Plan

If any issues arise:

1. **Backend fails to start**: Remove Anthropic dependency from `pom.xml`, rebuild
2. **API calls fail**: Check `ANTHROPIC_API_KEY` is set, verify with `echo $ANTHROPIC_API_KEY`
3. **Frontend errors**: Remove `react-markdown` import, comment out AI Analysis tab
4. **Type errors**: Regenerate types with `npx openapi-typescript`

To completely disable feature:
```bash
# Backend: Comment out AnalysisService and /simulation/analyze endpoint
# Frontend: Remove "Analyze with AI" button and AI Analysis tab
```

---

## Testing Checklist

- [ ] Backend compiles without errors (`mvn clean compile`)
- [ ] Server starts with API key configured
- [ ] OpenAPI spec includes `/simulation/analyze` endpoint
- [ ] Frontend types regenerated successfully
- [ ] Frontend compiles without errors (`npm run check`)
- [ ] "Analyze with AI" button appears on Overview tab
- [ ] Button click creates "AI Analysis" tab (disabled state)
- [ ] Loading spinner shows during analysis
- [ ] Analysis completes and markdown renders correctly
- [ ] Error handling works when API key not configured
- [ ] Headers, bullets, bold text render properly in markdown
- [ ] Tab remains accessible after analysis completes

---

## Future Enhancements (Out of Scope)

1. **Streaming**: Use SSE or WebSocket for real-time analysis display
2. **Caching**: Cache analysis results for same metrics to save API costs
3. **Interactive Chat**: Allow follow-up questions on analysis
4. **Export**: Download analysis as PDF or markdown file
5. **Comparison Mode**: Analyze multiple simulation runs side-by-side
6. **Custom Prompts**: Allow users to ask specific analysis questions
