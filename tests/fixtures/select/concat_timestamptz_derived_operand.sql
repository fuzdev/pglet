-- The `+00` offset through the two nodes whose declared type has to be **derived** from
-- an operand's rather than read off a leaf: a scalar subquery, and an arithmetic node
-- over one. `timestamptz` and `timestamp` share `Value::Timestamp` and differ only by the
-- offset in their text, so `||` and `::text` read the *declared* type; an operand whose
-- type the walk answers as the plain `timestamp` (or as nothing) silently drops the `+00`
-- and hands a driver an instant it reparses in its own zone.
--
-- The two arithmetic lines are the discriminating ones: `tz + interval` is `timestamptz`
-- only because `arith_result_type` says so over the operand types the eval walk carried
-- up, so a node that hands up nothing reddens here and nowhere else.
-- @setup
CREATE TABLE tzq (ts TIMESTAMPTZ);
INSERT INTO tzq VALUES ('2024-06-15 12:30:45.123+00');

-- @test
SELECT (SELECT max(ts) FROM tzq)::text AS cast_of_subquery,
       'v=' || (SELECT max(ts) FROM tzq) AS concat_of_subquery,
       ((SELECT max(ts) FROM tzq) + INTERVAL '1 hour')::text AS cast_of_subquery_arith,
       'v=' || ((SELECT max(ts) FROM tzq) + INTERVAL '1 hour') AS concat_of_subquery_arith,
       (ts + INTERVAL '1 hour')::text AS cast_of_column_arith,
       'v=' || (ts + INTERVAL '1 hour') AS concat_of_column_arith
FROM tzq
