-- A statement reading the data-modifying `WITH` through a read-only one reads it too.
-- @setup
CREATE TABLE dmy (id INT, v INT);
CREATE TABLE dmy_d (tid INT);
-- @test
WITH c AS (INSERT INTO dmy VALUES (10, 1) RETURNING id), r AS (SELECT id FROM c)
INSERT INTO dmy_d (tid) SELECT id FROM r RETURNING tid;
