-- `u = NULL` matches no row, over a `NULLS NOT DISTINCT` unique index too: the index holds
-- the NULL-keyed row under a NULL key, so the equality probe must not ask it for one.
-- @profile-lock
-- @setup
CREATE TABLE nk (id INT, u INT);
CREATE UNIQUE INDEX nk_u ON nk (u) NULLS NOT DISTINCT;
INSERT INTO nk VALUES (1, NULL), (2, 5);
-- @test
SELECT id FROM nk WHERE u = NULL;
