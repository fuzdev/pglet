-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A foreign key created inside the block references the committed parent row through a
-- unique index the same block built: the referencing row lands on both engines, where
-- pglet used to refuse it 23503 and so abort the whole setup.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_fkc;
DROP TABLE IF EXISTS ibi_fkp;
CREATE TABLE ibi_fkp (a INT);
INSERT INTO ibi_fkp VALUES (1);
BEGIN;
CREATE UNIQUE INDEX ibi_fkp_u ON ibi_fkp (a);
CREATE TABLE ibi_fkc (a INT REFERENCES ibi_fkp (a));
INSERT INTO ibi_fkc VALUES (1);
COMMIT;

-- @test
SELECT a FROM ibi_fkc;
