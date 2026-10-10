-- A `float8[]` primary key orders its elements as `float8` does: `-Infinity`, the
-- finite values, `Infinity`, then `NaN`.
-- @profile-lock
-- @setup
CREATE TABLE cko_f (id INT, a FLOAT8[] PRIMARY KEY);
INSERT INTO cko_f (a, id) VALUES ('{0}', 1), ('{NaN}', 2), ('{-Infinity}', 3), ('{1.5}', 4), ('{Infinity}', 5), ('{-1.5,2}', 6), ('{1.5,-1}', 7);
-- @test
SELECT id, a FROM cko_f ORDER BY a;
