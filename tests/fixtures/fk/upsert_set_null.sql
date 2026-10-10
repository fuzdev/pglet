-- An `ON CONFLICT DO UPDATE` that moves a referenced key nulls an `ON UPDATE SET NULL`
-- child's column.
-- @profile-lock
-- @setup
CREATE TABLE upsn_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upsn_c (pid INT REFERENCES upsn_p(id) ON UPDATE SET NULL, x INT);
INSERT INTO upsn_p VALUES (1, 1), (2, 2);
INSERT INTO upsn_c VALUES (1, 10), (2, 20);
INSERT INTO upsn_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9;

-- @test
SELECT pid, x FROM upsn_c ORDER BY x;
