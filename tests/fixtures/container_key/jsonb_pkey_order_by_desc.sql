-- The descending walk of `jsonb_pkey_order_by.sql`'s key, under a `LIMIT` — the shape
-- an index-ordered scan serves without reading the whole table.
-- @profile-lock
-- @setup
CREATE TABLE ckd_j (v INT, j JSONB PRIMARY KEY);
INSERT INTO ckd_j (j, v) VALUES
  ('{"b":2,"aa":1}', 1), ('[]', 2), ('{"a":1,"b":1}', 3), ('null', 4), ('"z"', 5),
  ('""', 6), ('{"b":1}', 7), ('"a"', 8), ('[[1,2],3]', 9), ('-1', 10), ('{"aa":1}', 11),
  ('0', 12), ('[[1],3]', 13), ('100', 14), ('{"a":1}', 15), ('false', 16), ('[0,0]', 17),
  ('true', 18), ('{"a":null}', 19), ('[null]', 20), ('[{}]', 21), ('["a"]', 22), ('{}', 23),
  ('[1]', 24), ('[[]]', 25), ('[true]', 26), ('{"b":1,"aa":2}', 27);
-- @test
SELECT v, j::text AS key FROM ckd_j ORDER BY j DESC LIMIT 9;
