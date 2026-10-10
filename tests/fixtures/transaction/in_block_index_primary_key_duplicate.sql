-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A PRIMARY KEY added inside a block builds its index over the committed row, and the
-- block's duplicate INSERT is 23505 on both engines.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_pk;
CREATE TABLE ibi_pk (a INT);
INSERT INTO ibi_pk VALUES (1);
BEGIN;
ALTER TABLE ibi_pk ADD PRIMARY KEY (a);

-- @expect-error: the committed row holds the key
-- @test
INSERT INTO ibi_pk VALUES (1);
