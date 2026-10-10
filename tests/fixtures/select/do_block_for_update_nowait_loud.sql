-- A `DO $$ … $$` body is parsed by a **sub-parser**, and a deferred refusal nobody
-- adopts is an accept: this body's `NOWAIT` was recorded on the sub-parser and dropped
-- with it, so the block reported `DO` and ran. 18.6 accepts `NOWAIT`, so this is the
-- pglet-stricter shape — any pglet error passes.
-- @expect-error: a DO body reaches the wait-policy refusal too
-- @setup
CREATE TABLE ldo (a INT);
CREATE TABLE ldos (a INT);
INSERT INTO ldo VALUES (5);
INSERT INTO ldos VALUES (1), (2);
-- @test
DO $$ BEGIN UPDATE ldo SET a = 1 WHERE a IN (SELECT a FROM ldos FOR UPDATE NOWAIT); END $$;
