-- A `RETURNING` error outranks the `NO ACTION` refusal of the key the upsert vacates: the
-- action runs after `RETURNING` is projected. The divisor reads the updated key, so neither
-- engine can fold it at plan time. A constant `1 / 0` would not do: 18.6 folds it and raises
-- `22012` before the statement runs, whatever order the two errors are raised in.
-- @profile-lock
-- @setup
CREATE TABLE urena_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urena_c (pid INT REFERENCES urena_p(id), x INT);
INSERT INTO urena_p VALUES (1, 1), (2, 2);
INSERT INTO urena_c VALUES (1, 10), (2, 20);

-- @test
-- @expect-error: RETURNING divides by zero before the NO ACTION check runs (22012)
INSERT INTO urena_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING 1 / (urena_p.id - 9);
