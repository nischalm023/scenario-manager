Foundry application lens regression inputs

These are authored application inputs preserved from World Proto's historical
lens tests. They are not WPS exports or production snapshots. The scenario
files retain their complete referenced layer closure. Native-reader regression
coverage uses them through studio/world-lenses/tests/fixtures.rs.

Application patterns: data/seed/application/lenses/world.lens
Application tests: studio/world-lenses
Generic matcher contracts: world-proto/lens/rust/tests

These inputs were copied byte for byte from their former homes; the executable
tests adapt paths and separate generic contracts from application assertions.
Generic assertions using synthetic entities remain in World Proto;
site count tables, handwritten census, app library metadata, explanations,
projections, budget equality and reference assertions remain in Foundry.

No fixture here is advertised as a server export. Changing an authored fixture
requires reviewing its independent expectations; do not regenerate expected
counts from the query implementation.

Runtime vocabulary distinction

Production world.lens retains Foundry's go.easyloop component and consumer
pattern names/variables. The WPS historical corpus uses legacy-wps/world.lens,
which is byte-identical to its source and retains its easyloop component.
Neither fixture family is silently renamed to make a test pass.

legacy-foundry/ holds the former embedded Foundry lens inputs that exist nowhere
else: its corner case file and the DFW6 and minimal EasyLoop simulator maps. Its
Cencora v3 and Walmart Dallas inputs were byte-identical to native/fixtures/, so
the tests read those instead.
