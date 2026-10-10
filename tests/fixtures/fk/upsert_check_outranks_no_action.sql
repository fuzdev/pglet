-- The referential check runs at the end of the statement, so a later row's own CHECK
-- failure is what the statement reports, not the earlier row's orphaned child.
-- @profile-lock
-- @setup
CREATE TABLE upck_p (id INT PRIMARY KEY, v INT CHECK (v < 100));
CREATE TABLE upck_c (pid INT REFERENCES upck_p(id), x INT);
INSERT INTO upck_p VALUES (1, 1), (2, 2);
INSERT INTO upck_c VALUES (1, 10);

-- @test
-- @expect-error: row 2's CHECK (23514) outranks row 1's vacated key (23503)
INSERT INTO upck_p VALUES (1, 5), (2, 6) ON CONFLICT (id) DO UPDATE SET v = CASE WHEN upck_p.id = 2 THEN 200 ELSE upck_p.v END, id = upck_p.id + 10;
