-- The sharpest face of the GIN-serves-an-ordered-scan bug: `ORDER BY … LIMIT 1`
-- answered a single, entirely ordinary-looking, wrong row (`1` for Postgres's `2`)
-- — no duplicate, no visible count anomaly, nothing for a client to notice.
--
-- Companion to `gin_method_order_by_jsonb.sql`, which pins the full ordering; this
-- one pins the reply a `LIMIT` client actually reads. Neither subsumes the other:
-- a build that fixed only the `LIMIT` early-stop would pass this and fail that, and
-- a fixture that only read the full order would leave the one-row reply unasserted.
-- Not `-- @profile-lock` — same reasoning as the companion.
-- @setup
CREATE TABLE gin_ord_jl (id int4, j jsonb NOT NULL);
INSERT INTO gin_ord_jl (id, j) VALUES
  (1, '{"z":1,"a":2}'),
  (2, '{"m":3}'),
  (3, '{"b":1,"c":2,"d":3}');
CREATE INDEX gin_ord_jl_idx ON gin_ord_jl USING gin(j);
-- @test
SELECT id, j FROM gin_ord_jl ORDER BY j LIMIT 1;
