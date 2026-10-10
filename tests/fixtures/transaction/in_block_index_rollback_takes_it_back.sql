-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- The block's ROLLBACK takes back the unique index it built and read through: after it the
-- committed key is free again, and the table holds no index, on both engines.
-- @setup
DROP TABLE IF EXISTS ibi_rb;
CREATE TABLE ibi_rb (a INT);
INSERT INTO ibi_rb VALUES (1);
BEGIN;
CREATE UNIQUE INDEX ibi_rb_u ON ibi_rb (a);
INSERT INTO ibi_rb VALUES (2);
ROLLBACK;
INSERT INTO ibi_rb VALUES (1);

-- @test
SELECT
  (SELECT count(*) FROM ibi_rb WHERE a = 1) AS ones,
  (SELECT count(*) FROM pg_indexes WHERE tablename = 'ibi_rb') AS indexes;
