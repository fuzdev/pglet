-- Index selection must ask what access method an index **is**. A `USING gin` index
-- stores one *decomposed* entry per object key, not one per row, so serving an
-- ordered scan from it returned each row once per key, in key order: `1,3,3,3,2,1`
-- against Postgres's `2,1,3`.
--
-- Every statement here is accepted by both engines — no refused DDL, no failed
-- back-fill, no empty index — so unlike the B-tree-encoder gaps this class is
-- pinnable by an ordinary differential fixture.
--
-- Three things in the setup are load-bearing:
--   * `NOT NULL` — the ORDER BY pushdown declines a nullable key column outright
--     (a plain index omits NULL-keyed rows), so a nullable column hides the bug.
--   * the key counts differ (2 / 1 / 3) — duplication changes the row *count*, so a
--     reader that only checks the order still sees it.
--   * Postgres's order (`2,1,3`) is neither a prefix nor a reverse of the buggy
--     one, so neither failure half can masquerade as the other.
--
-- Deliberately **not** `-- @profile-lock`: after the fix this plans to `Sort(Scan)`
-- on both profiles, so the directive's criterion (a *different* plan) does not
-- apply — and leaving it off is the stronger guard, since a revert makes the
-- lock run's completeness check 1 fail too (the `@test` would plan to an
-- `IndexRangeScan` with no directive), on top of this run's wrong rows.
-- @setup
CREATE TABLE gin_ord_j (id int4, j jsonb NOT NULL);
INSERT INTO gin_ord_j (id, j) VALUES
  (1, '{"z":1,"a":2}'),
  (2, '{"m":3}'),
  (3, '{"b":1,"c":2,"d":3}');
CREATE INDEX gin_ord_j_idx ON gin_ord_j USING gin(j);
-- @test
SELECT id, j FROM gin_ord_j ORDER BY j;
