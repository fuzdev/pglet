-- The swap idiom run from a `DO` body, which reaches the same executor.
-- @setup
CREATE TABLE sodb (a INT, b INT);
INSERT INTO sodb VALUES (1, 2), (3, 4);
DO $$ BEGIN UPDATE sodb SET a = b, b = a; END $$;

-- @test
SELECT a, b FROM sodb ORDER BY a;
