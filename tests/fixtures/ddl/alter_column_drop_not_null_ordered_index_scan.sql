-- An ordered scan over an index is planned on a column declared `NOT NULL`; once the
-- declaration is dropped and a NULL written, the scan must not be the one that leaves it
-- out. The NULL sorts last, ascending.
-- @profile-lock
-- @setup
CREATE TABLE dno (v int NOT NULL);
CREATE INDEX dno_v ON dno (v);
INSERT INTO dno VALUES (2), (1);
ALTER TABLE dno ALTER COLUMN v DROP NOT NULL;
INSERT INTO dno VALUES (NULL);
-- @test
SELECT v FROM dno ORDER BY v;
