-- A `RETURNING` error outranks the `RESTRICT` refusal of the key the upsert vacates. The
-- divisor reads the updated key, so neither engine can fold it at plan time.
-- @profile-lock
-- @setup
CREATE TABLE urer_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urer_c (pid INT REFERENCES urer_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO urer_p VALUES (1, 1), (2, 2);
INSERT INTO urer_c VALUES (1, 10), (2, 20);

-- @test
-- @expect-error: RETURNING divides by zero before the RESTRICT check runs (22012)
INSERT INTO urer_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING 1 / (urer_p.id - 9);
