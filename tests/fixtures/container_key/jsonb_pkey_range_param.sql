-- A range over a `jsonb` primary key, both bounds bound: the index's byte order and the
-- comparator's must agree on which keys lie between a three-element array and the empty
-- object — arrays by length first, so `[0,0]` is below `[[1],3]` and every array is below
-- `{}`.
-- @profile-lock
-- @setup
CREATE TABLE ckr_j (v INT, j JSONB PRIMARY KEY);
INSERT INTO ckr_j (j, v) VALUES
  ('{"b":2,"aa":1}', 1), ('[]', 2), ('{"a":1,"b":1}', 3), ('null', 4), ('"z"', 5),
  ('""', 6), ('{"b":1}', 7), ('"a"', 8), ('[[1,2],3]', 9), ('-1', 10), ('{"aa":1}', 11),
  ('0', 12), ('[[1],3]', 13), ('100', 14), ('{"a":1}', 15), ('false', 16), ('[0,0]', 17),
  ('true', 18), ('{"a":null}', 19), ('[null]', 20), ('[{}]', 21), ('["a"]', 22), ('{}', 23),
  ('[1]', 24), ('[[]]', 25), ('[true]', 26), ('{"b":1,"aa":2}', 27);
-- @param $1 jsonb [true]
-- @param $2 jsonb {}
-- @test
SELECT v, j::text AS key FROM ckr_j WHERE j > $1 AND j <= $2 ORDER BY j;
