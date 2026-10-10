-- `RESTRICT` takes no substitute: a row the same `INSERT` inserts under the vacated key
-- does not stop the refusal.
-- @profile-lock
-- @setup
CREATE TABLE uprn_p (id INT PRIMARY KEY, v INT);
CREATE TABLE uprn_c (pid INT REFERENCES uprn_p(id) ON UPDATE RESTRICT, x INT);
INSERT INTO uprn_p VALUES (1, 1), (2, 2);
INSERT INTO uprn_c VALUES (1, 10);

-- @test
-- @expect-error: RESTRICT refuses even though the second row re-inserts key 1 (23001)
INSERT INTO uprn_p VALUES (1, 5), (1, 6) ON CONFLICT (id) DO UPDATE SET id = 9;
