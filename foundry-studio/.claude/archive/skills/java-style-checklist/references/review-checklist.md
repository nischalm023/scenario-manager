# Java Code Review Checklist

Use when reviewing or writing Java code. Organized by concern.

## Class Level

- [ ] Class has multi-line Javadoc explaining purpose and responsibilities
- [ ] Key algorithms or patterns mentioned in class doc
- [ ] Uses `<p>` tags and bullet points for structure

## Methods

- [ ] Simple public methods have single-line Javadoc
- [ ] Complex public methods have multi-line Javadoc with `@param`/`@return`
- [ ] No boilerplate docs that just repeat parameter names
- [ ] Complex private methods document WHY and algorithm
- [ ] Simple private helpers are NOT over-documented

## Comments

- [ ] Comments explain WHY, not WHAT
- [ ] No comments that just repeat code
- [ ] Critical patterns (blocking calls, state changes) are marked
- [ ] Phase markers used for multi-step operations
- [ ] TODOs have actionable context
- [ ] SAFETY NOTEs for threading/concurrency assumptions

## Structure

- [ ] Section headers (40 `=` chars) for files >200 lines with 5+ methods
- [ ] Functional grouping (related public + private together)
- [ ] Canonical order: Constants > Fields > Records > Constructor > Domain Sections > Helpers
- [ ] 1 blank line between methods, 2 between sections

## Modern Java

- [ ] Records for immutable data (DTOs, configs, value objects)
- [ ] Classes for entities with mutable state or lifecycle
- [ ] `@Getter` instead of manual getters
- [ ] `@RequiredArgsConstructor` with final fields for DI
- [ ] `@Slf4j` for logging
- [ ] Field-level `@Setter` only (not class-level)
- [ ] No `@Data`, no `@AllArgsConstructor`
- [ ] All fields `final` where possible
- [ ] `Optional<T>` for possibly absent return values (not fields, not collections)
- [ ] Empty collections returned (not null or `Optional<List>`)
- [ ] Sealed types for closed hierarchies
- [ ] Pattern matching with instanceof (not manual casting)
- [ ] Switch expressions for exhaustive handling
- [ ] Defensive copies when exposing mutable collections
- [ ] Immutable collections (`List.of`, `Set.of`, `Map.of`)
- [ ] `@Builder` for complex construction (4+ fields)
- [ ] Enums for type-safe constants with behavior

## Records/DTOs

- [ ] Simple records undocumented (self-explanatory field names)
- [ ] Complex records have `@param` docs for non-obvious fields
- [ ] No boilerplate record documentation

## Formatting

- [ ] Line length under 100 chars (Google Java Format)
- [ ] Ran `mvn spotless:apply`
- [ ] Imports grouped: java.* > javax.* > external libs > internal packages
