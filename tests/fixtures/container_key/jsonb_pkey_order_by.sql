-- A `jsonb` primary key orders as 18.6's `jsonb_ops` does, whichever of an index-ordered
-- scan and a sort answers: `[]` below every top-level scalar, then type rank (null,
-- string, number, boolean, array, object), an array by length before its elements, an
-- object by pair count before its pairs. The rows are inserted out of order and the key
-- column is the table's second, so neither the heap order nor a column position can
-- stand in for the key's.
--
-- The strings are lower-case ASCII only: their order is the same under the oracle's
-- collation and under pglet's byte order (`EDGES.md` row 43), so the fixture pins the
-- `jsonb` order and not the collation.
-- @profile-lock
-- @setup
CREATE TABLE cko_j (v INT, j JSONB PRIMARY KEY);
INSERT INTO cko_j (j, v) VALUES
  ('{"b":2,"aa":1}', 1), ('[]', 2), ('{"a":1,"b":1}', 3), ('null', 4), ('"z"', 5),
  ('""', 6), ('{"b":1}', 7), ('"a"', 8), ('[[1,2],3]', 9), ('-1', 10), ('{"aa":1}', 11),
  ('0', 12), ('[[1],3]', 13), ('100', 14), ('{"a":1}', 15), ('false', 16), ('[0,0]', 17),
  ('true', 18), ('{"a":null}', 19), ('[null]', 20), ('[{}]', 21), ('["a"]', 22), ('{}', 23),
  ('[1]', 24), ('[[]]', 25), ('[true]', 26), ('{"b":1,"aa":2}', 27);
-- @test
SELECT v, j::text AS key FROM cko_j ORDER BY j;
