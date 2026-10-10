-- `bigint[]` keys either side of 2^53, where a `float8` cannot tell neighbours apart:
-- each is its own key, and they order as integers.
-- @profile-lock
-- @setup
CREATE TABLE ckb_a (id INT, a INT8[] NOT NULL UNIQUE);
INSERT INTO ckb_a (a, id) VALUES ('{9007199254740993}', 1), ('{9007199254740992}', 2), ('{9007199254740994}', 3), ('{-9007199254740993}', 4), ('{-9007199254740992}', 5);
-- @test
SELECT id, a FROM ckb_a ORDER BY a;
