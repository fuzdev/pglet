-- `GREATEST` in a stored expression was an **accept-then-brick**: the CHECK was
-- recorded, then `INSERT (NULL, 1)` succeeded (the NULL argument was dropped) while
-- `INSERT ('a', 1)` failed `42804` — and kept failing forever, for every row supplying
-- both columns. Postgres rejects the constraint at `CREATE TABLE`.
-- @test
-- @expect-error: GREATEST branches have no common type (42804)
CREATE TABLE ckg (t TEXT, i INT, CHECK (GREATEST(t, i) IS NOT NULL));
