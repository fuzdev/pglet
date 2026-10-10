-- The array half of the GIN-serves-an-ordered-scan bug. `TypeOid::is_container` is
-- `Jsonb ∪ is_array()`, so both families reach the same selector — and both were
-- wrong the same way (`1,3,3,3,2,1` against Postgres's `3,2,1`), one decomposed
-- entry per element instead of one per row.
--
-- Not a duplicate of the jsonb fixture: the two types decompose through different
-- arms and *sort* by different rules (array element-wise vs jsonb's type/key
-- ordering), so Postgres's correct answers differ (`3,2,1` here, `2,1,3` there) and
-- neither fixture's expectation can be derived from the other's.
-- Not `-- @profile-lock` — see `gin_method_order_by_jsonb.sql`.
-- @setup
CREATE TABLE gin_ord_a (id int4, t text[] NOT NULL);
INSERT INTO gin_ord_a (id, t) VALUES
  (1, '{z,a}'),
  (2, '{m}'),
  (3, '{b,c,d}');
CREATE INDEX gin_ord_a_idx ON gin_ord_a USING gin(t);
-- @test
SELECT id, t FROM gin_ord_a ORDER BY t;
