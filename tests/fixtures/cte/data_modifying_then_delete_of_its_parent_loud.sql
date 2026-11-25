-- A foreign key between the two targets is checked at the statement's end on 18.6, which
-- refuses deleting a parent whose last child the `WITH` deleted; refused here (`0A000`).
-- @setup
CREATE TABLE dmp (id INT PRIMARY KEY);
CREATE TABLE dmp_c (pid INT REFERENCES dmp (id));
INSERT INTO dmp VALUES (1);
INSERT INTO dmp_c VALUES (1);
-- @no-profile-lock: refused while the statement is parsed, before any index is chosen
-- @expect-error: a writing body tied to the WITH's target by a foreign key
-- @test
WITH c AS (DELETE FROM dmp_c RETURNING pid) DELETE FROM dmp WHERE id IN (SELECT pid FROM c);
