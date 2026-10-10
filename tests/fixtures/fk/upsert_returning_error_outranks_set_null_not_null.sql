-- A `RETURNING` error outranks the `23502` an `ON UPDATE SET NULL` earns against a
-- `NOT NULL` child column. The divisor reads the updated key, so neither engine can fold it
-- at plan time.
-- @profile-lock
-- @setup
CREATE TABLE urenn_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urenn_c (pid INT NOT NULL REFERENCES urenn_p(id) ON UPDATE SET NULL, x INT);
INSERT INTO urenn_p VALUES (1, 1), (2, 2);
INSERT INTO urenn_c VALUES (1, 10), (2, 20);

-- @test
-- @expect-error: RETURNING divides by zero before the SET NULL action runs (22012)
INSERT INTO urenn_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9 RETURNING 1 / (urenn_p.id - 9);
