-- A rewrite of one column of a composite referenced key cascades that column; a child
-- holding a NULL in its key (MATCH SIMPLE) references nothing and is untouched.
-- @profile-lock
-- @setup
CREATE TABLE upcc_p (a INT, b INT, v INT, PRIMARY KEY (a, b));
CREATE TABLE upcc_c (ca INT, cb INT, x INT, FOREIGN KEY (ca, cb) REFERENCES upcc_p (a, b) ON UPDATE CASCADE);
INSERT INTO upcc_p VALUES (1, 1, 1), (2, 2, 2);
INSERT INTO upcc_c VALUES (1, 1, 10), (1, NULL, 11), (2, 2, 20);
INSERT INTO upcc_p VALUES (1, 1, 5) ON CONFLICT (a, b) DO UPDATE SET b = 9;

-- @test
SELECT ca, cb, x FROM upcc_c ORDER BY x;
