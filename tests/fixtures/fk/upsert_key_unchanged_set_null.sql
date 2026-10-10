-- An `ON CONFLICT DO UPDATE` that assigns a referenced column its own value changes no key,
-- so an `ON UPDATE SET NULL` child keeps its reference.
-- @profile-lock
-- @setup
CREATE TABLE upkn_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upkn_c (pid INT REFERENCES upkn_p(id) ON UPDATE SET NULL, x INT);
INSERT INTO upkn_p VALUES (1, 1), (2, 2);
INSERT INTO upkn_c VALUES (1, 10), (2, 20);
INSERT INTO upkn_p VALUES (1, 5) ON CONFLICT (id) DO UPDATE SET id = upkn_p.id, v = 7;

-- @test
SELECT pid, x FROM upkn_c ORDER BY x;
