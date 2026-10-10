-- An uncast containment literal resolves against **any** typed container operand,
-- not just a bare column.
--
-- Postgres resolves an `unknown` literal from the operator signature (`jsonb @>
-- jsonb`), so it does not care what shape the other side is — a cast, a `->` or `#>`
-- result, and a `jsonb_build_object(…)` call all anchor the literal equally. pglet's
-- plan-time coercion first covered only `Expr::Column`, so every form here raised
-- `42883` ("operator does not exist: jsonb @> text") while the identical query
-- against a column worked. The cast form is the sharpest case: the documented
-- workaround was to cast the *literal*, and its mirror — cast the other side, leave
-- the literal bare — still failed.
--
-- The anchor is decided from the `ColumnScope` plus the node itself, so a scalar
-- **subquery** operand (`(SELECT d FROM t) @> '{…}'`) is still unresolved and fails
-- loud; it would need the sub-plan's output type, which that decision does not read.
-- @setup
CREATE TABLE jceo (id INT, d JSONB);
INSERT INTO jceo VALUES (1, '{"x":{"a":1},"tags":["p","q"]}');
-- @test
SELECT
  ('{"a":1}'::jsonb @> '{"a":1}') AS cast_left,
  ('{"a":1}' <@ '{"a":1,"b":2}'::jsonb) AS cast_right,
  ((d -> 'x') @> '{"a":1}') AS arrow_left,
  ((d #> '{tags}') @> '["p"]') AS path_left,
  (jsonb_build_object('k','v') @> '{"k":"v"}') AS build_object_left,
  (d @> '{"tags":["q"]}') AS column_left
FROM jceo;
