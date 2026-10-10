-- `NO ACTION` is checked at the end of the statement against every row it wrote, so a row
-- the same `INSERT` goes on to insert under the vacated key keeps the child valid.
-- @profile-lock
-- @setup
CREATE TABLE upni_p (id INT PRIMARY KEY, v INT);
CREATE TABLE upni_c (pid INT REFERENCES upni_p(id), x INT);
INSERT INTO upni_p VALUES (1, 1), (2, 2);
INSERT INTO upni_c VALUES (1, 10);
INSERT INTO upni_p VALUES (1, 5), (1, 6) ON CONFLICT (id) DO UPDATE SET id = 9;

-- @test
SELECT id, v FROM upni_p ORDER BY id;
