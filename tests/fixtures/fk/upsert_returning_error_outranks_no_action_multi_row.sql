-- A later row's `RETURNING` error outranks the `NO ACTION` refusal an earlier row's rewrite
-- earns: every row's `RETURNING` is projected before any action runs.
-- @profile-lock
-- @setup
CREATE TABLE urenm_p (id INT PRIMARY KEY, v INT);
CREATE TABLE urenm_c (pid INT REFERENCES urenm_p(id), x INT);
INSERT INTO urenm_p VALUES (1, 1), (2, 2);
INSERT INTO urenm_c VALUES (1, 10), (2, 20);

-- @test
-- @expect-error: row 2's RETURNING divides by zero before the NO ACTION check runs (22012)
INSERT INTO urenm_p VALUES (1, 5), (2, 6) ON CONFLICT (id) DO UPDATE SET id = urenm_p.id + 10
RETURNING 10 / (urenm_p.id - 12);
