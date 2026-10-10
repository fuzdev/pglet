-- An `ON CONFLICT DO UPDATE` that moves a referenced key cascades it into an
-- `ON UPDATE CASCADE` child: the child takes the rewritten key, not the old one.
-- @profile-lock
-- @setup
CREATE TABLE upca_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upca_c (pid INT REFERENCES upca_p(id) ON UPDATE CASCADE, x INT);
INSERT INTO upca_p VALUES (1, 1), (2, 2);
INSERT INTO upca_c VALUES (1, 10), (2, 20);
INSERT INTO upca_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = 9;

-- @test
SELECT pid, x FROM upca_c ORDER BY x;
