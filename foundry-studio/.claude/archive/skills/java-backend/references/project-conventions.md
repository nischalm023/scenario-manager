# Project Conventions

Rules that differ from typical Spring Boot projects. Violating these causes ArchUnit failures or codebase inconsistency.

---

## API Layer

- Controllers implement **OpenAPI-generated interfaces** (e.g., `implements SimulationApi, ComparisonApi` from `com.greyorange.dessimulator.api.generated`). Do NOT add `@RequestMapping` or `@GetMapping` — routes live in the generated interfaces.
- All controller classes are **package-private** (no `public` modifier) — Spring component-scans the package.
- URL path strings live in `ApiPaths` constants (`ApiPaths.SIMULATION`, etc.). Never hardcode paths.
- Every endpoint returns `ResponseEntity<ApiResponse<T>>` using `ApiResponse.success(data)` or `ApiResponse.error(msg)`.
- Controllers **throw exceptions**; `GlobalExceptionHandler` (`@ControllerAdvice`) maps them to HTTP status codes. Never try-catch in controllers.
- Constructor injection only — no `@Autowired`, no `@RequiredArgsConstructor`.

---

## DTO Layer

- DTOs are **records** with a compact constructor that sets defaults for null fields. Not `@Builder`.
  ```java
  public record SimulationConfigDTO(String name, Integer binsCount, /* ... */) {
      public SimulationConfigDTO {
          if (binsCount == null) binsCount = 10;
      }
  }
  ```
- DTOs carry a `toCoreConfig()` conversion method — no separate mapper classes.
- Enum-string validation uses a custom `@Constraint` annotation + `ConstraintValidator` pair (e.g., `@ValidOrderSelectionAlgorithm` + `OrderSelectionAlgorithmValidator`). Not `@Pattern` regex.

---

## Configuration

- All simulation config records (`SimulationConfig`, `RtpRobotConfig`, `RelayRobotConfig`, `OperatorConfig`, `AlgorithmConfig`) are nested inside the `SimulationConfigs` utility class — not one file per config.
- `@ConfigurationProperties` beans use `@Getter`/`@Setter` (Lombok), not `@Value` field injection.
- CORS is intentionally wide-open (`*`) in `CorsConfig` — Envoy gateway handles restriction in production. Do NOT add restrictive CORS to controllers.

---

## Domain Model

- Every enum follows the triple pattern: `@JsonValue` getter + `@JsonCreator` static `fromValue(String)` factory + snake_case `value` field. See `OrderSelectionAlgorithm.java`.
- Sealed interfaces for state/location modeling (e.g., `ToteLocation` with inner records `InRack`, `AtRelay`, `AtStation`, `WithRobot`). Use exhaustive `switch` pattern matching, not `instanceof` chains.
- Domain records validate in compact constructor with `IllegalArgumentException` — Jakarta annotations are for DTOs only.
- Utility classes: `private` constructor throwing `UnsupportedOperationException("Utility class")`.

---

## Testing

- **ArchUnit rules** in `ArchitectureTest.java` enforce that `domain` and `simulator` packages have **zero Spring dependencies**. Importing Spring in domain code compiles but fails tests.
- Integration tests use `@SpringBootTest` + `@AutoConfigureMockMvc`, NOT `@WebMvcTest` — gRPC context is required.
- Mockito **manual style**: `Mockito.mock()` / `Mockito.when()` in `@BeforeEach`. No `@ExtendWith(MockitoExtension.class)`, no `@Mock`/`@InjectMocks`.
- Service-dependent tests are marked `@Disabled("Requires running put-service")` — keep these, don't delete them.
- Navigation gRPC client uses **async** stub inside simulation loop — blocking stub hangs Desmo-J thread.
- Pass `null` for gRPC clients in unit tests — `WarehouseBuilder` handles gracefully.

---

## Source Files

| Convention | Reference file |
|------------|---------------|
| OpenAPI interface impl | `api/simulations/SimulationsController.java` |
| Response wrapper | `api/common/ApiResponse.java` |
| Exception mapping | `api/common/GlobalExceptionHandler.java` |
| Compact constructor defaults | `api/dto/SimulationConfigDTO.java` |
| Custom validation | `api/dto/ValidOrderSelectionAlgorithm.java` |
| Nested configs | `simulator/config/SimulationConfigs.java` |
| Sealed interface | `domain/inventory/ToteLocation.java` |
| Enum triple | `domain/order/OrderSelectionAlgorithm.java` |
| ArchUnit enforcement | `src/test/.../ArchitectureTest.java` |

All paths relative to `apps/fleet-simulator/src/main/java/com/greyorange/dessimulator/`.
