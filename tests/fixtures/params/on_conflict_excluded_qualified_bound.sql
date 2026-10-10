-- A bound parameter in the `DO UPDATE … WHERE`, compared against an
-- `EXCLUDED`-qualified column. `excluded` is not a catalog relation, so the type has
-- to come from the target's own column of that name — the pseudo-relation carries
-- the target's shape — and 18.6 reports `{bigint}` here, exactly as it does for the
-- target-qualified spelling.
--
-- The bound value discriminates the **resolution** as well as the type: `excluded.n`
-- is the would-be-inserted `99` while the stored row's `n` is `10`, so a predicate
-- that read the stored row would leave the row untouched and return no rows at all.
--
-- Before the `ON CONFLICT` clause was walked at all this statement reported
-- `ParameterDescription` n = 0 against the one parameter it carries. On the raw wire
-- that is the server's `08P01 bind message supplies 1 parameters, but prepared
-- statement "" requires 0`; under this harness it is tokio-postgres's own
-- `expected 0 parameters but got 1`, raised in `encode_bind` before any Bind message
-- is written, which the harness records as `CLIENT-FAULT` rather than a SQLSTATE.
--
-- What this fixture does **not** discriminate is *how* `excluded.n` resolves: with no
-- table named `excluded` in the catalog, a lookup through the action's own namespace
-- and a lookup through `TypeScope::Catalog`'s no-such-table fallback land on the same
-- column and report the same OID. The statement that tells them apart is
-- `on_conflict_excluded_not_shadowed_by_a_real_table.sql`, whose setup creates one.
-- @setup
CREATE TABLE oc_excl (id INT PRIMARY KEY, v TEXT, n BIGINT);
INSERT INTO oc_excl VALUES (1, 'old', 10);
-- @param $1 int8 99
-- @test
INSERT INTO oc_excl (id, v, n) VALUES (1, 'new', 99) ON CONFLICT (id) DO UPDATE SET v = excluded.v WHERE excluded.n = $1 RETURNING id, v, n;
